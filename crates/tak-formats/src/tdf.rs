//! TDF text definitions (`.tdf`, `.fbi`, `.ota`, `.tsf`): nested `[Section] { key = value; }`.
//!
//! Work package WP-A (M3). Format notes, including every leniency, live in
//! `docs/formats/tdf.md`.
//!
//! The parser is byte-oriented (Latin-1, one byte per char) and line-aware:
//!
//! - `[name]` opens a section; its body is `{ ... }`. Sections nest.
//! - Inside a body, a line `key = value;` is an entry. The value is
//!   everything after `=` up to the **last** `;` on the line, trimmed, so
//!   `English = ;;` yields `;` and a value with an embedded `;` is kept whole.
//!   A line with no `;` keeps the rest of the line (a recorded leniency).
//! - `//` starts a comment to end of line, `/* ... */` is a block comment.
//! - A stray `;` between entries is ignored (a recorded leniency).
//!
//! Section order, original key case and duplicate keys are preserved.
//! Lookups are ASCII case-insensitive and the last duplicate wins.

use crate::reader::{Reader, latin1};
use std::fmt;

/// Maximum section nesting depth. The retail corpus nests at most 3 deep;
/// the cap keeps hostile input from exhausting memory.
pub const MAX_DEPTH: usize = 32;

/// A byte range in the source, with the 1-based line of its start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
}

/// Why a TDF file failed to parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line} (offset {offset}): {kind}")]
pub struct TdfError {
    pub kind: TdfErrorKind,
    pub offset: usize,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TdfErrorKind {
    #[error("expected `[section]`, found {0:?}")]
    ExpectedSection(char),
    #[error("section header `[` is not closed on its line")]
    UnterminatedHeader,
    #[error("expected `{{` after section header `[{0}]`")]
    ExpectedOpenBrace(String),
    #[error("section `[{0}]` is not closed before end of file")]
    UnclosedSection(String),
    #[error("expected `key = value;`, found a line without `=`")]
    MissingEquals,
    #[error("`/*` comment is not closed")]
    UnterminatedComment,
    #[error("sections nest deeper than {MAX_DEPTH}")]
    TooDeep,
}

/// Something the parser accepted that a strict reading would reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LeniencyKind {
    /// `key = value` with no `;` before end of line.
    MissingSemicolon,
    /// A `;` on its own, outside any entry.
    StraySemicolon,
    /// Bytes after the last top-level section that are not whitespace or
    /// comments (e.g. a DOS `^Z` end-of-file marker).
    TrailingGarbage,
}

impl fmt::Display for LeniencyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LeniencyKind::MissingSemicolon => "missing `;` at end of line",
            LeniencyKind::StraySemicolon => "stray `;`",
            LeniencyKind::TrailingGarbage => "trailing bytes after the last section",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leniency {
    pub kind: LeniencyKind,
    pub span: Span,
}

/// One `key = value;` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Key with its original case.
    pub key: String,
    /// Raw value, trimmed of surrounding whitespace.
    pub value: String,
    /// From the first byte of the key to the last byte of the value.
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    /// Name between the brackets, original case, trimmed.
    pub name: String,
    /// The header `[name]`.
    pub span: Span,
    /// Entries in file order (duplicates kept).
    pub entries: Vec<Entry>,
    /// Nested sections in file order (duplicate names kept).
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    /// Top-level sections in file order.
    pub sections: Vec<Section>,
    /// Everything accepted leniently, in file order.
    pub leniencies: Vec<Leniency>,
}

/// A value's typed reading failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: `{key}` = {value:?} is not {expected}")]
pub struct ValueError {
    pub key: String,
    pub value: String,
    pub expected: &'static str,
    pub line: u32,
}

/// An exact decimal: `units / 10^scale`. Lets callers turn `1.25` into fixed
/// point without going through floats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decimal {
    pub units: i64,
    pub scale: u32,
}

/// Most fractional digits [`Decimal::parse`] keeps; the rest are truncated.
pub const MAX_DECIMAL_SCALE: u32 = 12;

