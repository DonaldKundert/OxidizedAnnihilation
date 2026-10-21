# Plan and work packages

The full design (mission, determinism rules, command layer, controller mapping) is in the Kingdoms Core Handoff. `CLAUDE.md` holds the rules every session follows. This page is the live plan: what is done, what can run in parallel now, and what waits on what.

## Done

| | Milestone | Where |
|---|---|---|
| M0 | Workspace, CI, `TAK_DATA_DIR` gating | `.github/workflows/ci.yml`, `tak_vfs::testing` |
| M1 | Install survey | `tak-cli survey`, `docs/formats/hapi.md` |
| M2 | HAPI v2 reader, VFS, mount order | `tak-vfs`, `tak-cli ls/cat/extract/shadowed/census`, `docs/formats/hapi.md` |

Shared groundwork for parallel work, already on `main`:

- `tak_formats::reader::Reader`: bounds-checked little-endian cursor. All parsers use it.
- Empty modules already declared in `tak-formats/src/lib.rs` (`tdf`, `gui`, `model3do`, `gaf`, `cob`, `tnt`), so packages never edit `lib.rs`.
- Common dependencies already in the workspace (`image`, `xxhash-rust`, `insta`, `proptest`, `serde`, `flate2`), so most packages never edit `Cargo.toml`.
- `tak_vfs::testing::HapiBuilder` for synthetic archives; `docs/formats/census.md` for what exists and where.
- `docs/getting-data.md`: how a session gets the game data.

## Waves

```
Wave 1 (now, all parallel)        Wave 2 (after its inputs)        Wave 3
WP-A  TDF + key census  ────────► WP-H  content definitions ──┐
WP-B  3DO models  ──────────────┐                             ├─► M10 mana and construction
WP-C  GAF/TAF + palettes        ├► WP-G  COB VM (M6) ──────────┤   combat, pathfinding, AI
WP-D  COB loader + disasm  ─────┘                             │   Bevy client
WP-E  maps: TNT + CRT (+OTA) ───────────────────────────────── ┤
WP-F  tak-math + tak-commands ──► WP-I  sim skeleton + replay ┤
                              └─► WP-J  input layer  ─────────┘
```

| Package | Milestone | Crates / files it owns | Needs data | Depends on |
|---|---|---|---|---|
| [WP-A](work-packages/wp-a-tdf.md) | M3 | `tak-formats/src/tdf.rs`, `gui.rs`, `docs/formats/tdf.md`, `tak-cli` subcommand `keys` | yes | — |
| [WP-B](work-packages/wp-b-3do.md) | M4a | `tak-formats/src/model3do.rs`, `docs/formats/3do.md`, `tak-cli dump-model` | yes | — |
| [WP-C](work-packages/wp-c-gaf.md) | M4b | `tak-formats/src/gaf.rs` (+ submodules), `docs/formats/gaf.md`, `palettes.md`, `tak-cli dump-sprite` | yes | — |
| [WP-D](work-packages/wp-d-cob.md) | M5 | `tak-formats/src/cob.rs`, `docs/formats/cob.md`, `tak-cli disasm` | yes | — |
| [WP-E](work-packages/wp-e-maps.md) | M7 | `tak-formats/src/tnt.rs` (+ submodules), `docs/formats/tnt.md`, `crt.md`, `tak-cli dump-map` | yes | OTA part reads TDF via WP-A |
| [WP-F](work-packages/wp-f-math-commands.md) | M8 prep | `tak-math`, `tak-commands` | no | — |
| [WP-G](work-packages/wp-g-script-vm.md) | M6 | `tak-script`, `tak-cli anim` | yes | B, D (merged) |
| [WP-H](work-packages/wp-h-content.md) | M3b | `tak-content`, `tak-cli content` | yes | A, E (merged) |
| [WP-I](work-packages/wp-i-sim-skeleton.md) | M8, M9 | `tak-sim`, `tak-headless` | no | F (merged) |
| [WP-J](work-packages/wp-j-input.md) | M11 | `tak-input` | no | F (merged) |

Wave 2 briefs are written when their inputs merge, so they start from what Wave 1 actually found. Wave 1 is fully merged (WP-A..F).

## Rules for parallel work

- **Stay in your lane.** Edit only the files your package owns. `tak-cli` is shared: add your subcommand in a new file `crates/tak-cli/src/<name>_cmd.rs`, and touch `main.rs` only to add one enum variant and one match arm.
- **Shared files get tiny, separate commits**: a new workspace dependency, a line in `docs/plan.md`. Rebase before opening the PR.
- **Branch** `wp/<letter>-<name>`, PR into `main`, CI green, then merge.
- **Report back in the PR description**: what is now Known, what is still Unknown, and anything another package needs to know (e.g. WP-C finding the TAF pixel formats unblocks the Bevy texture path).

## Starting a package

Paste this into a fresh session attached to this repo, with the letter filled in:

```
Read CLAUDE.md, docs/plan.md, and docs/work-packages/wp-<x>-*.md, then do that work package.
Get the game data per docs/getting-data.md (ask me to approve folder access on my Mac).
Work on branch wp/<x>-<name>, keep to the files the brief says you own, and open a PR to main
when the "Done when" list is met. Put Known / Believed / Unknown findings in the PR description.
```
