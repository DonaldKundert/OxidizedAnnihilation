# COB v6: unit and mission scripts (`.cob`)

Parser and disassembler: `crates/tak-formats/src/cob.rs` (`cob::parse`,
`cob::decode`, `cob::disassemble`). CLI: `tak-cli disasm <path>` and
`tak-cli disasm --stats <prefix>`.

Opcode values and TA stack effects come from Nanolathe's
`research/formats/cob.md` (MIT, <https://github.com/nanolathe-gg/nanolathe>),
which covers TA (version 4) and lists the two Kingdoms additions. Everything
below was checked against all 267 retail Kingdoms scripts; where Kingdoms
disagrees with TA, the file wins.

## Corpus

- **Known**: 267 `.cob` files: 204 under `scripts/` (unit scripts), 63 under
  `missions/` (mission scripts). All are version 6. All parse, all decode
  with **zero unknown opcodes**, zero truncated instructions and zero bad
  references (corpus test `cob::tests::corpus_loads_and_decodes`).
- **Known**: code is one contiguous instruction stream from word 0 to the
  end. A linear decode lands exactly on `code_words` in every file, every
  entry point and jump target is an instruction start, and entry points are
  in ascending order.
- **Known**: one file has a duplicate function name: `scripts/npcpeas2.cob`
  defines `walk_legs` twice. `Script::function` returns the first.

## Layout

All values are little-endian `u32`. Offsets are absolute byte offsets. Code
addresses (entry points, jump targets) are **word indexes into the code
array**, not byte offsets.

Retail layout, in file order (**Known**, all 267 files):

```
header (52 bytes = 13 words)
code              u32[code_words]          at 52
entry table       u32[script_count]        right after the code
script name offs  u32[script_count]
piece name offs   u32[piece_count]
sound name offs   u32[sound_count]
strings           NUL-terminated Latin-1, then 0-3 bytes of NUL padding
```

The parser does not rely on this order: it follows the header offsets and
bounds-checks each table and string. `Script::to_bytes` writes this order.

### Header

| Word | Byte | Field | Meaning | Status |
| ---: | ---: | --- | --- | --- |
| 0 | 0x00 | `version` | Always `6`. Parser rejects anything else. | **Known** |
| 1 | 0x04 | `script_count` | Number of named functions. | **Known** |
| 2 | 0x08 | `piece_count` | Number of piece names (0 in all mission scripts and a few units: 67 files). | **Known** |
| 3 | 0x0C | `code_words` | Code length in words. `code_at + 4*code_words == script_entries_at` in every file. | **Known** |
| 4 | 0x10 | `static_count` | Static-variable slots. Every static operand is `< static_count`; in 250 files the highest used index is exactly `static_count - 1`. No initial data in the file. | **Known** |
| 5 | 0x14 | `trailing_record_count` | TA's trailing 8-byte record count. Always 0. | **Known** (value); meaning **Believed** from TA |
| 6 | 0x18 | `script_entries_at` | Offset of `u32[script_count]` entry word indexes. | **Known** |
| 7 | 0x1C | `script_names_at` | Offset of `u32[script_count]` name string offsets. Always `script_entries_at + 4*script_count`. | **Known** |
| 8 | 0x20 | `piece_names_at` | Offset of `u32[piece_count]` name string offsets. Always `script_names_at + 4*script_count`. | **Known** |
| 9 | 0x24 | `code_at` | Code offset. Always 52. | **Known** |
| 10 | 0x28 | `trailing_records_at` | TA's trailing-record pointer. **Equals word 11 in every file.** TA docs describe it as "first script name offset"; in Kingdoms that only holds when there are no sounds (65 files). With zero records nothing is read through it. | **Known** (value); meaning **Believed** from TA |
| 11 | 0x2C | `sound_names_at` | Offset of `u32[sound_count]` string offsets. Always `piece_names_at + 4*piece_count`. | **Known** |
| 12 | 0x30 | `sound_count` | Number of sound strings. 0 in 65 files. | **Known** |

The worked sample `scripts/araarch.cob`: version 6, 23 functions, 33 pieces,
9659 code words, 10 statics, 2 sounds.

### The sound table has two uses

- **Known**: in unit scripts it holds sound names, indexed by the inline
  operand of `play-sound` (all 407 uses are in range).
- **Known**: in mission scripts it holds **mission command strings**,
  indexed by the first inline operand of `mission-command` (all 3885 uses are
  in range). The strings are command text with embedded arguments, e.g.
  `create <UNIT>`, `SetTrigger`, `RemoveTrigger`, `SetAttribute <name>`,
  `SetMission <order list>`. Their grammar is **Unknown** (WP for missions).

## Instruction encoding

Each instruction is one opcode word followed by 0, 1 or 2 inline operand
words. Stack effects are `( before -- after )`, top of stack on the right.
The table lives in one place: `enum Op` in `cob.rs`, with `Op::from_raw`,
`mnemonic()`, `operands()` (operand kinds) and `confidence()`.

Operand kinds: `piece` (index into the piece table), `axis` (0 x, 1 y, 2 z;
**Known**: every axis operand in the corpus is < 3), `const` (signed i32),
`local`, `static`, `target` (code word index), `function` (index into the
function table), `argc`, `sound`, `mcmd` (index into the sound table),
`count`, `ignored`.

"Uses" is the retail instruction count across all 267 files.

### Piece animation

| Opcode | Mnemonic | Inline | Stack | Uses | Status |
| --- | --- | --- | --- | ---: | --- |
| `0x10001000` | move | piece, axis | `( speed target -- )` | 17614 | **Known** |
| `0x10002000` | turn | piece, axis | `( speed target -- )` | 31765 | **Known** |
| `0x10003000` | spin | piece, axis | `( accel speed -- )` | 52 | **Known** |
| `0x10004000` | stop-spin | piece, axis | `( decel -- )` | 73 | **Known** |
| `0x10005000` | show | piece | `( -- )` | 667 | **Known** |
| `0x10006000` | hide | piece | `( -- )` | 1055 | **Known** |
| `0x10007000` | cache | piece | `( -- )` | 127 | **Known** |
| `0x10008000` | dont-cache | piece | `( -- )` | 515 | **Known** |
| `0x10009000` | legacy-effect | piece | `( a b -- )` (TA) | 0 | unused |
| `0x1000A000` | dont-shadow | piece | `( -- )` | 66 | **Known** |
| `0x1000B000` | move-now | piece, axis | `( target -- )` | 20838 | **Known** |
| `0x1000C000` | turn-now | piece, axis | `( target -- )` | 306910 | **Known** |
| `0x1000D000` | shade | piece | `( -- )` | 45 | **Known** |
| `0x1000E000` | dont-shade | piece | `( -- )` | 270 | **Known** |
| `0x1000F000` | emit-sfx | piece | `( sfxtype -- )` | 2382 | **Known** |
| `0x10011000` | wait-for-turn | piece, axis | `( -- )` | 263 | **Known** |
| `0x10012000` | wait-for-move | piece, axis | `( -- )` | 14 | **Known** |
| `0x10071000` | explode | piece | `( flags -- )` | 1788 | **Known** |

"Known" here means the opcode and its inline operand shape are confirmed by
every use (every piece operand is `< piece_count`, every axis `< 3`). Stack
effects are **Believed** from TA; they are consistent with the push patterns
seen (e.g. `push speed; push target; turn`).

### Values, variables, engine ports

| Opcode | Mnemonic | Inline | Stack | Uses | Status |
| --- | --- | --- | --- | ---: | --- |
| `0x10021001` | push-constant | const | `( -- v )` | 480258 | **Known** |
| `0x10021002` | push-local | local | `( -- v )` | 6735 | **Known** |
| `0x10021004` | push-static | static | `( -- v )` | 22770 | **Known** |
| `0x10022000` | alloc-local | — | `( -- )` add a local slot | 8586 | **Known** |
| `0x10023002` | pop-local | local | `( v -- )` | 3778 | **Known** |
| `0x10023004` | pop-static | static | `( v -- )` | 6478 | **Known** |
| `0x10024000` | discard | — | `( v -- )` | 2424 | **Known** |
| `0x10041000` | rand | — | `( low high -- r )` | 674 | **Known** |
| `0x10042000` | get-unit-value | — | `( id -- v )` | 924 | **Known** |
| `0x10043000` | get | — | `( id a b c d -- v )` | 254 | **Known** |
| `0x10082000` | set | — | `( id v -- )` | 1057 | **Known** |
| `0x10044000` | cargo-has | — | TA only | 0 | unused |
| `0x10045000` | carrier | — | TA only | 0 | unused |
| `0x10083000` | attach-unit | — | `( unit piece extra -- )` | 0 | unused |
| `0x10084000` | drop-unit | — | `( unit -- )` | 0 | unused |

### Arithmetic, comparison, logic

All `( a b -- r )` unless noted. Comparisons push 1 or 0.

| Opcode | Mnemonic | Uses | Status |
| --- | --- | ---: | --- |
| `0x10031000` | add | 996 | **Known** |
| `0x10032000` | sub | 480 | **Known** |
| `0x10033000` | mul | 1345 | **Known** |
| `0x10034000` | div | 396 | **Known** |
| `0x10035000` | bit-and | 260 | **Known** |
| `0x10036000` | bit-or | 1716 | **Known** |
| `0x10037000` | bit-xor | 0 | unused |
| `0x10038000` | bit-not `( a -- ~a )` | 0 | unused |
| `0x10039000` | shift-left `a << b` | 264 | **Believed** (see below) |
| `0x1003A000` | shift-right `a >> b` | 161 | **Believed** |
| `0x1003B000` | mod `a % b` | 66 | **Believed** |
| `0x10051000` | lt | 894 | **Known** |
| `0x10052000` | le | 91 | **Known** |
| `0x10053000` | gt | 511 | **Known** (value), name per Scriptor's table |
| `0x10054000` | ge | 66 | **Known** (value), name per Scriptor's table |
| `0x10055000` | eq | 2289 | **Known** |
| `0x10056000` | ne | 305 | **Known** |
| `0x10057000` | and (logical) | 5605 | **Known** |
| `0x10058000` | or (logical) | 170 | **Known** |
| `0x10059000` | xor | 0 | unused |
| `0x1005A000` | not `( a -- !a )` | 9494 | **Known** |

**Kingdoms uses three opcodes TA never does**: `0x10039000`, `0x1003A000`,
`0x1003B000`. Nanolathe records them only as Scriptor operator placeholders
(`??`, `???`, `????`). They appear in 33 building scripts (castles, keeps,
gates, towers, lodestones: e.g. `aracastl`, `verlight`, `zonglyph`), always
as zero-operand binary operators. Usage pins them down well:

- `x 0x1003B000 N` with `x = counter + 1` and `N` = 3, 4, 9... stored back
  into the counter: a cyclic counter, so **mod**.
- `3 0x10039000 (i * 2)`, then added into a word: packing 2-bit fields, so
  **shift-left**.
- `(w 0x1003A000 {2,4,6}) bit-and 3`: extracting 2-bit fields, so
  **shift-right**.

Status **Believed**: inferred from usage, not traced in the executable.
Unknown: signedness of shift-right and the sign of `mod` for negative
operands (no negative operands seen).

### Flow control and threads

| Opcode | Mnemonic | Inline | Stack | Uses | Status |
| --- | --- | --- | --- | ---: | --- |
| `0x10061000` | start-script | function, argc | `( args... -- )` spawn | 1027 | **Known** |
| `0x10062000` | call-script | function, argc | `( args... -- )` call, wait | 1320 | **Known** |
| `0x10063000` | pop-n | ignored, count | discard `count` values (TA) | 0 | unused |
| `0x10064000` | jump | target | `( -- )` | 3961 | **Known** |
| `0x10065000` | return | — | `( v -- )` | 4425 | **Known** |
| `0x10066000` | jump-if-false | target | `( cond -- )` jump when 0 | 22433 | **Known** |
| `0x10067000` | signal | — | `( mask -- )` | 878 | **Known** |
| `0x10068000` | set-signal-mask | — | `( mask -- )` | 677 | **Known** |
| `0x10013000` | sleep | — | `( ms -- )` | 18183 | **Known** |

Every function index is `< script_count` and every jump target is an
instruction start (**Known**). `argc` values seen: start-script 0/1/3/4,
call-script 0/1.

### Kingdoms-only

| Opcode | Mnemonic | Inline | Stack | Uses | Status |
| --- | --- | --- | --- | ---: | --- |
| `0x10072000` | play-sound | sound | `( priority -- result )` | 407 | **Known** shape |
| `0x10073000` | mission-command | mcmd, argc | `( args... -- result )` | 3885 | **Known** shape |

- **Known**: `play-sound` has **one** inline operand (sound index). All 407
  uses are `push-constant P; play-sound n; discard`, so it pops one value
  and pushes one result. Reading the popped value as priority is
  **Believed** (Scriptor's `play-sound("name", priority)`).
- **Known**: `mission-command` has **two** inline operands: index into the
  string table and an argument count (0 to 5). It only appears in
  `missions/*.cob`. Its result is either discarded or stored with
  `pop-static` (e.g. a trigger handle). Argument meanings per command are
  **Unknown**.

## Calling conventions (for the VM, WP-G)

- **Known**: every function body ends `alloc-local; return` (4384 of 4425
  `return`s follow `alloc-local`; the other 41 follow `push-constant`, an
  explicit `return N`). So the implicit return value comes from that
  `alloc-local`: **Believed** that the interpreter treats a freshly
  allocated local as the return cell or pushes 0. WP-G must make
  `alloc-local; return` return 0 without underflowing the stack.
- **Known** (sampled): functions open with a run of `alloc-local`, one per parameter
  and declared `var`. Parameters and locals are indistinguishable in the
  bytecode. **Believed** (TA): the caller's `argc` arguments seed locals
  0..argc-1, and the engine reads locals back after "query" callbacks
  (`QueryWeapon`, `SweetSpot`, `QueryBlood` write a piece index into a local).
- Statics: `static_count` slots per unit instance, **Believed** zeroed at
  creation; `Create` typically initializes them explicitly.
- Pieces are referenced by index into the piece table; resolve to 3DO piece
  names case-insensitively (`Hip`, `LegUL`, ...). Sounds are referenced by
  index into the sound table.
- Linear units: `[1.0]` compiles to 163840 (16.16 fixed point times 2.5).
  Angles: 65536 per turn (`<90>` about 16380). **Believed** from TA; values
  such as `-229376` and `8191` appear in Kingdoms walk cycles.

### Engine port IDs

The IDs used with `get-unit-value`, `get` and `set` differ from TA's beyond
20 (**Known** usage, meanings **Unknown** unless noted):

- `get-unit-value`: 4 (85), 7 (40), 17 (282, TA `BUILD_PERCENT_LEFT`, used to
  wait for construction in `Create`), 18 (30), 27, 28 (122), 29 (142), 30,
  32 (106), 33, 34 (94), 46.
- `get` (approximate, from the 5th-previous instruction): 5, 14, 30, 31, 35,
  36, 40, 41, 45.
- `set`: 1, 2, 5, 18, 19 (TA `YARD_OPEN`/`BUGGER_OFF`, always paired), 21
  (254), 22 (148), 23 (212), 26 (129), 31.

## Function names

Kingdoms renames most TA callbacks. Top names in the 204 unit scripts
(**Known** counts):

`Create` 204, `Killed` 199, `SweetSpot` 181 (+ `Sweetspot` 8), `Dying` 166,
`AimWeapon` 142, `FireWeapon` 142, `QueryWeapon` 141, `TargetCleared` 138,
`QueryBlood` 132, `TurnDirection` 130, `SetMaxReloadTime` 111, `HitByWeapon`
31, `StartBuilding`/`StopBuilding` 27, `setSFXoccupy` 23, `Activate`/
`Deactivate` 18, `MoveRate` 17, `QueryBuildInfo` 11, `MotionControl` 7.
Common helpers: `walk`, `walk_road`, `walk_water`, `walk_legs`, `restore_x`,
`restore_legs`, `attack1`..`attack4`, `death`, `MoveWatcher`,
`MeleeControl`, `StatusControl`, `RestoreAfterDelay`, `InitState`/
`RequestState`, `Go`/`Stop`, `OpenYard`/`CloseYard`, `FlightControl`.

**Known**: no Kingdoms script defines TA's `StartMoving`, `StopMoving` or
`AimPrimary`. Movement is polled by helper threads (`MoveWatcher` started
from `Create`), and weapons use a single `AimWeapon`/`FireWeapon`/
`QueryWeapon` family. Prologue `alloc-local` counts (params + vars, upper
bound on parameters): `AimWeapon` 3, `QueryWeapon` 2, `FireWeapon` 1,
`TargetCleared` 1, `SweetSpot` 1, `QueryBlood` 1, `TurnDirection` 1,
`SetMaxReloadTime` 1, `MoveRate` 1, `setSFXoccupy` 1, `QueryBuildInfo` 1.
Which of those are engine-supplied arguments is **Unknown**.

Mission scripts: `Start` 55, `TriggerHit` 42, `UnitDestroyed` 39,
`UnitCreated` 7, plus per-mission helpers.

## Unknowns

- Header word 5 / word 10 semantics beyond "TA trailing table, empty".
- Exact semantics of shift-right signedness and mod sign.
- Engine port IDs above 20 and the Kingdoms callback argument lists.
- Mission command string grammar and per-command arguments.
- Whether the interpreter tolerates jumps into the middle of an instruction
  (never happens in retail; the disassembler reports it).
