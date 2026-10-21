# WP-O: construction

**Owns:** `crates/tak-sim/src/construction.rs` (and a `construction/` submodule folder), `docs/construction.md`.
**Lane API:** `construction::apply_command` (Build), `construction::tick`, `UnitBuild::percent_left()` (scripts read it as port 17). Spawn through `lifecycle::spawn_unit`, pay through `economy::drain` (a stub that grants everything until WP-N lands), walk builders with `pathfinding::go_to`, notify scripts with `lifecycle::call_script` / `start_script`.

## Starting point (on main)
- tak-content: `UnitDef` economy fields (`buildcost`, `buildtime` as `WideFx`, `workertime`, `builddistance`), `builder`, `canbuild`, `yardmap`, `footprintx/z`; `BuildMenu` from `canbuild/` (and `canbuildcb/` for the cb ruleset). Read `docs/content.md`.
- Scripts: `StartBuilding`, `StopBuilding`, `QueryBuildInfo` (answers a piece in `locals[0]`), `Activate`/`Deactivate`, and port 17 counting down from 100 while a unit is under construction. See `docs/formats/cob-vm.md`.
- `SimCommand::Build { units, def, at: Option<WorldPos>, queue }`: `at: Some` for mobile builders placing structures, `None` for factories adding to their queue. `Order::Build { def, at }` exists.

## Build
1. Validate build orders against the builder's build menu for the active ruleset.
2. Mobile builders: walk to within `builddistance` of the site, check the footprint is free and passable, spawn the structure at 0% (through `lifecycle::spawn_unit`, in an under-construction state: not functional, no weapons, scripts see port 17), call the builder's `StartBuilding`, add progress per tick = `workertime / buildtime` (document units; Believed TA convention), scaled by the mana fraction granted by `economy::drain(buildcost / build ticks)`. `StopBuilding` on finish or interruption; partially built structures persist.
3. Several builders on one site add their work.
4. Factories: a queue; units appear at the factory (via `QueryBuildInfo` piece if available, else the factory centre plus offset), then leave the yard.
5. Construction state hashed; cancellation refunds per the original rule if known (document).

## Done when
- A monarch builds a structure from a Build command on a real map: walks there, the structure appears, port 17 counts 100 → 0 in its script, completion at the documented time (unit test with synthetic defs checks the exact tick).
- A factory builds a queue of two units that spawn and walk out.
- Cross-process determinism test; all existing tests pass; no floats.
- `docs/construction.md` documents formulas, timings, and confidence tags.
