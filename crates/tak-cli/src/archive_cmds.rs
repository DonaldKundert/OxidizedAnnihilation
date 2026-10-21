//! Commands over the mounted VFS: ls, cat, extract, shadowed, census.

use crate::survey::{Signature, ascii, signature};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use tak_vfs::Vfs;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

pub fn ls(vfs: &Vfs, prefix: &str, long: bool) -> Res<()> {
    let mounts = vfs.mounts();
    let mut out = std::io::stdout().lock();
    for f in vfs.list(prefix) {
        if long {
            let from = mounts[f.origin.mount]
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let n = vfs.origins(&f.path).len();
            let shadow = if n > 1 {
                format!(" (+{} shadowed)", n - 1)
            } else {
                String::new()
            };
            writeln!(
                out,
                "{:>10}  {:<16} {}{shadow}",
                f.size, from, f.display_path
            )?;
        } else {
            writeln!(out, "{}", f.display_path)?;
        }
    }
    Ok(())
}

pub fn cat(vfs: &Vfs, path: &str) -> Res<()> {
    std::io::stdout().lock().write_all(&vfs.read(path)?)?;
    Ok(())
}

/// Extracts every visible file under `prefix` into `out`, keeping the
/// archive's original path case.
pub fn extract(vfs: &Vfs, prefix: &str, out: &Path) -> Res<()> {
    let files = vfs.list(prefix);
    if files.is_empty() {
        return Err(format!("nothing matches {prefix:?}").into());
    }
    for f in &files {
        let dest = out.join(&f.display_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, vfs.read(&f.path)?)?;
    }
    eprintln!("extracted {} files to {}", files.len(), out.display());
    Ok(())
}

/// Lists paths provided by more than one mount, with whether contents differ.
pub fn shadowed(vfs: &Vfs, prefix: &str) -> Res<()> {
    let mounts = vfs.mounts();
    let mut out = std::io::stdout().lock();
    let (mut total, mut differ) = (0usize, 0usize);
    for f in vfs.list(prefix) {
        let origins = vfs.origins(&f.path);
        if origins.len() < 2 {
            continue;
        }
        let datas: Vec<Vec<u8>> = origins
            .iter()
            .map(|o| vfs.read_origin(&f.path, *o))
            .collect::<Result<_, _>>()?;
        let same = datas.windows(2).all(|w| w[0] == w[1]);
        total += 1;
        differ += usize::from(!same);
        let chain: Vec<String> = origins
            .iter()
            .map(|o| {
                mounts[o.mount]
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        writeln!(
            out,
            "{}  {}  {}",
            if same { "same" } else { "DIFF" },
            f.display_path,
            chain.join(" < ")
        )?;
    }
    eprintln!("{total} shadowed paths, {differ} with differing contents");
    Ok(())
}

#[derive(Default, serde::Serialize)]
struct ExtStats {
    count: usize,
    bytes: u64,
    signatures: BTreeMap<String, usize>,
    examples: Vec<String>,
    archives: BTreeMap<String, usize>,
}

/// Inventory of the winning files by extension and leading bytes.
pub fn census(vfs: &Vfs, out: &Path) -> Res<()> {
    let mounts = vfs.mounts();
    let mut by_ext: BTreeMap<String, ExtStats> = BTreeMap::new();
    let mut magic_ascii: BTreeMap<String, String> = BTreeMap::new();
    for f in vfs.list("") {
        let data = vfs.read(&f.path)?;
        let head = &data[..data.len().min(64)];
        let sig = match signature(head) {
            Signature::Empty => "empty".to_string(),
            Signature::Text => "text".to_string(),
            Signature::Magic(h) => {
                magic_ascii
                    .entry(h.clone())
                    .or_insert_with(|| ascii(&head[..head.len().min(4)]));
                h
            }
        };
        let ext = f
            .path
            .rsplit_once('.')
            .map_or("(none)", |(_, e)| e)
            .to_string();
        let s = by_ext.entry(ext).or_default();
        s.count += 1;
        s.bytes += data.len() as u64;
        *s.signatures.entry(sig).or_default() += 1;
        let arc = mounts[f.origin.mount]
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let arc = if arc.to_ascii_lowercase().ends_with(".kmp") {
            "Maps/*.kmp".into()
        } else {
            arc
        };
        *s.archives.entry(arc).or_default() += 1;
        if s.examples.len() < 3 {
            s.examples.push(f.display_path.clone());
        }
    }

    let mut md = String::new();
    let _ = writeln!(md, "# Archive census\n");
    let _ = writeln!(
        md,
        "Winning version of every path in the mounted VFS ({} paths), grouped by extension. Signatures are the first 4 bytes; `text` means the first 64 bytes are printable.\n",
        vfs.len()
    );
    let mut exts: Vec<_> = by_ext.iter().collect();
    exts.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
    let _ = writeln!(
        md,
        "| Ext | Files | Bytes | Signatures | Archives | Examples |"
    );
    let _ = writeln!(md, "|---|--:|--:|---|---|---|");
    for (ext, s) in exts {
        let mut sigs: Vec<_> = s.signatures.iter().collect();
        sigs.sort_by(|a, b| b.1.cmp(a.1));
        let sigs = sigs
            .iter()
            .take(4)
            .map(|(k, n)| match magic_ascii.get(*k) {
                Some(a) => format!("`{k}` {} ×{n}", a.replace('|', "\\|").replace('`', "'")),
                None => format!("{k} ×{n}"),
            })
            .collect::<Vec<_>>()
            .join(", ");
        let more = if s.signatures.len() > 4 {
            format!(" (+{} more)", s.signatures.len() - 4)
        } else {
            String::new()
        };
        let arcs = s
            .archives
            .iter()
            .map(|(a, n)| format!("{a} ×{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let ex = s
            .examples
            .iter()
            .map(|e| format!("`{e}`"))
            .collect::<Vec<_>>()
            .join("<br>");
        let _ = writeln!(
            md,
            "| {ext} | {} | {} | {sigs}{more} | {arcs} | {ex} |",
            s.count, s.bytes
        );
    }
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("census.md"), md)?;
    std::fs::write(
        out.join("census.json"),
        serde_json::to_string_pretty(&by_ext)?,
    )?;
    eprintln!("census of {} paths -> {}", vfs.len(), out.display());
    Ok(())
}
