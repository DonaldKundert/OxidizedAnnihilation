//! 3DO models as render meshes, with their textures packed into one atlas
//! per (model, player).
//!
//! - Textures: a primitive names a GAF entry. Entries are looked up in every
//!   `textures/*.gaf` (first file in path order wins). Each GAF is decoded
//!   with `palettes/<first three letters of the GAF name>_textures.pcx`
//!   (**Believed**, `docs/formats/gaf.md`), falling back to
//!   `palettes/palette.pal`. Index 5 and the frame's colour key are
//!   transparent (index 5 **Believed** cutout).
//! - Team colour: entries with several frames are taken as per-player
//!   variants and frame `player % frames` is used (**Believed**, TA's
//!   convention for 10-frame "logo" textures).
//! - UVs: there are none in the file. A quad's four indices take the
//!   texture's corners in the TA order (0,0) (1,0) (1,1) (0,1); a textured
//!   triangle takes the first three of them (**Chosen**, see
//!   `docs/client.md`).
//! - Untextured faces take their colour index from the model side's texture
//!   palette. 2-index primitives (emit segments) and the root's selection
//!   primitive (the ground plate) are not drawn.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use tak_formats::gaf::palette::Palette;
use tak_formats::gaf::pcx::Pcx;
use tak_formats::gaf::{FrameContent, Gaf};
use tak_formats::model3do::{self, FIXED_ONE, Model};
use tak_vfs::Vfs;

/// Index 5 is the cutout colour in model textures (**Believed**).
pub const CUTOUT_INDEX: u8 = 5;

/// Every model texture in the install, by lowercase entry name.
///
/// Search order (**Chosen**; the engine's is Unknown, `3do.md`): every
/// `textures/*.gaf` first (path order), then every `anims/*.gaf`. The
/// `anims/` files are only scanned for entry names at load (a cheap header
/// walk) and parsed on first use. Names that resolve nowhere fall back to
/// the nearest texture sharing a name stem of at least six letters
/// (`gryphwingl1` → `gryphwingL2`, **Chosen**), else to a neutral colour.
pub struct TextureIndex {
    gafs: Vec<(String, Gaf)>,
    by_name: BTreeMap<String, (usize, usize)>,
    palettes: BTreeMap<String, Palette>,
    /// `anims/` entry names not found in `textures/`, to their file.
    anim_names: BTreeMap<String, String>,
    /// Lazily parsed `anims/` GAFs (and the VFS bytes they come from).
    anim_gafs: RefCell<BTreeMap<String, Option<Arc<(Gaf, Palette)>>>>,
    vfs_anims: BTreeMap<String, Vec<u8>>,
}

/// Colour for texture names that resolve nowhere and have no stem match
/// (**Chosen**: a muted brown, closer to the hide and wood these corpse
/// parts show than TA's flat colour 209).
pub const MISSING_RGB: [u8; 3] = [104, 88, 70];

/// Entry names of a GAF from its header alone (no frame decoding). Never
/// panics: a truncated or odd file gives the names read so far.
pub fn gaf_entry_names(data: &[u8]) -> Vec<String> {
    let u32_at = |o: usize| -> Option<u32> {
        let b = data.get(o..o.checked_add(4)?)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let Some(count) = u32_at(4) else {
        return Vec::new();
    };
    let count = (count as usize).min(data.len() / 4);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let Some(eo) = u32_at(12 + i * 4) else { break };
        let start = eo as usize + 8;
        let Some(raw) = data.get(start..start.saturating_add(32)) else {
            continue;
        };
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        out.push(String::from_utf8_lossy(&raw[..end]).into_owned());
    }
    out
}

/// Load a palette from a `.pal` or a PCX trailer.
pub fn load_palette(vfs: &Vfs, path: &str) -> Option<Palette> {
    let data = vfs.read(path).ok()?;
    if data.len() == Palette::PAL_LEN {
        return Palette::from_pal(&data).ok();
    }
    Pcx::parse(&data).ok()?.palette
}

fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}

