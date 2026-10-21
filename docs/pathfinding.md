# Pathfinding and movement (`tak-sim`, WP-L)

Code: `crates/tak-sim/src/pathfinding.rs` (state, planner, tunables),
`pathfinding/grid.rs` (layers), `pathfinding/search.rs` (A*), and the
`orders` / `movement` systems in `crates/tak-sim/src/systems.rs`. Tests:
`crates/tak-sim/tests/path_synthetic.rs`, `path_real_map.rs` (needs
`TAK_DATA_DIR`).

Tags: **Known** (observed in the Kingdoms data), **TA-derived (Nanolathe)**
(documented for original Total Annihilation by Nanolathe's retail-executable
research, not confirmed for Kingdoms), **Believed** (inferred, not
contradicted), **Chosen** (ours; deterministic, easy to change). Every number
below is a constant at the top of `pathfinding.rs` (or `grid.rs` for the
stored-flag limits).

## Tick

Order inside the sim tick (see `systems.rs`): commands → `orders` (pop reached
move orders, rotate reached patrol legs) → `pathfinding::tick` (build layers on
the first tick, track goals, detect stalls, plan under the budget) → `movement`
(steer every unit, then separate overlapping units). Units are always visited
in slot order.

## Layers (per movement class)

Built once, on the first tick, for every distinct *class key* among the unit
definitions: footprint plus the eight terrain limits. Units sharing a key
share a layer. Flyers (`canfly`) and maps without terrain get no layer and move
freely.

| Item | Rule | Tag |
|---|---|---|
| Grid | the 16-px TNT attribute cell; one tier per cell, then one per footprint anchor | TA-derived (Nanolathe) |
| Cell heights | the cell's own height plus its east, south and south-east neighbours (edge-guarded): `lo`, `hi` | TA-derived (Nanolathe); same corners as `PassGrid` |
| Feature | a blocking feature (`FEATURE` flag) blocks | Known flag, TA-derived rule |
| Water | `lo < sea level` (or the stored `WATER` flag) | TA-derived |
| Deep | blocked if `lo < sea − MaxWaterDepth` | TA-derived |
| Shallow | blocked if `hi > sea − MinWaterDepth` (boats) | TA-derived |
| Slope | `hi − lo` against `MaxSlope`/`BadSlope` on land, `MaxWaterSlope`/`BadWaterSlope` in water: `> max` blocked, `<= bad` clear, else steep | TA-derived |
| Stored flags | a stored `STEEP`/`DEEP` flag also blocks classes at least as strict as `GROUND2` (slope 30, depth 20), so hand-made synthetic maps work | Chosen |
| Footprint | the anchor is the footprint's top-left cell; the anchor tier is the minimum over the `fx × fz` cells (outside the map = blocked); a clear result next to any non-clear cell of the one-cell ring is demoted to steep | TA-derived |
| Unit ↔ anchor | a unit stands at its footprint centre: anchor = `floor((pos − (f−1)·8) / 16)`, waypoint = `(2·anchor + f)·8` | TA-derived |
| Components | 4-neighbour flood over passable anchors (equals the search's connectivity, since diagonals need both side anchors) | Chosen |

Movement class data (`Motion` on the sim `UnitDef`, from
`content::unit_def_from_ruleset`): a resolved `movementclass` from
`gamedata/moveinfo.tdf` supplies footprint and limits; otherwise the FBI's own
`maxslope`, `maxwaterdepth`, `minwaterdepth`, `badslope`, `maxwaterslope`,
`badwaterslope` and footprint. Omitted keys take TA's template (slopes 255,
depths ±10000), `BadSlope` defaults to `MaxSlope >> 1`, then `MaxWaterSlope`
caps `MaxSlope` and each max caps its bad value (TA-derived). Synthetic units
without motion data default to `GROUND2` (Known: `CLASS0`, footprint 2×2,
slope 30, water depth 20, water slope 30). All five monarchs but Zhon are
`GROUND2` or `HOVER2` (Known); `ZONHUNT` flies.

## Dynamic obstacles (WP-P)

The hook for things that block movement after the map loads (today:
blocking corpses, `docs/combat.md`; later structures):

```rust
pathfinding::add_obstacle(world, CellRect { x, z, w, h })    // one more blocker per cell
pathfinding::remove_obstacle(world, rect)                    // one fewer (never below 0)
pathfinding::is_obstacle(world, cx, cz) -> bool
```

- `PathState` keeps a blocker count per attribute cell (allocated on the
  first add; hashed sparsely: index and count of every non-zero cell). A
  cell with a non-zero count is classified exactly like a map `FEATURE`
  cell (blocked for every class). Counts make overlapping obstacles
  independent.
