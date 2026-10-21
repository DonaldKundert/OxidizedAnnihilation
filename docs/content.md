# Content definitions (`tak-content`)

Crate: `crates/tak-content`. CLI: `tak-cli content [--ruleset base|cb] [--unit NAME] [--feature NAME] [--map "kmap/Adamantine Gate"] [--coverage]`.
Work package WP-H (M3b). Tags: **Known** (observed in every retail file), **Believed** (TA-derived, inferred, or holds with exceptions), **Unknown**.

```rust
let vfs = tak_vfs::open_kingdoms(&dir)?;
let rs = tak_content::Ruleset::load(&vfs, tak_content::RulesetKind::Base);
let map = tak_content::MapDef::load(&vfs, &rs, "kmap/Adamantine Gate")?;
```

## Rules every def follows

- **No floats.** Integers, or `tak_math::Fx` (16.16 in `i32`) for keys that hold decimals anywhere in the corpus. Decimal text goes through `tdf::Decimal::to_fixed(16)`, never through `f32`.
- `buildtime` reaches 100000, past `Fx`'s ±32767, so it is a `WideFx` (16.16 in `i64`).
- `Fx` and `WideFx` serialize as their raw integer (`1.25` → `81920`).
- Ordered maps only (`BTreeMap`/`BTreeSet`). Keys of the lookup maps are lowercased ids.
- **Nothing is lost.** Each def has `extra: BTreeMap<String, String>` with every entry no field maps (lowercased key → last value). Units also list unmapped sections in `extra_sections` (none in the corpus).
- **Never fails on content.** Bad values become `None` plus an `Issue { file, section, key, message }`. Missing targets become `DanglingRef { from, field, kind, target }`. Both are zero or listed below for the retail corpus.
- Empty values (`key = ;`) read as absent. Duplicate keys: the last wins (Believed, as in `tdf.md`).

## Rulesets

| Kind | Units | Build menus | Tag |
|---|---|---|---|
| `Base` (default) | `units/*.fbi` (203) | `canbuild/<builder>/<unit>.tdf` (196) | Known layout |
| `Cb` | `units/` then `unitscb/` over it (203 = 197 from `unitscb/` + 6 only in `units/`) | `canbuildcb/` (171) | Believed |

What selects `cb` in the original game is **Unknown**. Falling back to `units/` for the 6 units `unitscb/` lacks (`monelem`, `monmum`, `tarhel`, ...) is a choice so missions still resolve; `Ruleset::overrides` lists every replaced def. Build menus are **not** overlaid: `cb` uses `canbuildcb/` alone.

Shared by both: features (`features/**/*.tdf`, 412 files, 1379 sections, 1344 distinct names), sides (`gamedata/sidedata.tdf`, 8), movement classes (`gamedata/moveinfo.tdf`, 11).

**Known**: 35 feature names are defined in more than one world folder (e.g. `cremana01` in caves, creon and volcano; the `veramb*` sound emitters). The last file in VFS path order wins and the replacement is listed in `overrides`. Whether the engine picks per map world is **Unknown**.

## `UnitDef` (`[UNITINFO]` of an `.fbi`)

