# Simulation (`tak-sim`, `tak-headless`)

Crates: `crates/tak-sim` (world, systems, hashing, COB host, content
conversion) and `crates/tak-headless` (runner). Work packages WP-I (skeleton,
M8/M9) and WP-K (integration). Tags: **Known** (observed in the retail data),
**Believed** (TA-derived or inferred, not contradicted), **Chosen** (our
definition where the original is unknown; deterministic, can change),
**Unknown**.

```sh
tak-headless gen-log --map "kmap/Adamantine Gate" --players ARA,ZON --ticks 10000 \
    --kind walk --every 600 --out walk.json
tak-headless run --data <install> --map "kmap/Adamantine Gate" --players ARA,ZON \
    --log walk.json --ticks 10000 --hashes out.txt     # timing + script stats on stderr
tak-headless verify --map ... --players ... --log walk.json --hashes out.txt
tak-cli render --map "kmap/Adamantine Gate" --players ARA,ZON --log walk.json \
    --tick 6000 --out DIR                              # DIR/render.png
```

`--data` defaults to `$TAK_DATA_DIR`. `--world-seed` sets the world RNG
(default 1), `--no-scripts` runs without COB. Synthetic runs keep the WP-I
form (`--scenario file.json`). `gen-log --kind random` is the WP-I fuzz log;
`--kind walk` orders every monarch to a random in-map point every `--every`
ticks.

## Tick

30 ticks per second. System order per tick: spawn monarchs (tick 0 only, then
`ScriptHost::create_unit` per spawn) → commands → orders (pop reached
waypoints) → movement → `ScriptHost::tick_unit` per live unit in slot order →
advance tick → hash (world, then script host). Commands are the only input.

## Coordinates

| Quantity | Sim unit | Source | Tag |
|---|---|---|---|
| Ground position | `(x, y)` world units, 16.16 `Fx`; origin top-left, `+x` east, `+y` south | | Believed (1 world unit = 1 map pixel, TA) |
| TNT/OTA `z` | sim `y` | | Known (row-major north→south) |
| Attribute cell `(cx, cz)` | covers `[16cx, 16cx+16) × [16cz, 16cz+16)` | TNT: one height and one feature entry per 16 px | Known (32-px terrain pieces cover 2×2 cells) |
| Map size | `cells × 16` (Adamantine Gate: 192 → 3072) | `MapDef.width/height` | Known |
| Start position | cell centre `16c + 8` | OTA `StartPosN` `xpos`/`zpos` (whole cells) | Chosen (centre vs corner Unknown) |
| Height | raw TNT byte as world units, `Fx::from_int(h)`; containing cell's sample, no interpolation | `MapDef.heights` | Believed scale; corner vs centre Unknown |
| Heading | `u16`, 65536 per turn, 0 = east, `0x4000` = south | | Chosen (WP-I) |

Checked against the map dumps: `tak-cli render` draws start rings, trails and
the passability tint on the terrain mosaic at the same scale (8 px per 32 world
units); the tint lands on cliff edges, the start rings on the start plateaus.

## Unit conversion (FBI → sim)

One function each in `crates/tak-sim/src/content.rs`:

| FBI key | Sim | Function | Tag |
|---|---|---|---|
| `maxvelocity` | world units per tick (16.16) | `fbi_linear` = identity | **Believed** |
| `acceleration`, `brakerate` | world units per tick per tick | `fbi_linear` = identity | **Believed** |
| `turnrate` | angle units per tick, clamped `0..=32767` | `fbi_turnrate` = identity | **Believed** |

Evidence: the TA convention (and what the Spring engine assumes when it reads
TA FBIs) is pixels per 30 Hz tick. Per second would be implausible: the Aramon
monarch (`ARAKING`, `maxvelocity = 1.7`) would take about 30 minutes to cross
the 3072-px Adamantine Gate; per tick it takes about a minute (51 px/s), and
the swordsman (`1.1`) walks about one 32-px tile per second. The install's
`Docs/*.txt` list "Maximum Velocity" values (`0.55` to `3.50`) without units,
so nothing in the data settles it. Confidence: moderate; correct one function
if footage timing disagrees. `acceleration = 10` (most monarchs) is far above
`maxvelocity`, so they reach top speed in one tick; `ZONHUNT` (0.5) takes 5.