impl TextureIndex {
    /// Parse every `textures/*.gaf`.
    pub fn load(vfs: &Vfs) -> TextureIndex {
        let mut files: Vec<String> = vfs
            .list("textures/")
            .into_iter()
            .map(|f| f.path)
            .filter(|p| p.ends_with(".gaf"))
            .collect();
        files.sort();
        let mut gafs = Vec::new();
        let mut by_name = BTreeMap::new();
        for path in files {
            let Ok(bytes) = vfs.read(&path) else { continue };
            let Ok(gaf) = Gaf::parse(&bytes) else {
                continue;
            };
            let gi = gafs.len();
            for (ei, e) in gaf.entries.iter().enumerate() {
                by_name
                    .entry(e.name.to_ascii_lowercase())
                    .or_insert((gi, ei));
            }
            gafs.push((path, gaf));
        }
        let mut palettes = BTreeMap::new();
        let fallback = load_palette(vfs, "palettes/palette.pal").unwrap_or_else(Palette::grayscale);
        for (path, _) in &gafs {
            let side = side_of(file_stem(path));
            palettes.entry(side.clone()).or_insert_with(|| {
                load_palette(vfs, &format!("palettes/{side}_textures.pcx"))
                    .unwrap_or_else(|| fallback.clone())
            });
        }
        palettes.insert(String::new(), fallback);
        let mut anim_names = BTreeMap::new();
        let mut vfs_anims = BTreeMap::new();
        let mut anim_files: Vec<String> = vfs
            .list("anims/")
            .into_iter()
            .map(|f| f.path)
            .filter(|p| p.ends_with(".gaf"))
            .collect();
        anim_files.sort();
        for path in anim_files {
            let Ok(bytes) = vfs.read(&path) else { continue };
            let mut used = false;
            for n in gaf_entry_names(&bytes) {
                let key = n.to_ascii_lowercase();
                if !by_name.contains_key(&key) && !anim_names.contains_key(&key) {
                    anim_names.insert(key, path.clone());
                    used = true;
                }
            }
            if used {
                vfs_anims.insert(path, bytes);
            }
        }
        // Palettes the anims files may need.
        for side in ["mon", "npc"] {
            palettes.entry(side.to_string()).or_insert_with(|| {
                load_palette(vfs, &format!("palettes/{side}_textures.pcx"))
                    .unwrap_or_else(|| palettes_fallback(vfs))
            });
        }
        let mut sibling = BTreeMap::new();
        for path in vfs_anims.keys() {
            if let Some(p) = load_palette(vfs, &path.replace(".gaf", ".pcx")) {
                sibling.insert(path.clone(), p);
            }
        }
        let mut idx = TextureIndex {
            gafs,
            by_name,
            palettes,
            anim_names,
            anim_gafs: RefCell::new(BTreeMap::new()),
            vfs_anims,
        };
        for (path, p) in sibling {
            idx.palettes.insert(format!("file:{path}"), p);
        }
        idx
    }

    /// Palette for an `anims/` GAF used as a model texture: its sibling
    /// PCX when present, else `<side>_textures.pcx` by the first three
    /// letters, with `*monst*` files on `mon_textures` (**Chosen**), else
    /// the fallback.
    fn anim_palette(&self, path: &str) -> Palette {
        if let Some(p) = self.palettes.get(&format!("file:{path}")) {
            return p.clone();
        }
        let stem = file_stem(path).to_ascii_lowercase();
        let side = side_of(&stem);
        if let Some(p) = self.palettes.get(&side) {
            return p.clone();
        }
        if stem.contains("monst") {
            return self.palette("mon").clone();
        }
        self.palette("").clone()
    }

    fn anim_gaf(&self, path: &str) -> Option<Arc<(Gaf, Palette)>> {
        let mut cache = self.anim_gafs.borrow_mut();
        cache
            .entry(path.to_string())
            .or_insert_with(|| {
                let bytes = self.vfs_anims.get(path)?;
                let gaf = Gaf::parse(bytes).ok()?;
                Some(Arc::new((gaf, self.anim_palette(path))))
            })
            .clone()
    }

    /// Where a texture name resolves: `"textures/x.gaf"`, `"anims/y.gaf"`,
    /// or `None`.
    pub fn source_of(&self, name: &str) -> Option<&str> {
        let key = name.to_ascii_lowercase();
        if let Some(&(gi, _)) = self.by_name.get(&key) {
            return Some(&self.gafs[gi].0);
        }
        self.anim_names.get(&key).map(String::as_str)
    }

    /// The nearest known name sharing a stem of at least six letters with
    /// `name` (trailing digits dropped first), for names that resolve
    /// nowhere.
    pub fn stem_fallback(&self, name: &str) -> Option<String> {
        let key = name.to_ascii_lowercase();
        let mut stem = key.trim_end_matches(|c: char| c.is_ascii_digit());
        while stem.len() >= 6 {
            let hit = self
                .by_name
                .range(stem.to_string()..)
                .next()
                .map(|(k, _)| k)
                .filter(|k| k.starts_with(stem))
                .or_else(|| {
                    self.anim_names
                        .range(stem.to_string()..)
                        .next()
                        .map(|(k, _)| k)
                        .filter(|k| k.starts_with(stem))
                });
            if let Some(h) = hit {
                return Some(h.clone());
            }
            stem = &stem[..stem.len() - 1];
        }
        None
    }

