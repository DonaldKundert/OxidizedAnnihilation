//! GAF/TAF sprite and animation containers (`.gaf`, `.taf`, `.gao`, `.gao2`),
//! plus PCX images, palettes and palette lookup tables in submodules.
//! Work package WP-C (M4b). See `docs/formats/gaf.md`.
//!
//! The container is TA's GAF layout. Kingdoms reuses TA's "compressed" byte as
//! a pixel-format code: 0 raw 8-bit indexed, 1 RLE 8-bit indexed, 4 raw
//! ARGB4444, 5 raw ARGB1555 (all **Known** from the full corpus).
//!
//! Parsing decodes every frame, so a successful parse means every pixel was
//! read. Indexed frames keep palette indices; [`Frame::to_rgba`] converts to
//! RGBA8 for export only.

pub mod palette;
pub mod pcx;

use crate::reader::{ReadError, Reader};
use palette::Palette;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GafError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("empty file")]
    Empty,
    #[error("frame at {offset}: unknown pixel format {code}")]
    UnknownFormat { offset: u32, code: u8 },
    #[error("frame at {offset}: subframes nested deeper than {MAX_DEPTH}")]
    TooDeep { offset: u32 },
    #[error("decoded pixels exceed the budget of {budget} for a {len}-byte file")]
    PixelBudget { budget: u64, len: usize },
}

/// Composite frames nest one level in retail data; allow a little more.
pub const MAX_DEPTH: usize = 4;
/// Decoded pixels allowed per input byte, summed over the whole file, plus a
/// fixed floor. Bounds RLE expansion and frames sharing one pixel block.
const PIXELS_PER_BYTE: u64 = 256;
const PIXEL_FLOOR: u64 = 1 << 20;

/// The pixel-format code stored in a frame header (TA's "compressed" byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PixelFormat {
    /// 0: `width × height` palette indices; `color_key` marks transparency.
    Indexed,
    /// 1: per-row RLE palette indices with transparent skip runs.
    IndexedRle,
    /// 4: `width × height` little-endian u16, `AAAA RRRR GGGG BBBB`.
    Argb4444,
    /// 5: `width × height` little-endian u16, `A RRRRR GGGGG BBBBB`.
    Argb1555,
}

