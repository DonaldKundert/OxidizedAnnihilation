# Oxidized Annihilation

A from-scratch, Rust-only engine core for **Total Annihilation: Kingdoms** (Cavedog, 1999), in the spirit of OpenMW: you bring your own copy of the game, and this project reads its data and runs it on a new engine.

The core is headless and deterministic. It mounts the original archives, parses the formats, runs unit scripts, and simulates the game at a fixed 30 ticks per second, driven only by commands. Mouse and controller both produce the same commands, so a gamepad is a first-class way to play. A Bevy client comes later as a thin shell around the core.

**No game data is included or ever committed.** You need your own copy of TA: Kingdoms (the GOG release with The Iron Plague is what this project is developed against).

## Status

| Milestone | State |
|---|---|
| M0 Workspace and CI | done |
| M1 Install survey (`tak-cli survey`) | done, see [`docs/formats/hapi.md`](docs/formats/hapi.md) |
| M2 Archive reader and VFS (`tak-cli ls/cat/extract`) | done |
| M3–M7 formats, math, commands | in parallel, see [`docs/plan.md`](docs/plan.md) |

The full plan is in [`docs/plan.md`](docs/plan.md).

## Building

```sh
cargo build --workspace
cargo test --workspace                      # skips data-dependent tests
TAK_DATA_DIR=/path/to/kingdoms cargo test   # also runs corpus tests
cargo run -p tak-cli -- survey "$TAK_DATA_DIR" --out /tmp/tak-survey
```

## Crates

| Crate | Job |
|---|---|
| `tak-vfs` | Mount archives and folders, precedence, case-insensitive paths |
| `tak-formats` | Pure byte parsers, one module per format |
| `tak-math` | 16.16 fixed point, u16 angles, integer trig, seeded RNG |
| `tak-script` | COB v6 loader, disassembler, VM |
| `tak-content` | Parsed files into game definitions |
| `tak-commands` | Command enums and command logs |
| `tak-sim` | World state, fixed tick, state hashing |
| `tak-input` | Device events into commands |
| `tak-headless` | Run a sim from a map and a command log |
| `tak-cli` | Inspection tools |

## License

Licensed under the [MIT license](LICENSE).

Total Annihilation: Kingdoms is a trademark of its respective owners. This project is not affiliated with or endorsed by them.