    /// Number of distinct entry names.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Number of extra names found only in `anims/`.
    pub fn anim_len(&self) -> usize {
        self.anim_names.len()
    }

    /// `true` if no textures were found.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// The palette for a side prefix (`ara`, ...), or the fallback.
    pub fn palette(&self, side: &str) -> &Palette {
        self.palettes
            .get(side)
            .or_else(|| self.palettes.get(""))
            .expect("fallback palette is always present")
    }

    /// Decode texture `name` for `player` as RGBA8 `(w, h, pixels)`.
    pub fn decode(&self, name: &str, player: usize) -> Option<(u32, u32, Vec<u8>)> {
        let key = name.to_ascii_lowercase();
        if let Some(&(gi, ei)) = self.by_name.get(&key) {
            let (path, gaf) = &self.gafs[gi];
            let entry = gaf.entries.get(ei)?;
            return decode_entry(entry, player, self.palette(&side_of(file_stem(path))));
        }
        let path = self.anim_names.get(&key)?;
        let g = self.anim_gaf(path)?;
        let entry = g.0.entry(&key)?;
        decode_entry(entry, player, &g.1)
    }
}

fn palettes_fallback(vfs: &Vfs) -> Palette {
    load_palette(vfs, "palettes/palette.pal").unwrap_or_else(Palette::grayscale)
}

/// Decode one entry's frame for `player` (per-player variants), with index
/// 5 and the colour key transparent.
fn decode_entry(
    entry: &tak_formats::gaf::Entry,
    player: usize,
    pal: &Palette,
) -> Option<(u32, u32, Vec<u8>)> {
    if entry.frames.is_empty() {
        return None;
    }
    {
        let frame = &entry.frames[player % entry.frames.len()].frame;
        let (w, h) = (u32::from(frame.width), u32::from(frame.height));
        if w == 0 || h == 0 {
            return None;
        }
        let mut px = vec![0u8; (w * h * 4) as usize];
        match &frame.content {
            FrameContent::Indexed { index, opaque } => {
                for (i, (&ix, &op)) in index.iter().zip(opaque).enumerate() {
                    if op && ix != CUTOUT_INDEX {
                        let [r, g, b] = pal.rgb(ix);
                        px[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, 255]);
                    }
                }
            }
            _ => {
                let img = frame.to_rgba(pal);
                px = img.data;
            }
        }
        Some((w, h, px))
    }
}

/// The three-letter side prefix of a texture file or model name.
pub fn side_of(stem: &str) -> String {
    stem.chars()
        .take(3)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// A packed RGBA8 atlas.
pub struct Atlas {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8 pixels.
    pub rgba: Vec<u8>,
}

/// Where an image landed in the atlas: `(x, y, w, h)` in pixels.
pub type Rect = (u32, u32, u32, u32);

const PAD: u32 = 2;

/// Shelf-pack images (w, h, rgba) into an atlas of the given width, with
/// edge-extended padding so filtering does not bleed.
pub fn pack(images: &[(u32, u32, Vec<u8>)], width: u32) -> (Atlas, Vec<Rect>) {
    let width = images
        .iter()
        .map(|i| i.0 + 2 * PAD)
        .max()
        .unwrap_or(1)
        .max(width);
    let mut order: Vec<usize> = (0..images.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(images[i].1), i));
    let mut rects = vec![(0, 0, 0, 0); images.len()];
    let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
    for &i in &order {
        let (w, h) = (images[i].0 + 2 * PAD, images[i].1 + 2 * PAD);
        if x + w > width {
            x = 0;
            y += shelf;
            shelf = 0;
        }
        rects[i] = (x + PAD, y + PAD, images[i].0, images[i].1);
        x += w;
        shelf = shelf.max(h);
    }
    let height = (y + shelf).max(1).next_power_of_two();
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for (i, (w, h, px)) in images.iter().enumerate() {
        let (rx, ry, _, _) = rects[i];
        for dy in 0..h + 2 * PAD {
            for dx in 0..w + 2 * PAD {
                let sx = dx.saturating_sub(PAD).min(w - 1);
                let sy = dy.saturating_sub(PAD).min(h - 1);
                let s = ((sy * w + sx) * 4) as usize;
                let (ox, oy) = (rx - PAD + dx, ry - PAD + dy);
                let d = ((oy * width + ox) * 4) as usize;
                rgba[d..d + 4].copy_from_slice(&px[s..s + 4]);
            }
        }
    }
    (
        Atlas {
            width,
            height,
            rgba,
        },
        rects,
    )
}

