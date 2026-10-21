//! GUI layout files (`.gui`): whitespace-separated integers and
//! length-prefixed strings. Not TDF. Work package WP-A (M3, stretch).
//!
//! Format notes live in `docs/formats/gui.md`. This module only tokenizes;
//! the layout model on top of the tokens is still Unknown.
//!
//! A string is written as its byte length, one space, then the bytes:
//! `7 gui.gaf`. The tokenizer treats an integer `n` as a string prefix when
//! exactly one space follows it, the next `n` bytes contain no line break or
//! tab, neither start nor end with a space, are followed by whitespace or end
//! of file, and do not start with an integer word. Everything else must be an
//! integer.

use crate::reader::{Reader, latin1};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Int(i64),
    Str(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// 1-based line number in the file.
    pub line: u32,
    pub tokens: Vec<Token>,
}

/// A string accepted although its length prefix does not match exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiLeniency {
    /// The string runs to end of line and the prefix also counts the line
    /// terminator (`7 Win %` then CR LF: 5 bytes plus 2).
    CountsLineEnd,
    /// The prefix disagrees with the length of the following word; the whole
    /// word was taken.
    LengthMismatch,
}

/// A tokenized `.gui` file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Gui {
    /// Non-empty lines in file order.
    pub lines: Vec<Line>,
    /// (line number, what) for every leniently accepted string.
    pub leniencies: Vec<(u32, GuiLeniency)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line} (offset {offset}): {kind}")]
