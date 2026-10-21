# WP-H: content definitions (M3b)

**Owns:** `crates/tak-content/`, `docs/content.md`, `crates/tak-cli/src/content_cmd.rs`, and `crates/tak-cli/src/map_cmd.rs` (only to swap its `.ota` stopgap for your typed view).
**Inputs on main:** `tak_formats::tdf` (WP-A), `tak_formats::tnt` + `tnt::crt` (WP-E), `tak_formats::model3do` (WP-B), `tak_formats::cob` (WP-D), `tak_math` (WP-F). Read `docs/formats/tdf.md`, `fbi-keys.md`, `gamedata-keys.md`, `gamedata-sub-keys.md`, `features-keys.md`, `ota-keys.md`, `tnt.md` first.

## What wave 1 established
- Every `.tdf`, `.fbi`, `.ota`, `.tsf` parses. Read numbers with `Entry::decimal()` then `Decimal::to_fixed(16)`, because many keys mix ints and decimals across files (e.g. `maxvelocity`); the census "Shapes" column shows which. Use `sections_named()` for repeating sections; `section()` returns only the first.
- Duplicate keys exist (16); the last wins (Believed). Keys are case-insensitive.
- **No `weapons/` folder**: weapons are `[WEAPONn]` sections inside each `.fbi`, each with a nested `[DAMAGE]` section. Some units have `[EXPLODEAS]`/`[DAMAGE]` and `[ADJUSTARMOR]`, `[ADJUSTATTACK]`, `[ADJUSTJOY]` sections.
- `units/` (203) and `unitscb/` (197) are parallel balance sets; `cb` has tiered damage categories and per-armor multipliers. What selects `cb` in the original is Unknown. Support both as named "rulesets", default `units/`.
- `features/` (412 tdf) define map features; TNT feature tables reference them by name. `canbuild/` + `canbuildcb/` define build menus. `gamedata/` holds sound classes, explosions, damage flames, effects.
- OTA shape: `GLOBALHEADER / MAP DATA / {SPECIALS/SPECIAL#, UNITS/UNIT#}`; empty values are common.
- TNT (WP-E): heights u8 per 16-px cell, sea level, feature grid + table (u16 index, `0xFFFF` empty, `0xFFFB` paved Believed, `0xFFFC` Unknown), tile planes into `terrain/*.jpg`. No passability plane: derive it from slope, water, features, paved cells. Height world scale is Unknown.
- CRT: 12 of 214 files hold what look like pre-placed unit records (Believed).

## Build
1. **Typed definitions** in `tak-content`, sim-ready: all numbers as `tak_math::Fx` or integers, ordered maps only (`BTreeMap`), no floats. `UnitDef` (identity, side/kingdom, model and script names, cost/build time, mana fields, speed/accel/brake/turn, footprint, hit points, armor, sight, build options, categories, weapons), `WeaponDef` (range, reload, damage per armor class, projectile, area of effect), `FeatureDef`, `BuildMenu`, and a `Ruleset` that loads one balance set.
2. Keep unknown keys: each def carries the raw TDF entries it did not map, so nothing is lost. Report the coverage (mapped vs unmapped keys per section) in `docs/content.md`.
3. **Cross-references**: resolve and validate model (`objects3d/<objectname>.3do`), script (`scripts/<...>.cob`), build-menu targets, weapon references, feature names in TNT tables. Report every dangling reference instead of failing.
4. **MapDef**: combine TNT + typed OTA + CRT header into one struct: dimensions, heights, sea level, features placed with their defs, start positions per player, kingdom/palette, per-map metadata. A first derived passability grid (document the rule; tunable).
5. Swap `map_cmd.rs`'s raw `kingdom=` text scan for the typed OTA view.
6. `tak-cli content [--ruleset cb] [--unit ARAARCH] [--map "kmap/Adamantine Gate"]`: print a summary or one def as JSON.
7. Coordinate with WP-I's sim stand-ins: its `UnitDef` stand-in uses FBI key names (`maxvelocity`, `acceleration`, `brakerate`, `turnrate`) and its `MapInfo` has width, height, heights and start positions. Provide conversions (`impl From<&UnitDef> for ...` lives in tak-sim later; just make the fields line up and document them).

## Done when
- Both rulesets load with zero errors; every unit resolves its model and script (or the misses are listed in `docs/content.md`).
- All 310 playable maps build a `MapDef` with start positions for every player slot.
- Corpus tests assert counts and zero errors; unit tests use synthetic TDF text only.
- `docs/content.md` documents each def's fields with their source keys and confidence.