/// Triangle mesh arrays for one piece, in the piece's local space.
#[derive(Default, Clone, Debug)]
pub struct MeshData {
    /// Positions.
    pub positions: Vec<[f32; 3]>,
    /// Flat normals.
    pub normals: Vec<[f32; 3]>,
    /// Atlas UVs.
    pub uvs: Vec<[f32; 2]>,
    /// Triangles.
    pub indices: Vec<u32>,
}

/// One piece of a built model.
#[derive(Clone, Debug)]
pub struct BuiltPiece {
    /// Name (as in the file).
    pub name: String,
    /// Parent piece index.
    pub parent: Option<usize>,
    /// Offset from the parent, model units.
    pub offset: [f32; 3],
    /// Geometry, if the piece has drawable faces.
    pub mesh: Option<MeshData>,
}

/// A model ready to upload.
pub struct BuiltModel {
    /// Pieces in file (pre-)order; parents come first.
    pub pieces: Vec<BuiltPiece>,
    /// Texture atlas.
    pub atlas: Atlas,
    /// Texture names that resolved nowhere and had no stem match (drawn
    /// in [`MISSING_RGB`]).
    pub missing: Vec<String>,
    /// Names drawn with a stem match instead: `(name, stand-in)`.
    pub stand_ins: Vec<(String, String)>,
}

/// Texture corners in TA order: a quad's indices 0..3 take these.
pub const QUAD_UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

fn fx(v: i32) -> f32 {
    v as f32 / FIXED_ONE as f32
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Skin {
    Texture(usize),
    Flat(u8),
    Missing,
}

/// Load and build `objects3d/<object>.3do` for `player`.
pub fn load_model(
    vfs: &Vfs,
    tex: &TextureIndex,
    object: &str,
    player: usize,
) -> Result<BuiltModel, String> {
    let path = format!("objects3d/{object}.3do");
    let bytes = vfs.read(&path).map_err(|e| format!("{path}: {e}"))?;
    let model = model3do::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    Ok(build_model(&model, tex, &side_of(object), player))
}

/// Build meshes and an atlas for a parsed model.
pub fn build_model(model: &Model, tex: &TextureIndex, side: &str, player: usize) -> BuiltModel {
    // Collect skins.
    let mut tex_names: Vec<String> = Vec::new();
    let mut images: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    let mut slot_of_name: BTreeMap<String, Skin> = BTreeMap::new();
    let mut flat_slot: BTreeMap<u8, usize> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut stand_ins = Vec::new();
    let pal = tex.palette(side);
    let mut skin_of = |prim: &model3do::Primitive| -> Skin {
        match &prim.texture {
            Some(name) => {
                let key = name.to_ascii_lowercase();
                if let Some(s) = slot_of_name.get(&key) {
                    return *s;
                }
                let found = tex.decode(name, player).or_else(|| {
                    let alt = tex.stem_fallback(name)?;
                    stand_ins.push((key.clone(), alt.clone()));
                    tex.decode(&alt, player)
                });
                let s = match found {
                    Some(img) => {
                        images.push(img);
                        tex_names.push(key.clone());
                        Skin::Texture(images.len() - 1)
                    }
                    None => {
                        missing.push(key.clone());
                        Skin::Missing
                    }
                };
                slot_of_name.insert(key, s);
                s
            }
            None => Skin::Flat((prim.color_index & 0xff) as u8),
        }
    };
    let mut skins: Vec<Vec<Skin>> = Vec::new();
    for p in &model.pieces {
        skins.push(p.primitives.iter().map(&mut skin_of).collect());
    }
    // Flat swatches go after the textures.
    for s in skins.iter().flatten() {
        let c = match s {
            Skin::Flat(c) => *c,
            Skin::Missing => 0,
            Skin::Texture(_) => continue,
        };
        let key = if matches!(s, Skin::Missing) { 255 } else { c };
        flat_slot.entry(key).or_insert_with(|| {
            let rgb = if matches!(s, Skin::Missing) {
                MISSING_RGB
            } else {
                pal.rgb(c)
            };
            images.push((4, 4, [rgb[0], rgb[1], rgb[2], 255].repeat(16)));
            images.len() - 1
        });
    }
    let (atlas, rects) = pack(&images, 512);
    let (aw, ah) = (atlas.width as f32, atlas.height as f32);
    let uv_in = |slot: usize, c: [f32; 2]| -> [f32; 2] {
        let (x, y, w, h) = rects[slot];
        let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);
        // Half-texel inset keeps samples inside the texture.
        [
            (x + 0.5 + c[0] * (w - 1.0)) / aw,
            (y + 0.5 + c[1] * (h - 1.0)) / ah,
        ]
    };

    let mut pieces = Vec::with_capacity(model.pieces.len());
    for (pi, p) in model.pieces.iter().enumerate() {
        let verts: Vec<[f32; 3]> = p.vertices.iter().map(|v| v.map(fx)).collect();
        let mut m = MeshData::default();
        for (ri, prim) in p.primitives.iter().enumerate() {
            if prim.indices.len() < 3 {
                continue;
            }
            if pi == 0 && usize::try_from(p.selection_primitive).ok() == Some(ri) {
                continue;
            }
            let idx: Vec<usize> = prim.indices.iter().map(|&i| usize::from(i)).collect();
            if idx.iter().any(|&i| i >= verts.len()) {
                continue;
            }
            let pts: Vec<[f32; 3]> = idx.iter().map(|&i| verts[i]).collect();
            let n = newell(&pts);
            let skin = skins[pi][ri];
            let base = m.positions.len() as u32;
            for (k, pt) in pts.iter().enumerate() {
                m.positions.push(*pt);
                m.normals.push(n);
                let uv = match skin {
                    Skin::Texture(s) => uv_in(s, QUAD_UV[k.min(3)]),
                    Skin::Flat(c) => uv_in(flat_slot[&c], [0.5, 0.5]),
                    Skin::Missing => uv_in(flat_slot[&255], [0.5, 0.5]),
                };
                m.uvs.push(uv);
            }
            for k in 1..pts.len() as u32 - 1 {
                m.indices.extend_from_slice(&[base, base + k, base + k + 1]);
            }
        }
        pieces.push(BuiltPiece {
            name: p.name.clone(),
            parent: p.parent.filter(|&q| q < pi),
            offset: p.offset.map(fx),
            mesh: (!m.indices.is_empty()).then_some(m),
        });
    }
    missing.sort();
    missing.dedup();
    let _ = tex_names;
    BuiltModel {
        pieces,
        atlas,
        missing,
        stand_ins,
    }
}

