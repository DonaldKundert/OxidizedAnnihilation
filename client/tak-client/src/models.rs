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

use std::collections::BTreeMap;

use tak_formats::gaf::palette::Palette;
use tak_formats::gaf::pcx::Pcx;
use tak_formats::gaf::{FrameContent, Gaf};
use tak_formats::model3do::{self, FIXED_ONE, Model};
use tak_vfs::Vfs;

/// Index 5 is the cutout colour in model textures (**Believed**).
pub const CUTOUT_INDEX: u8 = 5;

/// Every model texture in the install, by lowercase entry name.
pub struct TextureIndex {
    gafs: Vec<(String, Gaf)>,
    by_name: BTreeMap<String, (usize, usize)>,
    palettes: BTreeMap<String, Palette>,
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
        TextureIndex {
            gafs,
            by_name,
            palettes,
        }
    }

    /// Number of distinct entry names.
    pub fn len(&self) -> usize {
        self.by_name.len()
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
        let &(gi, ei) = self.by_name.get(&name.to_ascii_lowercase())?;
        let (path, gaf) = &self.gafs[gi];
        let entry = gaf.entries.get(ei)?;
        if entry.frames.is_empty() {
            return None;
        }
        let frame = &entry.frames[player % entry.frames.len()].frame;
        let pal = self.palette(&side_of(file_stem(path)));
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
    /// Texture names that resolved nowhere (drawn flat grey).
    pub missing: Vec<String>,
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
    let pal = tex.palette(side);
    let mut skin_of = |prim: &model3do::Primitive| -> Skin {
        match &prim.texture {
            Some(name) => {
                let key = name.to_ascii_lowercase();
                if let Some(s) = slot_of_name.get(&key) {
                    return *s;
                }
                let s = match tex.decode(name, player) {
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
                [128, 128, 128]
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
