# WP-L: pathfinding and movement

**Owns:** `crates/tak-sim/src/pathfinding.rs` (and a `pathfinding/` submodule folder if you split it), the `orders`, `order_step` and `movement` functions in `crates/tak-sim/src/systems.rs`, `docs/pathfinding.md`.
**Lane API you must keep:** `pathfinding::go_to(world, unit, to)` (combat and construction call it), `pathfinding::apply_command` (Patrol), `pathfinding::tick`.

## Starting point (on main)
- Units drive straight at waypoints and halt at the first impassable cell, testing only their centre point (`docs/sim.md`).
- `MapDef` (tak-content) has a passability grid built by `PassRule` (water depth > 20, corner height spread > 30, blocking feature footprints). `MoveClassDef` (11 classes from `gamedata/moveinfo.tdf`, e.g. `GROUND2`) holds per-class limits; `UnitDef.movement_class` names a unit's class. FBI motion keys include `maxslope`, `maxwaterdepth`, `canfly`, `roadmultiplier`, `watermultiplier`, `turninplacerate`, `footprintx/z`.
- Terrain flags per cell include water, road and paved (`crates/tak-sim/src/map.rs`).

## Build
1. Per-movement-class passability grids built once per match from the map and `MoveClassDef` (slope, water depth, features), footprint-aware (a unit of footprint N×M fits where all N×M cells pass). Flyers (`canfly`) ignore terrain.
2. Deterministic A* (or a hierarchical variant) on the 16-px cell grid: fixed tie-breaking, integer costs (diagonals included, no corner cutting), a fixed per-tick expansion budget shared across units in slot order (multiplayer-safe: budgets are counts, never time).
3. Path following: waypoint smoothing (line-of-sight on the grid), replanning when the next cell becomes blocked or the unit is stuck, arrival behaviour unchanged from WP-I.
4. Speed multipliers from FBI `roadmultiplier`/`watermultiplier` (Believed semantics; document).
5. Simple unit-unit separation so units don't stack on one point (deterministic, cheap; full crowd steering is out of scope).
6. `Patrol` orders: legs re-queued at the back when reached.
7. `go_to` routes through the pathfinder.

## Done when
- On Adamantine Gate, a monarch ordered from one plateau to the other routes around cliffs through the canyon gaps and arrives; a render (`tak-cli render`) shows the route. Look at it before reporting.
- 100 units given random targets on a real map all arrive or report unreachable within a bounded number of ticks, under the per-tick budget.
- Cross-process determinism test with pathing on; all existing tests pass; no floats.
- `docs/pathfinding.md` documents grids, costs, budgets, and every Believed/Chosen value.