| Field | Key(s) | Type | Tag / notes |
|---|---|---|---|
| `unitname` | `unitname` (else file stem) | string | Known: the id used by build menus, OTA units, sides |
| `name` | `name` | string | display name, runtime only |
| `side`, `tedclass`, `description` | same | string | Known: `description` is a kingdom word in Kingdoms, not prose |
| `objectname` | `objectname` | string | Known: model `objects3d/<objectname>.3do` (203/203 resolve) |
| `script` | derived | `scripts/<unitname>.cob` | Believed (no key names the script; 203/203 resolve) |
| `unitnumber` | `unitnumber` | i32 | |
| `categories` | `category` split on whitespace | Vec | Known |
| `damage_category` | `damagecategory` | string | Believed: the armor class weapons look up in `[DAMAGE]` |
| `body_type`, `movement_class`, `default_mission_type` | `bodytype`, `movementclass`, `defaultmissiontype` | string | `movementclass` resolves against `moveinfo.tdf` (all resolve) |
| `economy.build_cost` | `buildcost` | i64 | Believed: mana cost |
| `economy.build_time` | `buildtime` | WideFx | |
| `economy.worker_time`, `build_distance` | `workertime`, `builddistance` | i32 | |
| `economy.max_mana`, `mana_recharge_rate` | `maxmana`, `manarechargerate` | i32 | Known keys; the unit's own mana pool (34 base units) |
| `economy.mogrium_income`, `mogrium_storage` | `mogriumincome`, `mogriumstorage` | i32 | Believed: player mana income/storage ("mogrium" is the internal resource name) |
| `economy.economy_bonus`, `total_allowed` | `economybonus`, `totalallowed` | i32 | |
| `motion.maxvelocity`, `acceleration`, `brakerate` | same | Fx | Known: mix ints and decimals |
| `motion.turnrate`, `turninplacerate` | same | i32 | Believed (TA): angle units per tick |
| `motion.maxslope`, `maxwaterdepth`, `waterline`, `maneuverleashlength` | same | i32 | |
| `motion.cruisealt` | `cruisealt`, typo `cruisalt` | i32 | |
| `motion.roadmultiplier` | `roadmultiplier`, typo `roadmultplier` | Fx | Believed: speed factor on paved cells |
| `motion.watermultiplier` | `watermultiplier`, typo `watermultipliser` | Fx | |
| `motion.bankscale`, `pitchscale`, `moverate1`, `moverate2` | same | Fx | |
| `motion.attackrunlength`, `hoverattackdistance`, `hoverattackaltitude` | same | i32 | |
| `footprint.x`, `.z`, `.yardmap`, `.build_angle` | `footprintx`, `footprintz`, `yardmap`, `buildangle` | u8/u8/string/i32 | |
| `max_damage` | `maxdamage` | i32 | hit points |
| `heal_time` | `healtime` | Fx | |
| `sight_distance`, `radar_distance`, `sonar_distance`, `min_cloak_distance` | `sightdistance`, `radardistance`, `sonardistance`, `mincloakdistance` | i32 | |
| `cloak_cost`, `cloak_cost_moving` | `cloakcost`, `cloakcostmoving` | Fx | |
| `transport.*` | `transportcapacity`, `transportsize`, `transportsizecapacity`, `transportdistance`, `transmaxunits` | i32 | |
| `flags` | 38 boolean keys (`unit::FLAG_KEYS`: `canmove`, `builder`, `commander`, `canfly`, `canhover`, `upright`, `floater`, ...) | set of names | set when nonzero |
| `corpse`, `frozen`, `stone` | same | string | Known: feature names |
| `sound_category`, `sound_class`, `experience_points` | `soundcategory`, `soundclass`, `experiencepoints` | | |
| `weapons` | `[WEAPONn]` top-level sections | Vec<WeaponDef> | Known: there is no `weapons/` folder |
| `explode_as` | `[EXPLODEAS]` | WeaponDef | death explosion (6 base, 12 base+cb files) |
| `auras` | `[UNITINFO]/[ADJUSTARMOR]`, `[ADJUSTATTACK]`, `[ADJUSTJOY]` | `AuraDef { adjustment: Fx, affects_enemy, edge_effectiveness: Fx, radius }` | Believed: area buffs around monarchs/temples |
| `build_options` | `canbuild*/<this unit>/*.tdf` | Vec<BuildOption> | |

## `WeaponDef` (`[WEAPONn]`, `[EXPLODEAS]`)

| Field | Key | Type | Notes |
|---|---|---|---|
| `slot` | section name | string | `WEAPON1`, `EXPLODEAS` |
| `name`, `kind`, `subtype`, `damage_type` | `name`, `type`, `subtype`, `damagetype` | string | |
| `range`, `min_range`, `weapon_velocity`, `area_of_effect`, `aim_tolerance`, `turnrate` | `range`, `minrange`, `weaponvelocity`, `areaofeffect`, `aimtolerance`, `turnrate` | i32 | |
| `reload_time` | `reloadtime` | Fx | Believed (TA): seconds |
| `edge_effectiveness`, `gravity_adjustment`, `buildup_time`, `decay_time`, `duration` | same names | Fx | |
| `model` | `model` | string | projectile `objects3d/<model>.3do`; 3 weapons write the extension (`zonrock.3do`), which is stripped |
| `mana_per_shot` | `manapershot` | i32 | Known key: spells draw from the unit's `maxmana` pool (Believed) |
| `explosion_class`, `water_explosion_class`, `sound_hit_class`, `sound_hit`, `weapon_art` | same | string | |
| `lob_preferred`, `no_air_weapon`, `units_only`, `fire_starter`, `dont_lead_targets` | same | bool | |
| `veteran_level`, `veteran_model` | `veteranlevel`, `veteranmodel` / `vetmodel` | | |
| `damage` | `[DAMAGE]` | DamageTable | |