Missing keys: `maxvelocity` 0; `acceleration`/`brakerate` default to
`maxvelocity` (instant) and are at least one raw step; `turnrate` falls back to
`turninplacerate`, then 0 (cannot turn). Footprint defaults to 1×1. All Chosen.
`turninplacerate`, `roadmultiplier`, `watermultiplier`, `maxslope` are not used
yet.

## Real-map scenario

`Scenario::from_content(&MapDef, &Ruleset, &[PlayerSetup])`; `load_match(vfs,
map, players, seed)` loads the base ruleset, the map and the scripts.

- Player `i` takes the map's `i`-th start position (sorted by OTA player
  number) and spawns its side's `commander` from `gamedata/sidedata.tdf`
  (`ARA` → `ARAKING`, `TAR` → `TARNECRO`, `VER` → `VERMAGE`, `ZON` →
  `ZONHUNT`, `CRE` → `CRESAGE`). Sides match `nameprefix` or `name`, any case.
- One `UnitDef` per distinct monarch. The map keeps heights, a per-cell
  terrain grid (`MapInfo.terrain`, the `PassGrid` flags verbatim) and the sea
  level. Synthetic scenarios have no terrain grid (open ground everywhere),
  so their JSON is unchanged.

## Terrain in movement

**Superseded by WP-L**: units now path around terrain on per-movement-class,
footprint-aware layers, with TA-derived steering and road/water speed
factors. See `docs/pathfinding.md`. The notes below describe the WP-K
baseline (still used by `systems::move_unit` for a lone unit outside the sim).

- Passability is `tak_content::PassGrid` with the default rule (`GROUND2`:
  max slope 30, max water depth 20, blocking features, top-left anchor). All
  monarchs are `GROUND2`. Flags: `WATER`, `DEEP`, `STEEP`, `FEATURE`, `PAVED`,
  `MARKER`; `DEEP | STEEP | FEATURE` block.
- A step is refused when its destination cell blocks and differs from the
  current cell; the unit stops dead (speed 0) and drops its active order;
  queued orders still run. **Chosen**. No pathfinding, no sliding along walls.
- Leaving a blocked cell is allowed, so a unit spawned on a blocker can walk
  off (**Chosen**; 6 of 1086 start markers sit on blocking features).
- Only the unit's centre point is tested, not its footprint (**Chosen**).
- Shallow water (`WATER` without `DEEP`) is walkable at full speed;
  `watermultiplier`/`roadmultiplier` are not applied yet.
- `Unit.ground` (height under the unit) and `Unit.terrain` (flags) are
  refreshed after every move, hashed, and passed to scripts in `ScriptCtx`.
- Flying units (`ZONHUNT` has `canfly = 1`) walk as ground units for now.

## Script host (`CobScripts`)

- One `tak_script::Instance` per unit, `Create` started on spawn, one `tick`
  per sim tick in slot order. Programs are shared per unit type; COB pieces
  map to 3DO pieces **by case-insensitive name** once per type
  (`ScriptType::piece_map`; 0 unmapped for the monarchs and the corpus walk).
- `setSFXoccupy(4)` (land) or `(2)` (water) is called after the tick whenever
  the terrain class under the unit changes, first on tick 0 (**Believed**, TA
  values; Zhon scripts treat 5 as airborne).
- `StartMoving`/`StopMoving` are not called: the `ARAKING`, `TARNECRO`,
  `VERMAGE` and `ZONHUNT` scripts have no such functions; walking is driven
  by `MoveWatcher` polling port 29.
- Random numbers come from the world RNG, so script `rand` advances the hashed
  RNG state. The hash covers each instance's `state_hash()` (statics, piece
  animation, threads), the port registers scripts wrote and the last
  `setSFXoccupy` value. Diagnostics (`ScriptStats`) are not hashed; the
  headless runner prints them.
- Combat callbacks (`AimWeapon`, `FireWeapon`, `TargetCleared`, `Killed`,
  ...) are called by the combat lane (`docs/combat.md`); build callbacks
  (`StartBuilding`, `StopBuilding`, `QueryBuildInfo`, `Activate`,
  `Deactivate`) by the construction lane (`docs/construction.md`).

### Script ports