impl Decimal {
    /// Parses `[+-]digits[.digits]` (either side may be empty, not both).
    /// Fractional digits beyond [`MAX_DECIMAL_SCALE`] are truncated.
    pub fn parse(s: &str) -> Option<Decimal> {
        let s = s.trim();
        let (neg, body) = match s.as_bytes().first()? {
            b'-' => (true, &s[1..]),
            b'+' => (false, &s[1..]),
            _ => (false, s),
        };
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        if int.is_empty() && frac.is_empty() {
            return None;
        }
        if !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut units: i64 = 0;
        for b in int.bytes() {
            units = units.checked_mul(10)?.checked_add(i64::from(b - b'0'))?;
        }
        let mut scale = 0;
        for b in frac.bytes().take(MAX_DECIMAL_SCALE as usize) {
            units = units.checked_mul(10)?.checked_add(i64::from(b - b'0'))?;
            scale += 1;
        }
        Some(Decimal {
            units: if neg { -units } else { units },
            scale,
        })
    }

    /// Converts to fixed point with `frac_bits` fractional bits, rounding to
    /// nearest (ties away from zero). `None` on overflow.
    pub fn to_fixed(self, frac_bits: u32) -> Option<i64> {
        let den = 10i128.checked_pow(self.scale)?;
        let num = i128::from(self.units).checked_mul(1i128.checked_shl(frac_bits)?)?;
        let half = den / 2;
        let q = if num >= 0 {
            (num + half) / den
        } else {
            (num - half) / den
        };
        i64::try_from(q).ok()
    }
}

impl Entry {
    fn err(&self, expected: &'static str) -> ValueError {
        ValueError {
            key: self.key.clone(),
            value: self.value.clone(),
            expected,
            line: self.span.line,
        }
    }

    /// The value as a whole number. Accepts a leading sign; rejects
    /// fractions (use [`Entry::decimal`] for those).
    pub fn int(&self) -> Result<i64, ValueError> {
        self.value
            .trim()
            .parse()
            .map_err(|_| self.err("an integer"))
    }

    /// The value as an exact decimal (`1`, `-0.5`, `.25`).
    pub fn decimal(&self) -> Result<Decimal, ValueError> {
        Decimal::parse(&self.value).ok_or_else(|| self.err("a decimal number"))
    }

    /// Whitespace-separated integers (`160 35 0`). Empty value gives `[]`.
    pub fn ints(&self) -> Result<Vec<i64>, ValueError> {
        self.value
            .split_ascii_whitespace()
            .map(|t| t.parse().map_err(|_| self.err("a list of integers")))
            .collect()
    }

    /// `0` is false, any other integer is true.
    pub fn bool(&self) -> Result<bool, ValueError> {
        Ok(self.int()? != 0)
    }
}

impl Section {
    /// The last entry whose key matches, ignoring ASCII case.
    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.key.eq_ignore_ascii_case(key))
    }

    /// The raw value of the last matching entry.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entry(key).map(|e| e.value.as_str())
    }

    pub fn get_int(&self, key: &str) -> Option<Result<i64, ValueError>> {
        self.entry(key).map(Entry::int)
    }

    pub fn get_decimal(&self, key: &str) -> Option<Result<Decimal, ValueError>> {
        self.entry(key).map(Entry::decimal)
    }

    pub fn get_ints(&self, key: &str) -> Option<Result<Vec<i64>, ValueError>> {
        self.entry(key).map(Entry::ints)
    }

    pub fn get_bool(&self, key: &str) -> Option<Result<bool, ValueError>> {
        self.entry(key).map(Entry::bool)
    }

    /// The first nested section with this name, ignoring ASCII case.
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Every nested section with this name, in file order.
    pub fn sections_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Section> {
        self.sections
            .iter()
            .filter(move |s| s.name.eq_ignore_ascii_case(name))
    }
}

impl Document {
    /// The first top-level section with this name, ignoring ASCII case.
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Every top-level section with this name, in file order.
    pub fn sections_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Section> {
        self.sections
            .iter()
            .filter(move |s| s.name.eq_ignore_ascii_case(name))
    }

