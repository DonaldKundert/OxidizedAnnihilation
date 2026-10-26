//! The virtual filesystem: an ordered stack of mounts with case-insensitive
//! lookup. Later mounts override earlier ones.

use crate::hapi::{Archive, Entry, HapiError, Limits};
use crate::source::{FileSource, read_to_end_bounded};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    #[error("{path}: {source}")]
    Archive {
        path: PathBuf,
        #[source]
        source: HapiError,
    },
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: file requires at least {size} bytes, limit is {limit}")]
    LimitExceeded {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("origin {0:?} does not belong to this VFS")]
    BadOrigin(Origin),
}

pub type Result<T> = std::result::Result<T, VfsError>;

/// Normalizes a game path: `\` to `/`, lowercase ASCII, no leading or
/// trailing slashes, no empty or `.` segments.
pub fn normalize(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .map(|s| s.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

enum Mount {
    Archive {
        path: PathBuf,
        archive: Archive<FileSource>,
    },
    Dir {
        root: PathBuf,
        /// Real relative paths of the indexed files; `Origin::entry` indexes this.
        files: Vec<String>,
    },
}

/// Where a file comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Origin {
    /// Index into [`Vfs::mounts`]; higher wins.
    pub mount: usize,
    /// Index of the entry inside the mount.
    pub entry: usize,
}

/// A file visible in the VFS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// Normalized path.
    pub path: String,
    /// Path with original case, from the winning mount.
    pub display_path: String,
    pub size: u64,
    pub origin: Origin,
}

#[derive(Default)]
pub struct Vfs {
    mounts: Vec<Mount>,
    /// Normalized path -> origins, lowest priority first.
    index: BTreeMap<String, Vec<Origin>>,
    limits: Limits,
}

impl Vfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the policy for all archive mounts and loose-file reads in this VFS.
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    /// Mounts a HAPI archive on top of everything mounted so far.
    pub fn mount_archive(&mut self, path: &Path) -> Result<()> {
        let source = FileSource::open(path).map_err(|source| VfsError::Io {
            path: path.into(),
            source,
        })?;
        let archive =
            Archive::open_with_limits(source, self.limits).map_err(|source| VfsError::Archive {
                path: path.into(),
                source,
            })?;
        let mount = self.mounts.len();
        for (i, e) in archive.entries().iter().enumerate() {
            self.index
                .entry(normalize(&e.path))
                .or_default()
                .push(Origin { mount, entry: i });
        }
        self.mounts.push(Mount::Archive {
            path: path.into(),
            archive,
        });
        Ok(())
    }

    /// Mounts a loose directory on top of everything mounted so far. Files
    /// are indexed now; contents are read on demand. Symlinks are skipped.
    /// If two files differ only in case (possible on Linux), the one that
    /// sorts first by byte order is used and the other is ignored.
    pub fn mount_dir(&mut self, root: &Path) -> Result<()> {
        let mount = self.mounts.len();
        let mut found = Vec::new();
        collect(root, root, &mut found)?;
        found.sort();
        let mut files = Vec::new();
        for rel in found {
            let origins = self.index.entry(normalize(&rel)).or_default();
            if origins.last().is_some_and(|o| o.mount == mount) {
                continue; // case collision inside this directory
            }
            origins.push(Origin {
                mount,
                entry: files.len(),
            });
            files.push(rel);
        }
        self.mounts.push(Mount::Dir {
            root: root.into(),
            files,
        });
        Ok(())
    }

    /// Mount sources, lowest priority first.
    pub fn mounts(&self) -> Vec<&Path> {
        self.mounts
            .iter()
            .map(|m| match m {
                Mount::Archive { path, .. } => path.as_path(),
                Mount::Dir { root, .. } => root.as_path(),
            })
            .collect()
    }

    /// Number of distinct visible paths.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Every origin for a path, lowest priority first. More than one means
    /// later mounts shadow earlier ones.
    pub fn origins(&self, path: &str) -> &[Origin] {
        self.index
            .get(&normalize(path))
            .map_or(&[], |v| v.as_slice())
    }

    /// The winning file for a path.
    pub fn stat(&self, path: &str) -> Option<FileInfo> {
        let key = normalize(path);
        let origin = *self.index.get(&key)?.last()?;
        self.info(key, origin)
    }

    fn info(&self, path: String, origin: Origin) -> Option<FileInfo> {
        Some(match self.mounts.get(origin.mount)? {
            Mount::Archive { archive, .. } => {
                let e = archive.entries().get(origin.entry)?;
                FileInfo {
                    path,
                    display_path: e.path.clone(),
                    size: e.size.into(),
                    origin,
                }
            }
            Mount::Dir { root, files } => {
                let rel = files.get(origin.entry)?;
                let size = std::fs::metadata(root.join(rel)).map_or(0, |m| m.len());
                FileInfo {
                    path,
                    display_path: rel.clone(),
                    size,
                    origin,
                }
            }
        })
    }

    /// All visible files whose normalized path starts with `prefix`
    /// (normalized; empty lists everything), sorted by path.
    pub fn list(&self, prefix: &str) -> Vec<FileInfo> {
        let p = normalize(prefix);
        self.index
            .range(p.clone()..)
            .take_while(|(k, _)| k.starts_with(&p))
            .filter(|(k, _)| p.is_empty() || k.len() == p.len() || k.as_bytes()[p.len()] == b'/')
            .filter_map(|(k, v)| v.last().and_then(|o| self.info(k.clone(), *o)))
            .collect()
    }

    /// Reads the winning version of a file, subject to the VFS resource policy.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        self.read_bounded(path, self.limits.max_file_bytes)
    }

    /// Reads at most `max_bytes` decoded bytes. Archive reads validate SQSH
    /// header claims before payload allocation; loose reads remain bounded even
    /// if the file grows after metadata is checked. Also honors the VFS policy.
    pub fn read_bounded(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>> {
        let key = normalize(path);
        let origin = self
            .index
            .get(&key)
            .and_then(|v| v.last())
            .ok_or_else(|| VfsError::NotFound(path.to_string()))?;
        self.read_origin_bounded(&key, *origin, max_bytes)
    }

    /// Reads a specific origin (for comparing shadowed versions).
    pub fn read_origin(&self, path: &str, origin: Origin) -> Result<Vec<u8>> {
        self.read_origin_bounded(path, origin, self.limits.max_file_bytes)
    }

    /// Bounded equivalent of [`Self::read_origin`].
    pub fn read_origin_bounded(
        &self,
        _path: &str,
        origin: Origin,
        max_bytes: u64,
    ) -> Result<Vec<u8>> {
        let max_bytes = max_bytes.min(self.limits.max_file_bytes);
        match self.mounts.get(origin.mount) {
            Some(Mount::Archive { path: ap, archive }) => {
                let e: &Entry = archive
                    .entries()
                    .get(origin.entry)
                    .ok_or(VfsError::BadOrigin(origin))?;
                archive
                    .read_bounded(e, max_bytes)
                    .map_err(|source| VfsError::Archive {
                        path: ap.clone(),
                        source,
                    })
            }
            Some(Mount::Dir { root, files }) => {
                let full = root.join(files.get(origin.entry).ok_or(VfsError::BadOrigin(origin))?);
                read_loose_bounded(&full, max_bytes)
            }
            None => Err(VfsError::BadOrigin(origin)),
        }
    }
}

