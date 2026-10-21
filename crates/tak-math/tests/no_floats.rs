//! Enforces "no floating point in tak-math's runtime path": scans every file
//! under `src/` for float type names (anywhere, comments included) and for
//! unsuffixed float literals in code. Tests under `tests/` may use floats as
//! a reference; `src/` may not.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Describes the first float-looking token in `line`, if any.
fn find_float(line: &str) -> Option<String> {
    let b = line.as_bytes();
    for name in ["f32", "f64"] {
        let mut start = 0;
        while let Some(pos) = line[start..].find(name) {
            let i = start + pos;
            let end = i + name.len();
            let after_ok = end >= b.len() || !is_ident(b[end]);
            // A digit before still counts (literal suffix such as `1f32`).
            let before_ok = i == 0 || (!b[i - 1].is_ascii_alphabetic() && b[i - 1] != b'_');
            if after_ok && before_ok {
                return Some(format!("`{name}`"));
            }
            start = end;
        }
    }
    let code = match line.find("//") {
        Some(p) => &line[..p],
        None => line,
    };
    let cb = code.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < cb.len() {
        let c = cb[i];
        if c == b'"' && (i == 0 || cb[i - 1] != b'\\') {
            in_str = !in_str;
        } else if !in_str
            && c.is_ascii_digit()
            && (i == 0 || (!is_ident(cb[i - 1]) && cb[i - 1] != b'.'))
        {
            let radix_prefix = c == b'0' && matches!(cb.get(i + 1), Some(b'x' | b'b' | b'o'));
            let mut j = i;
            while j < cb.len() && is_ident(cb[j]) {
                j += 1;
            }
            let tok = &code[i..j];
            if !radix_prefix
                && tok.contains('e')
                && tok
                    .chars()
                    .all(|ch| ch.is_ascii_digit() || ch == 'e' || ch == '_')
            {
                return Some(format!("float literal `{tok}`"));
            }
            if !radix_prefix
                && j < cb.len()
                && cb[j] == b'.'
                && cb.get(j + 1).is_some_and(|d| d.is_ascii_digit())
            {
                return Some(format!("float literal `{}`", &code[i..j + 2]));
            }
            i = j;
            continue;
        }
        i += 1;
    }
    None
}

#[test]
fn no_floats_in_src() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(!files.is_empty());
    let mut hits = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).expect("read source");
        for (n, line) in text.lines().enumerate() {
            if let Some(what) = find_float(line) {
                hits.push(format!(
                    "{}:{}: {what}: {}",
                    f.display(),
                    n + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "floating point in tak-math src:\n{}",
        hits.join("\n")
    );
}

#[test]
fn scanner_catches_floats() {
    assert!(find_float("let x: f64 = 0;").is_some());
    assert!(find_float("let x = 1.5;").is_some());
    assert!(find_float("let x = 1f32;").is_some());
    assert!(find_float("let x = 1e3;").is_some());
    assert!(find_float("let x = self.0.0;").is_none());
    assert!(find_float("for i in 0..10 {}").is_none());
    assert!(find_float("let x = 0x3243_F6A8_885A_308D;").is_none());
    assert!(find_float("// signed 16.16 fixed point").is_none());
    assert!(find_float("let s = \"1.5\";").is_none());
}
