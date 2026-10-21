# Construction (`tak_sim::construction`, WP-O)

Build orders, frames (units under construction), mobile builders, assisting,
and factory queues. Code: `crates/tak-sim/src/construction.rs` and
`construction/` (`build_def`, `placement`, `progress`, `content`). Tests:
`crates/tak-sim/tests/construction_*.rs`.

Tags: **Known** (seen in the Kingdoms data), **TA-Known** (established for
original TA in Nanolathe's MIT research docs, `research/retail-executable-spec/05-*.md`,
not confirmed for Kingdoms), **Believed**, **Chosen** (our pick where the
original is unknown), **Unknown**.

## Data (`BuildDef`, on `UnitDef.build`)

| Field | FBI key | Sim unit | Tag |
|---|---|---|---|
| `buildcost` | `buildcost` | whole mana | Believed (one resource) |
| `buildtime` | `buildtime` | raw 16.16 (`WideFx`, up to 100000) | Known key |
| `workertime` | `workertime` | work per second | Known: 10 for almost every builder and factory (30 `TARPRIE2`, 50 `ZONHURT`) |
| `builddistance` | `builddistance` | world units | Believed identity |
| `builder` | `builder` flag | | |
| `mobile` | `bmcode` flag | mobile builders place at a site; others are factories | TA-Known (the factory handler tests bmcode); Known in Kingdoms: `ARAKING`, `ARASWORD`, `ZONTRAIN` have it, `ARAKEEP`, `ARAWALL` do not |
| `canbuild` | `canbuild*/<unit>/` menu of the active ruleset | ids, menu order | |
| `yardmap` | `yardmap`, expanded per cell | | TA-Known expansion rule, below |

Real matches: `load_match` (and so `tak-headless` and `tak-cli render`)
adds every unit reachable from the monarchs' menus (breadth first, menu
order, via `Scenario::with_build_trees(&rs)`) and resolves menus to ids
(WP-P). The monarchs keep ids `0..`. `load_match_monarchs` is the old
monarchs-only loader; `construction::content::load_match_with_builds` is
kept as an alias of `load_match`.

## Build time and progress

- A frame needs `buildtime × 30` work units (raw 16.16); each builder adds
  `workertime` (raw 16.16) per tick. A lone builder finishes in
  `ceil(30 × buildtime / workertime)` ticks = `buildtime / workertime`
  seconds. **Believed**, TA's rule (`remaining -= worker / buildtime` per
  tick with `worker = workertime / 30`).
- **Difference from TA**: TA truncates `workertime / 30` to an integer
  before use. With Kingdoms' `workertime = 10` that is zero, so nothing
  would ever build; Kingdoms must differ there, and we keep the exact
  quotient.
- Examples: `ARAWALL` (65) by the monarch 195 ticks (6.5 s); `ARAKEEP`
  (600) 1800 ticks (60 s); `ARASWORD` (99) from a keep 297 ticks (9.9 s).
  Test-checked to the tick (`construction_synthetic`, `construction_real_map`).
- Work starts on the tick the frame is placed. The frame is complete on the
  tick its work reaches the total; builders call `StopBuilding` that same
  tick.
- Several builders on one frame add their work, in slot order; the
  lowest-slot builder that finishes it wins, later ones do nothing that tick.
  TA-Known.
- Health of frames (WP-P, TA-Known): 0 at placement, each work step adds
  `trunc(max × old remaining) − trunc(max × new remaining)`; damage is
  health loss, never progress loss. `work` calls `combat::build_progress`
  after every step; rules in `docs/combat.md` ("Frame health"). A builder
  whose frame is destroyed drops the order (Believed).

## Mana

- Each tick each working builder asks `economy::drain(owner, want)` where
  `want` is what the frame owes after this step minus what it has paid:
  `floor(cost × done / total)` cumulatively, so a whole build costs exactly
  `buildcost` with no rounding drift. No drain for finished, paused, or
  waiting work.
- A partial grant advances the frame only as far as the mana paid covers
  (progress ∝ granted / want). **Difference from TA**: TA commits a step
  only when the whole demand is admitted. Over time both build at the
  income rate; ours is smoother.
- A request is capped at `Fx::MAX` per tick (never reached by retail data).

## Port 17 and incomplete units

- `UnitBuild::percent_left()` = `ceil(100 × remaining / total)`: 100 on a
  fresh frame, more than 0 until complete, then 0. Scripts' `Create` waits
  for it to read 0 (Known from the scripts); the keep's script then starts
  its flag and smoke threads.