    /// Visits every section depth-first, with its depth (0 = top level).
    pub fn walk(&self, mut f: impl FnMut(usize, &Section)) {
        let mut stack: Vec<(usize, &Section)> =
            self.sections.iter().rev().map(|s| (0, s)).collect();
        while let Some((d, s)) = stack.pop() {
            f(d, s);
            stack.extend(s.sections.iter().rev().map(|c| (d + 1, c)));
        }
    }
}

/// Parses a TDF-syntax file.
pub fn parse(data: &[u8]) -> Result<Document, TdfError> {
    Parser {
        r: Reader::new(data),
        line: 1,
        leniencies: Vec::new(),
    }
    .document()
}

struct Parser<'a> {
    r: Reader<'a>,
    line: u32,
    leniencies: Vec<Leniency>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.r.clone().u8("peek").ok()
    }

    fn peek2(&self) -> Option<[u8; 2]> {
        self.r.clone().array::<2>("peek").ok()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.r.u8("byte").ok()?;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn err(&self, kind: TdfErrorKind) -> TdfError {
        TdfError {
            kind,
            offset: self.r.pos(),
            line: self.line,
        }
    }

    fn span_from(&self, start: usize, line: u32) -> Span {
        Span {
            start,
            end: self.r.pos(),
            line,
        }
    }

    fn lenient(&mut self, kind: LeniencyKind, span: Span) {
        self.leniencies.push(Leniency { kind, span });
    }

    /// Skips whitespace and comments. With `semis`, also skips stray `;`.
    fn skip_trivia(&mut self, semis: bool) -> Result<(), TdfError> {
        loop {
            match (self.peek(), self.peek2()) {
                (Some(b), _) if b.is_ascii_whitespace() => {
                    self.bump();
                }
                (_, Some([b'/', b'/'])) => self.skip_line(),
                (_, Some([b'/', b'*'])) => self.skip_block_comment()?,
                (Some(b';'), _) if semis => {
                    let (start, line) = (self.r.pos(), self.line);
                    self.bump();
                    let span = self.span_from(start, line);
                    self.lenient(LeniencyKind::StraySemicolon, span);
                }
                _ => return Ok(()),
            }
        }
    }

    fn skip_line(&mut self) {
        while let Some(b) = self.peek() {
            if b == b'\n' {
                return;
            }
            self.bump();
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), TdfError> {
        let at = self.err(TdfErrorKind::UnterminatedComment);
        self.bump();
        self.bump();
        loop {
            if self.peek2() == Some(*b"*/") {
                self.bump();
                self.bump();
                return Ok(());
            }
            if self.bump().is_none() {
                return Err(at);
            }
        }
    }

    fn document(mut self) -> Result<Document, TdfError> {
        let mut sections = Vec::new();
        loop {
            self.skip_trivia(true)?;
            match self.peek() {
                None => break,
                Some(b'[') => sections.push(self.section()?),
                Some(0x1a) | Some(0) => {
                    // Trailing EOF marker or padding: accept and stop.
                    let (start, line) = (self.r.pos(), self.line);
                    while self.bump().is_some() {}
                    let span = self.span_from(start, line);
                    self.lenient(LeniencyKind::TrailingGarbage, span);
                    break;
                }
                Some(b) => return Err(self.err(TdfErrorKind::ExpectedSection(b as char))),
            }
        }
        Ok(Document {
            sections,
            leniencies: self.leniencies,
        })
    }

    /// Reads `[name]` and the opening `{`. The caller is at `[`.
    fn header(&mut self) -> Result<Section, TdfError> {
        let (start, line) = (self.r.pos(), self.line);
        self.bump();
        let name_start = self.r.pos();
        loop {
            match self.peek() {
                Some(b']') => break,
                None | Some(b'\n') | Some(b'\r') => {
                    return Err(self.err(TdfErrorKind::UnterminatedHeader));
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
        let name = latin1(&self.r.data()[name_start..self.r.pos()])
            .trim()
            .to_string();
        self.bump();
        let span = self.span_from(start, line);
        self.skip_trivia(false)?;
        if self.peek() != Some(b'{') {
            return Err(self.err(TdfErrorKind::ExpectedOpenBrace(name)));
        }
        self.bump();
        Ok(Section {
            name,
            span,
            ..Section::default()
        })
    }

    /// Parses a section and everything nested in it, iteratively.
    fn section(&mut self) -> Result<Section, TdfError> {
        let mut stack = vec![self.header()?];
        loop {
            self.skip_trivia(true)?;
            match self.peek() {
                None => {
                    let name = stack.pop().map(|s| s.name).unwrap_or_default();
                    return Err(self.err(TdfErrorKind::UnclosedSection(name)));
                }
                Some(b'}') => {
                    self.bump();
                    let done = stack.pop().expect("stack is never empty here");
                    match stack.last_mut() {
                        Some(parent) => parent.sections.push(done),
                        None => return Ok(done),
                    }
                }
                Some(b'[') => {
                    if stack.len() >= MAX_DEPTH {
                        return Err(self.err(TdfErrorKind::TooDeep));
                    }
                    let s = self.header()?;
                    stack.push(s);
                }
                Some(_) => {
                    let e = self.entry()?;
                    stack.last_mut().expect("non-empty").entries.push(e);
                }
            }
        }
    }

    /// Parses one `key = value;` line. The caller is at the key.
    fn entry(&mut self) -> Result<Entry, TdfError> {
        let (start, line) = (self.r.pos(), self.line);
        let data = self.r.data();
        // Key: up to `=` on this line.
        loop {
            match self.peek() {
                Some(b'=') => break,
                None | Some(b'\n') => return Err(self.err(TdfErrorKind::MissingEquals)),
                Some(_) => {
                    self.bump();
                }
            }
        }
        let key = latin1(&data[start..self.r.pos()]).trim().to_string();
        self.bump();
        // Value: rest of the line, minus a `//` comment, up to the last `;`.
        let value_start = self.r.pos();
        let mut content_end = value_start;
        while let Some(b) = self.peek() {
            if b == b'\n' || self.peek2() == Some(*b"//") {
                break;
            }
            self.bump();
            content_end = self.r.pos();
        }
        let rest = &data[value_start..content_end];
        let (raw, value_end) = match rest.iter().rposition(|&b| b == b';') {
            Some(i) => {
                // Resume right after the `;` so a `}` later on the line
                // (`{ k = 1; }`) still closes the section. The region
                // holds no newline, so the line count is unchanged.
                let end = value_start + i + 1;
                self.r
                    .seek(end as u64, "value end")
                    .map_err(|_| self.err(TdfErrorKind::MissingEquals))?;
                (&rest[..i], end)
            }
            None => {
                let span = Span {
                    start,
                    end: content_end,
                    line,
                };
                self.lenient(LeniencyKind::MissingSemicolon, span);
                (rest, content_end)
            }
        };
        Ok(Entry {
            key,
            value: latin1(raw).trim().to_string(),
            span: Span {
                start,
                end: value_end,
                line,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"// header comment\r\n\
[UNITINFO]\r\n\
{\r\n\
\tName = Test Unit;\r\n\
\tbuildcost = 325;\r\n\
\tmaxvelocity = 1.25;\r\n\
\tbloodcolor1 = 160 35 0;\r\n\
\tcopyright = Copyright 2000 Nobody. All rights reserved.;\r\n\
\tbuildcost = 400; // later duplicate wins\r\n\
}\r\n\
[WEAPON1]\r\n\
{\r\n\
\trange = 300;\r\n\
\t[DAMAGE]\r\n\
\t{\r\n\
\t\tdefault = 50;\r\n\
\t}\r\n\
}\r\n";

    #[test]
    fn parses_sections_entries_and_nesting() {
        let doc = parse(SAMPLE).unwrap();
        assert!(doc.leniencies.is_empty());
        assert_eq!(doc.sections.len(), 2);
        let u = doc.section("unitinfo").unwrap();
        assert_eq!(u.name, "UNITINFO");
        assert_eq!(u.entries[0].key, "Name");
        assert_eq!(u.get("NAME"), Some("Test Unit"));
        assert_eq!(u.get_int("buildcost").unwrap().unwrap(), 400);
        assert_eq!(u.entries.len(), 6, "duplicates kept");
        assert_eq!(
            u.get_ints("bloodcolor1").unwrap().unwrap(),
            vec![160, 35, 0]
        );
        assert_eq!(
            u.get_decimal("maxvelocity").unwrap().unwrap(),
            Decimal {
                units: 125,
                scale: 2
            }
        );
        assert_eq!(
            u.get("copyright"),
            Some("Copyright 2000 Nobody. All rights reserved.")
        );
        assert!(u.get_int("maxvelocity").unwrap().is_err());
        let d = doc.section("weapon1").unwrap().section("damage").unwrap();
        assert_eq!(d.get_int("default").unwrap().unwrap(), 50);
        assert_eq!(d.span.line, 14);
        let mut seen = Vec::new();
        doc.walk(|depth, s| seen.push((depth, s.name.clone())));
        assert_eq!(
            seen,
            [
                (0, "UNITINFO".to_string()),
                (0, "WEAPON1".into()),
                (1, "DAMAGE".into())
            ]
        );
    }

    #[test]
    fn spans_point_at_source() {
        let doc = parse(SAMPLE).unwrap();
        let e = doc.sections[0].entry("maxvelocity").unwrap();
        assert_eq!(&SAMPLE[e.span.start..e.span.end], b"maxvelocity = 1.25;");
        assert_eq!(e.span.line, 6);
    }

    #[test]
    fn line_oriented_values() {
        let doc = parse(
            b"[K]\n{\n a = ;;\n b = };\n c = {;\n d = x; y;\n e =;\n f = /;\n g = v; // c;\n}\n",
        )
        .unwrap();
        let s = &doc.sections[0];
        assert_eq!(s.get("a"), Some(";"));
        assert_eq!(s.get("b"), Some("}"));
        assert_eq!(s.get("c"), Some("{"));
        assert_eq!(s.get("d"), Some("x; y"));
        assert_eq!(s.get("e"), Some(""));
        assert_eq!(s.get("f"), Some("/"));
        assert_eq!(s.get("g"), Some("v"));
        assert!(doc.leniencies.is_empty());
    }

    #[test]
    fn leniencies_are_recorded() {
        let doc = parse(b"[A] {}\n[B]\n{\n h = 24\n w = 1;\n;\n}\n\x1a").unwrap();
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[1].get("h"), Some("24"));
        assert_eq!(doc.sections[1].get("w"), Some("1"));
        let kinds: Vec<_> = doc.leniencies.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [
                LeniencyKind::MissingSemicolon,
                LeniencyKind::StraySemicolon,
                LeniencyKind::TrailingGarbage
            ]
        );
        assert_eq!(doc.leniencies[0].span.line, 4);
    }

    #[test]
    fn comments_everywhere() {
        let doc = parse(b"/* block\n [X] { */\n[S] // trailing\n// between\n{ /* in */ k = 1; }\n")
            .unwrap();
        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].get("k"), Some("1"));
    }

    #[test]
    fn duplicate_sections_kept() {
        let doc = parse(b"[a]{\n[line]{\nx=1;\n}\n[LINE]{\nx=2;\n}\n}").unwrap();
        let a = &doc.sections[0];
        assert_eq!(a.sections_named("line").count(), 2);
        assert_eq!(a.section("line").unwrap().get("x"), Some("1"));
    }

    #[test]
    fn errors() {
        let kind = |b: &[u8]| parse(b).unwrap_err().kind;
        assert_eq!(kind(b"k = v;"), TdfErrorKind::ExpectedSection('k'));
        assert_eq!(kind(b"[a\n]{}"), TdfErrorKind::UnterminatedHeader);
        assert_eq!(
            kind(b"[a] k = 1;"),
            TdfErrorKind::ExpectedOpenBrace("a".into())
        );
        assert_eq!(
            kind(b"[a]{ [b]{ }"),
            TdfErrorKind::UnclosedSection("a".into())
        );
        assert_eq!(kind(b"[a]{\n novalue;\n}"), TdfErrorKind::MissingEquals);
        assert_eq!(kind(b"/* open"), TdfErrorKind::UnterminatedComment);
        assert_eq!(kind(b"[a]{ }}"), TdfErrorKind::ExpectedSection('}'));
        let deep = b"[a]{".repeat(MAX_DEPTH + 1);
        assert_eq!(kind(&deep), TdfErrorKind::TooDeep);
        let e = parse(b"[a]{\n\n x\n}").unwrap_err();
        assert_eq!(e.line, 3);
    }

    #[test]
    fn decimals() {
        let d = |s| Decimal::parse(s);
        assert_eq!(
            d("1.25"),
            Some(Decimal {
                units: 125,
                scale: 2
            })
        );
        assert_eq!(
            d("-0.5"),
            Some(Decimal {
                units: -5,
                scale: 1
            })
        );
        assert_eq!(d(".5"), Some(Decimal { units: 5, scale: 1 }));
        assert_eq!(d("3."), Some(Decimal { units: 3, scale: 0 }));
        assert_eq!(d("+7"), Some(Decimal { units: 7, scale: 0 }));
        for bad in ["", ".", "-", "1e3", "1.2.3", "abc", "99999999999999999999"] {
            assert_eq!(d(bad), None, "{bad}");
        }
        assert_eq!(d("1.25").unwrap().to_fixed(16), Some(81920));
        assert_eq!(d("-0.5").unwrap().to_fixed(16), Some(-32768));
        assert_eq!(d("0.520833333").unwrap().to_fixed(16), Some(34133));
        assert_eq!(d("1").unwrap().to_fixed(200), None);
    }

    #[test]
    fn truncations_and_byte_flips_never_panic() {
        for cut in 0..=SAMPLE.len() {
            let _ = parse(&SAMPLE[..cut]);
        }
        let mut buf = SAMPLE.to_vec();
        for i in 0..buf.len() {
            for b in [
                0u8, b'[', b']', b'{', b'}', b'=', b';', b'/', b'*', b'\n', 0xff,
            ] {
                let old = buf[i];
                buf[i] = b;
                let _ = parse(&buf);
                buf[i] = old;
            }
        }
    }

    /// Every TDF-syntax file in the install parses. Counts only, no contents.
    #[test]
    fn corpus_parses() {
        let Some(dir) = tak_vfs::testing::data_dir_or_skip("tdf corpus_parses") else {
            return;
        };
        let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
        let mut files = std::collections::BTreeMap::<String, usize>::new();
        let mut lenient = std::collections::BTreeMap::<LeniencyKind, usize>::new();
        let mut errors = Vec::new();
        for f in vfs.list("") {
            let Some((_, ext)) = f.path.rsplit_once('.') else {
                continue;
            };
            if !matches!(ext, "tdf" | "fbi" | "ota" | "tsf") {
                continue;
            }
            *files.entry(ext.to_string()).or_default() += 1;
            match parse(&vfs.read(&f.path).unwrap()) {
                Ok(doc) => {
                    for l in doc.leniencies {
                        *lenient.entry(l.kind).or_default() += 1;
                    }
                }
                Err(e) => errors.push(format!("{}: {e}", f.path)),
            }
        }
        eprintln!("files {files:?}, leniencies {lenient:?}");
        assert!(
            errors.is_empty(),
            "{} failures:\n{}",
            errors.len(),
            errors.join("\n")
        );
        assert_eq!(files.get("tdf"), Some(&1143));
        assert_eq!(files.get("fbi"), Some(&400));
        assert_eq!(files.get("ota"), Some(&310));
        assert_eq!(files.get("tsf"), Some(&5));
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(data in proptest::collection::vec(proptest::num::u8::ANY, 0..512)) {
            let _ = parse(&data);
        }

        #[test]
        fn tdf_alphabet_never_panics(data in proptest::collection::vec(
            proptest::sample::select(b"[]{}=;/* \n\rab1".to_vec()), 0..256)) {
            let _ = parse(&data);
        }
    }
}