- Each call re-stamps every built layer at once (`ClassGrid::restamp`):
  the cells in the rectangle, every anchor whose footprint or one-cell ring
  touches it (so footprint folding and ring demotion stay exact), then the
  connected components of the whole layer (a flood fill; cheap next to a
  search). Layers built later (`ClassGrid::build_with`) include the
  current counts. Movement therefore sees the change in the same tick.
- Units already under a new obstacle can walk out (the step rule lets a
  unit on a blocked anchor move to any point whose own cell is open);
  followers whose path crosses it are refused the step, stall and replan
  (Chosen; TA re-validates paths in the same way through stalls).
- Maps without a terrain grid (synthetic) ignore obstacles for movement
  (nothing blocks there); placement still sees them.
- No effect on the WP-K legacy centre-cell mover (`systems::move_unit`).

## Search

| Item | Value | Tag |
|---|---|---|
| Algorithm | A* over anchors, 8 neighbours in TA's order N, NW, W, SW, S, SE, E, NE | TA-derived order |
| Step cost | 16 cardinal, 22 diagonal | TA-derived |
| Turn penalty | by direction change in eighths: 0, 40, 60, 80, 100, 80, 60, 40 (start direction from the unit's heading) | TA-derived |
| Short run | +75 when turning again within 5 straight steps | TA-derived |
| Steep tier | +30 per step onto a steep anchor | TA-derived |
| Heuristic | octile `18·max + 7·min − 4` (goal radius 4), times **9/2** | TA-derived form; weight Chosen (TA's middle tier, see comparison) |
| Diagonals | no corner cutting: both side anchors must be passable | Chosen (brief); TA tests only the destination |
| Ties | heap ordered by `(f, h, push order)`; equal cost never re-parents | Chosen order; no-reparent TA-derived |
| Unreachable goal | goal anchor blocked or in another component: aim at the anchor of the start's component nearest the goal (square rings, then octile distance, then row, then column), at the point of it closest to the goal | Chosen |
| Blocked start | walk first to the nearest passable anchor | Chosen |
| Straight line first | if the line from the unit to the goal is walkable, no A* | Chosen |

## Budget

| Item | Value | Tag |
|---|---|---|
| Steps per tick | 1333, shared by all units, never exceeded | TA-derived (`stepAllowance`) |
| Admission | 100 steps per request; a request is admitted only with at least 100 left | TA-derived charge; the "at least" guard Chosen |
| Pop | 1 step per heap pop | TA-derived |
| Line of sight | 1 step per anchor crossed (saturating at the remaining budget) | Chosen (TA charges its pre-search ray per cell) |
| Concurrency | one search at a time, globally; a suspended search resumes first next tick | TA-derived |
| Order of admission | units in slot order from a rotating cursor (the slot after the last admitted) | Chosen (TA rotates per player) |
| Not charged | layer building (once), ring searches for nearest anchors, route smoothing | Chosen; all bounded by the map size |

Budgets are counts, never time, so the planner is multiplayer-safe. A
search's working set is a pure function of its seed and pop count, so the
hash covers `(unit, start, goal, pops, target)` instead of the arrays.

## Following

| Item | Value | Tag |
|---|---|---|
| Route | A* cells smoothed greedily by line of sight (exact integer traversal of the anchor grid; a corner crossing needs both side anchors) into waypoints at anchor centres; the last waypoint is the order's point (or the nearest reachable point) | Chosen (brief) |
| Waypoint consumed | within 5 whole world units (squared integer test, inclusive), one per tick | TA-derived |
| Lookahead | more than 80 units from the waypoint, aim at a point dragged back along the incoming leg | TA-derived |
| Turning | at most `turnrate` per tick toward the aim | Known key, TA-derived rule |
| Accelerate / brake | accelerate iff the aim is more than twice the turn-finishing distance `|err|·speed/turnrate` away **and** the following waypoint (the end, on the last leg) is beyond braking distance `speed²/(2·brakerate)`; otherwise brake | TA-derived |
| Landing | within 45° of the bearing and this tick's travel reaches the waypoint: land exactly on it | Chosen (kept from WP-I) |
| Flow-through | a non-last order's end is not braked for | Chosen (kept from WP-I) |
| Arrival | Move order done within 2 world units (WP-I), unit stops dead | Chosen (kept from WP-I; brief) |
| Terrain step | refused if the destination anchor is blocked (a unit on a blocked anchor may move onto any point whose own cell is passable, so it can walk out of an obstruction's margin); diagonal anchor changes use the line-of-sight rule | Chosen |
| Refused step | not taken; speed capped at half `maxvelocity`; path marked blocked | TA-derived (TA also clamps inside the old footprint) |
| Stall | no 1-unit progress toward the current waypoint for `60 + 32768/turnrate` ticks (max 600) | Chosen |
| Stall near the end | within 48 units of the final point: the order counts as arrived (lets a group share a destination) | Chosen |
| Replan | on a stall, at most once per 60 ticks per unit | TA-derived interval |
| Give up | after 3 replans without progress: order dropped, outcome `GaveUp` | Chosen (TA retries forever) |
| Outcome | `UnitPath.outcome`: `Arrived`, `Unreachable`, `GaveUp` (hashed) | Chosen |

## Speed

| Item | Rule | Tag |
|---|---|---|
| Cap | `maxvelocity × factor` | |
| Road | FBI `roadmultiplier` on `PAVED` cells (Aramon/Taros monarchs 1.21) | Believed (Kingdoms key; meaning inferred) |
| Water | FBI `watermultiplier` in `WATER` cells (Aramon monarch 0.81, Veruna 1.0); without the key, half speed unless `canhover`/`floater` | key Believed; the half-speed default TA-derived |
| Flyers | never slowed | Chosen |
| Slope | no slowdown (TA's pitch cap applies only to non-`upright` units; every monarch is `upright`) | TA-derived |

## Separation

After movement: ground units whose circles (radius half the larger footprint
side, 16 for a 2×2 monarch) overlap are pushed apart by the overlap, split
evenly between two mobile units (an immobile one is not moved), at most 1
world unit per unit per tick, never onto terrain the unit could not step to.
Pairs come from 64-unit buckets; pushes are computed from one snapshot and
applied in slot order; coincident units part along an angle hashed from their
slots. **Chosen** (TA has no pushing at all; see the comparison).

## Patrol

`Patrol` with Replace: the queue becomes `[Patrol(to), Patrol(position)]`.
Append: `Patrol(to)` joins the queue, plus a return leg to where the queue
ended if it had no patrol legs yet. A reached leg (2 units) moves to the back
of the queue, at most one per unit per tick; an unreachable or abandoned leg
rotates too. **Chosen**; TA rotates the record to the tail on arrival
(TA-derived), with a zero goal radius.

## `go_to`

`pathfinding::go_to(world, unit, to)` replaces the unit's orders with
`Move(to)` (clamped); the planner picks it up next tick. Combat and
construction use it.

## Results

- **Adamantine Gate, plateau to plateau** (`path_real_map.rs`): the Aramon
  monarch from its start marker (1816, 328) to (1000, 2400) on the other
  plateau routes down the land bridge between the two canyons and arrives in
  about 1,440 ticks, walking about 2,420 units for a 2,226-unit straight line;
  one request, one search of about 630 pops. It never stands in a blocked
  cell. Render: `tak-cli render --map "kmap/Adamantine Gate" --players ARA,TAR
  --log plateau.json --tick 6000` with one Move for unit 0.
- **100 units, random targets** (same file; 100 Aramon monarchs on random
  open cells, all ordered at once): 80 arrive, 20 stop at the nearest
  reachable point (targets inside cliffs or the canyon floor's enclosed
  pockets), none give up. All 159 requests (156 A* searches, 717k pops; a
  few canyon-floor searches take 20–34k pops each) are planned by tick 359
  and every unit is done by tick 3,486. No tick exceeds 1,333 steps; the
  budget is the limit while the queue drains. About 5,900 ticks/s in a
  release build without scripts while all 100 move (310 in a debug build).
- **Determinism**: `path_synthetic.rs` re-runs a 40-unit maze run with moves
  and patrols in a child process and compares all 2,999 hashes; the
  `tak-headless` real-map two-process test now runs with pathing too.

## Comparison with original TA (via Nanolathe)

Source: Nanolathe `research/retail-executable-spec/04-units-orders-scripts-and-movement.md`
(cited by section; its tags are mostly **Established**, i.e. traced in the
TA executable) and `01-…determinism.md` for the 30 Hz / 16.16 basis, which we
already share. These are findings for original TA. Kingdoms shares the engine
lineage and the data formats (`moveinfo.tdf` has the same keys), but none of
this is confirmed for Kingdoms.

| Behaviour | Original TA (per Nanolathe) | Ours | Verdict |
|---|---|---|---|
| Search algorithm | A* with a pre-search greedy ray plus two-sided wall follow that sets an acceptance threshold; directed 5-way fan after the first expansion (§7.1, R-PATH-01 §4–5) | plain A*, all 8 neighbours, straight-line check first | differs (results similar; TA's ray and fan are speed tricks) |
| Grid / cell size | 16-px attribute cells, one 2-bit tier layer per movement class (§6.1, §7.1) | same, one tier byte per anchor | match |
| Cost model | 16/22 steps, turn penalties 0–100, +75 short run, +30 steep (R-PATH-01 §3) | same | match |
| Heuristic | octile 18/7 minus goal radius, weight 1.5 × per-player tier 6/3/1 by recent service; steady state probably ×1 (§7.3, R-PATH-01 §6; tier in force is "Supported", not traced) | fixed weight 4.5 (the ×3 tier) | close; differs in adaptivity |
| Per-tick budget | 1333 steps, 100 per admission, 1 per pop, ray steps charged; one search at a time; round-robin per player (§7.3) | same numbers and single search; slot order with a rotating cursor | match (admission order differs) |
| Unreachable goal | ray finds the local heuristic minimum; route ends short or is empty; the order re-arms with a `30 + rand(30)` wait and retries indefinitely (R-PATH-01 §4, §8.3) | precomputed components; walk to the nearest reachable point, then drop the order (`Unreachable`) | differs (deliberate: avoids TA's endless retries and full-map floods) |
| Path smoothing | none; route keeps only direction-change cells (max 20 published) and is followed with an 80-unit pure-pursuit carrot (§7.3, R-MOV-01 §3) | line-of-sight smoothing (brief) plus the same 80-unit carrot | differs: our units take any-angle lines where TA units walk 8-direction legs |
| Replanning | blocked or route-exhausted followers re-request at most every 60 ticks, no retry ceiling (R-MOV-01 §7) | stall-triggered, at most every 60 ticks, give up after 3 | close (ceiling Chosen) |
| Footprint handling | class footprint baked into the layer (window minimum + ring demotion), anchor = top-left, unit at footprint centre (R-SLOPE-01 §3) | same | match |
| Unit-unit collision | hard occupancy: a step into a cell held by another unit is refused (half speed, position clamped), sequential claims in sweep order, no pushing, sliding or yielding; recently moved units do not block the search (§8.2, R-MOV-02A) | soft separation push; units never block the search or each other's steps | differs (Chosen per the brief; TA's version causes the familiar traffic jams) |
| Slope / water limits | per-cell classifier: feature, occupant, deep (`hmin < sea − MaxWaterDepth`), shallow (`hmax > sea − MinWaterDepth`), slope tier; strict `>` blocks; template 255 / ±10000 (§6.1, R-DOC04-A/B) | same, minus the occupant gate | match |
| Speed modifiers | half speed below sea level unless `canhover`/`floater`; pitch-table cap for non-`upright` units; no road or water keys (R-MOV-01 §4) | Kingdoms `roadmultiplier`/`watermultiplier` when present, else TA's half speed in water; no pitch cap | close (Kingdoms keys take precedence) |
| Turning / acceleration | turn clamped by `TurnRate`; accelerate iff aim > 2 × turn distance and the point after beyond braking distance, else brake; speed never reverses (R-MOV-01 §2, §4) | same tests; plus WP-I's exact landing on waypoints | match (landing is extra) |
| Arrival | order completes when the unit's committed anchor equals the goal anchor (goal radius 4 → squared threshold 0); waypoints pruned at 5 units (§8.3) | 2 world units from the order point (WP-I); waypoints pruned at 5 units | differs (kept per the brief; TA stops "somewhere in the goal cell") |

Where TA's rule was cheap and affects feel we now match it: class limits and
the per-cell classifier, footprint folding, costs, budget numbers, the 60-tick
repath interval, 5-unit waypoint pruning, the 80-unit carrot, the
accelerate/brake tests, half-speed on blocked steps and TA's water slowdown as
the default when a unit has no `watermultiplier`.

Possible `classic` ruleset options (not implemented): hard unit occupancy
instead of separation; 8-direction routes without line-of-sight smoothing;
tile-equality arrival; TA's adaptive heuristic tier; endless retries for
unreachable goals. TA's single global search with no starvation guard is kept
because it is what TA does, but at high unit counts it is a known flaw: with
100 units ordered at once, plans trickle out over a few hundred ticks. Raising
`STEPS_PER_TICK` is the knob if that feels worse than the original.

## Unknown / open

- Whether Kingdoms changed any of TA's pathing rules (fan, ray, tiers, the
  budget numbers). Footage or a Kingdoms executable trace would settle it.
- `roadmultiplier` / `watermultiplier` semantics (Believed: plain factors on
  the speed cap); whether Kingdoms keeps TA's half-speed rule for units
  without the key.
- Building occupancy in the layers: the dynamic-obstacle hook exists
  (below) and wrecks use it; structures do not yet (TA's occupancy also
  holds structures and units).
- Air movement beyond "ignores terrain"; boats and hovers are classified by
  their classes but untested on real water maps.
