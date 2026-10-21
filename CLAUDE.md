# Oxidized Annihilation: working rules

Rust-only, headless, deterministic engine core for Total Annihilation: Kingdoms. Read `docs/plan.md` for milestones and work packages, and `docs/formats/` for what we know about each file format.

## Hard rules

- **Never commit game data**: no archives, extracted files, sprites, models, or byte dumps of retail files, including in test fixtures, snapshots, or docs. Hand-build synthetic fixtures instead. `.gitignore` blocks common extensions, but you are responsible.
- **No GPL code.** Do not open, copy, or translate Open Annihilation (GPLv3) or other GPL projects. Treat Robot War Engine the same way.
- **Nanolathe** (https://github.com/nanolathe-gg/nanolathe, MIT, Go, original TA only) is reference material. Clone it outside this repo. Prefer its `research/` docs over its code. If you adapt code closely, keep its MIT notice in that file.
- **Tag format claims** in `docs/formats/*.md` as **Known** (observed in the real files), **Believed** (TA-derived or holds with exceptions), or **Unknown**. When a Kingdoms file disagrees with TA docs, the file wins.
- **Parsers never panic or over-allocate on bad input.** Return typed errors (`thiserror`). Cap allocations by remaining input length.

## Determinism (applies to tak-sim, tak-math, tak-script, tak-commands)

- 30 ticks per second. Game speed changes ticks per wall-second, never tick length.
- Positions are signed 16.16 fixed point (`tak_math::Fx`). Angles are `u16`, 65536 per turn, table-driven trig.
- No `f32`/`f64` in sim state. One seeded RNG owned by the world. No `HashMap`/`HashSet` iteration in the sim; use `Vec` arenas with generational IDs, or `BTreeMap`.
- Hash world state every tick. Commands are the only sim input.

## Fidelity and multiplayer (Tyler's decisions)

- **Familiar to experienced TA:K players.** Aim to match TA:K's behaviour wherever possible: its rules and numbers from the data, the install's docs, or careful observation, and how things feel in play (speeds, timings, targeting, build times, pathing quirks that players rely on). It does not have to be 1:1: where an exact match is complicated, costly, or reproduces a known flaw, get close, document the difference, and move on.
- Rebalancing is allowed, but only behind an explicit, named option (e.g. a ruleset flag), never as the default.
- When the original rule is unknown, pick the most TA-faithful reading, tag it Believed or Chosen in the docs, and keep it in one easily changed place.
- **Multiplayer is not a goal yet, but must stay possible.** Every system is deterministic: no floats in sim state, ordered iteration, one world RNG, integer budgets instead of time budgets, and a cross-process determinism test for each new system.

## Crate boundaries

`tak-sim` must not depend on `tak-input`, `tak-cli`, or any renderer. `tak-script` exposes a trait the sim calls; it does not depend on `tak-sim`. `tak-formats` does not touch the filesystem: it takes `&[u8]`. Nothing in the core workspace depends on `bevy`, a GPU, or a window stack.

The Bevy client lives in `client/`, a separate Cargo workspace excluded from the core one (`exclude = ["client"]`). It depends on the core crates by path; the core never depends on it. Core CI does not build it. The client reads sim state and submits commands; it never changes sim rules. Anything gameplay-relevant belongs in `tak-sim`.

## Game data

Tests needing real data call `tak_vfs::testing::data_dir_or_skip(...)` and return early when `TAK_DATA_DIR` is unset, so CI stays green. Corpus tests assert counts and zero errors; they never snapshot file contents.

The data lives on Tyler's Mac, not in the repo. To get it into a cloud session, see "Getting the game data" in `docs/plan.md`.

## Before every commit

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Work on a branch named after the work package (`wp/m3-tdf`) and open a PR against `main`. Keep each PR inside its package's crates and docs, so parallel work does not collide. Shared files (`Cargo.toml` workspace deps, `docs/plan.md`) get small, separate edits.