`DamageTable { default: i32, multipliers: BTreeMap<String, Fx> }`. `default` is an integer in every file (Known). Every other key is a damage category (`fort`, `tier1`, `monarch`, `human`, ...). **Believed**: those values multiply `default` for a target whose `damagecategory` matches. Evidence: `unitscb/arapult.fbi` has `default = 1250; fort = 1.25;` where `units/` has only `default = 1250;`, and most values are fractions. `DamageTable::against(category)` applies this and rounds down. The engine's rounding is **Unknown**.

## `FeatureDef` (`features/**/*.tdf`, one per top-level section)

`name` (section name, Known: TNT feature tables use it case-insensitively), `world`, `description` (runtime only), `category`, `footprint_x`/`footprint_z` (u8, default 1), `height`, `blocking`, `hit_density`, `damage` (hit points), `indestructible`, `reclaimable`, `auto_reclaimable`, `resurrectable`, `flamable`, `permanent`, `is_building`, `is_stone`, `is_frozen`, `no_display_info`, `object` (3D model, Known: resolves in `objects3d/`), `filename` + `seqname` + `seqname_shadow` (sprite art, Believed `anims/<filename>.gaf`, not checked), `energy` (Believed: mana from reclaiming), `sacred_site: Fx` (`sacredsite`, Believed: lodestone income multiplier on that site), `feature_dead`/`feature_burnt`/`feature_reclamate` (`featurereclamate`, typo `featurereclamamate`), `burn_weapon`, `burn_min`, `burn_max`, `spread_chance`, `decompose_time`. Keys are named after the TDF keys.

## Build menus, sides, movement classes

- `BuildOption { unit, priority: Option<Fx>, source }`. **Known**: every `canbuild*/<builder>/<unit>.tdf` has one section (usually `[Menu]`) with a single `priority` key. Ten priorities are fractional (`2.5`, `7.75`), so `priority` is `Fx`. Menus are sorted by priority, then name. **Believed**: lower priority shows first.
- `SideDef`: `name`, `name_prefix` (matches `UnitDef::side`), `commander`, `god`, `palette`, `build_palette`, `water_height` (values 58 and 40, the same as the TNT sea levels).
- `MoveClassDef`: `name`, `footprint_x/z`, `max_slope`, `max_water_depth`, `min_water_depth`, `max_water_slope`, `bad_slope`, `bad_water_slope`, `bad_min_water_depth`.

## `OtaDef` (typed `.ota`)

From `[GLOBALHEADER]`: `kingdom` (lowercased; picks `palettes/<kingdom>.pcx`, which `dump-map` now uses), `num_players_raw` and `num_players` (every number in the value; empty in 74 missions), `size` (`"6 x 6"` → `(6, 6)`), `memory`, `mission_name`/`mission_description` (runtime only), `is_mission`, `has_scenario`, `max_units`, `gravity`, `kill_mul`, `time_mul`, `line_of_sight`, `mapping`, wind/solar/tidal strengths, `water_does_damage`, `water_damage`, `lava_world`, `no_sea_level_trigger`. Victory keys (`ota::CONDITION_KEYS`, e.g. `commanderkilled`, `killalloftype`) are kept as raw strings in `conditions`, and `playerN` keys in `players`.
From `[MAP DATA]`: `map_type` (`type`), `ai_profile`; `specials` (`[SPECIALS]/[SPECIALn]`: `specialwhat`, `xpos`, `zpos`); `units` (`[UNITS]/[UNITn]`: `unitname`, `player`, `xpos/ypos/zpos`, `angle`, `healthpercentage`, `manapercentage`, `kills`, `ident`, `initialmission`, `aiignore`, `aiprioritytarget`, `missioncriticalunit`). 7341 placed units in 74 missions; all name existing units.

## `MapDef`

`MapDef::load(vfs, ruleset, "kmap/Name")` reads `<name>.tnt` (required), `<name>.ota` and `<name>.crt` (optional).

| Field | Source | Tag |
|---|---|---|
| `width`, `height` | TNT header, attribute cells (16 px) | Known |
| `heights` | TNT heights, row-major `u8` | Known layout |
| `sea_level` | TNT header | Believed (see `tnt.md`) |
| `kingdom`, `palette` | OTA `kingdom` → `palettes/<kingdom>.pcx` | Believed |
| `features` | TNT feature grid: one `PlacedFeature { name, def, x, z }` per cell holding an index; `def` is the ruleset key | Known: 88,319 placements in 310 maps. Believed one cell per instance: only 44 have a same-name east neighbour |
| `paved_cells`, `marker_cells` | `0xFFFB` / `0xFFFC` cells | Believed / Unknown |
| `start_positions` | OTA `StartPosN` specials, `source: Marker` | Known: all lie inside the map |
| `player_slots` | largest `numplayers` value, else the number of markers | Believed |
| `crt` | `CrtSummary { kind, count, first_name, body_len }` (body not decoded) | see `crt.md` |
| `passability` | derived, below | rule is ours, tunable |
| `ota` | the full `OtaDef` | |