/// Polygon normal by Newell's method (unit length, or +Y if degenerate).
pub fn newell(p: &[[f32; 3]]) -> [f32; 3] {
    let mut n = [0.0f32; 3];
    for i in 0..p.len() {
        let a = p[i];
        let b = p[(i + 1) % p.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l < 1e-9 {
        [0.0, 1.0, 0.0]
    } else {
        [n[0] / l, n[1] / l, n[2] / l]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_places_without_overlap() {
        let imgs = vec![
            (8, 8, vec![255; 256]),
            (4, 4, vec![0; 64]),
            (16, 2, vec![9; 128]),
        ];
        let (atlas, rects) = pack(&imgs, 16);
        assert!(atlas.width >= 20);
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let sep = a.0 + a.2 + PAD <= b.0
                    || b.0 + b.2 + PAD <= a.0
                    || a.1 + a.3 + PAD <= b.1
                    || b.1 + b.3 + PAD <= a.1;
                assert!(sep, "{a:?} overlaps {b:?}");
            }
        }
        let (x, y, _, _) = rects[0];
        let i = ((y * atlas.width + x) * 4) as usize;
        assert_eq!(atlas.rgba[i], 255);
    }

    #[test]
    fn entry_names_from_header() {
        // Header: version, count 2, 0, two entry offsets; entries are
        // frame_count, flags, 0, 32-byte name (no frames).
        let mut b = Vec::new();
        for v in [0x0001_0100u32, 2, 0, 20, 60] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for name in ["Alpha", "beta2"] {
            b.extend_from_slice(&[0, 0, 1, 0, 0, 0, 0, 0]);
            let mut n = [0u8; 32];
            n[..name.len()].copy_from_slice(name.as_bytes());
            b.extend_from_slice(&n);
        }
        assert_eq!(gaf_entry_names(&b), vec!["Alpha", "beta2"]);
        // Truncated or garbage input never panics.
        for cut in 0..b.len() {
            let _ = gaf_entry_names(&b[..cut]);
        }
        let mut huge = b.clone();
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let _ = gaf_entry_names(&huge);
    }

    #[test]
    fn newell_of_ccw_square_points_up() {
        let n = newell(&[
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
        ]);
        assert!((n[1] - 1.0).abs() < 1e-6, "{n:?}");
    }
}