- Frames are inert: construction clears their orders and speed every tick.
  **Combat and movement should also check `build.is_complete()`** (no
  aiming, firing, being counted as finished); the economy already excludes
  `percent_left() > 0`.

## Mobile builders

1. `Build { at: Some(p) }`: the builder must be complete, own the unit, and
   have `def` in its menu. `p` snaps to the footprint grid (anchor = nearest
   cell to `p − footprint × 8`; Believed, TA "cell index biased by half its
   extent"). Queue mode is honoured.
2. Out of range: `pathfinding::go_to(site)` and the rest of the queue is
   re-appended behind the walk. In range: the walk is dropped and the builder
   stops. If its walk ends (or terrain halts it) still out of range, the
   order is dropped (Chosen; pathfinding makes this rare).
3. Range: centre distance (whole units) − `trunc(8 × hypot(builder
   footprint))` − the same for the target ≤ `builddistance`, inclusive.
   TA-Known. We compute the pad exactly; TA's float routine can be one unit
   lower when the hypotenuse is whole (Nanolathe's `(20, 99)`).
4. In range: if a frame of the same def and owner stands on the site, assist
   it (also how a second builder joins); otherwise check the site and place
   a frame through `lifecycle::spawn_unit`, facing south (Chosen; TA samples a
   random angle around `buildangle`). Then `StartBuilding` (no arguments:
   Known, no Kingdoms `StartBuilding` reads a local) and work. The builder
   turns toward the site at its turn rate.
5. Done: `StopBuilding`, drop the order. Interrupted (Stop, a replacing
   order): `StopBuilding`; the frame stays where it is and can be resumed by
   building the same thing on the same spot.
6. Abandoned frames: TA decays them (`−11 × buildtime / buildcost` per
   visit) until they vanish. Not implemented; they stay (Unknown for
   Kingdoms).

### Site check

- Bounds: anchor ≥ 1 and far edge < map size in cells (TA-Known: buildings
  never touch the outermost row/column).
- Yard map expansion (TA-Known): characters outside `. c C f G o O w Y y`
  are skipped (spaces between rows), the last usable character repeats for
  unfilled cells, extras are ignored. Kingdoms adds `S` (Known: lodestones).
- **Sacred sites (WP-P)**: a definition with `yardmap = S` (all ten
  lodestones and Divine Lodestones; also `NPCTEMP2`) must be centred on a
  free sacred site: one whose footprint plus the economy's `site_slack`
  (one cell) contains the snapped centre and that no other sacred-site
  unit, built or under construction, stands on
  (`economy::free_site_at`, the same test income uses, so whatever
  placement allows earns the site's multiplier). Otherwise
  `SiteError::NotSacredSite`. **Believed**: WP-N found `S` only on
  lodestones, and guides say lodestones are built on the sacred stones; the
  exact original tolerance is Unknown.
- Every cell whose yard character is not `.` must not be `STEEP` or `DEEP`,
  and not `FEATURE` unless it is `S` (the lodestone sits on its sacred-site
  feature), and must hold no blocking wreck (`SiteError::Feature`, WP-P).
- No structure or frame may occupy those cells (both yard cells non-`.`).
  Mobile units do not block (Chosen; TA's ground word also holds mobile
  units). Slope sampling per definition (`maxslope`) is not applied; the
  terrain grid's `STEEP` flag is used instead.

### Rejections

A build order dropped at the site check is logged in
`ConstructionState.rejections` (the latest 64, hashed): tick, builder,
player, definition, snapped site and `SiteError`, whose `reason()` gives the
text ("must be built on a free sacred site", "a wreck is in the way", ...).
A UI can also call `placement::check_site` before ordering, to colour the
footprint.

## Assisting through Guard (WP-P)

TA: a builder ordered to guard a unit under construction, a factory, or
another builder helps build (TA-Known behaviour; Nanolathe's single shared
work step serves "every build, assist, factory-product and deconstruction
step"). Here:

- A complete mobile builder whose combat order is `Guard(g)` and that has
  no build job assists `construction::assist_target(g)`: `g` itself while
  under construction; else the frame `g` is building (its build job or its
  own assist, one level); else `g`'s factory product. Same owner only.
- Out of range (the build range rule above) it walks over
  (`pathfinding::go_to`); a walk that ends out of range is not repeated
  unless the target moved. In range it stops, calls `StartBuilding` once,
  turns toward the frame and adds its own `workertime` every tick through
  the same `work` step, paid by its owner through `economy::drain` (so a
  shortage slows it like any builder).
- When the frame completes (or dies) it calls `StopBuilding`; the guard
  stays, so it moves on to the next product of a guarded factory, or the
  next frame of a guarded builder.
- Any other player order suspends it; while assisting, combat's guard
  follow and chase are off (Chosen).
- State: `UnitBuild.assist` (`Assist { target, walk_to, building }`),
  hashed.
- Results (`tests/open_items.rs`): a mason guarding the king's wall frame
  from tick 3 finishes it on tick ~100 instead of 196; guarding the king
  while it builds does the same; guarding a factory roughly halves a
  product's build time.
- Not done: repair of damaged complete units by guarding builders (TA does
  it), reclaim, and assisting allies.

## Factories

`Build { at: None }` appends to the factory's order queue as
`Order::Build { at: None }` (Believed: factory buttons always queue). Each
order is one unit (no count coalescing). Per tick, following TA's five-state
handler (TA-Known):

1. With orders and not active: `Activate`.
2. Wait until the script sets `INBUILDSTANCE` (port 5); the door animation
   is the script's. Units without a script skip the wait.
3. Exit spot: `QueryBuildInfo` (`locals[0]` pre-set to −1) names a piece;
   its static 3DO position, turned by the factory's heading (model −Z
   forward, +X right, 1 model unit = 1 world unit; Chosen/Believed), is the
   spawn point. For `ARAKEEP` that is `emitbuild`, about 66 units south of
   the centre, in the yard rows. No answer: the centre of the yard's `c`
   cells, else the factory centre (Chosen).
4. If any unit other than the factory overlaps the product's footprint
   there, retry in exactly 15 ticks (TA-Known), e.g. while the previous
   product is still on the pad.
5. Place the frame (copying the factory's fire state), `StartBuilding`, and
   work with the factory's `workertime`.
6. Complete: `StopBuilding`, drop the order, and `Deactivate` if the queue is
   empty. The product inherits the factory's queued `Move`s as a rally
   (TA-Known); without one it walks straight out of the yard to 24 units
   past the footprint edge (Chosen: TA's product release is Unknown even in
   Nanolathe's research).
7. Cancel: if the queue loses its production orders while a product is on
   the pad (Stop, a replacing order), the frame is removed and what it had
   paid is queued as a `Refund` in `ConstructionState.refunds` for the
   economy to credit (TA-Known: refund `(1 − remaining) × cost`, product
   vanishes). If the product is destroyed, the factory drops all its orders
   (TA-Known).

## Determinism

All integer (`i64`/`i128` intermediates), slot-order iteration, no RNG use.
`UnitBuild`, `ConstructionState` and `BuildDef` are hashed.
`construction_determinism` runs a busy log in-process, after a JSON round
trip, and in two child processes, and compares hashes.

## Unknown / open

- Kingdoms build-time rule (TA's, minus the truncation, matches the data's
  scale); partial-mana behaviour.
- Product release from the factory yard; frame decay; refunds for
  computer players (TA scales by difficulty).
- The original's sacred-site tolerance; per-definition slope limits;
  structures blocking movement (the `pathfinding::add_obstacle` hook now
  exists; structures do not use it yet).
- Repair, reclaim, resurrection; assisting allies.

## Real-map run (WP-P)

`tests/open_items_real_map.rs`, Adamantine Gate, ARA vs TAR with full build
trees and the original COB scripts. Commands are decided from the state as
the run goes and recorded; a child process replays the log and must
produce the same hashes.

- The Aramon monarch builds a keep next to its start (complete on tick
  2867; 1800 ticks of work slowed by the opening mana shortage: 1955 cost
  against 1000 stock + 10/s), with health rising from 0 to exactly 17162.
- The Taros monarch walks about 1100 units and builds three walls on the
  approach, about 900 units from the keep.
- The keep trains swordsmen and a catapult; each marches on the walls and
  they attack. The Taros monarch's fireballs kill three swordsmen (bodies,
  non-blocking, rot after 30 s) and the catapult, whose 4x4 wreck
  (`arapult_dead`, blocking) is an obstacle in the swordsmen's movement
  layer. Run ends at tick 8159; zero script errors.
- A real Lodestone is refused next to the monarch (`NotSacredSite`) and
  built on the nearest sacred stone, which it then claims.
- Render: `TAK_OPEN_ITEMS_LOG_OUT=<file> cargo test -p tak-sim --test
  open_items_real_map`, then `tak-cli render --map "kmap/Adamantine Gate"
  --players ARA,TAR --world-seed 7 --log <file> --tick 8159 --out DIR`
  (now draws every unit, corpses and wrecks).