impl PixelFormat {
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => PixelFormat::Indexed,
            1 => PixelFormat::IndexedRle,
            4 => PixelFormat::Argb4444,
            5 => PixelFormat::Argb1555,
            _ => return None,
        })
    }
    pub fn code(self) -> u8 {
        match self {
            PixelFormat::Indexed => 0,
            PixelFormat::IndexedRle => 1,
            PixelFormat::Argb4444 => 4,
            PixelFormat::Argb1555 => 5,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            PixelFormat::Indexed => "indexed",
            PixelFormat::IndexedRle => "indexed-rle",
            PixelFormat::Argb4444 => "argb4444",
            PixelFormat::Argb1555 => "argb1555",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gaf {
    /// `0x00010100` in all files but `anims/terrain.gaf` (0). Not validated.
    pub version: u32,
    pub entries: Vec<Entry>,
}

/// A named sequence of frames (one image or one animation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// `1` in every retail entry; TA treats the low byte as a loop flag.
    pub flags: u16,
    pub frames: Vec<FrameRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRef {
    /// Second word of the frame reference. Believed (TA): ticks per frame.
    pub duration: u32,
    pub frame: Frame,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u16,
    pub height: u16,
    /// Anchor position inside the frame (TA semantics: the pixel at
    /// `(x_offset, y_offset)` lands on the sprite's anchor point).
    pub x_offset: i16,
    pub y_offset: i16,
    /// Transparent index for raw indexed frames (9 in GAFs, 0 in TAFs).
    pub color_key: u8,
    pub format: PixelFormat,
    /// The raw subframe word. The low byte is the subframe count; some TAF
    /// frames carry `0xFF00` (count 0, meaning of the high byte Unknown).
    pub subframe_word: u16,
    /// Trailing u32 (0 in every Kingdoms frame).
    pub unknown: u32,
    pub content: FrameContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameContent {
    /// Palette indices with a per-pixel opacity mask, row-major.
    Indexed { index: Vec<u8>, opaque: Vec<bool> },
    /// 16-bit pixels as stored (see [`PixelFormat`] for the bit layout).
    Direct(Vec<u16>),
    /// Child frames drawn in order onto this frame's canvas.
    Composite(Vec<Frame>),
}

/// Plain RGBA8 pixels, row-major, 4 bytes per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Rgba8Image {
    pub fn new(width: u32, height: u32) -> Self {
        Rgba8Image {
            width,
            height,
            data: vec![0; width as usize * height as usize * 4],
        }
    }

    /// Source-over composite of `src` with its top-left at `(x, y)`, clipped.
    pub fn draw(&mut self, src: &Rgba8Image, x: i32, y: i32) {
        for sy in 0..src.height as i32 {
            let dy = y + sy;
            if dy < 0 || dy >= self.height as i32 {
                continue;
            }
            for sx in 0..src.width as i32 {
                let dx = x + sx;
                if dx < 0 || dx >= self.width as i32 {
                    continue;
                }
                let s = ((sy as usize * src.width as usize) + sx as usize) * 4;
                let d = ((dy as usize * self.width as usize) + dx as usize) * 4;
                let sp = &src.data[s..s + 4];
                let a = u32::from(sp[3]);
                if a == 0 {
                    continue;
                }
                let dp = &mut self.data[d..d + 4];
                if a == 255 {
                    dp.copy_from_slice(sp);
                    continue;
                }
                let da = u32::from(dp[3]);
                let out_a = a + da * (255 - a) / 255;
                for c in 0..3 {
                    let v = (u32::from(sp[c]) * a + u32::from(dp[c]) * da * (255 - a) / 255)
                        / out_a.max(1);
                    dp[c] = v as u8;
                }
                dp[3] = out_a as u8;
            }
        }
    }
}

/// Expands a 5-bit or 4-bit channel to 8 bits.
fn expand5(v: u16) -> u8 {
    let v = (v & 31) as u8;
    (v << 3) | (v >> 2)
}
fn expand4(v: u16) -> u8 {
    (v & 15) as u8 * 17
}

/// ARGB1555 to RGBA8.
pub fn argb1555_to_rgba(p: u16) -> [u8; 4] {
    let a = if p & 0x8000 != 0 { 255 } else { 0 };
    [expand5(p >> 10), expand5(p >> 5), expand5(p), a]
}

/// ARGB4444 to RGBA8.
pub fn argb4444_to_rgba(p: u16) -> [u8; 4] {
    [
        expand4(p >> 8),
        expand4(p >> 4),
        expand4(p),
        expand4(p >> 12),
    ]
}

impl Frame {
    /// Renders the frame as RGBA8. Indexed pixels go through `palette`; the
    /// palette is ignored for 16-bit frames. Composite children are placed at
    /// `(parent.x_offset - child.x_offset, parent.y_offset - child.y_offset)`
    /// on a parent-sized canvas (TA rule) and clipped to it.
    pub fn to_rgba(&self, palette: &Palette) -> Rgba8Image {
        let (w, h) = (u32::from(self.width), u32::from(self.height));
        let mut img = Rgba8Image::new(w, h);
        match &self.content {
            FrameContent::Indexed { index, opaque } => {
                for (i, (&ix, &op)) in index.iter().zip(opaque).enumerate() {
                    if op {
                        let [r, g, b] = palette.rgb(ix);
                        img.data[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, 255]);
                    }
                }
            }
            FrameContent::Direct(px) => {
                let conv = match self.format {
                    PixelFormat::Argb4444 => argb4444_to_rgba,
                    _ => argb1555_to_rgba,
                };
                for (i, &p) in px.iter().enumerate() {
                    img.data[i * 4..i * 4 + 4].copy_from_slice(&conv(p));
                }
            }
            FrameContent::Composite(children) => {
                for c in children {
                    let ci = c.to_rgba(palette);
                    let x = i32::from(self.x_offset) - i32::from(c.x_offset);
                    let y = i32::from(self.y_offset) - i32::from(c.y_offset);
                    img.draw(&ci, x, y);
                }
            }
        }
        img
    }

    /// This frame and all nested subframes, depth first.
    pub fn walk(&self, f: &mut impl FnMut(&Frame, usize)) {
        fn go(fr: &Frame, depth: usize, f: &mut impl FnMut(&Frame, usize)) {
            f(fr, depth);
            if let FrameContent::Composite(cs) = &fr.content {
                for c in cs {
                    go(c, depth + 1, f);
                }
            }
        }
        go(self, 0, f)
    }
}

struct Parser<'a> {
    data: &'a [u8],
    budget: u64,
    spent: u64,
}

