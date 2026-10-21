//! `tak-cli survey`: an inventory of an install directory by file signature.
//!
//! Records each file's relative path, extension, size and first 64 bytes, and
//! groups files by their leading bytes. It never reads more than the head of a
//! file, so the output carries no retail content beyond those 64 bytes.

use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Bytes recorded from the start of every file.
pub const HEAD_LEN: usize = 64;

#[derive(Debug, Serialize)]
pub struct Survey {
    pub root: String,
    pub files: Vec<FileRecord>,
    pub symlinks: Vec<SymlinkRecord>,
    /// Paths that could not be read, with the error.
    pub errors: Vec<(String, String)>,
    pub signatures: Vec<SignatureGroup>,
    pub extensions: Vec<ExtensionGroup>,
}

#[derive(Debug, Serialize)]
pub struct FileRecord {
    /// Relative to the survey root, `/`-separated.
    pub path: String,
    /// Lowercased extension without the dot; empty when there is none.
    pub ext: String,
    pub size: u64,
    pub head_hex: String,
    pub signature: Signature,
    /// Present when the file starts with `HAPI`.
    pub hapi: Option<HapiHeader>,
}

#[derive(Debug, Serialize)]
pub struct SymlinkRecord {
    pub path: String,
    pub target: String,
}

/// How a file is grouped. Text files are grouped together because their first
/// bytes are content, not a format marker.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Signature {
    Empty,
    Text,
    /// The first 4 bytes (fewer if the file is shorter), as lowercase hex.
    Magic(String),
}

/// The fixed fields we can read from any `HAPI` header without interpreting
/// the layout: the version word and the six words that follow it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HapiHeader {
    pub version: u32,
    /// Little-endian u32 words at offsets 8, 12, 16, 20, 24, 28.
    pub words: [u32; 6],
}

#[derive(Debug, Serialize)]
pub struct SignatureGroup {
    pub signature: Signature,
    /// Printable rendering of the magic bytes (non-printables as `.`).
    pub ascii: String,
    pub count: usize,
    pub total_bytes: u64,
    /// Extension -> count.
    pub extensions: BTreeMap<String, usize>,
    /// Up to five example paths.
    pub examples: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ExtensionGroup {
    pub ext: String,
    pub count: usize,
    pub total_bytes: u64,
    pub signatures: Vec<Signature>,
}

pub fn run(root: &Path) -> io::Result<Survey> {
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("not a directory: {}", root.display()),
        ));
    }
    let mut files = Vec::new();
    let mut symlinks = Vec::new();
    let mut errors = Vec::new();
    walk(root, root, &mut files, &mut symlinks, &mut errors);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    symlinks.sort_by(|a, b| a.path.cmp(&b.path));
    errors.sort();
    let signatures = group_signatures(&files);
    let extensions = group_extensions(&files);
    Ok(Survey {
        root: root.display().to_string(),
        files,
        symlinks,
        errors,
        signatures,
        extensions,
    })
}

fn walk(
    root: &Path,
    dir: &Path,
    files: &mut Vec<FileRecord>,
    symlinks: &mut Vec<SymlinkRecord>,
    errors: &mut Vec<(String, String)>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            errors.push((rel(root, dir), e.to_string()));
            return;
        }
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        // symlink_metadata: never follow links (the GOG install has a dangling one).
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                errors.push((rel(root, &path), e.to_string()));
                continue;
            }
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            let target = fs::read_link(&path)
                .map(|t| t.display().to_string())
                .unwrap_or_else(|e| format!("<unreadable: {e}>"));
            symlinks.push(SymlinkRecord {
                path: rel(root, &path),
                target,
            });
        } else if ft.is_dir() {
            walk(root, &path, files, symlinks, errors);
        } else if ft.is_file() {
            match record(root, &path, meta.len()) {
                Ok(r) => files.push(r),
                Err(e) => errors.push((rel(root, &path), e.to_string())),
            }
        }
    }
}

