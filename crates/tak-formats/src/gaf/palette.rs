//! 256-colour palettes (`.pal`, PCX trailers) and the per-palette lookup
//! tables in `palettes/` (`.alp`, `.lht`, `.shd`, `.gry`, `.blu`).
//!
//! See `docs/formats/palettes.md`.

use crate::reader::{ReadError, Reader};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaletteError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("{what}: expected {expected} bytes, found {found}")]
    WrongSize {
        what: &'static str,
        expected: usize,
        found: usize,
    },
}

/// 256 RGB colours. Index 0 is the first entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub colors: [[u8; 3]; 256],
}

impl Palette {
    /// Size of a `.pal` file: 256 entries of `r, g, b, 0`.
    pub const PAL_LEN: usize = 1024;

    /// Reads a 1,024-byte `.pal` (RGBX, the fourth byte is padding).
    pub fn from_pal(data: &[u8]) -> Result<Self, PaletteError> {
        if data.len() != Self::PAL_LEN {
            return Err(PaletteError::WrongSize {
                what: ".pal",
                expected: Self::PAL_LEN,
                found: data.len(),
            });
        }
        let mut colors = [[0u8; 3]; 256];
        for (c, px) in colors.iter_mut().zip(data.chunks_exact(4)) {
            *c = [px[0], px[1], px[2]];
        }
        Ok(Palette { colors })
    }

    /// Reads 768 bytes of packed RGB triples (the PCX trailer layout).
    pub fn from_rgb(data: &[u8]) -> Result<Self, PaletteError> {
        let mut r = Reader::new(data);
        let rgb = r.bytes(768, "palette rgb")?;
        let mut colors = [[0u8; 3]; 256];
        for (c, px) in colors.iter_mut().zip(rgb.chunks_exact(3)) {
            *c = [px[0], px[1], px[2]];
        }
        Ok(Palette { colors })
    }

    /// A grey ramp, for dumping indexed art when the real palette is unknown.
    pub fn grayscale() -> Self {
        let mut colors = [[0u8; 3]; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            let v = i as u8;
            *c = [v, v, v];
        }
        Palette { colors }
    }

    pub fn rgb(&self, index: u8) -> [u8; 3] {
        self.colors[usize::from(index)]
    }
}

fn exact<'a>(data: &'a [u8], len: usize, what: &'static str) -> Result<&'a [u8], PaletteError> {
    if data.len() == len {
        Ok(data)
    } else {
        Err(PaletteError::WrongSize {
            what,
            expected: len,
            found: data.len(),
        })
    }
}

/// `.alp`: a 256×256 table of palette indices. **Known**: symmetric with an
/// identity diagonal in all 41 files. For `aramon` entry `[a][b]` is the
/// colour nearest the 50/50 average of `a` and `b`; `fx.alp` does not fit
/// that rule against `fx.pcx` (Unknown why).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlendTable {
    table: Vec<u8>,
}

impl BlendTable {
    pub const LEN: usize = 256 * 256;

    pub fn parse(data: &[u8]) -> Result<Self, PaletteError> {
        Ok(BlendTable {
            table: exact(data, Self::LEN, ".alp")?.to_vec(),
        })
    }

    /// The index that represents a half-and-half mix of `a` and `b`.
    pub fn blend(&self, a: u8, b: u8) -> u8 {
        self.table[usize::from(a) * 256 + usize::from(b)]
    }
}

/// `.lht` and `.shd`: 32 rows of 256 palette indices, one row per level.
///
/// **Known**: `.shd` row 0 maps everything to index 0 (black) in all 41
/// files and row 15 is the closest to identity in 40 of 41, brightening past
/// it. `.lht` rows brighten with level; row 0 is identity in 21 of 41.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelTable {
    table: Vec<u8>,
}

impl LevelTable {
    pub const LEVELS: usize = 32;
    pub const LEN: usize = Self::LEVELS * 256;

    pub fn parse(data: &[u8]) -> Result<Self, PaletteError> {
        Ok(LevelTable {
            table: exact(data, Self::LEN, ".lht/.shd")?.to_vec(),
        })
    }

    /// The index for colour `index` at `level` (clamped to 0..32).
    pub fn get(&self, level: usize, index: u8) -> u8 {
        self.table[level.min(Self::LEVELS - 1) * 256 + usize::from(index)]
    }

    pub fn row(&self, level: usize) -> &[u8] {
        let l = level.min(Self::LEVELS - 1);
        &self.table[l * 256..(l + 1) * 256]
    }
}

/// `.gry` and `.blu`: one 256-entry index remap. `.gry` maps each colour to a
/// desaturated one (**Known** by sampling); `.blu` maps to darker, cooler
/// colours (**Believed** role: a tint such as fog-of-war or underwater).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemapTable {
    pub map: [u8; 256],
}

impl RemapTable {
    pub fn parse(data: &[u8]) -> Result<Self, PaletteError> {
        let d = exact(data, 256, ".gry/.blu")?;
        let mut map = [0u8; 256];
        map.copy_from_slice(d);
        Ok(RemapTable { map })
    }

    pub fn get(&self, index: u8) -> u8 {
        self.map[usize::from(index)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pal_reads_rgbx() {
        let mut d = vec![0u8; 1024];
        d[4..8].copy_from_slice(&[10, 20, 30, 0]);
        let p = Palette::from_pal(&d).unwrap();
        assert_eq!(p.rgb(1), [10, 20, 30]);
        assert!(Palette::from_pal(&d[..1000]).is_err());
        assert!(Palette::from_rgb(&d[..700]).is_err());
    }

    #[test]
    fn tables_check_sizes_and_index() {
        let mut alp = vec![0u8; 65536];
        alp[3 * 256 + 7] = 42;
        let t = BlendTable::parse(&alp).unwrap();
        assert_eq!(t.blend(3, 7), 42);
        assert!(BlendTable::parse(&alp[1..]).is_err());

        let mut lvl = vec![0u8; 8192];
        lvl[31 * 256 + 5] = 9;
        let l = LevelTable::parse(&lvl).unwrap();
        assert_eq!(l.get(31, 5), 9);
        assert_eq!(l.get(99, 5), 9, "levels clamp");
        assert_eq!(l.row(31)[5], 9);
        assert!(LevelTable::parse(&[]).is_err());

        let mut r = [0u8; 256];
        r[1] = 2;
        assert_eq!(RemapTable::parse(&r).unwrap().get(1), 2);
        assert!(RemapTable::parse(&r[..255]).is_err());
    }
}