impl<'a> Parser<'a> {
    fn spend(&mut self, pixels: u64) -> Result<(), GafError> {
        self.spent = self.spent.saturating_add(pixels);
        if self.spent > self.budget {
            return Err(GafError::PixelBudget {
                budget: self.budget,
                len: self.data.len(),
            });
        }
        Ok(())
    }

    fn frame(&mut self, offset: u32, depth: usize) -> Result<Frame, GafError> {
        if depth > MAX_DEPTH {
            return Err(GafError::TooDeep { offset });
        }
        let mut r = Reader::at(self.data, offset.into(), "frame header")?;
        let width = r.u16("frame width")?;
        let height = r.u16("frame height")?;
        let x_offset = r.i16("frame x offset")?;
        let y_offset = r.i16("frame y offset")?;
        let color_key = r.u8("frame color key")?;
        let code = r.u8("frame pixel format")?;
        let subframe_word = r.u16("frame subframe count")?;
        let _zero = r.u32("frame unknown")?;
        let data_offset = r.u32("frame data offset")?;
        let unknown = r.u32("frame trailing word")?;
        let format =
            PixelFormat::from_code(code).ok_or(GafError::UnknownFormat { offset, code })?;

        let count = subframe_word & 0xff;
        let content = if count > 0 {
            let mut t = Reader::at(self.data, data_offset.into(), "subframe table")?;
            let n = t.check_count(count.into(), 4, "subframe table")?;
            let mut children = Vec::with_capacity(n);
            for _ in 0..n {
                let off = t.u32("subframe offset")?;
                children.push(self.frame(off, depth + 1)?);
            }
            FrameContent::Composite(children)
        } else {
            let n = usize::from(width) * usize::from(height);
            self.spend(n as u64)?;
            let mut p = Reader::at(self.data, data_offset.into(), "frame pixels")?;
            match format {
                PixelFormat::Indexed => {
                    let index = p.bytes(n, "indexed pixels")?.to_vec();
                    let opaque = index.iter().map(|&i| i != color_key).collect();
                    FrameContent::Indexed { index, opaque }
                }
                PixelFormat::IndexedRle => decode_rle(p, width, height)?,
                PixelFormat::Argb4444 | PixelFormat::Argb1555 => {
                    p.check_count(n as u64, 2, "16-bit pixels")?;
                    let raw = p.bytes(n * 2, "16-bit pixels")?;
                    FrameContent::Direct(
                        raw.chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect(),
                    )
                }
            }
        };
        Ok(Frame {
            width,
            height,
            x_offset,
            y_offset,
            color_key,
            format,
            subframe_word,
            unknown,
            content,
        })
    }
}