fn rel(root: &Path, path: &Path) -> String {
    let r = path.strip_prefix(root).unwrap_or(path);
    r.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn record(root: &Path, path: &Path, size: u64) -> io::Result<FileRecord> {
    let mut head = Vec::with_capacity(HEAD_LEN);
    fs::File::open(path)?
        .take(HEAD_LEN as u64)
        .read_to_end(&mut head)?;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    Ok(FileRecord {
        path: rel(root, path),
        ext,
        size,
        head_hex: hex(&head),
        signature: signature(&head),
        hapi: hapi_header(&head),
    })
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

/// Text if every byte in the head is printable ASCII, whitespace, or high
/// Latin-1 (accented letters in localized strings), and there is no NUL.
fn looks_like_text(head: &[u8]) -> bool {
    !head.is_empty()
        && head
            .iter()
            .all(|&b| matches!(b, b'\t' | b'\n' | b'\r' | 0x20..=0x7e | 0xa0..=0xff))
}

pub fn signature(head: &[u8]) -> Signature {
    if head.is_empty() {
        Signature::Empty
    } else if looks_like_text(head) {
        Signature::Text
    } else {
        Signature::Magic(hex(&head[..head.len().min(4)]))
    }
}

pub fn hapi_header(head: &[u8]) -> Option<HapiHeader> {
    if head.len() < 32 || &head[..4] != b"HAPI" {
        return None;
    }
    let word = |i: usize| u32::from_le_bytes(head[i..i + 4].try_into().expect("4 bytes"));
    Some(HapiHeader {
        version: word(4),
        words: [word(8), word(12), word(16), word(20), word(24), word(28)],
    })
}

fn group_signatures(files: &[FileRecord]) -> Vec<SignatureGroup> {
    let mut map: BTreeMap<Signature, SignatureGroup> = BTreeMap::new();
    for f in files {
        let g = map
            .entry(f.signature.clone())
            .or_insert_with(|| SignatureGroup {
                signature: f.signature.clone(),
                ascii: match &f.signature {
                    Signature::Magic(_) => {
                        let n = (f.head_hex.len() / 2).min(4);
                        ascii(&unhex(&f.head_hex[..n * 2]))
                    }
                    _ => String::new(),
                },
                count: 0,
                total_bytes: 0,
                extensions: BTreeMap::new(),
                examples: Vec::new(),
            });
        g.count += 1;
        g.total_bytes += f.size;
        *g.extensions.entry(f.ext.clone()).or_default() += 1;
        if g.examples.len() < 5 {
            g.examples.push(f.path.clone());
        }
    }
    let mut groups: Vec<_> = map.into_values().collect();
    groups.sort_by(|a, b| b.count.cmp(&a.count).then(a.signature.cmp(&b.signature)));
    groups
}

fn group_extensions(files: &[FileRecord]) -> Vec<ExtensionGroup> {
    let mut map: BTreeMap<&str, ExtensionGroup> = BTreeMap::new();
    for f in files {
        let g = map.entry(&f.ext).or_insert_with(|| ExtensionGroup {
            ext: f.ext.clone(),
            count: 0,
            total_bytes: 0,
            signatures: Vec::new(),
        });
        g.count += 1;
        g.total_bytes += f.size;
        if !g.signatures.contains(&f.signature) {
            g.signatures.push(f.signature.clone());
        }
    }
    let mut groups: Vec<_> = map.into_values().collect();
    for g in &mut groups {
        g.signatures.sort();
    }
    groups
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hapi_bytes(version: u32) -> Vec<u8> {
        let mut b = b"HAPI".to_vec();
        b.extend(version.to_le_bytes());
        for w in 1u32..=6 {
            b.extend(w.to_le_bytes());
        }
        b.extend([0u8; 40]);
        b
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(unhex("00abff"), vec![0x00, 0xab, 0xff]);
    }

    #[test]
    fn signatures() {
        assert_eq!(signature(b""), Signature::Empty);
        assert_eq!(signature(b"[UNITINFO]\r\n\t{"), Signature::Text);
        assert_eq!(
            signature(b"MZ\x90\x00\x03"),
            Signature::Magic("4d5a9000".into())
        );
        assert_eq!(signature(b"\x01\x02"), Signature::Magic("0102".into()));
    }

    #[test]
    fn hapi_header_reads_version_and_words() {
        let h = hapi_header(&hapi_bytes(0x0002_0000)).unwrap();
        assert_eq!(h.version, 0x0002_0000);
        assert_eq!(h.words, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn hapi_header_rejects_short_or_foreign() {
        assert_eq!(hapi_header(b"HAPI\x00\x00"), None);
        assert_eq!(hapi_header(&[0u8; 64]), None);
    }

    /// A scratch directory under the system temp dir, removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("tak-survey-test-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Scratch(p)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn survey_walks_groups_and_sorts() {
        let s = Scratch::new("walk");
        let d = &s.0;
        fs::create_dir_all(d.join("Maps")).unwrap();
        fs::write(d.join("data.HPI"), hapi_bytes(0x0002_0000)).unwrap();
        fs::write(d.join("Maps/b.kmp"), hapi_bytes(0x0002_0000)).unwrap();
        fs::write(d.join("Startup.tdf"), b"[GAME]\r\n{\r\n}\r\n").unwrap();
        fs::write(d.join("empty"), b"").unwrap();
        let big = vec![0x7fu8; 1000];
        fs::write(d.join("big.bin"), &big).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/nowhere/at/all", d.join("SavedGames")).unwrap();

        let sv = run(d).unwrap();
        let paths: Vec<_> = sv.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            ["Maps/b.kmp", "Startup.tdf", "big.bin", "data.HPI", "empty"]
        );
        assert!(sv.errors.is_empty(), "{:?}", sv.errors);
        #[cfg(unix)]
        assert_eq!(sv.symlinks.len(), 1);

        let big = sv.files.iter().find(|f| f.path == "big.bin").unwrap();
        assert_eq!(big.size, 1000);
        assert_eq!(big.head_hex.len(), HEAD_LEN * 2, "head is capped");

        let top = &sv.signatures[0];
        assert_eq!(top.signature, Signature::Magic("48415049".into()));
        assert_eq!(top.ascii, "HAPI");
        assert_eq!(top.count, 2);
        assert_eq!(top.extensions.get("hpi"), Some(&1));
        assert_eq!(top.extensions.get("kmp"), Some(&1));

        let hpi = sv.extensions.iter().find(|e| e.ext == "hpi").unwrap();
        assert_eq!(hpi.signatures, vec![Signature::Magic("48415049".into())]);
    }

    #[test]
    fn survey_rejects_missing_root() {
        assert!(run(Path::new("/definitely/not/here")).is_err());
    }

    /// Corpus check: the real install surveys with no read errors.
    #[test]
    fn corpus_survey_has_no_errors() {
        let Some(dir) = tak_vfs::testing::data_dir_or_skip("corpus_survey_has_no_errors") else {
            return;
        };
        let sv = run(&dir).unwrap();
        assert!(!sv.files.is_empty());
        assert!(sv.errors.is_empty(), "{:?}", sv.errors);
    }
}