pub struct GuiError {
    pub kind: GuiErrorKind,
    pub offset: usize,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuiErrorKind {
    #[error("token {0:?} is neither an integer nor a length-prefixed string")]
    BadToken(String),
}

/// Tokenizes a `.gui` file into non-empty lines of tokens.
pub fn tokenize(data: &[u8]) -> Result<Gui, GuiError> {
    let mut r = Reader::new(data);
    let mut line = 1u32;
    let mut out: Vec<Line> = Vec::new();
    let mut cur: Vec<Token> = Vec::new();
    let mut leniencies = Vec::new();
    while let Ok(b) = r.clone().u8("peek") {
        if b == b'\n' {
            let _ = r.u8("newline");
            if !cur.is_empty() {
                out.push(Line {
                    line,
                    tokens: std::mem::take(&mut cur),
                });
            }
            line += 1;
            continue;
        }
        if b.is_ascii_whitespace() {
            let _ = r.u8("space");
            continue;
        }
        let start = r.pos();
        let rest = &data[start..];
        let len = rest
            .iter()
            .position(|c| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let word = &rest[..len];
        let text = latin1(word);
        let Ok(n) = text.parse::<i64>() else {
            return Err(GuiError {
                kind: GuiErrorKind::BadToken(text),
                offset: start,
                line,
            });
        };
        let _ = r.skip(len, "token");
        if let Some((s, how)) = string_after(&r, n) {
            let _ = r.skip(1 + s.len(), "string");
            cur.push(Token::Str(latin1(s)));
            if let Some(how) = how {
                leniencies.push((line, how));
            }
        } else {
            cur.push(Token::Int(n));
        }
    }
    if !cur.is_empty() {
        out.push(Line { line, tokens: cur });
    }
    Ok(Gui {
        lines: out,
        leniencies,
    })
}

/// The string after a single space, if `n` is its length prefix.
fn string_after<'a>(r: &Reader<'a>, n: i64) -> Option<(&'a [u8], Option<GuiLeniency>)> {
    let n = usize::try_from(n).ok().filter(|&n| n > 0)?;
    let mut probe = r.clone();
    if probe.u8("space").ok()? != b' ' {
        return None;
    }
    let rest = &probe.data()[probe.pos()..];
    let is_int = |w: &[u8]| latin1(w).parse::<i64>().is_ok();
    let first_len = rest
        .iter()
        .position(|c| c.is_ascii_whitespace())
        .unwrap_or(rest.len());
    let first = &rest[..first_len];
    // `9 1 C` is the integer 9 then the string `C`, not a 9-byte string:
    // a string never starts with an integer word.
    if first.is_empty() || is_int(first) {
        return None;
    }
    // Exact: n bytes on this line, no edge spaces, then whitespace or EOF.
    if let Some(s) = rest.get(..n) {
        let ends_cleanly = rest.get(n).is_none_or(|c| c.is_ascii_whitespace());
        let plain = !s.iter().any(|c| c.is_ascii_whitespace() && *c != b' ') && !s.ends_with(b" ");
        if ends_cleanly && plain {
            return Some((s, None));
        }
    }
    // The prefix counts the line terminator.
    let line_len = rest
        .iter()
        .position(|&c| c == b'\r' || c == b'\n')
        .unwrap_or(rest.len());
    let term = match rest.get(line_len..) {
        Some([b'\r', b'\n', ..]) => 2,
        Some([_, ..]) => 1,
        _ => 0,
    };
    if term > 0 && n == line_len + term {
        return Some((&rest[..line_len], Some(GuiLeniency::CountsLineEnd)));
    }
    // The prefix is wrong; take the word.
    Some((first, Some(GuiLeniency::LengthMismatch)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_and_strings() {
        let gui =
            tokenize(b"2 1 \r\n13 SyntheticPane 1 2 \r\n\r\n1 7 big.gaf 9 Two Words 0 -18\r\n1 0")
                .unwrap();
        assert!(gui.leniencies.is_empty());
        let lines = gui.lines;
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].tokens, [Token::Int(2), Token::Int(1)]);
        assert_eq!(
            lines[1].tokens,
            [
                Token::Str("SyntheticPane".into()),
                Token::Int(1),
                Token::Int(2)
            ]
        );
        assert_eq!(lines[2].line, 4);
        assert_eq!(
            lines[2].tokens,
            [
                Token::Int(1),
                Token::Str("big.gaf".into()),
                Token::Str("Two Words".into()),
                Token::Int(0),
                Token::Int(-18)
            ]
        );
        assert_eq!(lines[3].tokens, [Token::Int(1), Token::Int(0)]);
    }

    #[test]
    fn lenient_lengths() {
        let gui = tokenize(b"2 6 Ab %\r\n1 5 four.gaf 0\n9 1 C 2").unwrap();
        assert_eq!(gui.lines[0].tokens[1], Token::Str("Ab %".into()));
        assert_eq!(gui.lines[1].tokens[1], Token::Str("four.gaf".into()));
        assert_eq!(gui.lines[1].tokens[2], Token::Int(0));
        assert_eq!(
            gui.lines[2].tokens,
            [
                // `1` is the prefix of `C`, so it is not a token of its own.
                Token::Int(9),
                Token::Str("C".into()),
                Token::Int(2)
            ]
        );
        assert_eq!(
            gui.leniencies,
            [
                (1, GuiLeniency::CountsLineEnd),
                (2, GuiLeniency::LengthMismatch)
            ]
        );
    }

    #[test]
    fn bare_words_are_errors() {
        let e = tokenize(b"1 2\nhello 3").unwrap_err();
        assert_eq!(e.line, 2);
        assert_eq!(e.kind, GuiErrorKind::BadToken("hello".into()));
    }

    #[test]
    fn truncations_never_panic() {
        let s = b"1 7 gui.gaf 12 DefaultPanel 0 18\r\n3 255 48 48 48";
        for cut in 0..=s.len() {
            let _ = tokenize(&s[..cut]);
        }
        let _ = tokenize(b"99999999999999999999999 4");
        let _ = tokenize(b"9223372036854775807 x");
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(data in proptest::collection::vec(proptest::num::u8::ANY, 0..256)) {
            let _ = tokenize(&data);
        }
    }

    /// Every `.gui` file tokenizes. Counts only.
    #[test]
    fn corpus_tokenizes() {
        let Some(dir) = tak_vfs::testing::data_dir_or_skip("gui corpus_tokenizes") else {
            return;
        };
        let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
        let (mut files, mut strings, mut errors) = (0, 0, Vec::new());
        let mut lenient = std::collections::BTreeMap::<String, usize>::new();
        for f in vfs.list("") {
            if !f.path.ends_with(".gui") {
                continue;
            }
            files += 1;
            match tokenize(&vfs.read(&f.path).unwrap()) {
                Ok(gui) => {
                    for (_, l) in gui.leniencies {
                        *lenient.entry(format!("{l:?}")).or_default() += 1;
                    }
                    strings += gui
                        .lines
                        .iter()
                        .flat_map(|l| &l.tokens)
                        .filter(|t| matches!(t, Token::Str(_)))
                        .count()
                }
                Err(e) => errors.push(format!("{}: {e}", f.path)),
            }
        }
        eprintln!("{files} gui files, {strings} strings, leniencies {lenient:?}");
        assert!(errors.is_empty(), "{}", errors.join("\n"));
        assert_eq!(files, 112);
    }
}
