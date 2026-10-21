//! Maps: TNT v0x4000 terrain (`.tnt`, also the editor pieces under
//! `sections/`), plus the CRT companion file in [`crt`]. Work package WP-E (M7).
//!
//! Layout (see `docs/formats/tnt.md` for confidence tags): a 52-byte header of
//! thirteen little-endian `u32`s, then blocks reached through absolute
//! pointers in any order:
//!
//! | block | size | meaning |
//! | --- | --- | --- |
//! | heights | `W*H` u8 | one height per attribute cell |
//! | feature grid | `W*H` u16 | feature index per cell, or a reserved code |
//! | feature table | `count * 132` | `u32 index` + 128-byte name |
//! | tile ids | `(W/2)*(H/2)` u32 | `terrain/<id as %08x>.jpg` per tile cell |
//! | tile columns | `(W/2)*(H/2)` u8 | column of the 32×32-px piece in that JPEG |
//! | tile rows | `(W/2)*(H/2)` u8 | row of the piece |
//! | small minimap | `8 + w*h` | `u32 w, u32 h`, 8-bit pixels |
//! | large minimap | `8 + w*h` | same; pointer 0 in sections |
//!
//! Bytes the header does not account for are kept in [`Tnt::unclaimed`].

pub mod crt;

use crate::reader::{ReadError, Reader};

/// Version word at offset 0 of every Kingdoms TNT (TA uses `0x2000`).
pub const TNT_MAGIC: u32 = 0x4000;
/// Size of the fixed header; the first block starts here in every file seen.
pub const HEADER_LEN: usize = 52;
/// Bytes per feature-table record.
pub const FEATURE_RECORD_LEN: usize = 132;
/// Side in source pixels of the JPEG piece one tile cell shows. A JPEG of
/// `w x h` pixels holds `(w/32) x (h/32)` pieces (512 px: 16, 1024 px: 32).
pub const PIECE_PX: u32 = 32;
/// Largest piece index seen in retail data (31, in 1024-px JPEGs).
pub const MAX_PIECE_INDEX: u8 = 31;

/// Feature-grid code: nothing on this cell.
pub const CELL_EMPTY: u16 = 0xFFFF;
/// Feature-grid code seen on paved ground (plazas, roads, castle floors).
pub const CELL_PAVED: u16 = 0xFFFB;
/// Feature-grid code seen on isolated cells at wall and canal corners.
pub const CELL_MARKER: u16 = 0xFFFC;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TntError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("not a Kingdoms TNT: version word {0:#x} (expected 0x4000)")]
    BadMagic(u32),
    #[error("bad map size {width} x {height}")]
    BadSize { width: u32, height: u32 },
    #[error("block {a} ({a_start}..{a_end}) overlaps block {b} ({b_start}..{b_end})")]
    Overlap {
        a: &'static str,
        a_start: usize,
        a_end: usize,
        b: &'static str,
        b_start: usize,
        b_end: usize,
    },
}

/// The 13 header words, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Width in attribute cells (OTA `size` × 32 for multiplayer maps).
    pub width: u32,
    /// Height in attribute cells.
    pub height: u32,
    /// Believed: water level in height units (cells below it are underwater).
    pub sea_level: u32,
    pub ptr_heights: u32,
    pub ptr_feature_grid: u32,
    pub ptr_features: u32,
    pub feature_count: u32,
    pub ptr_tile_ids: u32,
    pub ptr_tile_cols: u32,
    pub ptr_tile_rows: u32,
    pub ptr_minimap_small: u32,
    pub ptr_minimap: u32,
}

/// One entry of the feature table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feature {
    /// Stored index; equals the record's position in every retail file.
    pub index: u32,
    /// Feature name (e.g. `TarSpire07`), up to the first NUL of the 128-byte field.
    pub name: String,
}

/// What a feature-grid cell holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Empty,
    /// Index into [`Tnt::features`].
    Feature(u16),
    Paved,
    Marker,
    /// Any other value (none seen in retail data).
    Other(u16),
}