**Start slots**: every one of the 310 maps has a position for every slot. Three missions (`takmission11_dh`, `takmission13_ph`, `takmission22_ph`) have no marker for one AI slot (`takmission11_dh` has `StartPos1` and `StartPos4` only). For those, the slot uses the first OTA unit that player owns (`source: FirstUnit`). **Known**: in many missions a marker sits in deep water (height 0). **Believed**: missions place units directly, so their markers are only camera or placeholder positions.

### Passability (first rule, `PassRule`, tunable)

`PassGrid { width, height, rule, cells: Vec<u8> }`, one flag byte per attribute cell:

| Flag | Rule |
|---|---|
| `WATER` | cell height `< sea_level` |
| `DEEP` | `sea_level - height > rule.max_water_depth` (default 20) |
| `STEEP` | the four corner heights `(x,z) (x+1,z) (x,z+1) (x+1,z+1)` (clamped at the edge) spread by more than `rule.max_slope` (default 30) |
| `FEATURE` | inside the `footprint_x × footprint_z` of a feature whose def has `blocking = 1` |
| `PAVED`, `MARKER` | TNT `0xFFFB` / `0xFFFC`; informational, do not block |

`passable(x, z)` = none of `DEEP | STEEP | FEATURE`. The defaults are `moveinfo.tdf` `GROUND2` (MaxSlope 30, MaxWaterDepth 20). Treating heights as corner samples follows TA (Believed). Per-movement-class grids (`WATER*`, `HOVER*`) can be derived by building a `PassRule` from a `MoveClassDef`. On the corpus: 33.2M cells, 6.0M deep, 1.03M steep, 0.78M feature-blocked.

**Unknown, so it is a tunable**: where a footprint sits relative to its anchor cell. `FeatureAnchor::TopLeft` (default) and `Center` score almost the same. With `TopLeft`, 6 of 1086 start markers sit on a blocking feature and 1814 other anchors fall inside a footprint. With `Center`, the numbers are 13 and 1976.

## Cross-references

Checked by `Ruleset::check_refs` and `MapDef::build`: unit model and script, unit `corpse`/`frozen`/`stone` → feature, `movementclass` → move class, weapon `model` → 3DO, build-menu builder and options → unit, feature `object` → 3DO, feature `featuredead`/`featureburnt`/`featurereclamate` → feature, side `commander`/`god` → unit, TNT feature table → feature, OTA `[UNITn]` → unit.

Dangling references in the retail data (**Known**):

| Ruleset/map | From | Field | Missing |
|---|---|---|---|
| `cb` | unit `ARAPAL` (`unitscb/arapal.fbi`) | `frozen` | feature `arapal_frozen` |
| any | map `kmap/Temple of Blood` | TNT feature table | feature `zonhand_dead` |

Every unit in both rulesets resolves its model and script. Every build-menu target, OTA unit and side unit resolves.

## Key coverage (`tak-cli content --coverage`)

Base ruleset plus all 310 OTAs. "Mapped" means a typed field reads the key. Everything else is in `extra`. In `cb`, `UNITINFO` adds `firestandorder` (unmapped) and `DAMAGE` has 17 categories.

