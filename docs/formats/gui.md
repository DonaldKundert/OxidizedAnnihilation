# GUI layout files (`.gui`)

Tokenizer: `crates/tak-formats/src/gui.rs` (`gui::tokenize(&[u8]) -> Result<Gui, GuiError>`).
Tags follow [README.md](README.md).

## What it is

- **Known**: 112 `.gui` files, mostly under `guis/`. They are **not** TDF: plain ASCII text of whitespace-separated integers and length-prefixed strings, one record per line, with blank lines between groups of records. Line endings are mixed (CRLF in most files, bare LF in about a quarter).
- **Known**: a string is its byte length, one space, then the bytes: `7 gui.gaf`, `12 DefaultPanel`, `13 InGameDesktop`. Strings can contain spaces (UI captions). A string never starts with an integer word in the corpus, which is how the tokenizer tells `9 1 C` (integer 9, string `C`) apart from a 9-byte string.
- **Known**: every file tokenizes with zero errors (`gui::tests::corpus_tokenizes`), giving 13,302 strings.

## Leniencies

| Leniency | Corpus count | Where | Handling |
|---|--:|---|---|
| `CountsLineEnd` | 3 | `guis/byfilterhousesdialogue.gui` ×2, `guis/byloginconsole.gui` | The string runs to end of line and its prefix also counts the CR LF (5 visible bytes, prefix 7). The tokenizer takes the visible text. |
| `LengthMismatch` | 32 | all in `guis/byhouse0.gui` | Prefix `23` before a 24-byte `.gaf` name. The tokenizer takes the whole word. |

- **Believed**: `CountsLineEnd` means the original reader is line-based for a trailing string. **Unknown**: what the original did with the `byhouse0.gui` mismatch; a byte-exact reader would look up a gaf name missing its last letter, so that screen may be unused or broken in retail.

## Structure

- **Known**: the first line of a file is a small integer list (`1`, `2`, `9` or `19` as the first token).
- **Known** (shape only): records seen in `guis/araingame.gui` include a rectangle line (`2 0 0 640 480 1 1 1 0`: x, y, width, height and flags, Believed), a colour line (`3 255 48 48 48`, Believed ARGB or index plus RGB), a named-control line (`13 InGameDesktop 1 2`), and image references (`1 7 gui.gaf 12 DefaultPanel 0 18`: GAF file, entry name, frame, then an unknown integer).
- **Believed**: the leading integer of each line is a record type or field count.
- **Unknown**: the record grammar, how controls nest, and how the first-line header selects a version. A layout model on top of `gui::Line` tokens is the next step (stretch for WP-A, not done).