impl Cell {
    pub fn classify(v: u16, feature_count: usize) -> Cell {
        match v {
            CELL_EMPTY => Cell::Empty,
            CELL_PAVED => Cell::Paved,
            CELL_MARKER => Cell::Marker,
            v if usize::from(v) < feature_count => Cell::Feature(v),
            v => Cell::Other(v),
        }
    }
}

/// An 8-bit indexed image. The palette is not in the TNT; minimaps match
/// `palettes/<kingdom>.pcx` (Believed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indexed {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The terrain texture reference for each tile cell (2×2 attribute cells).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileGrid {
    pub width: u32,
    pub height: u32,
    /// JPEG id per cell; the file is `terrain/{id:08x}.jpg` (case-insensitive).
    pub ids: Vec<u32>,
    /// Piece column inside the JPEG (pixel x = `col * PIECE_PX`).
    pub cols: Vec<u8>,
    /// Piece row inside the JPEG (pixel y = `row * PIECE_PX`).
    pub rows: Vec<u8>,
}

impl TileGrid {
    /// `(jpeg id, piece column, piece row)` for tile cell `(x, y)`.
    pub fn get(&self, x: u32, y: u32) -> Option<(u32, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = y as usize * self.width as usize + x as usize;
        Some((*self.ids.get(i)?, *self.cols.get(i)?, *self.rows.get(i)?))
    }

    /// The VFS path of a tile id.
    pub fn jpeg_path(id: u32) -> String {
        format!("terrain/{id:08x}.jpg")
    }
}

/// A byte range of the file that no header pointer covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBlock {
    pub offset: usize,
    pub bytes: Vec<u8>,
}

/// A parsed Kingdoms TNT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tnt {
    pub header: Header,
    /// `width * height` heights, row-major.
    pub heights: Vec<u8>,
    /// `width * height` raw feature-grid words; see [`Tnt::cell`].
    pub feature_grid: Vec<u16>,
    pub features: Vec<Feature>,
    pub tiles: TileGrid,
    /// Fixed-size overview image (126×126 in maps, 128×128 in sections).
    pub minimap_small: Option<Indexed>,
    /// Larger overview image fitted inside roughly 511×432; absent in sections.
    pub minimap: Option<Indexed>,
    /// Gaps between blocks or trailing bytes (none in retail files).
    pub unclaimed: Vec<RawBlock>,
}

impl Tnt {
    pub fn width(&self) -> u32 {
        self.header.width
    }
    pub fn height(&self) -> u32 {
        self.header.height
    }

    pub fn height_at(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width() || y >= self.height() {
            return None;
        }
        self.heights
            .get(y as usize * self.width() as usize + x as usize)
            .copied()
    }

    pub fn cell(&self, x: u32, y: u32) -> Option<Cell> {
        if x >= self.width() || y >= self.height() {
            return None;
        }
        let v = *self
            .feature_grid
            .get(y as usize * self.width() as usize + x as usize)?;
        Some(Cell::classify(v, self.features.len()))
    }
}

/// Upper bound on either map dimension; the largest retail map is 450 cells.
const MAX_DIM: u32 = 1 << 14;