/// TA's row RLE. Each row is `u16 len` then commands, decoded until `width`
/// pixels are covered: bit 0 set skips `cmd >> 1` transparent pixels; else
/// bit 1 set repeats the next byte `(cmd >> 2) + 1` times; else copies
/// `(cmd >> 2) + 1` literal bytes. Runs are clamped to the row. The next row
/// starts at `row + 2 + len` whatever was consumed; `len == 0` is a blank row.
fn decode_rle(mut r: Reader<'_>, width: u16, height: u16) -> Result<FrameContent, GafError> {
    let w = usize::from(width);
    // Every row costs at least its 2-byte header, which bounds `height`.
    r.check_count(height.into(), 2, "rle rows")?;
    let n = w * usize::from(height);
    let mut index = vec![0u8; n];
    let mut opaque = vec![false; n];
    for y in 0..usize::from(height) {
        let len = r.u16("rle row length")?;
        let next = r.pos() as u64 + u64::from(len);
        if len > 0 && w > 0 {
            let row_ix = &mut index[y * w..(y + 1) * w];
            let row_op = &mut opaque[y * w..(y + 1) * w];
            let mut c = r.clone();
            let mut x = 0usize;
            while x < w {
                let cmd = c.u8("rle command")?;
                if cmd & 1 != 0 {
                    x += usize::from(cmd >> 1);
                } else if cmd & 2 != 0 {
                    let v = c.u8("rle repeat value")?;
                    let k = (usize::from(cmd >> 2) + 1).min(w - x);
                    row_ix[x..x + k].fill(v);
                    row_op[x..x + k].fill(true);
                    x += k;
                } else {
                    let k = (usize::from(cmd >> 2) + 1).min(w - x);
                    row_ix[x..x + k].copy_from_slice(c.bytes(k, "rle literal")?);
                    row_op[x..x + k].fill(true);
                    x += k;
                }
            }
        }
        r.seek(next, "rle next row")?;
    }
    Ok(FrameContent::Indexed { index, opaque })
}

impl Gaf {
    pub fn parse(data: &[u8]) -> Result<Self, GafError> {
        if data.is_empty() {
            return Err(GafError::Empty);
        }
        let mut r = Reader::new(data);
        let version = r.u32("gaf version")?;
        let count = r.u32("gaf entry count")?;
        let _zero = r.u32("gaf header unknown")?;
        let n = r.check_count(count.into(), 4, "gaf entry table")?;
        let mut p = Parser {
            data,
            budget: (data.len() as u64)
                .saturating_mul(PIXELS_PER_BYTE)
                .saturating_add(PIXEL_FLOOR),
            spent: 0,
        };
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let eo = r.u32("gaf entry offset")?;
            let mut e = Reader::at(data, eo.into(), "gaf entry")?;
            let frame_count = e.u16("entry frame count")?;
            let flags = e.u16("entry flags")?;
            let _zero = e.u32("entry unknown")?;
            let name = e.fixed_str(32, "entry name")?;
            let fc = e.check_count(frame_count.into(), 8, "entry frame table")?;
            let mut frames = Vec::with_capacity(fc);
            for _ in 0..fc {
                let fo = e.u32("frame offset")?;
                let duration = e.u32("frame duration")?;
                frames.push(FrameRef {
                    duration,
                    frame: p.frame(fo, 0)?,
                });
            }
            entries.push(Entry {
                name,
                flags,
                frames,
            });
        }
        Ok(Gaf { version, entries })
    }

    /// Case-insensitive entry lookup.
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }
}

/// Builds GAF bytes in memory for tests. Frames are written in order.
#[cfg(test)]
pub(crate) mod build {
    pub struct F {
        pub w: u16,
        pub h: u16,
        pub x: i16,
        pub y: i16,
        pub key: u8,
        pub code: u8,
        pub sub_hi: u8,
        pub data: Vec<u8>,
        pub children: Vec<F>,
    }

