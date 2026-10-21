# TDF text definitions (`.tdf`, `.fbi`, `.ota`, `.tsf`)

Parser: `crates/tak-formats/src/tdf.rs` (`tdf::parse(&[u8]) -> Result<Document, TdfError>`).
Key census: `tak-cli keys` (see [Key census docs](#key-census-docs)).
Tags follow [README.md](README.md).

## Corpus

- **Known**: every TDF-syntax file in the GOG install parses with zero errors: 1,143 `.tdf`, 400 `.fbi`, 310 `.ota`, 5 `.tsf` (corpus test `tdf::tests::corpus_parses`).
- **Known**: `.tsf` (5 files under `anims/`, from the BY/meta-player archives) is TDF syntax. All 5 start with a `/* */` block comment, have one top-level section and nest 2 deep.
- **Known**: files are Latin-1 text with CRLF line endings and tab indents. No BOM, no NUL, no `^Z` in any file. 34 `.tdf` files (translations, `keys.tdf`) contain bytes ≥ 0x80; the parser decodes one byte per char.

## Syntax

```
// comment
[Section Name]
{
	key = value with spaces, punctuation.;
	[Nested]
	{
		other = 1 2 3;
	}
}
```

- **Known**: `[name]` opens a section; the name can contain spaces and digits (`[creon lightbeam]`, `[MAP DATA]`). The body follows in `{ }`, either on the next line or on the same line (`[ARAARCH]	{}` in `maps/*.tdf`). A `//` comment may sit between `]` and `{`.
- **Known**: sections nest. Maximum depth in the corpus: `.ota` 4 levels (`GLOBALHEADER/MAP DATA/SPECIALS/SPECIAL#`, `.../UNITS/UNIT#`), `.tsf` 3, one `.tdf` (`gamedata/effects/effects.tdf`) 3, `.fbi` 2. The parser caps depth at 32 and errors beyond.
- **Known**: entries are one per line, `key = value;`. Whitespace around `=` is optional (`seqname=X;`). No file puts two entries on one line, and no entry spans lines.
- **Known**: values are raw text: numbers (`1100`, `0.52083`, `-3`), space-separated lists (`160 35 0`), identifiers, or free text with spaces and punctuation (copyright strings, translations). There is no quoting; `"` appears only inside prose.
- **Known**: `//` comments appear at line start, after an entry's `;`, and after a `]`. `/* */` block comments appear at the start of 2 translate files and all 5 `.tsf` files. No value contains `//`.
- **Known**: no content appears outside a top-level section.
- **Known**: keys never contain spaces and never repeat with different case inside one key path across a census (the "spellings" note in the census docs never fires).
- **Believed** (TA): keys are case-insensitive. Lookups (`Section::get`, `section()`) ignore ASCII case; original case is preserved in `Entry::key` and `Section::name`.

### Value termination: line-oriented

The parser takes a value as everything after `=` up to the **last** `;` on the line, after removing a `//` comment, trimmed. Parsing resumes right after that `;`, so a `}` later on the same line still closes the section.

- **Known**, and why: `translate/customkeys.tdf` labels the `;` key as `English = ;;` and `}` as `English = };`; `translate/messages.tdf` has a French string with an embedded `;`. A first-`;` rule would lose these. `gamedata/keys.tdf` writes `SYMBOL_3B =; // ';'`, which the comment strip handles.
- **Unknown**: whether the original engine reads to the first or last `;`. Only those 2 values differ between the two readings, and both are UI text.
- Limitation, by design: `a = 1; b = 2;` on one line would read as `a` = `1; b = 2`. The corpus never does this.

## Leniencies

Each is recorded in `Document::leniencies` with a span, never silently.

| Leniency | Corpus count | Where | Handling |
|---|--:|---|---|
| `MissingSemicolon` | 7 | `gamedata/effects/effects.tdf` ×3, `translate/crusades.tdf` ×2, `features/zhon/zonplant.tdf`, `fonts/bodfontbody.tdf` | Value runs to end of line. Each case is followed by a normal entry or `}` on the next line. |
| `StraySemicolon` | 1 | `features/zhon/zonruin.tdf` (a lone `;` line) | Ignored. |
| `TrailingGarbage` | 0 | none | A `^Z` or NUL after the last section ends the file. Defensive only. |

- **Believed** (TA): the original tolerates a missing `;` before `}`. In Kingdoms the missing `;` cases are mid-section, so the original must also end a value at the line break, or it would have swallowed the next line into the value (`height = 24` + `anchorx = 0;`). **Unknown** which the engine did; line-ending is the reading that keeps every key.

## Duplicates and order

- **Known**: duplicate keys inside one section occur 16 times, all in `features/*` (e.g. `reclaimable` twice). `Section::entries` keeps every occurrence in file order; `get` returns the last.
- **Believed** (TA): last duplicate wins. Not confirmed against the engine.
- **Known**: duplicate section names occur: 14 nested (e.g. repeated `[line lightning]` emitters in `effects.tdf`) and 147 top-level (e.g. unit lists in `missions/*.tdf`). Both are kept in order. `section(name)` returns the first; `sections_named(name)` iterates all. Repeated nested sections are meaningful (a list), so content code must not collapse them.
- **Known**: 4,652 sections in `.tdf` files are empty (`{}`): unit allow/deny lists in `maps/` and `missions/`, font tables.
- **Known**: empty values (`key =;` or `key = ;`) are common: 7,257 in `.ota` (mostly `ident` in `UNIT#`), 596 in `.tdf`.

## Typed getters

- `Entry::int` / `Section::get_int`: `i64`, sign allowed, rejects fractions.
- `Entry::decimal` / `get_decimal`: exact `Decimal { units, scale }` (`1.25` → 125 × 10⁻²), with `Decimal::to_fixed(frac_bits)` for float-free conversion to fixed point (16 bits for `tak_math::Fx`). Up to 12 fractional digits kept, the rest truncated.
- `Entry::ints` / `get_ints`: whitespace-separated integers.
- `Entry::bool` / `get_bool`: integer, nonzero is true.
- **Known**: many numeric keys mix ints and decimals across files (`maxvelocity`: int 53, decimal 246), so content code should read them as `Decimal`. The census "Shapes" column shows which.

## Directory notes

- **Known**: `units/` (203 `.fbi`) and `unitscb/` (197 `.fbi`) both come from `V3Rocket.hpi`. `unitscb/` lacks 6 units that `units/` has (e.g. `monelem`, `monmum`, `tarhel`). Compared files differ in balance values: `unitscb` uses tiered `damagecategory` values (`Tier1`) and adds per-armor damage multipliers in `[DAMAGE]` (`fort`, `factory`, `stalwart`, `mechanical`, `naval`). `canbuildcb/` (171) parallels `canbuild/` (196) the same way.
- **Believed**: `cb` is an alternate balance ruleset shipped with the 3.x patch, selectable at game setup. **Unknown**: what selects it and what `cb` stands for.
- **Known**: Kingdoms has no `weapons/` directory. Weapons are `[WEAPON1]`..`[WEAPONn]` sections inside each `.fbi` (277 files have one or more), each with a nested `[DAMAGE]` section of per-category multipliers. `[EXPLODEAS]` (12 files) uses the same shape for death explosions.
- **Known**: `canbuild*/<builder>/<unit>.tdf` files each hold one section named after a unit with a single key, `priority`.
- **Known**: `gamedata/soundclasses/*.tdf` (231) use sound file names as keys under `[SELECT]`, `[MOVE]`, `[ATTACK]` etc., with a decimal weight as the value. Keys there are data, not schema.
- **Known**: `fonts/*.tdf` hold one `[Kerning Table]` with sub-sections per case pairing (`[Uppercase Lowercase Pairs]`, ...); keys are two-letter pairs. Case-insensitive lookup is safe because each pairing has its own section.

## Key census docs

Generated by `tak-cli keys` (key names, counts, value shapes, short numeric/identifier examples only):

- [fbi-keys.md](fbi-keys.md): `--ext fbi`, units and weapons.
- [gamedata-keys.md](gamedata-keys.md): `--ext tdf --prefix gamedata --no-subdirs`: sides, classes, key bindings, render settings, AI names, timing, music.
- [gamedata-sub-keys.md](gamedata-sub-keys.md): sound classes, explosions, damage flames, effects (`--wild-top --by-dir`).
- [features-keys.md](features-keys.md): `--ext tdf --prefix features --wild-top`.
- [ota-keys.md](ota-keys.md): `--ext ota`, map and mission headers.

Regenerate with `TAK_DATA_DIR=... cargo run -p tak-cli -- keys <flags> --out docs/formats/<name>.md`; the command line is in each file's header.

## Errors

`TdfError { kind, offset, line }` with kinds `ExpectedSection`, `UnterminatedHeader`, `ExpectedOpenBrace`, `UnclosedSection`, `MissingEquals`, `UnterminatedComment`, `TooDeep`. Truncation, byte flips and random bytes never panic (unit and proptest tests); allocations are bounded by input length and depth by `MAX_DEPTH`.
