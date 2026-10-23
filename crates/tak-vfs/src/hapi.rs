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

use crate::source::{Source, read_to_end_bounded};
use flate2::read::ZlibDecoder;
use std::collections::HashSet;

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
const DEFAULT_STORED_OVERHEAD_BYTES: u64 = 1 << 20;

/// Resource policy, not a format limit. Applies separately to each archive and
/// each read; callers must account for aggregate/nested-archive usage themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum decoded bytes returned by a file read (including raw files).
    pub max_file_bytes: u64,
    /// Maximum decoded bytes in each names or directory block.
    pub max_metadata_bytes: u64,
    /// Maximum stored bytes in a single SQSH chunk, including its header.
    pub max_stored_bytes: u64,
    /// Maximum excess of stored SQSH bytes (including its header) over the
    /// declared decoded size; allows framing but bounds oversized/junk payloads.
    pub max_stored_overhead_bytes: u64,
    /// Maximum combined directory and file records visited during indexing.
    pub max_directory_records: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 512 << 20,
            max_metadata_bytes: 16 << 20,
            max_stored_bytes: (512 << 20) + DEFAULT_STORED_OVERHEAD_BYTES,
            max_stored_overhead_bytes: DEFAULT_STORED_OVERHEAD_BYTES,
            max_directory_records: 65_536,
        }
    }
}

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
    #[error("{what} requires {size} bytes/records, limit is {limit}")]
    LimitExceeded {
        what: &'static str,
        size: u64,
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
    limits: Limits,
}

impl<S: Source> Archive<S> {
    pub fn open(source: S) -> Result<Self> {
        Self::open_with_limits(source, Limits::default())
    }

