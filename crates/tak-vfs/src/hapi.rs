//! HAPI version 2 archives (`.hpi`, `.kmp`), as used by TA: Kingdoms.
//!
//! See `docs/formats/hapi.md`. Layout:
//!
//! ```text
//! [32-byte header][file data][names block][directory block][trailer]
//! ```
//!
//! The names and directory blocks are either raw or a single obfuscated,
//! zlib-compressed `SQSH` chunk. Each file is either stored raw or as one
//! `SQSH` chunk. Every offset and size read from the file is bounds-checked;
//! malformed input yields an error, never a panic.

use crate::source::Source;
use flate2::read::ZlibDecoder;
use std::collections::HashSet;
use std::io::Read;

pub const MAGIC: [u8; 4] = *b"HAPI";
pub const VERSION_2: u32 = 0x0002_0000;
pub const HEADER_LEN: usize = 32;
const SQSH_MAGIC: [u8; 4] = *b"SQSH";
const SQSH_HEADER_LEN: usize = 19;
const DIR_ENTRY_LEN: usize = 20;
const FILE_ENTRY_LEN: usize = 24;
const MAX_DEPTH: usize = 64;
/// Longest path accepted, in bytes.
const MAX_PATH_LEN: usize = 1024;
/// zlib cannot expand data by more than about 1032:1.
const MAX_ZLIB_RATIO: u64 = 1032;

#[derive(Debug, thiserror::Error)]
pub enum HapiError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a HAPI archive (magic {0:02x?})")]
    BadMagic([u8; 4]),
    #[error("unsupported HAPI version 0x{0:08x}")]
    UnsupportedVersion(u32),
    #[error("{what} out of bounds (offset {offset}, len {len}, limit {limit})")]
    OutOfBounds {
        what: &'static str,
        offset: u64,
        len: u64,
        limit: u64,
    },
    #[error("bad SQSH chunk at {offset}: {reason}")]
    BadChunk { offset: u64, reason: String },
    #[error("SQSH compression {0} is not supported")]
    UnsupportedCompression(u8),
    #[error("directory is malformed: {0}")]
    BadDirectory(String),
    #[error("file {path}: decompressed {got} bytes, expected {expected}")]
    SizeMismatch {
        path: String,
        got: u64,
        expected: u64,
    },
}

pub type Result<T> = std::result::Result<T, HapiError>;

/// The fixed header fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub dir_offset: u32,
    pub dir_size: u32,
    pub names_offset: u32,
    pub names_size: u32,
    /// Always 32 in the GOG install (start of file data). Not relied on.
    pub data_start: u32,
    /// Meaning unknown; see docs/formats/hapi.md.
    pub unknown_28: u32,
}

/// One file in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path inside the archive with original case, `/`-separated, no leading slash.
    pub path: String,
    pub data_offset: u32,
    /// Decompressed size.
    pub size: u32,
    /// Size of the stored SQSH chunk, or 0 when the file is stored raw.
    pub stored_size: u32,
    /// Unix modification time.
    pub timestamp: u32,
    /// Algorithm unknown; not verified.
    pub checksum: u32,
}

impl Entry {
    pub fn is_compressed(&self) -> bool {
        self.stored_size != 0
    }
}

/// An opened archive. Holds the directory in memory; file data is read from
/// the source on demand.
pub struct Archive<S> {
    source: S,
    len: u64,
    header: Header,
    entries: Vec<Entry>,
}

