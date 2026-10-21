# WP-A: TDF text definitions and key census (M3)

**Owns:** `crates/tak-formats/src/tdf.rs`, `crates/tak-formats/src/gui.rs`, `docs/formats/tdf.md`, `docs/formats/gui.md`, `crates/tak-cli/src/keys_cmd.rs`.

## Why it matters
Almost every gameplay number lives in TDF syntax: units, weapons, features, build menus, maps, missions. WP-H (content definitions) and the OTA half of WP-E build on this parser.

## What we know
- **Known** (from the census, `docs/formats/census.md`): TDF syntax in `.tdf` (1,143 visible), `.fbi` (400: `units/` 203, `unitscb/` 197), `.ota` (310). `.tsf` (5) is text, syntax unchecked.
- **Known**: shape is `[Section]` then `{ key = value; ... }`, sections nest, keys are case-insensitive in practice, values can contain spaces and punctuation (copyright strings), tabs for indent, CRLF line endings.
- **Believed** (TA): `//` and `/* */` comments; a missing `;` before `}` is tolerated by the original; duplicate keys keep the last value. Confirm against the corpus and record exceptions.
- **Known**: `.gui` (112 files in `guis/`) is **not** TDF. It is whitespace-separated numeric tokens with occasional string tokens (`13 InGameDesktop 1 2`, `1 7 gui.gaf 12 DefaultPanel 0 18`). It looks like length-prefixed strings (`7 gui.gaf`). Stretch goal only.
- Directories: `gamedata/` 241 tdf, `features/` 412, `canbuild/` 196 + `canbuildcb/` 171, `translate/` 39, `fonts/` 11, `missions/` 64, `maps/` 6, `camps/` 3.
- Unknown: what `unitscb/` and `canbuildcb/` are (a second unit set? campaign variants?). Compare against `units/`.

## Tasks
1. `tdf::parse(&[u8]) -> Result<Document, TdfError>` preserving section order, original key case, and byte spans for error messages. Lenient where the original is lenient; record every leniency in `docs/formats/tdf.md`.
2. Case-insensitive lookup helpers (`doc.section("UNITINFO")?.get("buildcost")`) and typed getters (int, list of ints, string).
3. Corpus test: every TDF-syntax file in the VFS parses with zero errors.
4. `tak-cli keys [--ext fbi] [--section UNITINFO]`: key census over the corpus, listing every key with file count, distinct value count, and up to three example values (short numeric or identifier values only, no long strings). Write the result to `docs/formats/fbi-keys.md` etc. Key names and small numbers are fine to commit; prose strings are not.
5. Stretch: `.gui` tokenizer and a first layout model.

## Done when
- 100% of `.tdf`, `.fbi`, `.ota` files parse; `.tsf` checked and documented.
- Key census docs committed for `.fbi`, `gamedata/*.tdf` (weapons and similar), `features/*.tdf`, `.ota`.
- Fuzz-style test: truncations and byte flips never panic.