    /// Opens an archive with an explicit resource policy. Nested archives are
    /// independent: pass the policy again and maintain any aggregate budget.
    pub fn open_with_limits(source: S, limits: Limits) -> Result<Self> {
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
            limits,
        )?;
        let dir = read_block(
            &source,
            len,
            header.dir_offset,
            header.dir_size,
            "directory",
            limits,
        )?;
        let entries = parse_directory(&dir, &names, limits.max_directory_records)?;
        Ok(Archive {
            source,
            len,
            header,
            entries,
            limits,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Files in directory order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Reads and decompresses one file, subject to the archive's resource policy.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>> {
        self.read_bounded(entry, self.limits.max_file_bytes)
    }

    /// Reads at most `max_bytes` decoded bytes, also honoring the archive policy.
    /// Rejects oversize directory/header claims before reading the payload.
    /// Stored chunks are additionally capped by the archive's stored-byte and
    /// stored-overhead limits (defaults: 513 MiB and decoded size + 1 MiB).
    pub fn read_bounded(&self, entry: &Entry, max_bytes: u64) -> Result<Vec<u8>> {
        let max_bytes = max_bytes.min(self.limits.max_file_bytes);
        check_limit(u64::from(entry.size), max_bytes, "file decoded bytes")?;
        if entry.is_compressed() {
            read_sqsh(
                &self.source,
                self.len,
                u64::from(entry.data_offset),
                ChunkBounds {
                    stored: u64::from(entry.stored_size),
                    decoded: Some(u64::from(entry.size)),
                    max_decoded: max_bytes,
                    max_stored: self.limits.max_stored_bytes,
                    max_overhead: self.limits.max_stored_overhead_bytes,
                },
            )
        } else {
            let offset = u64::from(entry.data_offset);
            let size = u64::from(entry.size);
            check_range(self.len, offset, size, "file data")?;
            let mut buf = zeroed(size, "file data")?;
            self.source.read_exact_at(offset, &mut buf)?;
            Ok(buf)
        }
    }
}

fn check_limit(size: u64, limit: u64, what: &'static str) -> Result<()> {
    if size > limit {
        return Err(HapiError::LimitExceeded { what, size, limit });
    }
    Ok(())
}

fn zeroed(size: u64, what: &'static str) -> Result<Vec<u8>> {
    let count = checked_alloc(size, isize::MAX as u64, what)?;
    let mut out = Vec::new();
    out.try_reserve_exact(count)
        .map_err(std::io::Error::other)?;
    out.resize(count, 0);
    Ok(out)
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
    check_range(len, offset, buf.len() as u64, what)?;
    src.read_exact_at(offset, buf)?;
    Ok(())
}

fn check_range(len: u64, offset: u64, size: u64, what: &'static str) -> Result<()> {
    if offset.checked_add(size).is_none_or(|end| end > len) {
        return Err(HapiError::OutOfBounds {
            what,
            offset,
            len: size,
            limit: len,
        });
    }
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
    limits: Limits,
) -> Result<Vec<u8>> {
    let offset = u64::from(offset);
    let mut magic = [0u8; 4];
    read_checked(src, len, offset, &mut magic, what)?;
    if magic == SQSH_MAGIC {
        return read_sqsh(
            src,
            len,
            offset,
            ChunkBounds {
                stored: u64::from(size),
                decoded: None,
                max_decoded: limits.max_metadata_bytes,
                max_stored: limits.max_stored_bytes,
                max_overhead: limits.max_stored_overhead_bytes,
            },
        );
    }
    let size = u64::from(size).min(len - offset);
    check_limit(size, limits.max_metadata_bytes, what)?;
    let mut buf = zeroed(size, what)?;
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

struct ChunkBounds {
    /// Authoritative size from the archive header or directory record.
    stored: u64,
    decoded: Option<u64>,
    max_decoded: u64,
    max_stored: u64,
    max_overhead: u64,
}

/// Validates all header claims before allocating/reading the stored payload.
fn read_sqsh<S: Source>(src: &S, len: u64, offset: u64, bounds: ChunkBounds) -> Result<Vec<u8>> {
    let bad = |reason: String| HapiError::BadChunk { offset, reason };
    if bounds.stored < SQSH_HEADER_LEN as u64 {
        return Err(bad("declared chunk is shorter than its header".into()));
    }
    let mut h = [0u8; SQSH_HEADER_LEN];
    read_checked(src, len, offset, &mut h, "SQSH header")?;
    if h[..4] != SQSH_MAGIC {
        return Err(bad("missing SQSH magic".into()));
    }
    let (compression, obfuscated) = (h[5], h[6] != 0);
    let w = |i: usize| u32::from_le_bytes(h[i..i + 4].try_into().expect("4"));
    let (stored, size, checksum) = (u64::from(w(7)), u64::from(w(11)), w(15));
    let total = SQSH_HEADER_LEN as u64 + stored;
    if total != bounds.stored {
        return Err(bad(format!(
            "chunk is {total} bytes, directory/header says {}",
            bounds.stored
        )));
    }
    if let Some(expected) = bounds.decoded {
        if size != expected {
            return Err(bad(format!(
                "header claims {size} decoded bytes, directory says {expected}"
            )));
        }
    }
    check_limit(size, bounds.max_decoded, "SQSH decoded bytes")?;
    check_limit(
        total,
        bounds
            .max_stored
            .min(size.saturating_add(bounds.max_overhead)),
        "SQSH stored bytes",
    )?;
    check_range(len, offset, total, "SQSH chunk")?;
    match compression {
        0 if size != stored => {
            return Err(bad(format!(
                "raw payload is {stored} bytes, header says {size}"
            )));
        }
        0 => (),
        2 if size > stored.saturating_mul(MAX_ZLIB_RATIO) => {
            return Err(bad(format!("claims {size} bytes from {stored}")));
        }
        2 => (),
        c => return Err(HapiError::UnsupportedCompression(c)),
    }
    let mut payload = zeroed(stored, "SQSH payload")?;
    src.read_exact_at(offset + SQSH_HEADER_LEN as u64, &mut payload)?;
    let sum = payload.iter().fold(0u32, |a, &b| a.wrapping_add(b.into()));
    if sum != checksum {
        return Err(bad(format!("checksum {sum:#010x} != {checksum:#010x}")));
    }
    if obfuscated {
        deobfuscate(&mut payload);
    }
    let out = if compression == 0 {
        payload
    } else {
        // Grow only as real output arrives. A one-byte stack probe detects an
        // overrun without appending it or growing the vector past the budget.
        let mut decoder = ZlibDecoder::new(&payload[..]);
        let (out, exceeded) =
            read_to_end_bounded(&mut decoder, size).map_err(|e| bad(format!("zlib: {e}")))?;
        if exceeded {
            return Err(bad(format!("decoded more than the declared {size} bytes")));
        }
        out
    };
    if out.len() as u64 != size {
        return Err(bad(format!(
            "decoded {} bytes, header says {size}",
            out.len()
        )));
    }
    Ok(out)
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
    let end = rest[..rest.len().min(MAX_PATH_LEN + 1)]
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| HapiError::BadDirectory(format!("name at {off} is not terminated")))?;
    // Check UTF-8 expansion before collecting a Latin-1 name into a String.
    let utf8_len = end + rest[..end].iter().filter(|&&b| b >= 128).count();
    if utf8_len > MAX_PATH_LEN {
        return Err(HapiError::BadDirectory(format!(
            "name longer than {MAX_PATH_LEN} bytes"
        )));
    }
    // Names are Latin-1.
    Ok(rest[..end].iter().map(|&b| b as char).collect())
}

/// Directory entry: name, first subdir, subdir count, first file, file count.
/// File entry: name, data offset, size, stored size, timestamp, checksum.
fn parse_directory(dir: &[u8], names: &[u8], max_records: usize) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut seen = Seen {
        max_records,
        ..Seen::default()
    };
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
    max_records: usize,
}

