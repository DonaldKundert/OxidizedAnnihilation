//! Markdown summary of a [`Survey`]: signatures, extensions and archive
//! headers. Paths and header numbers only, never file contents.

use crate::survey::{Signature, Survey};
use std::fmt::Write;

pub fn markdown(s: &Survey) -> String {
    let mut o = String::new();
    let total: u64 = s.files.iter().map(|f| f.size).sum();
    let _ = writeln!(o, "# Install survey\n");
    let _ = writeln!(o, "- Root: `{}`", s.root);
    let _ = writeln!(o, "- Files: {} ({})", s.files.len(), human(total));
    let _ = writeln!(o, "- Symlinks (not followed): {}", s.symlinks.len());
    let _ = writeln!(o, "- Read errors: {}\n", s.errors.len());

    let _ = writeln!(o, "## Signatures\n");
    let _ = writeln!(
        o,
        "| Signature | ASCII | Files | Bytes | Extensions | Example |"
    );
    let _ = writeln!(o, "|---|---|--:|--:|---|---|");
    for g in &s.signatures {
        let sig = match &g.signature {
            Signature::Empty => "(empty)".to_string(),
            Signature::Text => "(text)".to_string(),
            Signature::Magic(h) => format!("`{h}`"),
        };
        let exts = g
            .extensions
            .iter()
            .map(|(e, n)| format!("{} ×{n}", if e.is_empty() { "(none)" } else { e }))
            .collect::<Vec<_>>()
            .join(", ");
        let ascii = if g.ascii.is_empty() {
            String::new()
        } else {
            format!("`{}`", g.ascii.replace('|', "\\|").replace('`', "'"))
        };
        let ex = g
            .examples
            .first()
            .map(|p| format!("`{p}`"))
            .unwrap_or_default();
        let _ = writeln!(
            o,
            "| {sig} | {ascii} | {} | {} | {exts} | {ex} |",
            g.count,
            human(g.total_bytes)
        );
    }

    let _ = writeln!(o, "\n## Extensions\n");
    let _ = writeln!(o, "| Extension | Files | Bytes | Signatures |");
    let _ = writeln!(o, "|---|--:|--:|---|");
    for e in &s.extensions {
        let sigs = e
            .signatures
            .iter()
            .map(|s| match s {
                Signature::Empty => "empty".to_string(),
                Signature::Text => "text".to_string(),
                Signature::Magic(h) => h.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        let ext = if e.ext.is_empty() { "(none)" } else { &e.ext };
        let _ = writeln!(
            o,
            "| {ext} | {} | {} | {sigs} |",
            e.count,
            human(e.total_bytes)
        );
    }

    let archives: Vec<_> = s.files.iter().filter(|f| f.hapi.is_some()).collect();
    if !archives.is_empty() {
        let mut versions: Vec<u32> = archives
            .iter()
            .filter_map(|f| f.hapi.as_ref().map(|h| h.version))
            .collect();
        versions.sort_unstable();
        versions.dedup();
        let _ = writeln!(o, "\n## HAPI archives\n");
        let vs = versions
            .iter()
            .map(|v| format!("`0x{v:08x}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(o, "{} archives. Header versions: {vs}.\n", archives.len());
        let _ = writeln!(
            o,
            "Words are the little-endian u32s at offsets 8..32, uninterpreted. \
             Loose `.kmp` maps are listed by count only.\n"
        );
        let _ = writeln!(
            o,
            "| Path | Size | Version | w8 | w12 | w16 | w20 | w24 | w28 |"
        );
        let _ = writeln!(o, "|---|--:|---|--:|--:|--:|--:|--:|--:|");
        let mut kmp = 0usize;
        for f in &archives {
            if f.ext == "kmp" {
                kmp += 1;
                continue;
            }
            let h = f.hapi.as_ref().expect("filtered");
            let w = h.words.map(|w| w.to_string());
            let _ = writeln!(
                o,
                "| `{}` | {} | `0x{:08x}` | {} |",
                f.path,
                f.size,
                h.version,
                w.join(" | ")
            );
        }
        if kmp > 0 {
            let _ = writeln!(o, "\nPlus {kmp} `.kmp` map archives.");
        }
    }

    if !s.symlinks.is_empty() {
        let _ = writeln!(o, "\n## Symlinks\n");
        for l in &s.symlinks {
            let _ = writeln!(o, "- `{}` → `{}`", l.path, l.target);
        }
    }
    if !s.errors.is_empty() {
        let _ = writeln!(o, "\n## Errors\n");
        for (p, e) in &s.errors {
            let _ = writeln!(o, "- `{p}`: {e}");
        }
    }
    o
}

fn human(n: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(51_735_874), "49.3 MB");
    }
}
