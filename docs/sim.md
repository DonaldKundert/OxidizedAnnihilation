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
`ScriptHost::create_unit` per spawn) → commands → construction → economy →
orders (pop reached waypoints) → pathfinding → movement → combat (ending
with feature decay, WP-P) → victory (WP-U) → `ScriptHost::tick_unit` per live unit in slot
order → advance tick → hash (world, then script host). Commands are the only
input. The authoritative list is the module doc of `systems.rs`.

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
map, players, seed)` loads the base ruleset, the map, the monarchs plus
their whole build trees (WP-P; `docs/construction.md`), the map's
gameplay features and every unit's corpse (`Scenario::features`,
`docs/combat.md`), and the scripts. `load_match_monarchs` loads only the
monarchs (the pre-WP-P behaviour).

- Player `i` takes the map's `i`-th start position (sorted by OTA player
  number) and spawns its side's `commander` from `gamedata/sidedata.tdf`
  (`ARA` → `ARAKING`, `TAR` → `TARNECRO`, `VER` → `VERMAGE`, `ZON` →
  `ZONHUNT`, `CRE` → `CRESAGE`). Sides match `nameprefix` or `name`, any case.
- One `UnitDef` per distinct monarch first (ids `0..`), then the build
  trees. The map keeps heights, a per-cell
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

## Victory and defeat (WP-U)

`crates/tak-sim/src/victory.rs`; tests `crates/tak-sim/tests/victory*.rs`.
API for the client: `Sim::match_state() -> &MatchState` (`Running` or
`Over { winners }`), `World::is_defeated(PlayerId)`, and `World::victory()`
(decision tick, per-player defeat tick, `summary()`). Rules are
`Scenario::victory` (`VictorySetup`, optional in JSON). The system runs every
tick right after combat; its state (setup digest, result, defeat ticks,
mission counters) is hashed. `tak-headless run` prints `match: running` or
`match: over at tick T: winners [..]` on stderr.

Sources: the install's lobby GUIs (`guis/battlemenumulti.gui`,
`BattleMenuSingle.gui`: a **Monarch Expendable** checkbox, "Game continues
if monarch is killed", control `MonarchDeath`; team buttons Team 1-4); the
OTA condition census (`tak-cli keys --ext ota`, per-map dump); Nanolathe's MIT
research on the TA executable (`research/retail-executable-spec/08`,
"Victory and defeat triggers", "Commander death: the whole chain"
[R-SKIR-01 §3], [R-TRIG-01 §1-§6]). The install's `readme.htm` says nothing
about end conditions. Kingdoms descends from TA's engine, so TA rules are
**Believed** for Kingdoms unless the data says otherwise.

### Skirmish (default)

TA's skirmish and multiplayer sessions never read the OTA condition keys
(they are polled only in campaign sessions), and none of the 236 non-mission
maps author any. **Known** (no keys in `kmap/`), **Believed** (TA).

| Rule | Ours | Tag |
|---|---|---|
| Default defeat | the monarch dies (`DefeatRule::MonarchKilled`, "Monarch Expendable" unticked) | Believed (TA rule word 1 "Game Ends"; Kingdoms lobby box) |
| Alternative | `DefeatRule::AllUnitsKilled`: only losing every unit defeats ("Monarch Expendable") | Known option, Believed semantics (TA rule 0) |
| Either rule | a player that has had units and has none left (dying units still count until removed) is defeated | Believed (TA: `live unit count == 0`) |
| When | monarch: the tick its death starts; wipe-out: the tick the last unit is removed | Chosen (TA files the death when the dying unit is processed) |
| Defeated player's units | `DefeatedUnits::SelfDestruct` (default): each living unit takes 30000 damage (explosion kind) through the normal damage path, in slot order, and plays its death; `Remain`: they stay, keep defending, take no orders | Believed (TA's owner sweep, 30000 self-damage); Remain is Chosen |
| Units that survive the sweep | units outside combat or with more than 30000 health stay, inert | Believed (TA uses the same damage path) |
| Commands | dropped for defeated players (`apply_commands`) | Chosen |
| Teams | `VictorySetup::teams` (per player, `None` = alone); only victory reads them, combat still treats every other player as an enemy | Known (lobby teams), Chosen representation |
| Over | when the undefeated players all share one team; `winners` = those players; a simultaneous wipe-out is a draw (`winners: []`) | Believed (TA kind-2 sweep: no non-allied slot with units) |
| One team from the start | never ends by elimination (sandboxes, tests) | Chosen |
| After Over | nothing in victory changes again; the sim keeps ticking | Chosen |

**Difference from TA, documented:** TA polls defeat once per second and
latches the end ~150 ticks (five polls) after the first true poll, which is
when its end screen appears. We decide on the tick itself; a client that
wants TA's pause can delay its display by 150 ticks. TA's rule 2
("Deathmatch", monarch respawn) is not implemented; the Kingdoms lobby shows
only the checkbox.

### Missions (`VictorySetup::mission`)

`MissionRules::from_conditions(&OtaDef::conditions, &unit_defs)` parses the
`[GLOBALHEADER]` keys; `Scenario::from_content` leaves it `None` (a skirmish
on any map), so missions are opt-in for a campaign loader. Player 0 is
"local" (OTA `Player=1`), player 1 the "enemy" (TA's literal slots, no
alliance lookup). Victory needs **all** victory conditions (none authored:
`DestroyAllUnits`), defeat **any** defeat condition (none: `AllUnitsKilled`);
victory wins a tie. On victory the enemy is marked defeated, on defeat the
local player; no sweep. Conditions are evaluated every tick (TA: once per
second). Unit types resolve by case-insensitive name; a name-only key whose
value carries extra arguments stays verbatim and matches nothing (TA).
"Commander" is the owner's monarch definition.

| Key (files in corpus) | Meaning | Status |
|---|---|---|
| `destroyallunits` (22) | enemy has no units (dying count) | implemented, re-evaluated |
| `killenemycommander` (2) | enemy monarch removed | implemented |
| `killallmobileunits` (2) | enemy's last mobile unit removed; mobile = `maxvelocity > 0` | implemented (Believed proxy for TA's `BMcode`) |
| `killalloftype` (6) | enemy's last unit of the type removed | implemented |
| `killunittype` (2, `TYPE, n`) | n enemy units of the type removed | implemented |
| `moveunittoradius` (8, `TYPE\|ANYTYPE, x, z, r`) | a complete, living local unit within r of (x, z) | implemented |
| `unittypepassesx/z` (3/4, `TYPE\|ANYTYPE, c`) | a local unit's cell within 2 cells of column/row c | implemented |
| `victorytimerrunsout` (4, seconds) | tick >= seconds × 30 | implemented |
| `commanderkilled` (15) | local monarch removed | implemented |
| `allunitskilled` (29) | no complete, living local unit | implemented |
| `allunitskilledoftype` (21) | last unit of the type (local or enemy) removed | implemented |
| `unittypekilled` (2, `TYPE, n`) | n units of the type (any owner) removed | implemented |
| `deathtimerrunsout` (1, seconds) | tick >= seconds × 30 | implemented |
| `buildunittype`, `anyunitpassesx/z` | TA vocabulary, not in the Kingdoms corpus | implemented |
| `captureunittype` | needs capture | unsupported (listed in `MissionRules::unsupported`) |
| `useonlyunits` (6) | a build restriction, not a condition | unsupported |

Coordinates: **Believed whole 16-unit cells** in Kingdoms, not TA's pixels.
Evidence: `takmission42_dh` (200 × 350 cells) has the wagon at row 38 and
`UnitTypePassesZ=NPCWAGON, 321`; `takmission40_dh` (400 × 320) has
`UnitTypePassesX=ZONHURT, 384`; `takmission28_dh` has the unit at row 262 and
the line at row 11. Each line lies near the far map edge only if read as
cells. A cell is centred at `16c + 8` (as start positions, Chosen); the
radius is `16r`; `UnitTypePasses` tests the unit's centre cell (TA: its
footprint anchor cell), tolerance `|cell - c| < 3` (TA). TA's projected-view
de-projection of `MoveUnitToRadius` is not applied (Unknown for Kingdoms).
Removal-notified conditions are found by diffing the unit set each tick
(TA notifies on removal; same result, removals are seen at the end of the
tick). Mission scenarios still spawn one monarch per player; placing the
OTA `[UNITS]` is the campaign loader's job (not built).

### Results

- Synthetic (`tests/victory.rs`): each rule above, teams, draws, dropped
  commands, every implemented mission family, timers on the exact tick, and
  a cross-process hash comparison of a monarch duel (decided at tick 361)
  and a mission kill (tick 32).
- Real map (`tests/victory_real_map.rs`, `TAK_DATA_DIR`): `ARAKING` and
  `ZONHUNT` placed face to face on Adamantine Gate with their scripts,
  ordered to attack each other: the Zhon monarch dies, the match is over at
  tick 939 with `winners [0]`, player 1 defeated, zero VM errors.

## Unknown / open

- FBI speed units (Believed per tick, above); height scale and corner vs
  centre sampling; start position cell centre vs corner.
- Ports 27, 30, 33, 40, 41, 45, 46 and callback argument lists.
- Footprint-aware blocking, feature footprint anchoring (`content.md`),
  road/water speed multipliers, slopes slowing units.
- Flight (`canfly`), water units, per-movement-class passability grids.