impl Seen {
    fn check_budget(&self) -> Result<()> {
        check_limit(
            (self.dirs.len() + self.files.len()) as u64 + 1,
            self.max_records as u64,
            "directory records",
        )
    }
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
    seen.check_budget()?;
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
        seen.check_budget()?;
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

    #[derive(Default)]
    struct TrackingSource {
        bytes: Vec<u8>,
        reads: std::cell::RefCell<Vec<(u64, usize)>>,
    }

    impl Source for TrackingSource {
        fn len(&self) -> std::io::Result<u64> {
            Ok(self.bytes.len() as u64)
        }

        fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
            self.reads.borrow_mut().push((offset, buf.len()));
            self.bytes.read_exact_at(offset, buf)
        }
    }

    #[test]
    fn file_header_mismatch_rejects_before_payload_read() {
        let bytes = HapiBuilder::new().file("bomb", &[0; 100_000], true).build();
        let archive = Archive::open(TrackingSource {
            bytes,
            ..TrackingSource::default()
        })
        .unwrap();
        let mut entry = archive.entries()[0].clone();
        entry.size = 1;
        archive.source.reads.borrow_mut().clear();
        let err = archive.read(&entry).unwrap_err();
        assert!(
            matches!(err, HapiError::BadChunk { ref reason, .. } if reason.contains("directory says 1"))
        );
        assert_eq!(
            *archive.source.reads.borrow(),
            [(u64::from(entry.data_offset), SQSH_HEADER_LEN)]
        );
    }

    #[test]
    fn file_stored_size_mismatch_rejects_before_payload_read() {
        let bytes = HapiBuilder::new().file("x", b"data", true).build();
        let archive = Archive::open(TrackingSource {
            bytes,
            ..TrackingSource::default()
        })
        .unwrap();
        let mut entry = archive.entries()[0].clone();
        entry.stored_size += 1;
        archive.source.reads.borrow_mut().clear();
        assert!(matches!(
            archive.read(&entry),
            Err(HapiError::BadChunk { .. })
        ));
        assert_eq!(
            *archive.source.reads.borrow(),
            [(u64::from(entry.data_offset), SQSH_HEADER_LEN)]
        );
    }

    #[test]
    fn bounded_file_read_checks_claim_before_header_read() {
        for compressed in [false, true] {
            let bytes = HapiBuilder::new().file("x", b"data", compressed).build();
            let archive = Archive::open(TrackingSource {
                bytes,
                ..TrackingSource::default()
            })
            .unwrap();
            let entry = &archive.entries()[0];
            assert_eq!(archive.read_bounded(entry, 4).unwrap(), b"data");
            archive.source.reads.borrow_mut().clear();
            assert!(matches!(
                archive.read_bounded(entry, 3),
                Err(HapiError::LimitExceeded {
                    size: 4,
                    limit: 3,
                    ..
                })
            ));
            assert!(archive.source.reads.borrow().is_empty());
        }
    }

    #[test]
    fn stored_payload_budget_is_checked_before_payload_read() {
        let bytes = HapiBuilder::new().file("x", &[0; 2000], true).build();
        let mut archive = Archive::open(TrackingSource {
            bytes,
            ..TrackingSource::default()
        })
        .unwrap();
        let entry = archive.entries()[0].clone();
        archive.limits.max_stored_bytes = u64::from(entry.stored_size) - 1;
        archive.source.reads.borrow_mut().clear();
        assert!(matches!(
            archive.read(&entry),
            Err(HapiError::LimitExceeded {
                what: "SQSH stored bytes",
                ..
            })
        ));
        assert_eq!(
            *archive.source.reads.borrow(),
            [(u64::from(entry.data_offset), SQSH_HEADER_LEN)]
        );
    }

    #[test]
    fn stored_overhead_budget_prevents_tiny_output_huge_input() {
        let bytes = HapiBuilder::new().file("x", b"x", true).build();
        let mut archive = Archive::open(TrackingSource {
            bytes,
            ..TrackingSource::default()
        })
        .unwrap();
        let entry = archive.entries()[0].clone();
        archive.limits.max_stored_overhead_bytes = u64::from(entry.stored_size - entry.size) - 1;
        archive.source.reads.borrow_mut().clear();
        assert!(matches!(
            archive.read(&entry),
            Err(HapiError::LimitExceeded {
                what: "SQSH stored bytes",
                ..
            })
        ));
        assert_eq!(
            *archive.source.reads.borrow(),
            [(u64::from(entry.data_offset), SQSH_HEADER_LEN)]
        );
    }

    #[test]
    fn uncompressed_sqsh_still_obeys_exact_sizes_and_budgets() {
        let payload = b"raw";
        let mut bytes = SQSH_MAGIC.to_vec();
        bytes.extend([2, 0, 0]);
        for word in [
            payload.len() as u32,
            payload.len() as u32,
            payload.iter().map(|&b| u32::from(b)).sum(),
        ] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(payload);
        let bounds = || ChunkBounds {
            stored: bytes.len() as u64,
            decoded: Some(3),
            max_decoded: 3,
            max_stored: 100,
            max_overhead: 100,
        };
        assert_eq!(
            read_sqsh(&bytes, bytes.len() as u64, 0, bounds()).unwrap(),
            payload
        );
        let mut bad_size = bytes.clone();
        bad_size[11..15].copy_from_slice(&2u32.to_le_bytes());
        let bad = ChunkBounds {
            decoded: None,
            ..bounds()
        };
        assert!(matches!(
            read_sqsh(&bad_size, bad_size.len() as u64, 0, bad),
            Err(HapiError::BadChunk { .. })
        ));
        let tiny = ChunkBounds {
            max_decoded: 2,
            ..bounds()
        };
        assert!(matches!(
            read_sqsh(&bytes, bytes.len() as u64, 0, tiny),
            Err(HapiError::LimitExceeded { .. })
        ));
    }

    #[test]
    fn payload_source_range_is_checked_before_allocation() {
        let bytes = HapiBuilder::new().file("x", b"data", true).build();
        let archive = Archive::open(bytes).unwrap();
        let mut entry = archive.entries()[0].clone();
        entry.data_offset = u32::MAX;
        assert!(matches!(
            archive.read(&entry),
            Err(HapiError::OutOfBounds { .. })
        ));
        entry.stored_size = 0;
        assert!(matches!(
            archive.read(&entry),
            Err(HapiError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn actual_zlib_output_cannot_exceed_matching_header_claims() {
        let mut bytes = HapiBuilder::new()
            .file("x", &[0; 100_000], true)
            .raw_dir(true)
            .build();
        let original = Archive::open(bytes.clone()).unwrap();
        let entry = &original.entries()[0];
        let header_size = entry.data_offset as usize + 11;
        bytes[header_size..header_size + 4].copy_from_slice(&3u32.to_le_bytes());
        let directory = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        bytes[directory + DIR_ENTRY_LEN + 8..directory + DIR_ENTRY_LEN + 12]
            .copy_from_slice(&3u32.to_le_bytes());
        let archive = Archive::open(bytes).unwrap();
        let err = archive.read(&archive.entries()[0]).unwrap_err();
        assert!(
            matches!(err, HapiError::BadChunk { ref reason, .. } if reason.contains("more than the declared 3 bytes"))
        );
    }

    #[test]
    fn compressed_metadata_claims_and_stored_sizes_are_bounded() {
        for offset_word in [8, 16] {
            let mut bytes = HapiBuilder::new().file("x", b"data", false).build();
            let block = u32::from_le_bytes(bytes[offset_word..offset_word + 4].try_into().unwrap())
                as usize;
            bytes[block + 11..block + 15].copy_from_slice(&(17u32 << 20).to_le_bytes());
            assert!(matches!(
                Archive::open(bytes),
                Err(HapiError::LimitExceeded {
                    what: "SQSH decoded bytes",
                    ..
                })
            ));
        }
        for size_word in [12, 20] {
            let mut bytes = HapiBuilder::new().file("x", b"data", false).build();
            let declared = u32::from_le_bytes(bytes[size_word..size_word + 4].try_into().unwrap());
            bytes[size_word..size_word + 4].copy_from_slice(&(declared + 1).to_le_bytes());
            assert!(matches!(
                Archive::open(bytes),
                Err(HapiError::BadChunk { .. })
            ));
        }
    }

    #[test]
    fn raw_metadata_and_directory_records_honor_explicit_limits() {
        let bytes = HapiBuilder::new()
            .file("x", b"data", false)
            .raw_dir(true)
            .raw_names(true)
            .build();
        let limits = Limits {
            max_metadata_bytes: 4,
            ..Limits::default()
        };
        assert!(matches!(
            Archive::open_with_limits(bytes.clone(), limits),
            Err(HapiError::LimitExceeded { .. })
        ));
        let limits = Limits {
            max_directory_records: 1,
            ..Limits::default()
        };
        assert!(matches!(
            Archive::open_with_limits(bytes.clone(), limits),
            Err(HapiError::LimitExceeded {
                what: "directory records",
                size: 2,
                limit: 1
            })
        ));
        let limits = Limits {
            max_directory_records: 2,
            ..Limits::default()
        };
        assert!(Archive::open_with_limits(bytes, limits).is_ok());
    }

    #[test]
    fn oversize_names_are_rejected_before_string_allocation() {
        let bytes = vec![b'x'; MAX_PATH_LEN + 2];
        assert!(matches!(
            name_at(&bytes, 0),
            Err(HapiError::BadDirectory(_))
        ));
        let mut latin1 = vec![255; MAX_PATH_LEN / 2 + 1];
        latin1.push(0);
        assert!(matches!(
            name_at(&latin1, 0),
            Err(HapiError::BadDirectory(_))
        ));
        assert_eq!(name_at(&[255, 0], 0).unwrap(), "ÿ");
    }

    #[test]
    fn nested_archive_requires_its_own_resource_policy() {
        let inner = HapiBuilder::new().file("x", b"data", true).build();
        let outer = Archive::open(HapiBuilder::new().file("inner", &inner, true).build()).unwrap();
        let bytes = outer
            .read_bounded(&outer.entries()[0], inner.len() as u64)
            .unwrap();
        let limits = Limits {
            max_file_bytes: 3,
            ..Limits::default()
        };
        let archive = Archive::open_with_limits(bytes, limits).unwrap();
        assert!(matches!(
            archive.read(&archive.entries()[0]),
            Err(HapiError::LimitExceeded {
                size: 4,
                limit: 3,
                ..
            })
        ));
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