| Section path | Sections | Keys mapped | Keys unmapped | Unmapped keys |
|---|--:|--:|--:|---|
| `canbuild:*` | 196 | 1 | 0 |  |
| `fbi:EXPLODEAS` | 6 | 8 | 0 |  |
| `fbi:EXPLODEAS/DAMAGE` | 6 | 3 | 0 |  |
| `fbi:UNITINFO` | 203 | 104 | 18 | `admultiplier`, `animatetype`, `bloodcolor1`, `bloodcolor2`, `bloodcolor3`, `copyright`, `corpseadjustx`, `corpseadjustz`, `damagetype`, `nochasecategory`, `notargetcategory`, `shadowart`, `shadowgaf`, `standingmoveorder`, `standingunitorder`, `unitstandorders`, `version`, `wpri_badtargetcategory` |
| `fbi:UNITINFO/ADJUSTARMOR` | 2 | 4 | 0 |  |
| `fbi:UNITINFO/ADJUSTATTACK` | 1 | 4 | 0 |  |
| `fbi:UNITINFO/ADJUSTJOY` | 5 | 4 | 0 |  |
| `fbi:WEAPON#` | 198 | 31 | 34 | `buttonimagedisabled`, `buttonimagedown`, `buttonimageselected`, `buttonimageup`, `emittime`, `hweffect`, `innercolor`, `lightmap`, `maxvariation`, `middlecolor`, `nimbus`, `outercolor`, `particlespersecond`, `radiusart0`, `radiusart1`, `radiusart2`, `ringcount`, `ringdelay`, `ringduration`, `shadowart`, `shadowgaf`, `shakeduration`, `shakemagnitude`, `shotart`, `showeffect`, `soundclass`, `spinheading`, `spinpitch`, `spritecount`, `variationtime`, `wanderendart`, `wanderloopart`, `wanderstartart`, `weapongaf` |
| `fbi:WEAPON#/DAMAGE` | 198 | 14 | 0 |  |
| `feature:*` | 1379 | 34 | 18 | `animatable`, `animating`, `animtrans`, `nodrawundergray`, `noshadow`, `reproduce`, `reproducearea`, `seqnamebackflame`, `seqnameburn`, `seqnameburnshad`, `seqnamedie`, `seqnamefrontflame`, `seqnamereclamate`, `shadtrans`, `soundclass`, `sounddelay`, `soundvariance`, `sparktime` |
| `moveinfo:CLASS#` | 11 | 10 | 0 |  |
| `ota:GLOBALHEADER` | 310 | 43 | 1 | `copyright` |
| `ota:GLOBALHEADER/MAP DATA` | 310 | 2 | 0 |  |
| `ota:GLOBALHEADER/MAP DATA/SPECIALS/SPECIAL#` | 1083 | 3 | 0 |  |
| `ota:GLOBALHEADER/MAP DATA/UNITS/UNIT#` | 7341 | 14 | 0 |  |
| `sidedata:SIDE#` | 8 | 7 | 15 | `buildsparklyanim`, `buildsparklygaf`, `fogcolor`, `frozenanim`, `frozengaf`, `logoart`, `logogaf`, `musictracks`, `nimbus`, `resurrectsparklyanim`, `resurrectsparklygaf`, `stoneanim`, `stonegaf`, `underattack_delay`, `underattack_sound` |

The unmapped keys are presentation (art, colours, sounds, UI buttons, effects) or AI targeting hints. They wait for the renderer and AI packages.

## Mana economy keys (Kingdoms has one resource)

Unit: `buildcost`, `maxmana`, `manarechargerate`, `mogriumincome`, `mogriumstorage`, `economybonus`. Weapon: `manapershot`. Feature: `sacredsite`, `energy`. OTA unit: `manapercentage`. Feature names `*Mana01..03` (e.g. `TarMana02`) are the lodestone sites placed on maps. **Unknown**: how income is computed from these. **Believed**: lodestone buildings (`mogriumincome`) on `sacredsite` features, scaled by the site value.

## Alignment with WP-I's sim stand-ins

- Unit stand-in (FBI key names) ↔ `UnitDef.motion`: `maxvelocity: Option<Fx>`, `acceleration: Option<Fx>`, `brakerate: Option<Fx>`, `turnrate: Option<i32>`. Field names match the keys exactly. Units whose `.fbi` has no such key (buildings: `maxvelocity` is in 299 of 400 files) get `None`. A `From<&UnitDef>` in tak-sim should treat `None` as zero.
- Map stand-in (`MapInfo`) ↔ `MapDef`: `width`, `height` (attribute cells), `heights: Vec<u8>` (row-major, `height_at(x, z)`), `start_positions: Vec<StartPosition { player, x, z, source }>` in attribute cells. Use `passability.passable(x, z)` for blocking and `sea_level` for water.
- Units of measure: positions in attribute cells (16 px). The world scale of heights is **Unknown** (`tnt.md`).

## Unknown / open

- What selects the `cb` ruleset. The `units/` fallback is our choice.
- `[DAMAGE]` non-default values as multipliers (Believed), and the rounding.
- Feature footprint anchoring (tunable); whether `0xFFFB` paving changes speed (`roadmultiplier`) or only build rules.
- Which of a duplicated feature name a map uses (world-specific?).
- CRT body layout (pre-placed units for 12 maps) is not decoded, so `MapDef` does not add CRT units.
- The meaning of `priority` order, and how mana income is computed.
