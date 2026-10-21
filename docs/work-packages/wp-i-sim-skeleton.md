# WP-I: simulation skeleton, headless runner, replay (M8, M9)

**Owns:** `crates/tak-sim/`, `crates/tak-headless/`.
**Inputs on main:** `tak_math` and `tak_commands` (WP-F). Read their docs and the Determinism section of `CLAUDE.md` first. No game data is required for this package.

## What wave 1 established (WP-F notes)
- Hash with `Fx::raw().to_le_bytes()`, never `std::hash::Hash`.
- Use explicit wrapping/checked ops for sim integers.
- `Tick(u32)` lives in tak-commands. Commands apply on their tick in `(tick, player, seq)` order via `CommandLog::for_tick`.
- `WorldPos = Vec2` on the ground plane; axis orientation is the sim's choice (document it: the 3DO models face -Z with Y up, per `docs/formats/3do.md`).
- The sim must validate unit ownership and stale `UnitId`s itself. `UnitSet` is sorted and deduplicated.

## Build
1. **World**: players, a unit arena (`Vec` slots with generational `UnitId`), per-unit position, heading (`Angle`), speed, owner, def id, and an order queue. Ordered containers only.
2. **Map stand-in**: until the TNT and OTA packages land, a `MapInfo` with width, height, a flat or synthetic height grid, and start positions. Keep it a small struct the real loader will fill later.
3. **Unit definitions stand-in**: a minimal `UnitDef` (max speed, acceleration, brake rate, turn rate, footprint) loaded from a plain struct. WP-H will replace the source with real `.fbi` data; keep the field names close to FBI keys (`maxvelocity`, `acceleration`, `brakerate`, `turnrate`).
4. **Tick**: fixed 30 Hz, systems in a fixed documented order (apply commands → orders → movement → ...). Spawn a monarch-equivalent unit per player at start positions on tick 0.
5. **Movement**: straight-line movement toward the order target with acceleration, braking and turn rate; arrival tolerance; `Move` (with queue modes), `Stop`. No pathfinding yet.
6. **Hashing**: xxHash64 over a canonical little-endian serialization of the world after every tick.
7. **Script hook**: a trait the sim calls for unit behaviour scripts (`on_create`, `on_tick`), with a no-op default. WP-G is building the VM in parallel; do not depend on `tak-script` yet.
8. **tak-headless**: `tak-headless run --scenario <json> [--log <commands.json>] --ticks N [--hashes out.txt]` prints or writes per-tick hashes; `tak-headless verify` re-runs a recorded log and compares against a hash file.

## Done when
- Same seed and command log give identical per-tick hashes across 10,000 ticks, in one process and in two separate processes (test spawns the binary twice).
- A recorded log replays to the same final hash; changing one command changes the hash from that tick on.
- Movement tests: a unit ordered to a point arrives within tolerance, respects turn rate, and stops on `Stop`.
- No floats in sim state (reuse WP-F's no-floats test approach).
