//! `tak-cli disasm`: disassemble a COB script, or with `--stats` print an
//! opcode frequency table for every `.cob` under a prefix.

use std::collections::BTreeMap;
use std::io::Write as _;
use tak_formats::cob;
use tak_vfs::Vfs;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

pub fn run(vfs: &Vfs, path: &str, stats: bool) -> Res<()> {
    if stats {
        return frequency(vfs, path);
    }
    let script = cob::parse(&vfs.read(path)?)?;
    let d = cob::disassemble(&script);
    std::io::stdout().lock().write_all(d.text.as_bytes())?;
    if d.issues.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} issues (listed at the end of the output)",
            d.issues.len()
        )
        .into())
    }
}

fn frequency(vfs: &Vfs, prefix: &str) -> Res<()> {
    let mut freq = BTreeMap::<cob::Op, u64>::new();
    let (mut files, mut issues) = (0, 0);
    for f in vfs.list(prefix) {
        if !f.path.ends_with(".cob") {
            continue;
        }
        files += 1;
        let script = match cob::parse(&vfs.read(&f.path)?) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", f.display_path);
                issues += 1;
                continue;
            }
        };
        let l = cob::decode_all(&script);
        for e in &l.issues {
            eprintln!("{}: {e}", f.display_path);
        }
        issues += l.issues.len();
        for i in &l.instrs {
            *freq.entry(i.op).or_default() += 1;
        }
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "{files} files, {issues} issues")?;
    let mut rows: Vec<_> = freq.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (op, n) in rows {
        writeln!(out, "{n:>8}  {:#010x}  {}", op.raw(), op.mnemonic())?;
    }
    Ok(())
}