`get-unit-value` and `get` read these; `get`'s first operand is a unit id or a
packed point. Unit ids other than 0 (self) are not supported and read as
self. Packed points are `x << 16 | (y & 0xffff)` in whole world units
(**Believed**, TA `PACKXZ`).

| Port | Name | Answer | Tag |
|---:|---|---|---|
| 1, 2, 5, 6, 18, 19, 20 | ACTIVATION, STANDINGMOVEORDERS, INBUILDSTANCE, BUSY, YARD_OPEN, BUGGER_OFF, ARMORED | last value the script `set` (0 initially) | Believed (TA names) |
| 3 | STANDINGFIREORDERS | fire state: 0 hold, 1 return, 2 at will | Believed |
| 4 | HEALTH | `UnitCombat::health_percent()` (`docs/combat.md`) | Believed (percent) |
| 7, 9 | PIECE_XZ, UNIT_XZ | packed unit position (piece offsets ignored) | Chosen |
| 8, 10 | PIECE_Y, UNIT_Y | ground height, 16.16 | Chosen |
| 11 | UNIT_HEIGHT | 0 | Chosen |
| 12 | XZ_ATAN | angle to the packed point relative to the heading, signed | Believed (TA), sign Chosen |
| 13 | XZ_HYPOT | distance to the packed point, 16.16 | Believed |
| 14, 15 | ATAN, HYPOT | `atan2(a, b)` in angle units; `hypot(a, b)` on raw 16.16 | Believed, operand order Chosen |
| 16 | GROUND_HEIGHT | height at the packed point, 16.16 | Believed |
| 17 | BUILD_PERCENT_LEFT | `UnitBuild::percent_left()`, rounds up, 0 when complete (`docs/construction.md`) | Believed |
| 21, 22, 23 | aim aborted / ready / launch now | last value set | Believed (Spring adopted these numbers from Kingdoms) |
| 28 | IN_WATER | 1 if the cell has `WATER` | Believed |
| 29 | CURRENT_SPEED | speed, raw 16.16 world units per tick; `MoveWatcher` treats `> 5` as moving, i.e. any motion | Believed |
| 32 | VETERAN_LEVEL | 0 | Believed |
| 34 | ON_ROAD | 1 if the cell has `PAVED` | Believed |
| other | | 0, counted as unknown | Unknown |

Writes to any port are stored per unit (hashed); writes to ports other than
1–23 are counted as unknown.

## Results (Adamantine Gate, 10,000 ticks, walk log every 600 ticks)

- **Known**: two `tak-headless` processes give identical hash files with
  scripts on (`crates/tak-headless/tests/real_map.rs`, ARA vs ZON); dropping
  the scripts changes the hash from tick 0.
- Monarchs spawn on the start markers (`(113, 20)` → `(1816, 328)`,
  `(60, 153)` → `(968, 2456)`), reach exactly their FBI top speed, arrive
  within the 2-unit tolerance, halt facing blockers and never enter a blocked
  cell (`crates/tak-sim/tests/real_map.rs`).
- Scripts see "moving": ARA vs ZON, 3,334 port-29 reads, 662 answered moving;
  1,601 pose-changing unit-ticks, 1,023 of them while walking. Ports read:
  17, 28, 29, 34. **No unknown ports, no runaway loops, no VM errors.**
- Corpus walk (every one of the 152 mobile base units as a lone monarch,
  walking 160 units then idling, 600 ticks): all walk, zero runaway loops,
  zero VM errors. Unknown ports read: 30 ×173, 33 ×560, 46 ×380.
- The 11 `walk`/`swim` runaway loops from the WP-G sweep needed `FireWeapon`
  / `TargetCleared` mid-attack. Under real combat (WP-M) 10 of 11 are gone;
  only `MONPIRAN:swim` still hits the cap, when a water unit is run on land.
  The instruction cap stays.
- Performance: about 58,800 ticks/s with 2 scripted monarchs (release build,
  stepping only; loading the ruleset and map takes well under a second).

## Unknown / open

- FBI speed units (Believed per tick, above); height scale and corner vs
  centre sampling; start position cell centre vs corner.
- Ports 27, 30, 33, 40, 41, 45, 46 and callback argument lists.
- Footprint-aware blocking, feature footprint anchoring (`content.md`),
  road/water speed multipliers, slopes slowing units.
- Flight (`canfly`), water units, per-movement-class passability grids.
