# WP-M: combat

**Owns:** `crates/tak-sim/src/combat.rs` (and a `combat/` submodule folder), `docs/combat.md`.
**Lane API:** `combat::apply_command` (Attack, Guard, CastSpell), `combat::tick`, `UnitCombat::health_percent()` (scripts read it as port 4). Use `pathfinding::go_to` to close to range, `lifecycle::call_script` / `start_script` for script callbacks, `lifecycle::remove_unit` for deaths, `economy::drain` for spell or shot mana.

## Starting point (on main)
- Weapons are `[WEAPONn]` sections inside each `.fbi` with nested `[DAMAGE]` (per armor/damage class). tak-content has `WeaponDef`, `DamageTable`, `AuraDef`; `UnitDef` has `maxdamage` (hit points), `armor`, `damagecategory`, `healtime`, `sightdistance`, `explodeas`/`selfdestructas`, `corpse`. Weapon keys include `range`, `reloadtime`, `weaponvelocity`, `areaofeffect`, `manapershot`. The `cb` ruleset adds tiered damage categories and per-armor multipliers. Read `docs/content.md` and `docs/formats/fbi-keys.md`.
- Script callbacks seen in the corpus: `AimWeapon`, `FireWeapon`, `QueryWeapon`, `TargetCleared`, `HitByWeapon`, `SweetSpot`, `QueryBlood`, `Killed`, `Dying`, `SetMaxReloadTime`. Query callbacks answer in `locals[0]` (`ScriptReturn.locals`). Argument lists are Unknown; derive them from the bytecode (`tak-cli disasm`) and TA conventions, and document.
- Kingdoms ports 21–23 are stored aim flags (`WEAPON_AIM_ABORTED`, `WEAPON_READY`, `WEAPON_LAUNCH_NOW`; see `docs/formats/cob-vm.md`, `docs/sim.md`).
- 11 walk/swim script loops never sleep under fake inputs; WP-K found they need `FireWeapon`/`TargetCleared` mid-attack. Re-test them here.

## Build
1. Health (from `maxdamage`), regeneration if the data supports it, death: call `Killed` (severity arg per TA convention), then remove the unit; spawn the corpse feature later (note it as follow-up if features aren't units yet).
2. Targeting: auto-acquire per fire state within weapon range / sight, deterministic choice (nearest, ties by slot), `Attack` (unit or ground) and `Guard` orders; close to range via `go_to`.
3. Weapons: reload timers in ticks, aim via `AimWeapon` (heading/pitch in COB angle units relative to the unit) honouring the script's answer and the aim flags, fire via `FireWeapon`, projectiles as simple deterministic fixed-point entities (instant hit for melee/hitscan, straight or ballistic for missiles), area damage, damage per the target's armor/damage class from `DamageTable` (base ruleset) or the `cb` multipliers.
4. `HitByWeapon` callback on the victim; `HEALTH` port live.
5. Stretch: `CastSpell` with mana from `economy::drain`; note what spells need if not done.

## Done when
- A scripted skirmish (e.g. 5 Aramon swordsmen vs 5 Zhon units on a real map) plays out to a winner: units close in, scripts run their attack callbacks with zero VM errors, units die and are removed.
- Damage per hit matches the FBI/DAMAGE numbers in unit tests built from synthetic defs.
- The 11 runaway loops are re-tested under real combat; report which remain.
- Cross-process determinism test with combat on; all existing tests pass; no floats.
- `docs/combat.md` documents formulas, callback argument lists, and confidence tags.