    impl F {
        pub fn new(w: u16, h: u16, code: u8, data: Vec<u8>) -> Self {
            F {
                w,
                h,
                x: 0,
                y: 0,
                key: 9,
                code,
                sub_hi: 0,
                data,
                children: vec![],
            }
        }
    }

    fn put_frame(out: &mut Vec<u8>, f: &F) -> u32 {
        let child_offs: Vec<u32> = f.children.iter().map(|c| put_frame(out, c)).collect();
        let data_off = out.len() as u32;
        if child_offs.is_empty() {
            out.extend_from_slice(&f.data);
        } else {
            for o in &child_offs {
                out.extend_from_slice(&o.to_le_bytes());
            }
        }
        let at = out.len() as u32;
        out.extend_from_slice(&f.w.to_le_bytes());
        out.extend_from_slice(&f.h.to_le_bytes());
        out.extend_from_slice(&f.x.to_le_bytes());
        out.extend_from_slice(&f.y.to_le_bytes());
        out.push(f.key);
        out.push(f.code);
        out.push(child_offs.len() as u8);
        out.push(f.sub_hi);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&data_off.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        at
    }

    pub fn gaf(entries: &[(&str, Vec<F>)]) -> Vec<u8> {
        let mut body = Vec::new();
        let base = 12 + 4 * entries.len();
        let mut body_abs = vec![0u8; base];
        let mut entry_offs = Vec::new();
        for (name, frames) in entries {
            let offs: Vec<u32> = frames.iter().map(|f| put_frame(&mut body_abs, f)).collect();
            entry_offs.push(body_abs.len() as u32);
            body_abs.extend_from_slice(&(frames.len() as u16).to_le_bytes());
            body_abs.extend_from_slice(&1u16.to_le_bytes());
            body_abs.extend_from_slice(&0u32.to_le_bytes());
            let mut n = name.as_bytes().to_vec();
            n.resize(32, 0);
            body_abs.extend_from_slice(&n);
            for o in offs {
                body_abs.extend_from_slice(&o.to_le_bytes());
                body_abs.extend_from_slice(&2u32.to_le_bytes());
            }
        }
        body.extend_from_slice(&0x0001_0100u32.to_le_bytes());
        body.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        for o in entry_offs {
            body.extend_from_slice(&o.to_le_bytes());
        }
        body.extend_from_slice(&body_abs[base..]);
        body
    }
}

#[cfg(test)]
mod tests {
    use super::build::{F, gaf};
    use super::*;

    fn pal() -> Palette {
        let mut p = Palette::grayscale();
        p.colors[1] = [255, 0, 0];
        p.colors[2] = [0, 255, 0];
        p
    }

    #[test]
    fn raw_indexed_uses_color_key() {
        let d = gaf(&[("Btn", vec![F::new(2, 2, 0, vec![1, 9, 2, 1])])]);
        let g = Gaf::parse(&d).unwrap();
        assert_eq!(g.version, 0x0001_0100);
        let e = g.entry("btn").unwrap();
        assert_eq!(e.flags, 1);
        assert_eq!(e.frames[0].duration, 2);
        let img = e.frames[0].frame.to_rgba(&pal());
        assert_eq!(&img.data[0..8], &[255, 0, 0, 255, 0, 0, 0, 0]);
        assert_eq!(&img.data[8..12], &[0, 255, 0, 255]);
    }

    #[test]
    fn rle_rows_skip_repeat_literal_and_blank() {
        // Row 0: skip 1, repeat value 2 twice, literal [1]. Row 1: blank.
        // Row 2: overlong repeat is clamped to the width.
        let rows: Vec<u8> = [
            vec![5, 0, 0x03, 0x06, 2, 0x00, 1],
            vec![0, 0],
            vec![2, 0, 0x3e, 1],
        ]
        .concat();
        let d = gaf(&[("a", vec![F::new(4, 3, 1, rows)])]);
        let g = Gaf::parse(&d).unwrap();
        let FrameContent::Indexed { index, opaque } = &g.entries[0].frames[0].frame.content else {
            panic!("indexed");
        };
        assert_eq!(&index[0..4], &[0, 2, 2, 1]);
        assert_eq!(&opaque[0..4], &[false, true, true, true]);
        assert!(opaque[4..8].iter().all(|o| !o));
        assert_eq!(&index[8..12], &[1, 1, 1, 1]);
    }

