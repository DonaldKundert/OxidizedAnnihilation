//! CRT: the per-map companion file (`<map>.crt`). See `docs/formats/crt.md`.
//!
//! Every file starts with three words: the float `1.0`, a small integer, and
//! a count. 199 of 214 retail files are the same 56-byte "empty" record
//! (count 0). The other 15 carry pre-placed objects: the count is followed
//! by records that begin with a unit name (`ZONHUNT`, `VERPULT`, `ARAKEEP`).
//! The record layout is not understood yet, so the body is kept raw.

use crate::reader::{ReadError, Reader};

/// IEEE bits of `1.0f32`, the first word of every retail CRT.
pub const CRT_MAGIC: u32 = 0x3F80_0000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CrtError {
    #[error(transparent)]
    Read(#[from] ReadError),
    #[error("not a CRT: first word {0:#010x} (expected 0x3f800000, f32 1.0)")]
    BadMagic(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crt {
    /// Word at offset 4: `0` in 213 files, `1` in one (with a different body).
    pub kind: u32,
    /// Word at offset 8: number of placed objects (0 in empty files). Only
    /// meaningful when `kind == 0`.
    pub count: u32,
    /// Everything after the 12-byte header.
    pub body: Vec<u8>,
}

impl Crt {
    /// True for the 56-byte placeholder shared by most maps.
    pub fn is_empty(&self) -> bool {
        self.kind == 0 && self.count == 0
    }

    /// The NUL-terminated name at the start of the body (the first record's
    /// unit name in files with objects), if it is plain ASCII.
    pub fn first_name(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let s = Reader::new(&self.body).cstr("crt name").ok()?;
        let ok = !s.is_empty()
            && s.len() <= 32
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        ok.then_some(s)
    }
}

pub fn parse(data: &[u8]) -> Result<Crt, CrtError> {
    let mut r = Reader::new(data);
    let magic = r.u32("crt magic")?;
    if magic != CRT_MAGIC {
        return Err(CrtError::BadMagic(magic));
    }
    let kind = r.u32("crt kind")?;
    let count = r.u32("crt count")?;
    let body = r.bytes(r.remaining(), "crt body")?.to_vec();
    Ok(Crt { kind, count, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(kind: u32, count: u32) -> Vec<u8> {
        [CRT_MAGIC, kind, count]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }

    #[test]
    fn parses_empty_placeholder() {
        let mut d = header(0, 0);
        d.extend_from_slice(&9u32.to_le_bytes());
        d.resize(56, 0);
        let c = parse(&d).unwrap();
        assert!(c.is_empty());
        assert_eq!(c.body.len(), 44);
        assert_eq!(c.first_name(), None);
    }

    #[test]
    fn reads_first_object_name() {
        let mut d = header(0, 2);
        d.extend_from_slice(b"ZONHUNT\0");
        d.extend_from_slice(&[1, 2, 3, 4]);
        let c = parse(&d).unwrap();
        assert!(!c.is_empty());
        assert_eq!(c.first_name().as_deref(), Some("ZONHUNT"));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(parse(&[0, 0]), Err(CrtError::Read(_))));
        assert!(matches!(parse(&[0u8; 12]), Err(CrtError::BadMagic(0))));
        let mut d = header(0, 1);
        d.push(0xFF);
        assert_eq!(parse(&d).unwrap().first_name(), None);
    }

    #[test]
    fn corpus_crt_files_parse() {
        let Some(dir) = tak_vfs::testing::data_dir_or_skip("corpus_crt_files_parse") else {
            return;
        };
        let vfs = tak_vfs::open_kingdoms(&dir).expect("mount install");
        let mut n = 0;
        let mut empty = 0;
        for f in vfs.list("") {
            if !f.path.to_ascii_lowercase().ends_with(".crt") {
                continue;
            }
            let c = parse(&vfs.read(&f.path).expect("read"))
                .unwrap_or_else(|e| panic!("{}: {e}", f.path));
            n += 1;
            if c.is_empty() {
                empty += 1;
            }
        }
        assert_eq!(n, 214);
        assert_eq!(
            empty, 202,
            "199 56-byte placeholders + 3 longer files with count 0"
        );
    }
}
