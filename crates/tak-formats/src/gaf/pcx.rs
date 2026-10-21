//! PCX images: 8 bits per pixel, one plane, RLE, with the 256-colour palette
//! trailer. This is the only PCX layout Kingdoms ships (**Known**, all 222
//! files incl. two named `.pal`). See `docs/formats/palettes.md`.

use super::palette::Palette;
use crate::reader::{ReadError, Reader};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PcxError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("not a PCX file (manufacturer byte {0:#04x})")]
    BadMagic(u8),
    #[error("unsupported PCX layout: encoding {encoding}, {bpp} bpp, {planes} planes")]
    Unsupported { encoding: u8, bpp: u8, planes: u8 },
    #[error("bad PCX geometry: {width}x{height}, {bytes_per_line} bytes per line")]
    BadGeometry {
        width: u32,
        height: u32,
        bytes_per_line: u16,
    },
}

/// A decoded PCX: one palette index per pixel, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcx {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// The 768-byte trailer after the `0x0C` marker, if present.
    pub palette: Option<Palette>,
}

const HEADER_LEN: usize = 128;
/// One RLE pair (2 bytes) expands to at most 63 bytes.
const MAX_RLE_RATIO: usize = 63;

impl Pcx {
    pub fn parse(data: &[u8]) -> Result<Self, PcxError> {
        let mut r = Reader::new(data);
        let manufacturer = r.u8("pcx manufacturer")?;
        if manufacturer != 0x0a {
            return Err(PcxError::BadMagic(manufacturer));
        }
        let _version = r.u8("pcx version")?;
        let encoding = r.u8("pcx encoding")?;
        let bpp = r.u8("pcx bpp")?;
        let x0 = r.u16("pcx xmin")?;
        let y0 = r.u16("pcx ymin")?;
        let x1 = r.u16("pcx xmax")?;
        let y1 = r.u16("pcx ymax")?;
        r.seek(65, "pcx planes")?;
        let planes = r.u8("pcx planes")?;
        let bytes_per_line = r.u16("pcx bytes per line")?;
        if encoding != 1 || bpp != 8 || planes != 1 {
            return Err(PcxError::Unsupported {
                encoding,
                bpp,
                planes,
            });
        }
        let width = u32::from(x1).wrapping_sub(u32::from(x0)).wrapping_add(1);
        let height = u32::from(y1).wrapping_sub(u32::from(y0)).wrapping_add(1);
        let bad = PcxError::BadGeometry {
            width,
            height,
            bytes_per_line,
        };
        if x1 < x0 || y1 < y0 || u32::from(bytes_per_line) < width {
            return Err(bad);
        }
        let (w, h, bpl) = (width as usize, height as usize, usize::from(bytes_per_line));
        let body_len = data.len().saturating_sub(HEADER_LEN);
        let scan_total = bpl.checked_mul(h).ok_or(bad.clone())?;
        if scan_total > body_len.saturating_mul(MAX_RLE_RATIO) {
            return Err(bad);
        }

        r.seek(HEADER_LEN as u64, "pcx body")?;
        // Decode the RLE stream as one run of scanlines; runs may cross lines.
        let mut scan = Vec::with_capacity(scan_total);
        while scan.len() < scan_total {
            let b = r.u8("pcx rle")?;
            if b >= 0xc0 {
                let v = r.u8("pcx rle value")?;
                let n = usize::from(b & 0x3f).min(scan_total - scan.len());
                scan.resize(scan.len() + n, v);
            } else {
                scan.push(b);
            }
        }
        let mut pixels = Vec::with_capacity(w * h);
        for row in scan.chunks_exact(bpl.max(1)).take(h) {
            pixels.extend_from_slice(&row[..w]);
        }
        let palette = match data.len().checked_sub(769) {
            Some(at) if at >= r.pos() && data[at] == 0x0c => {
                Some(Palette::from_rgb(&data[at + 1..]).expect("768 bytes present"))
            }
            _ => None,
        };
        Ok(Pcx {
            width,
            height,
            pixels,
            palette,
        })
    }
}

#[cfg(test)]
pub(crate) fn build_pcx(
    width: u16,
    height: u16,
    rows: &[&[u8]],
    palette: Option<[u8; 3]>,
) -> Vec<u8> {
    let bpl = width + (width & 1);
    let mut d = vec![0u8; HEADER_LEN];
    d[0..4].copy_from_slice(&[0x0a, 5, 1, 8]);
    d[8..10].copy_from_slice(&(width - 1).to_le_bytes());
    d[10..12].copy_from_slice(&(height - 1).to_le_bytes());
    d[65] = 1;
    d[66..68].copy_from_slice(&bpl.to_le_bytes());
    for row in rows {
        let mut row = row.to_vec();
        row.resize(usize::from(bpl), 0);
        for &px in &row {
            if px >= 0xc0 {
                d.extend_from_slice(&[0xc1, px]);
            } else {
                d.push(px);
            }
        }
    }
    if let Some(c) = palette {
        d.push(0x0c);
        for _ in 0..256 {
            d.extend_from_slice(&c);
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rle_and_crops_padding() {
        let mut d = build_pcx(3, 2, &[&[1, 2, 0xc5], &[4, 5, 6]], Some([1, 2, 3]));
        let p = Pcx::parse(&d).unwrap();
        assert_eq!((p.width, p.height), (3, 2));
        assert_eq!(p.pixels, vec![1, 2, 0xc5, 4, 5, 6]);
        assert_eq!(p.palette.unwrap().rgb(200), [1, 2, 3]);

        // A run spanning a whole line.
        d.truncate(HEADER_LEN);
        d.extend_from_slice(&[0xc4, 7, 0xc4, 8]);
        let p = Pcx::parse(&d).unwrap();
        assert_eq!(p.pixels, vec![7, 7, 7, 8, 8, 8]);
        assert!(p.palette.is_none());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(Pcx::parse(&[0x0b]), Err(PcxError::BadMagic(_))));
        let mut d = build_pcx(3, 2, &[&[1, 2, 3], &[4, 5, 6]], None);
        d.truncate(HEADER_LEN + 3);
        assert!(matches!(Pcx::parse(&d), Err(PcxError::Read(_))));
        // A huge declared size with no body must not allocate.
        let mut d = build_pcx(2, 1, &[&[0, 0]], None);
        d[8..12].copy_from_slice(&[0xff, 0xfe, 0xff, 0xfe]);
        d[66..68].copy_from_slice(&0xfffeu16.to_le_bytes());
        assert!(matches!(Pcx::parse(&d), Err(PcxError::BadGeometry { .. })));
        let mut d = build_pcx(2, 1, &[&[0, 0]], None);
        d[3] = 24;
        assert!(matches!(Pcx::parse(&d), Err(PcxError::Unsupported { .. })));
        for n in 0..HEADER_LEN + 2 {
            let _ = Pcx::parse(&build_pcx(2, 1, &[&[0, 0]], None)[..n]);
        }
    }
}