    #[test]
    fn sixteen_bit_formats_decode() {
        // 1555: opaque pure red, then transparent. 4444: half-alpha blue.
        let d = gaf(&[
            ("a", vec![F::new(2, 1, 5, vec![0x00, 0xfc, 0xff, 0x7f])]),
            ("b", vec![F::new(1, 1, 4, vec![0x0f, 0x80])]),
        ]);
        let g = Gaf::parse(&d).unwrap();
        let a = g.entries[0].frames[0].frame.to_rgba(&pal());
        assert_eq!(&a.data, &[255, 0, 0, 255, 255, 255, 255, 0]);
        let b = g.entries[1].frames[0].frame.to_rgba(&pal());
        assert_eq!(&b.data, &[0, 0, 255, 136]);
        assert_eq!(g.entries[1].frames[0].frame.format, PixelFormat::Argb4444);
    }

    #[test]
    fn composite_places_children_by_offsets() {
        let mut child = F::new(1, 1, 0, vec![1]);
        child.x = -1; // lands at parent.x - child.x = 1
        let mut parent = F::new(3, 1, 1, vec![]);
        parent.children = vec![F::new(1, 1, 0, vec![2]), child];
        let d = gaf(&[("c", vec![parent])]);
        let g = Gaf::parse(&d).unwrap();
        let f = &g.entries[0].frames[0].frame;
        let mut n = 0;
        f.walk(&mut |_, _| n += 1);
        assert_eq!(n, 3);
        let img = f.to_rgba(&pal());
        assert_eq!(&img.data[0..4], &[0, 255, 0, 255]);
        assert_eq!(&img.data[4..8], &[255, 0, 0, 255]);
        assert_eq!(img.data[11], 0);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert_eq!(Gaf::parse(&[]), Err(GafError::Empty));
        let d = gaf(&[("a", vec![F::new(2, 2, 7, vec![0; 4])])]);
        assert!(matches!(
            Gaf::parse(&d),
            Err(GafError::UnknownFormat { code: 7, .. })
        ));
        // Truncations at every length.
        let good = gaf(&[(
            "a",
            vec![
                F::new(2, 2, 0, vec![1; 4]),
                F::new(2, 1, 1, vec![1, 0, 0x02, 3]),
                F::new(1, 1, 5, vec![0, 0x80]),
            ],
        )]);
        Gaf::parse(&good).unwrap();
        for n in 0..good.len() {
            assert!(Gaf::parse(&good[..n]).is_err(), "len {n}");
        }
        // A huge entry count does not allocate.
        let mut d = good.clone();
        d[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Gaf::parse(&d).is_err());
        // A self-referencing subframe table hits the depth limit.
        let mut d = gaf(&[("a", vec![F::new(1, 1, 0, vec![0, 0, 0, 0])])]);
        let u32_at = |d: &[u8], o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let eo = u32_at(&d, 12) as usize;
        let fo = u32_at(&d, eo + 40);
        let data_off = u32_at(&d, fo as usize + 16) as usize;
        d[fo as usize + 10] = 1; // one subframe...
        d[data_off..data_off + 4].copy_from_slice(&fo.to_le_bytes()); // ...which is itself
        assert!(matches!(Gaf::parse(&d), Err(GafError::TooDeep { .. })));
    }

