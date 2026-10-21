//! `tak-cli keys`: a census of TDF keys across the corpus.
//!
//! For every key path (section chain plus key) it reports how many files use
//! it, how many distinct values it takes, what those values look like, and up
//! to three example values. Examples are restricted to short numbers and
//! identifiers so the output can be committed without retail prose.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;
use tak_formats::tdf;

#[derive(clap::Args)]
pub struct KeysArgs {
    /// Only files with this extension (e.g. `fbi`, `tdf`, `ota`). Repeatable.
    #[arg(long)]
    ext: Vec<String>,
    /// Only files under this path prefix (case-insensitive, e.g. `gamedata`).
    /// Repeatable; default is everything.
    #[arg(long)]
    prefix: Vec<String>,
    /// Only files directly in a prefix directory, not in its subdirectories.
    #[arg(long)]
    no_subdirs: bool,
    /// Only top-level sections with this name (case-insensitive).
    #[arg(long)]
    section: Option<String>,
    /// Collapse every top-level section name to `*` (for files whose top
    /// sections are named after the thing they define).
    #[arg(long)]
    wild_top: bool,
    /// Prefix each section path with the file's directory.
    #[arg(long)]
    by_dir: bool,
    /// Write Markdown here instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
}

/// The shape of one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Shape {
    Empty,
    Int,
    Decimal,
    Ints,
    Decimals,
    Word,
    Text,
}

impl Shape {
    fn of(v: &str) -> Shape {
        let toks: Vec<&str> = v.split_ascii_whitespace().collect();
        let int = |t: &str| t.parse::<i64>().is_ok();
        let dec = |t: &str| tdf::Decimal::parse(t).is_some();
        match toks.as_slice() {
            [] => Shape::Empty,
            [t] if int(t) => Shape::Int,
            [t] if dec(t) => Shape::Decimal,
            [t] if t
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b)) =>
            {
                Shape::Word
            }
            ts if ts.iter().all(|t| int(t)) => Shape::Ints,
            ts if ts.iter().all(|t| dec(t)) => Shape::Decimals,
            _ => Shape::Text,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Shape::Empty => "empty",
            Shape::Int => "int",
            Shape::Decimal => "decimal",
            Shape::Ints => "ints",
            Shape::Decimals => "decimals",
            Shape::Word => "word",
            Shape::Text => "text",
        }
    }
}

/// Values short and plain enough to quote in committed docs.
fn quotable(v: &str) -> bool {
    v.len() <= 24
        && matches!(
            Shape::of(v),
            Shape::Int | Shape::Decimal | Shape::Ints | Shape::Decimals | Shape::Word
        )
}

#[derive(Default)]
struct KeyStats {
    files: BTreeSet<usize>,
    uses: usize,
    values: BTreeMap<String, usize>,
    shapes: BTreeMap<Shape, usize>,
    /// Spellings of the key as written, to show case variation.
    spellings: BTreeSet<String>,
}

/// Generalizes a section name: uppercase, digit runs become `#`.
fn generic(name: &str) -> String {
    let mut out = String::new();
    let mut in_digits = false;
    for c in name.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push('#');
            }
            in_digits = true;
        } else {
            out.push(c.to_ascii_uppercase());
            in_digits = false;
        }
    }
    out
}