/// Parses a Kingdoms TNT (maps and sections share the layout).
pub fn parse(data: &[u8]) -> Result<Tnt, TntError> {
    let mut r = Reader::new(data);
    let magic = r.u32("tnt version")?;
    if magic != TNT_MAGIC {
        return Err(TntError::BadMagic(magic));
    }
    let mut w = [0u32; 12];
    for v in &mut w {
        *v = r.u32("tnt header")?;
    }
    let header = Header {
        width: w[0],
        height: w[1],
        sea_level: w[2],
        ptr_heights: w[3],
        ptr_feature_grid: w[4],
        ptr_features: w[5],
        feature_count: w[6],
        ptr_tile_ids: w[7],
        ptr_tile_cols: w[8],
        ptr_tile_rows: w[9],
        ptr_minimap_small: w[10],
        ptr_minimap: w[11],
    };
    let (width, height) = (header.width, header.height);
    if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
        return Err(TntError::BadSize { width, height });
    }
    let cells = width as usize * height as usize;
    let (tw, th) = (width / 2, height / 2);
    let tcells = tw as usize * th as usize;

    let mut spans: Vec<(&'static str, usize, usize)> = Vec::new();

    let heights = block(data, header.ptr_heights, cells, 1, "heights", &mut spans)?.to_vec();
    let feature_grid = block(
        data,
        header.ptr_feature_grid,
        cells,
        2,
        "feature grid",
        &mut spans,
    )?
    .chunks_exact(2)
    .map(|c| u16::from_le_bytes([c[0], c[1]]))
    .collect();

    let mut features = Vec::new();
    if header.feature_count > 0 {
        let n = usize::try_from(header.feature_count).unwrap_or(usize::MAX);
        let raw = block(
            data,
            header.ptr_features,
            n,
            FEATURE_RECORD_LEN,
            "feature table",
            &mut spans,
        )?;
        features.reserve(n);
        for rec in raw.chunks_exact(FEATURE_RECORD_LEN) {
            let mut fr = Reader::new(rec);
            let index = fr.u32("feature index")?;
            let name = fr.fixed_str(FEATURE_RECORD_LEN - 4, "feature name")?;
            features.push(Feature { index, name });
        }
    }

    let ids = block(data, header.ptr_tile_ids, tcells, 4, "tile ids", &mut spans)?
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let cols = block(
        data,
        header.ptr_tile_cols,
        tcells,
        1,
        "tile columns",
        &mut spans,
    )?
    .to_vec();
    let rows = block(
        data,
        header.ptr_tile_rows,
        tcells,
        1,
        "tile rows",
        &mut spans,
    )?
    .to_vec();

    let minimap_small = indexed(data, header.ptr_minimap_small, "small minimap", &mut spans)?;
    let minimap = indexed(data, header.ptr_minimap, "minimap", &mut spans)?;

    // Every block must sit after the header and not overlap another.
    spans.push(("header", 0, HEADER_LEN));
    spans.sort_by_key(|s| (s.1, s.2));
    let mut unclaimed = Vec::new();
    let mut end = 0usize;
    for pair in spans.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if b.1 < a.2 {
            return Err(TntError::Overlap {
                a: a.0,
                a_start: a.1,
                a_end: a.2,
                b: b.0,
                b_start: b.1,
                b_end: b.2,
            });
        }
    }
    for &(_, s, e) in &spans {
        if s > end {
            unclaimed.push(RawBlock {
                offset: end,
                bytes: data[end..s].to_vec(),
            });
        }
        end = end.max(e);
    }
    if end < data.len() {
        unclaimed.push(RawBlock {
            offset: end,
            bytes: data[end..].to_vec(),
        });
    }

    Ok(Tnt {
        header,
        heights,
        feature_grid,
        features,
        tiles: TileGrid {
            width: tw,
            height: th,
            ids,
            cols,
            rows,
        },
        minimap_small,
        minimap,
        unclaimed,
    })
}

/// Bounds-checks `count * elem` bytes at `ptr` and records the span.
fn block<'a>(
    data: &'a [u8],
    ptr: u32,
    count: usize,
    elem: usize,
    what: &'static str,
    spans: &mut Vec<(&'static str, usize, usize)>,
) -> Result<&'a [u8], TntError> {
    let mut r = Reader::at(data, u64::from(ptr), what)?;
    let n = r.check_count(count as u64, elem, what)?;
    let start = r.pos();
    let bytes = r.bytes(n * elem, what)?;
    spans.push((what, start, start + bytes.len()));
    Ok(bytes)
}