impl<S: Source> Archive<S> {
    pub fn open(source: S) -> Result<Self> {
        let len = source.len()?;
        let mut h = [0u8; HEADER_LEN];
        read_checked(&source, len, 0, &mut h, "header")?;
        let magic: [u8; 4] = h[..4].try_into().expect("4");
        if magic != MAGIC {
            return Err(HapiError::BadMagic(magic));
        }
        let w = |i: usize| u32::from_le_bytes(h[i..i + 4].try_into().expect("4"));
        let header = Header {
            version: w(4),
            dir_offset: w(8),
            dir_size: w(12),
            names_offset: w(16),
            names_size: w(20),
            data_start: w(24),
            unknown_28: w(28),
        };
        if header.version != VERSION_2 {
            return Err(HapiError::UnsupportedVersion(header.version));
        }
        let names = read_block(
            &source,
            len,
            header.names_offset,
            header.names_size,
            "names",
        )?;
        let dir = read_block(
            &source,
            len,
            header.dir_offset,
            header.dir_size,
            "directory",
        )?;
        let entries = parse_directory(&dir, &names)?;
        Ok(Archive {
            source,
            len,
            header,
            entries,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Files in directory order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Reads and decompresses one file.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>> {
        let out = if entry.is_compressed() {
            let (data, used) = read_sqsh(&self.source, self.len, u64::from(entry.data_offset))?;
            if used != u64::from(entry.stored_size) {
                return Err(HapiError::BadChunk {
                    offset: entry.data_offset.into(),
                    reason: format!(
                        "chunk is {used} bytes, directory says {}",
                        entry.stored_size
                    ),
                });
            }
            data
        } else {
            let mut buf = vec![0u8; checked_alloc(entry.size.into(), self.len, "file")?];
            read_checked(
                &self.source,
                self.len,
                entry.data_offset.into(),
                &mut buf,
                "file data",
            )?;
            buf
        };
        if out.len() as u64 != u64::from(entry.size) {
            return Err(HapiError::SizeMismatch {
                path: entry.path.clone(),
                got: out.len() as u64,
                expected: entry.size.into(),
            });
        }
        Ok(out)
    }
}

fn checked_alloc(want: u64, limit: u64, what: &'static str) -> Result<usize> {
    if want > limit {
        return Err(HapiError::OutOfBounds {
            what,
            offset: 0,
            len: want,
            limit,
        });
    }
    usize::try_from(want).map_err(|_| HapiError::OutOfBounds {
        what,
        offset: 0,
        len: want,
        limit,
    })
}

fn read_checked<S: Source>(
    src: &S,
    len: u64,
    offset: u64,
    buf: &mut [u8],
    what: &'static str,
) -> Result<()> {
    let n = buf.len() as u64;
    if offset.checked_add(n).is_none_or(|end| end > len) {
        return Err(HapiError::OutOfBounds {
            what,
            offset,
            len: n,
            limit: len,
        });
    }
    src.read_exact_at(offset, buf)?;
    Ok(())
}

/// Reads the names or directory block: an SQSH chunk if it starts with
/// `SQSH`, otherwise `size` raw bytes (clamped to the file end, because raw
/// names blocks can declare a few bytes too many).
fn read_block<S: Source>(
    src: &S,
    len: u64,
    offset: u32,
    size: u32,
    what: &'static str,
) -> Result<Vec<u8>> {
    let offset = u64::from(offset);
    let mut magic = [0u8; 4];
    read_checked(src, len, offset, &mut magic, what)?;
    if magic == SQSH_MAGIC {
        return Ok(read_sqsh(src, len, offset)?.0);
    }
    let size = u64::from(size).min(len - offset);
    let mut buf = vec![0u8; checked_alloc(size, len, what)?];
    read_checked(src, len, offset, &mut buf, what)?;
    Ok(buf)
}

/// Undoes the SQSH obfuscation: `plain[i] = (stored[i] - i) ^ i` (u8, wrapping).
pub fn deobfuscate(buf: &mut [u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        let k = i as u8;
        *b = b.wrapping_sub(k) ^ k;
    }
}

/// Inverse of [`deobfuscate`], for building test archives.
pub fn obfuscate(buf: &mut [u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        let k = i as u8;
        *b = (*b ^ k).wrapping_add(k);
    }
}

/// Reads one SQSH chunk; returns the decoded bytes and the chunk's total size.
fn read_sqsh<S: Source>(src: &S, len: u64, offset: u64) -> Result<(Vec<u8>, u64)> {
    let bad = |reason: String| HapiError::BadChunk { offset, reason };
    let mut h = [0u8; SQSH_HEADER_LEN];
    read_checked(src, len, offset, &mut h, "SQSH header")?;
    if h[..4] != SQSH_MAGIC {
        return Err(bad("missing SQSH magic".into()));
    }
    let (compression, obfuscated) = (h[5], h[6] != 0);
    let w = |i: usize| u32::from_le_bytes(h[i..i + 4].try_into().expect("4"));
    let (stored, size, checksum) = (u64::from(w(7)), u64::from(w(11)), w(15));

    let mut payload = vec![0u8; checked_alloc(stored, len, "SQSH payload")?];
    read_checked(
        src,
        len,
        offset + SQSH_HEADER_LEN as u64,
        &mut payload,
        "SQSH payload",
    )?;
    let sum = payload.iter().fold(0u32, |a, &b| a.wrapping_add(b.into()));
    if sum != checksum {
        return Err(bad(format!("checksum {sum:#010x} != {checksum:#010x}")));
    }
    if obfuscated {
        deobfuscate(&mut payload);
    }
    let out = match compression {
        0 => payload,
        2 => {
            if size > stored.saturating_mul(MAX_ZLIB_RATIO) {
                return Err(bad(format!("claims {size} bytes from {stored}")));
            }
            // Never trust the claimed size for the up-front reservation; let
            // the buffer grow as real output arrives (still capped by take()).
            let mut out = Vec::with_capacity(
                usize::try_from(size.min(stored.saturating_mul(4)).min(1 << 20)).unwrap_or(0),
            );
            ZlibDecoder::new(&payload[..])
                .take(size + 1)
                .read_to_end(&mut out)
                .map_err(|e| bad(format!("zlib: {e}")))?;
            out
        }
        c => return Err(HapiError::UnsupportedCompression(c)),
    };
    if out.len() as u64 != size {
        return Err(bad(format!(
            "decoded {} bytes, header says {size}",
            out.len()
        )));
    }
    Ok((out, SQSH_HEADER_LEN as u64 + stored))
}

fn u32_at(buf: &[u8], off: usize) -> Option<u32> {
    buf.get(off..off.checked_add(4)?)
        .map(|b| u32::from_le_bytes(b.try_into().expect("4")))
}

fn name_at(names: &[u8], off: u32) -> Result<String> {
    let start = off as usize;
    let rest = names
        .get(start..)
        .ok_or_else(|| HapiError::BadDirectory(format!("name offset {off} past names block")))?;
    let end = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| HapiError::BadDirectory(format!("name at {off} is not terminated")))?;
    // Names are Latin-1.
    Ok(rest[..end].iter().map(|&b| b as char).collect())
}

/// Directory entry: name, first subdir, subdir count, first file, file count.
/// File entry: name, data offset, size, stored size, timestamp, checksum.
fn parse_directory(dir: &[u8], names: &[u8]) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut seen = Seen::default();
    walk(dir, names, 0, "", 0, &mut seen, &mut entries)?;
    Ok(entries)
}

/// Records already visited, so a crafted directory cannot reuse entries to
/// multiply the output: each directory and each file record is used once,
/// which bounds the total entries by the directory size.
#[derive(Default)]
struct Seen {
    dirs: HashSet<usize>,
    files: HashSet<usize>,
}

fn walk(
    dir: &[u8],
    names: &[u8],
    at: usize,
    parent: &str,
    depth: usize,
    seen: &mut Seen,
    out: &mut Vec<Entry>,
) -> Result<()> {
    let bad = |m: String| HapiError::BadDirectory(m);
    if depth > MAX_DEPTH {
        return Err(bad("directory nesting too deep".into()));
    }
    if !seen.dirs.insert(at) {
        return Err(bad(format!("directory entry at {at} visited twice")));
    }
    let f = |i: usize| {
        u32_at(dir, at + 4 * i).ok_or_else(|| bad(format!("dir entry at {at} truncated")))
    };
    let (name_off, sub_at, sub_n, file_at, file_n) = (f(0)?, f(1)?, f(2)?, f(3)?, f(4)?);
    let name = name_at(names, name_off)?;
    let path = match (parent.is_empty(), name.is_empty()) {
        (_, true) => parent.to_string(),
        (true, false) => name,
        (false, false) => format!("{parent}/{name}"),
    };
    if path.len() > MAX_PATH_LEN {
        return Err(bad(format!("path longer than {MAX_PATH_LEN} bytes")));
    }
    let span = |start: u32, n: u32, size: usize| -> Result<()> {
        let end = (start as usize).checked_add(
            (n as usize)
                .checked_mul(size)
                .ok_or_else(|| bad("count overflow".into()))?,
        );
        match end {
            Some(e) if e <= dir.len() => Ok(()),
            _ => Err(bad(format!(
                "{n} entries at {start} exceed directory ({} bytes)",
                dir.len()
            ))),
        }
    };
    span(file_at, file_n, FILE_ENTRY_LEN)?;
    span(sub_at, sub_n, DIR_ENTRY_LEN)?;
    for i in 0..file_n as usize {
        let base = file_at as usize + i * FILE_ENTRY_LEN;
        if !seen.files.insert(base) {
            return Err(bad(format!("file record at {base} listed twice")));
        }
        let g = |k: usize| u32_at(dir, base + 4 * k).expect("span checked");
        let fname = name_at(names, g(0))?;
        if fname.is_empty() {
            return Err(bad(format!("file record at {base} has an empty name")));
        }
        if path.len() + fname.len() >= MAX_PATH_LEN {
            return Err(bad(format!("path longer than {MAX_PATH_LEN} bytes")));
        }
        out.push(Entry {
            path: if path.is_empty() {
                fname
            } else {
                format!("{path}/{fname}")
            },
            data_offset: g(1),
            size: g(2),
            stored_size: g(3),
            timestamp: g(4),
            checksum: g(5),
        });
    }
    for i in 0..sub_n as usize {
        walk(
            dir,
            names,
            sub_at as usize + i * DIR_ENTRY_LEN,
            &path,
            depth + 1,
            seen,
            out,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::HapiBuilder;

    fn sample() -> HapiBuilder {
        HapiBuilder::new()
            .file(
                "units/ARAMON/monarch.fbi",
                b"[UNITINFO]\r\n{\r\n}\r\n",
                true,
            )
            .file("units/zhon/beast.fbi", &[7u8; 5000], true)
            .file(
                "anims/logo.gaf",
                &(0..=255u8).cycle().take(70_000).collect::<Vec<_>>(),
                true,
            )
            .file("readme.txt", b"stored raw", false)
            .file("empty.txt", b"", false)
    }

    #[test]
    fn obfuscation_round_trips() {
        let orig: Vec<u8> = (0..600u32).map(|i| (i * 7) as u8).collect();
        let mut b = orig.clone();
        obfuscate(&mut b);
        assert_ne!(b, orig);
        deobfuscate(&mut b);
        assert_eq!(b, orig);
        // The property that let us spot it: 78 9c becomes 78 9e.
        let mut z = [0x78, 0x9c];
        obfuscate(&mut z);
        assert_eq!(z, [0x78, 0x9e]);
    }

    #[test]
    fn reads_every_layout_variant() {
        for (rn, rd) in [(false, false), (true, false), (false, true), (true, true)] {
            let b = sample().raw_names(rn).raw_dir(rd).build();
            let a = Archive::open(b).unwrap();
            let mut paths: Vec<_> = a.entries().iter().map(|e| e.path.as_str()).collect();
            paths.sort();
            assert_eq!(
                paths,
                [
                    "anims/logo.gaf",
                    "empty.txt",
                    "readme.txt",
                    "units/ARAMON/monarch.fbi",
                    "units/zhon/beast.fbi"
                ]
            );
            for e in a.entries() {
                let data = a.read(e).unwrap();
                assert_eq!(data.len() as u32, e.size, "{}", e.path);
            }
            let raw = a.entries().iter().find(|e| e.path == "readme.txt").unwrap();
            assert!(!raw.is_compressed());
            assert_eq!(a.read(raw).unwrap(), b"stored raw");
        }
    }

    #[test]
    fn rejects_bad_magic_and_version() {
        let mut b = sample().build();
        b[0] = b'X';
        assert!(matches!(Archive::open(b), Err(HapiError::BadMagic(_))));
        let mut b = sample().build();
        b[4..8].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        assert!(matches!(
            Archive::open(b),
            Err(HapiError::UnsupportedVersion(0x0001_0000))
        ));
    }

    #[test]
    fn detects_corrupt_chunk_checksum() {
        let b = sample().build();
        let a = Archive::open(b.clone()).unwrap();
        let e = a
            .entries()
            .iter()
            .find(|e| e.path.ends_with("beast.fbi"))
            .unwrap()
            .clone();
        let mut b2 = b;
        b2[e.data_offset as usize + 19] ^= 0xff;
        let a2 = Archive::open(b2).unwrap();
        assert!(matches!(a2.read(&e), Err(HapiError::BadChunk { .. })));
    }

    #[test]
    fn rejects_directory_cycles() {
        // Root's single subdir points back at the root entry.
        let mut b = HapiBuilder::new()
            .file("d/x", b"1", false)
            .raw_dir(true)
            .raw_names(true)
            .build();
        let dir_off = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        // Subdir entry lives at dir offset 20; its "first subdir" word -> 0, count 1.
        b[dir_off + 20 + 4..dir_off + 20 + 8].copy_from_slice(&0u32.to_le_bytes());
        b[dir_off + 20 + 8..dir_off + 20 + 12].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(Archive::open(b), Err(HapiError::BadDirectory(_))));
    }

    /// Every truncation and many single-byte corruptions: errors, never panics.
    #[test]
    fn malformed_input_never_panics() {
        let good = sample().build();
        for cut in 0..good.len() {
            if let Ok(a) = Archive::open(good[..cut].to_vec()) {
                for e in a.entries().to_vec() {
                    let _ = a.read(&e);
                }
            }
        }
        let mut x: u32 = 0x1234_5678;
        for _ in 0..3000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let mut b = good.clone();
            let i = x as usize % b.len();
            b[i] = (x >> 8) as u8;
            if let Ok(a) = Archive::open(b) {
                for e in a.entries().to_vec() {
                    let _ = a.read(&e);
                }
            }
        }
    }

    /// Two directories pointing at the same file records (review finding).
    #[test]
    fn rejects_shared_file_ranges() {
        let mut b = HapiBuilder::new()
            .file("a/x", b"1", false)
            .file("b/y", b"2", false)
            .raw_dir(true)
            .raw_names(true)
            .build();
        let d = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        // Root's subdirs are at 20 ("a") and 40 ("b"); point b's files at a's.
        let a_files = b[d + 20 + 12..d + 20 + 16].to_vec();
        b[d + 40 + 12..d + 40 + 16].copy_from_slice(&a_files);
        assert!(matches!(Archive::open(b), Err(HapiError::BadDirectory(_))));
    }

    /// A chunk claiming 4 GB of output must fail without reserving it.
    #[test]
    fn oversized_chunk_claim_fails_cheaply() {
        let b = HapiBuilder::new()
            .file("big", &[0u8; 100_000], true)
            .build();
        let a = Archive::open(b.clone()).unwrap();
        let e = a.entries()[0].clone();
        let mut b2 = b;
        let size_at = e.data_offset as usize + 11;
        b2[size_at..size_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let a2 = Archive::open(b2).unwrap();
        assert!(matches!(a2.read(&e), Err(HapiError::BadChunk { .. })));
    }

    #[test]
    fn rejects_absurd_sizes_without_allocating() {
        let mut b = sample().build();
        let a = Archive::open(b.clone()).unwrap();
        let e = a
            .entries()
            .iter()
            .find(|e| e.path == "readme.txt")
            .unwrap()
            .clone();
        let huge = Entry {
            size: u32::MAX,
            ..e
        };
        assert!(a.read(&huge).is_err());
        // Root directory claiming four billion files.
        b = sample().raw_dir(true).build();
        let d = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        b[d + 16..d + 20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(Archive::open(b), Err(HapiError::BadDirectory(_))));
    }
}