pub fn keys(vfs: &tak_vfs::Vfs, args: KeysArgs) -> Result<(), Box<dyn Error>> {
    let exts: Vec<String> = if args.ext.is_empty() {
        ["tdf", "fbi", "ota", "tsf"].map(String::from).to_vec()
    } else {
        args.ext.iter().map(|e| e.to_ascii_lowercase()).collect()
    };
    let mut stats: BTreeMap<(String, String), KeyStats> = BTreeMap::new();
    let mut sections: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut n_files = 0usize;
    let mut errors = Vec::new();

    let prefixes = if args.prefix.is_empty() {
        vec![String::new()]
    } else {
        args.prefix.iter().map(|p| tak_vfs::normalize(p)).collect()
    };
    let mut seen = BTreeSet::new();
    let files = prefixes.iter().flat_map(|p| {
        vfs.list(p).into_iter().filter(move |f| {
            !args.no_subdirs || {
                let rest = f.path[p.len()..].trim_start_matches('/');
                !rest.contains('/')
            }
        })
    });
    for f in files {
        if !seen.insert(f.path.clone()) {
            continue;
        }
        let ext = f.path.rsplit_once('.').map_or("", |(_, e)| e);
        if !exts.iter().any(|e| e == ext) {
            continue;
        }
        let doc = match tdf::parse(&vfs.read(&f.path)?) {
            Ok(d) => d,
            Err(e) => {
                errors.push(format!("{}: {e}", f.path));
                continue;
            }
        };
        let fi = n_files;
        n_files += 1;
        for top in &doc.sections {
            if let Some(want) = &args.section {
                if !top.name.eq_ignore_ascii_case(want) {
                    continue;
                }
            }
            let top_name = if args.wild_top {
                "*".to_string()
            } else {
                generic(&top.name)
            };
            // Depth-first with an explicit stack: (path, section).
            let dir = if args.by_dir {
                let d = f.path.rsplit_once('/').map_or("", |(d, _)| d);
                format!("{d}/")
            } else {
                String::new()
            };
            let mut stack = vec![(format!("{dir}[{top_name}]"), top)];
            while let Some((path, s)) = stack.pop() {
                sections.entry(path.clone()).or_default().insert(fi);
                for e in &s.entries {
                    let k = stats
                        .entry((path.clone(), e.key.to_ascii_lowercase()))
                        .or_default();
                    k.files.insert(fi);
                    k.uses += 1;
                    *k.values.entry(e.value.clone()).or_default() += 1;
                    *k.shapes.entry(Shape::of(&e.value)).or_default() += 1;
                    k.spellings.insert(e.key.clone());
                }
                for c in s.sections.iter().rev() {
                    stack.push((format!("{path}/[{}]", generic(&c.name)), c));
                }
            }
        }
    }

    let md = render(&args, &exts, n_files, &sections, &stats, &errors);
    match &args.out {
        Some(p) => {
            std::fs::write(p, md)?;
            eprintln!(
                "{} files, {} key paths -> {}",
                n_files,
                stats.len(),
                p.display()
            );
        }
        None => {
            use std::io::Write;
            std::io::stdout().write_all(md.as_bytes())?;
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("{} files failed to parse", errors.len()).into())
    }
}

/// Section paths with more one-file keys than this fold them into one row.
const FOLD_OVER: usize = 40;