/// An optional `u32 w, u32 h, w*h bytes` image; pointer 0 means absent.
fn indexed(
    data: &[u8],
    ptr: u32,
    what: &'static str,
    spans: &mut Vec<(&'static str, usize, usize)>,
) -> Result<Option<Indexed>, TntError> {
    if ptr == 0 {
        return Ok(None);
    }
    let mut r = Reader::at(data, u64::from(ptr), what)?;
    let start = r.pos();
    let width = r.u32(what)?;
    let height = r.u32(what)?;
    let n = r.check_count(u64::from(width) * u64::from(height), 1, what)?;
    let pixels = r.bytes(n, what)?.to_vec();
    spans.push((what, start, r.pos()));
    Ok(Some(Indexed {
        width,
        height,
        pixels,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a synthetic map: `w x h` cells, blocks in a shuffled order to
    /// prove the parser follows pointers.
    fn synth(w: u32, h: u32, features: &[&str]) -> Vec<u8> {
        let cells = (w * h) as usize;
        let tc = ((w / 2) * (h / 2)) as usize;
        let heights: Vec<u8> = (0..cells).map(|i| (i % 251) as u8).collect();
        let mut grid = Vec::new();
        for i in 0..cells {
            let v: u16 = match i % 4 {
                0 => CELL_EMPTY,
                1 => CELL_PAVED,
                2 => CELL_MARKER,
                _ => 0,
            };
            grid.extend_from_slice(&v.to_le_bytes());
        }
        let mut feat = Vec::new();
        for (i, n) in features.iter().enumerate() {
            feat.extend_from_slice(&(i as u32).to_le_bytes());
            let mut name = n.as_bytes().to_vec();
            name.resize(128, 0);
            name[127] = 0xAA; // junk after the NUL, as in retail files
            feat.extend_from_slice(&name);
        }
        let mut ids = Vec::new();
        for i in 0..tc {
            ids.extend_from_slice(&(0xCA42_00F8u32 + (i as u32 / 16)).to_le_bytes());
        }
        let cols: Vec<u8> = (0..tc).map(|i| (i % 16) as u8).collect();
        let rows: Vec<u8> = (0..tc).map(|i| ((i / 7) % 16) as u8).collect();
        let mini = |mw: u32, mh: u32| {
            let mut v = mw.to_le_bytes().to_vec();
            v.extend_from_slice(&mh.to_le_bytes());
            v.extend((0..mw * mh).map(|i| i as u8));
            v
        };
        let small = mini(3, 3);
        let large = mini(5, 4);

        // Order: heights, grid, ids, cols, rows, features, small, large.
        let blocks = [&heights, &grid, &ids, &cols, &rows, &feat, &small, &large];
        let mut ptrs = Vec::new();
        let mut body = Vec::new();
        for b in blocks {
            ptrs.push((HEADER_LEN + body.len()) as u32);
            body.extend_from_slice(b);
        }
        let hdr = [
            TNT_MAGIC,
            w,
            h,
            58,
            ptrs[0],
            ptrs[1],
            if features.is_empty() { 0 } else { ptrs[5] },
            features.len() as u32,
            ptrs[2],
            ptrs[3],
            ptrs[4],
            ptrs[6],
            ptrs[7],
        ];
        let mut out: Vec<u8> = hdr.iter().flat_map(|v| v.to_le_bytes()).collect();
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parses_synthetic_map() {
        let d = synth(8, 6, &["TarSpire07", "Tree01"]);
        let t = parse(&d).unwrap();
        assert_eq!((t.width(), t.height()), (8, 6));
        assert_eq!(t.header.sea_level, 58);
        assert_eq!(t.heights.len(), 48);
        assert_eq!(t.height_at(3, 1), Some(11));
        assert_eq!(t.height_at(8, 0), None);
        assert_eq!(t.cell(0, 0), Some(Cell::Empty));
        assert_eq!(t.cell(1, 0), Some(Cell::Paved));
        assert_eq!(t.cell(2, 0), Some(Cell::Marker));
        assert_eq!(t.cell(3, 0), Some(Cell::Feature(0)));
        assert_eq!(t.features.len(), 2);
        assert_eq!(t.features[0].name, "TarSpire07");
        assert_eq!(t.features[1].index, 1);
        assert_eq!((t.tiles.width, t.tiles.height), (4, 3));
        assert_eq!(t.tiles.get(1, 0), Some((0xCA42_00F8, 1, 0)));
        assert_eq!(t.tiles.get(4, 0), None);
        assert_eq!(TileGrid::jpeg_path(0xCA42_00F8), "terrain/ca4200f8.jpg");
        let s = t.minimap_small.as_ref().unwrap();
        assert_eq!((s.width, s.height, s.pixels.len()), (3, 3, 9));
        let m = t.minimap.as_ref().unwrap();
        assert_eq!((m.width, m.height), (5, 4));
        assert!(t.unclaimed.is_empty());
    }

    #[test]
    fn missing_features_and_minimap_are_allowed() {
        let mut d = synth(4, 4, &[]);
        // Section-style: no large minimap.
        d[48..52].copy_from_slice(&0u32.to_le_bytes());
        let t = parse(&d).unwrap();
        assert!(t.features.is_empty());
        assert!(t.minimap.is_none());
        // The orphaned minimap bytes are reported, not dropped.
        assert_eq!(t.unclaimed.len(), 1);
        assert_eq!(t.unclaimed[0].bytes.len(), 8 + 20);
    }

    #[test]
    fn rejects_bad_input_without_panicking() {
        assert!(matches!(parse(b""), Err(TntError::Read(_))));
        let mut d = synth(4, 4, &["a"]);
        d[1] = 0;
        assert!(matches!(parse(&d), Err(TntError::BadMagic(_))));

        let good = synth(4, 4, &["a"]);
        // Pointer past the end.
        let mut d = good.clone();
        d[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&d).is_err());
        // Huge dimensions.
        let mut d = good.clone();
        d[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(parse(&d), Err(TntError::BadSize { .. })));
        // Feature count far beyond the data.
        let mut d = good.clone();
        d[28..32].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        assert!(matches!(
            parse(&d),
            Err(TntError::Read(ReadError::CountTooLarge { .. }))
        ));
        // Two blocks at the same offset.
        let mut d = good.clone();
        let p = d[16..20].to_vec();
        d[20..24].copy_from_slice(&p);
        assert!(parse(&d).is_err());
        // Huge minimap size.
        let mut d = good.clone();
        let mp = u32::from_le_bytes(d[48..52].try_into().unwrap()) as usize;
        d[mp..mp + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&d).is_err());
        // Every truncation fails cleanly.
        for n in 0..good.len() {
            let _ = parse(&good[..n]);
        }
    }

    #[test]
    fn corpus_maps_and_sections_parse() {
        let Some(dir) = tak_vfs::testing::data_dir_or_skip("corpus_maps_and_sections_parse") else {
            return;
        };
        let vfs = tak_vfs::open_kingdoms(&dir).expect("mount install");
        let mut maps = 0;
        let mut sections = 0;
        let mut errors = Vec::new();
        for prefix in ["kmap", "maps", "missions", "sections"] {
            for f in vfs.list(prefix) {
                let lower = f.path.to_ascii_lowercase();
                let is_tnt = lower.ends_with(".tnt");
                let is_section = prefix == "sections"
                    && (is_tnt || lower.ends_with(".bad") || lower.ends_with(".old"));
                if !is_tnt && !is_section {
                    continue;
                }
                let data = vfs.read(&f.path).expect("read");
                match parse(&data) {
                    Ok(t) => {
                        let bad_piece = t
                            .tiles
                            .cols
                            .iter()
                            .chain(&t.tiles.rows)
                            .any(|&v| v > MAX_PIECE_INDEX);
                        let bad_cell = t.feature_grid.iter().any(|&v| {
                            matches!(Cell::classify(v, t.features.len()), Cell::Other(_))
                        });
                        let bad_index = t
                            .features
                            .iter()
                            .enumerate()
                            .any(|(i, f)| f.index as usize != i);
                        // `.bad`/`.old` editor leftovers parse but may hold odd values.
                        let strict = is_tnt;
                        if strict && (bad_piece || bad_cell || bad_index || !t.unclaimed.is_empty())
                        {
                            errors.push(format!(
                                "{}: piece>31 {bad_piece}, unknown cell {bad_cell}, \
                                 index mismatch {bad_index}, unclaimed {}",
                                f.path,
                                t.unclaimed.len()
                            ));
                        }
                        if prefix == "sections" {
                            sections += 1;
                        } else {
                            assert!(t.minimap.is_some(), "{} has no minimap", f.path);
                            maps += 1;
                        }
                    }
                    Err(e) => errors.push(format!("{}: {e}", f.path)),
                }
            }
        }
        assert!(errors.is_empty(), "{errors:#?}");
        assert_eq!(maps, 310);
        assert_eq!(sections, 3153);
    }
}
