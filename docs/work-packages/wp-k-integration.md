# WP-K: integration: real maps, real units, scripts in the sim

**Owns:** `crates/tak-sim/`, `crates/tak-headless/`, `docs/sim.md`, and a new `crates/tak-cli/src/render_cmd.rs`.
**Inputs on main:** everything from waves 1 and 2. Read `docs/content.md`, `docs/formats/cob-vm.md`, `crates/tak-sim/src/` (WP-I), `crates/tak-script/src/` (WP-G), `crates/tak-content/src/` (WP-H) first.

## Goal
A headless run of a real Kingdoms map: each player's monarch spawns at the map's start position, moves under real FBI stats across real terrain, with its original COB script running and animating it, deterministic across processes.

## What waves 1 and 2 established
- **Sim (WP-I):** `Sim::with_scripts(scenario, Box<dyn ScriptHost>)`; system order: spawn (tick 0) → commands → orders → movement → `on_tick` → advance + hash. Axes: origin top-left, +x east, +y south, 1 world unit per map pixel (Believed). `MapInfo` = width/height in world units (≤ 16384), `HeightGrid` (cols, rows, cell_size, row-major u8), ≤ 16 start positions. `UnitDef` stand-in: `maxvelocity`, `acceleration`, `brakerate` (16.16 per tick), `turnrate` (angle units per tick), footprint.
- **Content (WP-H):** `Ruleset` (base `units/` or `cb`), `UnitDef.motion` with `maxvelocity`, `acceleration`, `brakerate` (`Option<Fx>`), `turnrate` (`Option<i32>`), named as FBI keys, raw FBI units. `MapDef`: width, height, heights (u8 per 16-px cell), sea level, placed features, start positions `{player, x, z}` in attribute cells, passability grid (`PassRule`), kingdom/palette. `SideDef.commander` names the monarch unit per side. `WideFx` exists for large values like `buildtime`.
- **Script VM (WP-G):** `start("Create")` on spawn, `tick` once per sim tick in fixed unit order, fold `state_hash()` into the world hash, `call` for callbacks/queries (`locals[0]`). Implement the `Engine` trait over world state and the world RNG. Ports known so far: 29 = speed (MoveWatcher treats > 5 as moving), 17 = build left, 28/34 = water/road. Unknown ports return 0 and are counted. **COB piece order never matches 3DO pre-order: map pieces by case-insensitive name**, once per unit type. 11 walk/swim loops never sleep under fake inputs; real inputs may fix that; keep the instruction cap.

## Build
1. **Unit conversion**: FBI motion units to sim units. TA's documented convention is **Believed** to be: `maxvelocity` in pixels per tick... or per second. Find out from the data (compare a known unit's in-game speed if documented anywhere in the data, e.g. `docs/` or FBI comments, otherwise pick the TA convention) and document the choice and its confidence in `docs/sim.md`. Make the conversion one function so it is easy to correct.
2. **Real map scenario**: `Scenario::from_content(&MapDef, &Ruleset, players: &[PlayerSetup])` where `PlayerSetup` picks a side. World size = cells × 16; heights from the map; start positions converted from attribute cells to world units (document the cell size you use; check against the map dumps); the monarch is the side's `commander`.
3. **Terrain in movement**: units stop at impassable cells (use `MapDef`'s passability grid); ground height under a unit available to scripts and the hash. No pathfinding yet; straight lines that halt at blockers are fine.
4. **Script host**: a `ScriptHost` implementation in tak-sim that owns one `tak_script` instance per unit, maps pieces by name at spawn, implements `Engine` (ports from unit state, RNG from the world `Rng`), and hashes VM state into the world hash. Unknown-port counts are reported by the headless runner, not hashed.
5. **tak-headless**: `tak-headless run --data <install> --map "kmap/Adamantine Gate" --players ARA,ZON [--log cmds.json] --ticks N [--hashes out.txt]`, plus `gen-log` support for real maps.
6. **Visual**: `tak-cli render --map ... --players ... --log ... --tick N --out DIR`: a top-down PNG of the map (terrain mosaic, as `dump-map` does, downscaled) with unit markers, heading ticks, and movement trails up to tick N. Use `image` in tak-cli only.

## Done when
- Adamantine Gate with 2 players (two different sides) runs 10,000 ticks with scripts on, identical hashes across two processes (test spawns the binary twice; gate the test on `TAK_DATA_DIR`).
- Monarchs spawn at the map's start positions, walk to commanded points under real FBI stats, stop at impassable terrain, and their scripts see "moving" (port 29) while walking.
- A render PNG shows two monarchs' trails on the real terrain. Look at it with the Read tool before reporting.
- The synthetic-scenario tests from WP-I still pass unchanged (scripts off), and no floats in sim state.
- `docs/sim.md` documents unit conversions, coordinate systems, port mapping, and every Believed/Chosen value.