fn shapes(k: &KeyStats) -> String {
    k.shapes
        .iter()
        .map(|(s, n)| format!("{} {n}", s.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Up to three of the most common quotable values. When any value of the
/// key is free text, only numeric values are quoted, so names and titles
/// never leak into the docs.
fn examples(k: &KeyStats) -> String {
    let has_text = k.shapes.contains_key(&Shape::Text);
    let mut common: Vec<(&String, &usize)> = k
        .values
        .iter()
        .filter(|(v, _)| quotable(v) && !(has_text && Shape::of(v) == Shape::Word))
        .collect();
    common.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    common
        .iter()
        .take(3)
        .map(|(v, _)| format!("`{v}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render(
    args: &KeysArgs,
    exts: &[String],
    n_files: usize,
    sections: &BTreeMap<String, BTreeSet<usize>>,
    stats: &BTreeMap<(String, String), KeyStats>,
    errors: &[String],
) -> String {
    let mut cmd = format!("tak-cli keys --ext {}", exts.join(" --ext "));
    for p in &args.prefix {
        let _ = write!(cmd, " --prefix {p}");
    }
    if args.no_subdirs {
        cmd.push_str(" --no-subdirs");
    }
    if let Some(s) = &args.section {
        let _ = write!(cmd, " --section {s}");
    }
    if args.wild_top {
        cmd.push_str(" --wild-top");
    }
    if args.by_dir {
        cmd.push_str(" --by-dir");
    }
    let mut md = String::new();
    let _ = writeln!(
        md,
        "<!-- Generated by `{cmd}` on the GOG install (Kingdoms + Iron Plague 2.0.0.4). \
         Key names, counts and short numeric/identifier examples only. -->"
    );
    let _ = writeln!(md, "# Key census: `{cmd}`\n");
    let _ = writeln!(
        md,
        "{n_files} files, {} section paths, {} key paths, {} parse errors. \
         Section names are uppercased with digit runs shown as `#`{}. \
         Keys are lowercased; **Files** counts files using the key, **Uses** counts entries \
         (more than Files means repeated sections or duplicate keys). \
         **Shapes** classifies values: int, decimal, ints/decimals (space-separated lists), \
         word (one identifier-like token), text (anything else), empty. \
         Examples are the most common values that are short numbers or identifiers; \
         longer strings are never quoted, and identifiers are not quoted for keys that \
         also hold free text. Section paths with more than {FOLD_OVER} keys that each appear in only one file \
         (keys that are data, such as sound names) fold those keys into one row.\n",
        sections.len(),
        stats.len(),
        errors.len(),
        match (args.wild_top, args.by_dir) {
            (true, true) =>
                "; paths start with the file's directory and top-level names are collapsed to `*`",
            (true, false) => "; top-level names are collapsed to `*`",
            (false, true) => "; paths start with the file's directory",
            (false, false) => "",
        }
    );
    let _ = writeln!(md, "## Sections\n\n| Section path | Files |\n|---|--:|");
    for (p, f) in sections {
        let _ = writeln!(md, "| `{}` | {} |", p.replace('|', "\\|"), f.len());
    }
    let _ = writeln!(
        md,
        "\n## Keys\n\n| Section path | Key | Files | Uses | Distinct | Shapes | Examples |\n|---|---|--:|--:|--:|---|---|"
    );
    // Sections whose keys are data (sound names, key bindings) get their
    // single-file keys folded into one summary row.
    let mut per_section: BTreeMap<&str, Vec<(&str, &KeyStats)>> = BTreeMap::new();
    for ((sec, key), k) in stats {
        per_section.entry(sec).or_default().push((key, k));
    }
    for (sec, keys) in per_section {
        let sec_cell = sec.replace('|', "\\|");
        let fold = keys.iter().filter(|(_, k)| k.files.len() <= 1).count() > FOLD_OVER;
        let mut folded = KeyStats::default();
        let mut n_folded = 0usize;
        for (key, k) in keys {
            if fold && k.files.len() <= 1 {
                n_folded += 1;
                folded.files.extend(&k.files);
                folded.uses += k.uses;
                for (s, n) in &k.shapes {
                    *folded.shapes.entry(*s).or_default() += n;
                }
                continue;
            }
            let key_cell = if k.spellings.len() > 1 {
                format!("`{key}` ({} spellings)", k.spellings.len())
            } else {
                format!("`{key}`")
            };
            let _ = writeln!(
                md,
                "| `{sec_cell}` | {key_cell} | {} | {} | {} | {} | {} |",
                k.files.len(),
                k.uses,
                k.values.len(),
                shapes(k),
                examples(k),
            );
        }
        if n_folded > 0 {
            let _ = writeln!(
                md,
                "| `{sec_cell}` | *{n_folded} more keys, each in one file* | {} | {} |  | {} |  |",
                folded.files.len(),
                folded.uses,
                shapes(&folded),
            );
        }
    }
    if !errors.is_empty() {
        let _ = writeln!(md, "\n## Parse errors\n");
        for e in errors {
            let _ = writeln!(md, "- {e}");
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        assert_eq!(Shape::of(""), Shape::Empty);
        assert_eq!(Shape::of("12"), Shape::Int);
        assert_eq!(Shape::of("-1.5"), Shape::Decimal);
        assert_eq!(Shape::of("1 2 3"), Shape::Ints);
        assert_eq!(Shape::of("1 2.5"), Shape::Decimals);
        assert_eq!(Shape::of("ARA_unit"), Shape::Word);
        assert_eq!(Shape::of("two words"), Shape::Text);
    }

    #[test]
    fn quotable_rejects_prose() {
        assert!(quotable("325"));
        assert!(quotable("GROUND2"));
        assert!(!quotable("Copyright 1999 Somebody."));
        assert!(!quotable(""));
        assert!(!quotable("averyveryverylongidentifiername"));
    }

    #[test]
    fn generic_names() {
        assert_eq!(generic("Weapon12"), "WEAPON#");
        assert_eq!(generic("damage"), "DAMAGE");
    }
}
