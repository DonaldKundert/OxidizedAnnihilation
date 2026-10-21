//! Gating for tests that need the retail game data.
//!
//! Game data is never committed. Tests that need it call [`data_dir`] and
//! return early when it yields `None`, so CI stays green without the game.

use std::path::PathBuf;

/// Name of the environment variable that points at a Kingdoms install.
pub const DATA_DIR_VAR: &str = "TAK_DATA_DIR";

/// The Kingdoms install directory from `TAK_DATA_DIR`, if it is set, non-empty
/// and points at an existing directory.
pub fn data_dir() -> Option<PathBuf> {
    from_value(std::env::var_os(DATA_DIR_VAR))
}

fn from_value(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let path = PathBuf::from(value.filter(|v| !v.is_empty())?);
    path.is_dir().then_some(path)
}

/// Returns the data dir, or prints a skip notice and returns `None`.
///
/// ```ignore
/// let Some(dir) = tak_vfs::testing::data_dir_or_skip("archives_mount") else { return };
/// ```
pub fn data_dir_or_skip(test: &str) -> Option<PathBuf> {
    let dir = data_dir();
    if dir.is_none() {
        eprintln!("skipping {test}: {DATA_DIR_VAR} not set");
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_empty_is_none() {
        assert_eq!(from_value(None), None);
        assert_eq!(from_value(Some("".into())), None);
    }

    #[test]
    fn missing_dir_is_none() {
        assert_eq!(
            from_value(Some("/definitely/not/a/tak/install".into())),
            None
        );
    }

    #[test]
    fn existing_dir_is_some() {
        let tmp = std::env::temp_dir();
        assert_eq!(from_value(Some(tmp.clone().into())), Some(tmp));
    }
}

/// Builds HAPI v2 archives in memory for tests, so no retail data is needed.
///
/// ```
/// use tak_vfs::testing::HapiBuilder;
/// let bytes = HapiBuilder::new()
///     .file("units/a.fbi", b"[UNITINFO]{}", true)
///     .file("readme.txt", b"hi", false)
///     .build();
/// let archive = tak_vfs::hapi::Archive::open(bytes).unwrap();
/// assert_eq!(archive.entries().len(), 2);
/// ```
#[derive(Default, Clone)]
pub struct HapiBuilder {
    files: Vec<(String, Vec<u8>, bool)>,
    raw_names: bool,
    raw_dir: bool,
}

#[derive(Default)]
struct Node {
    dirs: std::collections::BTreeMap<String, Node>,
    files: Vec<(String, usize)>,
}

impl HapiBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file. `compress` stores it as an SQSH chunk, else raw.
    pub fn file(mut self, path: &str, data: &[u8], compress: bool) -> Self {
        self.files.push((path.to_string(), data.to_vec(), compress));
        self
    }

    /// Store the names block raw instead of as an SQSH chunk.
    pub fn raw_names(mut self, yes: bool) -> Self {
        self.raw_names = yes;
        self
    }

    /// Store the directory block raw instead of as an SQSH chunk.
    pub fn raw_dir(mut self, yes: bool) -> Self {
        self.raw_dir = yes;
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let mut root = Node::default();
        for (i, (path, _, _)) in self.files.iter().enumerate() {
            let mut parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            let name = parts.pop().expect("non-empty path").to_string();
            let mut node = &mut root;
            for p in parts {
                node = node.dirs.entry(p.to_string()).or_default();
            }
            node.files.push((name, i));
        }

        let mut out = vec![0u8; crate::hapi::HEADER_LEN];
        let mut placed = vec![(0u32, 0u32); self.files.len()];
        for (i, (_, data, compress)) in self.files.iter().enumerate() {
            let off = out.len() as u32;
            if *compress {
                let chunk = sqsh(data, false);
                placed[i] = (off, chunk.len() as u32);
                out.extend(chunk);
            } else {
                placed[i] = (off, 0);
                out.extend_from_slice(data);
            }
        }

        let mut names = vec![0u8];
        let mut name_off = |s: &str| {
            let o = names.len() as u32;
            names.extend(s.bytes());
            names.push(0);
            o
        };
        // Breadth-first: each directory's children are laid out contiguously.
        let mut dir = Vec::new();
        let mut queue = vec![(name_off(""), &root, 0usize)];
        dir.resize(20, 0);
        let mut qi = 0;
        while qi < queue.len() {
            let (nm, node, at) = (queue[qi].0, queue[qi].1, queue[qi].2);
            qi += 1;
            let sub_at = dir.len();
            dir.resize(sub_at + 20 * node.dirs.len(), 0);
            let mut kids = Vec::new();
            for (k, (dn, child)) in node.dirs.iter().enumerate() {
                kids.push((name_off(dn), child, sub_at + 20 * k));
            }
            let file_at = dir.len();
            for (fname, idx) in &node.files {
                let (off, stored) = placed[*idx];
                let size = self.files[*idx].1.len() as u32;
                for w in [name_off(fname), off, size, stored, 946_684_800, 0] {
                    dir.extend(w.to_le_bytes());
                }
            }
            for (k, w) in [
                nm,
                sub_at as u32,
                node.dirs.len() as u32,
                file_at as u32,
                node.files.len() as u32,
            ]
            .into_iter()
            .enumerate()
            {
                dir[at + 4 * k..at + 4 * k + 4].copy_from_slice(&w.to_le_bytes());
            }
            queue.extend(kids);
        }

        let names_off = out.len() as u32;
        let names_block = if self.raw_names {
            names.clone()
        } else {
            sqsh(&names, true)
        };
        out.extend(&names_block);
        let dir_off = out.len() as u32;
        let dir_block = if self.raw_dir {
            dir.clone()
        } else {
            sqsh(&dir, true)
        };
        out.extend(&dir_block);
        let end = out.len() as u32;
        out.extend(b"synthetic test archive");

        let header = [
            crate::hapi::VERSION_2,
            dir_off,
            dir_block.len() as u32,
            names_off,
            names_block.len() as u32,
            crate::hapi::HEADER_LEN as u32,
            end,
        ];
        out[..4].copy_from_slice(&crate::hapi::MAGIC);
        for (k, w) in header.iter().enumerate() {
            out[4 + 4 * k..8 + 4 * k].copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

/// One zlib SQSH chunk, optionally obfuscated.
fn sqsh(data: &[u8], obfuscate: bool) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).expect("in-memory write");
    let mut payload = enc.finish().expect("in-memory write");
    if obfuscate {
        crate::hapi::obfuscate(&mut payload);
    }
    let sum = payload.iter().fold(0u32, |a, &b| a.wrapping_add(b.into()));
    let mut c = b"SQSH".to_vec();
    c.extend([2, 2, obfuscate as u8]);
    for w in [payload.len() as u32, data.len() as u32, sum] {
        c.extend(w.to_le_bytes());
    }
    c.extend(payload);
    c
}