fn read_loose_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let io = |source| VfsError::Io {
        path: path.into(),
        source,
    };
    let mut file = std::fs::File::open(path).map_err(io)?;
    let size = file.metadata().map_err(io)?.len();
    if size > max_bytes {
        return Err(VfsError::LimitExceeded {
            path: path.into(),
            size,
            limit: max_bytes,
        });
    }
    let (out, exceeded) = read_to_end_bounded(&mut file, max_bytes).map_err(io)?;
    if exceeded {
        return Err(VfsError::LimitExceeded {
            path: path.into(),
            size: max_bytes.saturating_add(1),
            limit: max_bytes,
        });
    }
    Ok(out)
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| VfsError::Io { path, source }
    };
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(io(dir))?
        .map(|e| e.map(|e| e.path()).map_err(io(dir)))
        .collect::<Result<_>>()?;
    entries.sort();
    for p in entries {
        let ft = std::fs::symlink_metadata(&p).map_err(io(&p))?.file_type();
        if ft.is_dir() {
            collect(root, &p, out)?;
        } else if ft.is_file() {
            let rel = p.strip_prefix(root).unwrap_or(&p);
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// The mount order for a Kingdoms install, lowest priority first.
///
/// **Believed**, not confirmed against the original executable: base data,
/// then the V2 and V3 patches, then The Iron Plague, then the GOG fix-ups,
/// then loose `.kmp` maps, matching the archives' timestamps. Archives that
/// are missing are skipped; unknown `.hpi` files are appended (sorted) before
/// the maps so nothing is silently ignored. `data/data.hpi` is a duplicate of
/// `data.hpi` and is not mounted.
pub fn kingdoms_mount_order(install: &Path) -> Vec<PathBuf> {
    const ORDER: &[&str] = &[
        "data.hpi",
        "terrain.hpi",
        "sections.hpi",
        "maps.hpi",
        "missions.hpi",
        "english.hpi",
        "V2Rocket.hpi",
        "V3Rocket.hpi",
        "IPData.hpi",
        "IPSections.hpi",
        "IPMissions.hpi",
        "IPEnglish.hpi",
        "Jersey.hpi",
        "meta.hpi",
        "boneyards.hpi",
        "boneyards2.hpi",
        "gog-mod.hpi",
    ];
    let top: Vec<PathBuf> = std::fs::read_dir(install)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    let find = |name: &str| {
        top.iter()
            .find(|p| {
                p.file_name()
                    .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name))
            })
            .cloned()
    };
    let mut out: Vec<PathBuf> = ORDER.iter().filter_map(|n| find(n)).collect();
    let mut extra: Vec<PathBuf> = top
        .iter()
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hpi"))
                && !out.contains(p)
        })
        .cloned()
        .collect();
    extra.sort();
    out.extend(extra);
    if let Some(maps) = find("Maps") {
        let mut kmps: Vec<PathBuf> = std::fs::read_dir(maps)
            .map(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("kmp")))
                    .collect()
            })
            .unwrap_or_default();
        kmps.sort();
        out.extend(kmps);
    }
    out
}