    /// Every sprite, PCX, palette and lookup table in the install decodes.
    /// Prints the counts that `docs/formats/gaf.md` quotes.
    #[test]
    fn corpus_every_sprite_and_palette_decodes() {
        use std::collections::BTreeMap;
        let Some(dir) =
            tak_vfs::testing::data_dir_or_skip("corpus_every_sprite_and_palette_decodes")
        else {
            return;
        };
        let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
        let mut files: BTreeMap<String, usize> = BTreeMap::new();
        let mut empty = Vec::new();
        let mut errors = Vec::new();
        let (mut entries, mut top, mut all) = (0usize, 0usize, 0usize);
        let mut formats: BTreeMap<(String, &str, usize), usize> = BTreeMap::new();
        let mut max_ratio = 0f64;
        for f in vfs.list("") {
            let ext = f.path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
            let Some(ext) = ext else { continue };
            let data = match ext.as_str() {
                "gaf" | "taf" | "gao" | "gao2" | "pcx" | "pal" | "alp" | "lht" | "shd" | "gry"
                | "blu" => vfs.read(&f.path).unwrap(),
                _ => continue,
            };
            *files.entry(ext.clone()).or_default() += 1;
            let res: Result<(), String> = match ext.as_str() {
                "gaf" | "taf" | "gao" | "gao2" => match Gaf::parse(&data) {
                    Err(GafError::Empty) => {
                        empty.push(f.display_path.clone());
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                    Ok(g) => {
                        let mut px = 0u64;
                        entries += g.entries.len();
                        for e in &g.entries {
                            for r in &e.frames {
                                top += 1;
                                r.frame.walk(&mut |fr, depth| {
                                    all += 1;
                                    if !matches!(fr.content, FrameContent::Composite(_)) {
                                        px += u64::from(fr.width) * u64::from(fr.height);
                                    }
                                    *formats
                                        .entry((ext.clone(), fr.format.name(), depth))
                                        .or_default() += 1;
                                });
                            }
                        }
                        max_ratio = max_ratio.max(px as f64 / data.len() as f64);
                        Ok(())
                    }
                },
                // A `.pal` is either a 1,024-byte RGBX palette or a PCX.
                "pal" if data.len() == palette::Palette::PAL_LEN => {
                    palette::Palette::from_pal(&data)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                }
                "pcx" | "pal" => match pcx::Pcx::parse(&data) {
                    Ok(p) if p.palette.is_some() => Ok(()),
                    Ok(_) => Err("no palette trailer".into()),
                    Err(e) => Err(e.to_string()),
                },
                "alp" => palette::BlendTable::parse(&data)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                "lht" | "shd" => palette::LevelTable::parse(&data)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                _ => palette::RemapTable::parse(&data)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
            };
            if let Err(e) = res {
                errors.push(format!("{}: {e}", f.display_path));
            }
        }
        eprintln!("files by extension: {files:?}");
        eprintln!("entries {entries}, top-level frames {top}, frames incl. subframes {all}");
        for ((ext, fmt, depth), n) in &formats {
            eprintln!("  {ext:5} {fmt:12} depth {depth}: {n}");
        }
        eprintln!("empty files: {empty:?}; max decoded pixels per byte {max_ratio:.1}");
        assert!(errors.is_empty(), "{} errors: {errors:#?}", errors.len());
        assert!(empty.len() <= 1, "unexpected empty sprite files: {empty:?}");
        assert!(files.get("gaf").copied().unwrap_or(0) >= 500);
        assert!(files.get("taf").copied().unwrap_or(0) >= 270);
    }

    #[test]
    fn huge_rle_frame_hits_pixel_budget() {
        // 65535 × 2 blank rows from a few bytes would be 131k pixels: fine.
        // Many frames sharing one big block exhaust the budget instead.
        let rows = vec![0u8; 2 * 600];
        let mut frames = Vec::new();
        for _ in 0..40 {
            frames.push(F::new(u16::MAX, 600, 1, rows.clone()));
        }
        let d = gaf(&[("a", frames)]);
        assert!(matches!(Gaf::parse(&d), Err(GafError::PixelBudget { .. })));
    }
}
