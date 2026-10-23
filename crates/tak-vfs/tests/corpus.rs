//! Corpus tests against a real install. Skipped unless TAK_DATA_DIR is set.
//! They assert counts and zero errors only, never file contents.

use tak_vfs::testing::data_dir_or_skip;

#[test]
fn every_archive_opens_and_every_file_extracts() {
    let Some(dir) = data_dir_or_skip("every_archive_opens_and_every_file_extracts") else {
        return;
    };
    let order = tak_vfs::kingdoms_mount_order(&dir);
    assert!(!order.is_empty(), "no archives found in {}", dir.display());
    let mut files = 0usize;
    let mut bytes = 0u64;
    for path in &order {
        let src = tak_vfs::source::FileSource::open(path).unwrap();
        let a =
            tak_vfs::hapi::Archive::open(src).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for e in a.entries() {
            let data = a
                .read_bounded(e, u64::from(e.size))
                .unwrap_or_else(|err| panic!("{} / {}: {err}", path.display(), e.path));
            files += 1;
            bytes += data.len() as u64;
        }
    }
    eprintln!("{} archives, {files} files, {bytes} bytes", order.len());
    assert!(
        files > 10_000,
        "only {files} files: is this a full install?"
    );
    // GOG Kingdoms + Iron Plague 2.0.0.4 (data/data.hpi duplicate excluded).
    if order.len() == 198 {
        assert_eq!(files, 13_174, "file count changed for the GOG install");
    } else {
        eprintln!(
            "note: {} archives, not the GOG 198; exact count not checked",
            order.len()
        );
    }
}

#[test]
fn mounted_view_resolves_with_patch_precedence() {
    let Some(dir) = data_dir_or_skip("mounted_view_resolves_with_patch_precedence") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    assert!(vfs.len() > 10_000, "only {} paths", vfs.len());
    // A file present in base, V3 patch and Iron Plague resolves to Iron Plague.
    let o = vfs.origins("ai/default.txt");
    assert_eq!(o.len(), 3);
    let winner = vfs.mounts()[o[2].mount]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(winner, "IPData.hpi");
    for f in vfs.list("units").into_iter().take(50) {
        assert_eq!(vfs.read(&f.path).unwrap().len() as u64, f.size);
    }
}