/// Opens a Kingdoms install with [`kingdoms_mount_order`].
pub fn open_kingdoms(install: &Path) -> Result<Vfs> {
    let mut vfs = Vfs::new();
    for p in kingdoms_mount_order(install) {
        vfs.mount_archive(&p)?;
    }
    Ok(vfs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::HapiBuilder;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("tak-vfs-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Scratch(p)
        }
        fn write(&self, rel: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            p
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn bounded_reads_apply_to_archives_and_shadowed_origins() {
        let scratch = Scratch::new("bounded-archive");
        let path = scratch.write(
            "a.hpi",
            &HapiBuilder::new().file("x", b"data", true).build(),
        );
        let mut vfs = Vfs::new();
        vfs.mount_archive(&path).unwrap();
        assert_eq!(vfs.read_bounded("X", 4).unwrap(), b"data");
        assert!(matches!(
            vfs.read_bounded("x", 3),
            Err(VfsError::Archive {
                source: HapiError::LimitExceeded {
                    size: 4,
                    limit: 3,
                    ..
                },
                ..
            })
        ));
        let origin = vfs.origins("x")[0];
        assert!(vfs.read_origin_bounded("x", origin, 3).is_err());
        assert!(matches!(
            vfs.read_origin_bounded(
                "x",
                Origin {
                    mount: 99,
                    entry: 0
                },
                3
            ),
            Err(VfsError::BadOrigin(_))
        ));
    }

    #[test]
    fn bounded_loose_reads_recheck_current_size() {
        let scratch = Scratch::new("bounded-loose");
        scratch.write("x", b"a");
        scratch.write("empty", b"");
        let mut vfs = Vfs::with_limits(Limits {
            max_file_bytes: 4,
            ..Limits::default()
        });
        vfs.mount_dir(&scratch.0).unwrap();
        assert_eq!(vfs.stat("x").unwrap().size, 1);
        scratch.write("x", b"data");
        assert_eq!(vfs.read_bounded("x", 4).unwrap(), b"data");
        assert_eq!(vfs.read_bounded("empty", 0).unwrap(), b"");
        assert!(matches!(
            vfs.read_bounded("x", 3),
            Err(VfsError::LimitExceeded {
                size: 4,
                limit: 3,
                ..
            })
        ));
        scratch.write("x", b"larger");
        assert!(matches!(
            vfs.read("x"),
            Err(VfsError::LimitExceeded {
                size: 6,
                limit: 4,
                ..
            })
        ));
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(
            normalize("\\Units\\ARAMON\\Monarch.FBI"),
            "units/aramon/monarch.fbi"
        );
        assert_eq!(normalize("//a//b/"), "a/b");
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn later_mounts_win_and_lookup_ignores_case() {
        let s = Scratch::new("prec");
        let base = s.write(
            "data.hpi",
            &HapiBuilder::new()
                .file("ai/Default.txt", b"base", true)
                .file("only/base.txt", b"b", false)
                .build(),
        );
        let patch = s.write(
            "V3Rocket.hpi",
            &HapiBuilder::new()
                .file("AI/default.TXT", b"patched", true)
                .build(),
        );
        let mut v = Vfs::new();
        v.mount_archive(&base).unwrap();
        v.mount_archive(&patch).unwrap();
        assert_eq!(v.read("ai/default.txt").unwrap(), b"patched");
        assert_eq!(v.read("Ai\\DEFAULT.txt").unwrap(), b"patched");
        assert_eq!(v.origins("ai/default.txt").len(), 2);
        let first = v.origins("ai/default.txt")[0];
        assert_eq!(v.read_origin("ai/default.txt", first).unwrap(), b"base");
        assert_eq!(
            v.stat("ai/default.txt").unwrap().display_path,
            "AI/default.TXT"
        );
        assert_eq!(v.read("only/base.txt").unwrap(), b"b");
        assert!(matches!(v.read("nope"), Err(VfsError::NotFound(_))));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn list_respects_segment_boundaries() {
        let s = Scratch::new("list");
        let a = s.write(
            "a.hpi",
            &HapiBuilder::new()
                .file("units/a.fbi", b"1", false)
                .file("units2/b.fbi", b"2", false)
                .file("unitsx", b"3", false)
                .build(),
        );
        let mut v = Vfs::new();
        v.mount_archive(&a).unwrap();
        let got: Vec<_> = v.list("UNITS").into_iter().map(|f| f.path).collect();
        assert_eq!(got, ["units/a.fbi"]);
        assert_eq!(v.list("").len(), 3);
    }

    #[test]
    fn loose_directories_mount_and_override() {
        let s = Scratch::new("dir");
        let a = s.write(
            "a.hpi",
            &HapiBuilder::new()
                .file("gamedata/x.tdf", b"archive", true)
                .build(),
        );
        s.write("loose/GameData/X.tdf", b"loose");
        let mut v = Vfs::new();
        v.mount_archive(&a).unwrap();
        v.mount_dir(&s.0.join("loose")).unwrap();
        assert_eq!(v.read("gamedata/x.tdf").unwrap(), b"loose");
        assert_eq!(v.stat("gamedata/x.tdf").unwrap().size, 5);
    }

    #[test]
    fn bad_origins_error_instead_of_panicking() {
        let v = Vfs::new();
        let o = Origin { mount: 3, entry: 9 };
        assert!(matches!(v.read_origin("x", o), Err(VfsError::BadOrigin(_))));
    }

    #[test]
    fn case_collisions_in_loose_dirs_resolve_deterministically() {
        let s = Scratch::new("collide");
        s.write("d/Foo.txt", b"upper");
        s.write("d/foo.txt", b"lower");
        let mut v = Vfs::new();
        v.mount_dir(&s.0.join("d")).unwrap();
        assert_eq!(v.origins("FOO.TXT").len(), 1);
        // On case-insensitive filesystems the second write replaces the first;
        // there is no collision to resolve. Still verify case-insensitive lookup.
        let distinct = std::fs::read_dir(s.0.join("d")).unwrap().count() == 2;
        if distinct {
            assert_eq!(v.read("foo.txt").unwrap(), b"upper"); // "Foo" sorts first
        } else {
            assert_eq!(v.read("FOO.txt").unwrap(), b"lower");
            eprintln!("case-collision fixture: filesystem stores one case-insensitive file");
        }
    }

    #[test]
    fn loose_dir_under_later_archive_is_overridden() {
        let s = Scratch::new("under");
        s.write("loose/a.txt", b"loose");
        let arc = s.write(
            "p.hpi",
            &HapiBuilder::new().file("A.TXT", b"archive", false).build(),
        );
        let mut v = Vfs::new();
        v.mount_dir(&s.0.join("loose")).unwrap();
        v.mount_archive(&arc).unwrap();
        assert_eq!(v.read("a.txt").unwrap(), b"archive");
    }

    #[test]
    fn list_with_exact_file_prefix() {
        let s = Scratch::new("exact");
        let a = s.write(
            "a.hpi",
            &HapiBuilder::new().file("x/y.txt", b"1", false).build(),
        );
        let mut v = Vfs::new();
        v.mount_archive(&a).unwrap();
        assert_eq!(v.list("X/Y.TXT").len(), 1);
        assert_eq!(normalize("./x/./y.txt"), "x/y.txt");
    }

    #[test]
    fn mount_order_follows_profile_and_keeps_unknowns() {
        let s = Scratch::new("order");
        for n in [
            "IPData.hpi",
            "data.hpi",
            "V3Rocket.hpi",
            "gog-mod.hpi",
            "zzz-mod.hpi",
        ] {
            s.write(n, b"");
        }
        s.write("Maps/ B.kmp", b"");
        s.write("Maps/ A.kmp", b"");
        let names: Vec<String> = kingdoms_mount_order(&s.0)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "data.hpi",
                "V3Rocket.hpi",
                "IPData.hpi",
                "gog-mod.hpi",
                "zzz-mod.hpi",
                " A.kmp",
                " B.kmp"
            ]
        );
    }
}
