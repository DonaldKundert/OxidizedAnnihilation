# WP-D: COB v6 loader and disassembler (M5)

**Owns:** `crates/tak-formats/src/cob.rs`, `docs/formats/cob.md`, `crates/tak-cli/src/disasm_cmd.rs`.

## Why it matters
Unit scripts drive animation and behavior. WP-G builds the VM on this loader, so the opcode table and the header layout must be exact.

## What we know
- **Known**: 267 `.cob` (`scripts/` 204, `missions/` 63), all version `6`.
- **Known**: 13-word (52-byte) header, matching Nanolathe's Kingdoms notes. Sample (`scripts/araarch.cob`): `6, 23, 33, 9659, 10, 0, 38688, 38780, 38872, 52, 39004, 39004, 2`. Read as: version, script count, piece count, code length (in u32 words), static var count, unknown 0, offset to script code offsets, offset to script name offsets, offset to piece name offsets, offset to code (52), offset to name strings, offset to sound name array, sound count. **Believed** until checked on every file.
- **Known** (Nanolathe `cob.md`): Kingdoms adds opcodes `0x10072000` (play sound) and `0x10073000` (mission command). Everything else is assumed shared with TA until a script proves otherwise. Code words in the sample start `0x10022004`, a TA-style opcode.
- `missions/*.cob` are mission scripts, likely using the mission-command opcode.

## Tasks
1. `cob::parse(&[u8]) -> Result<Script, CobError>`: header, script names and entry points, piece names, sound names, static count, code words. Validate every offset and count.
2. Opcode table with operand counts, one place in the code, documented in `docs/formats/cob.md`.
3. Disassembler producing stable text: script labels, mnemonics, operands, jump targets resolved to labels. Unknown opcodes are reported with their offset and value, never skipped.
4. Corpus test: every script loads; every opcode is recognized; print an opcode frequency table.
5. Snapshot tests with `insta` on **synthetic** hand-assembled scripts only (never snapshot retail disassembly).

## Done when
- 267/267 load, zero unknown opcodes (or each unknown is documented with where it appears).
- `tak-cli disasm scripts/araarch.cob` is readable enough to follow a walk animation.
- Header field meanings are Known or explicitly Unknown in the doc.
