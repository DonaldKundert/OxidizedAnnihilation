# Combat (`tak-sim/src/combat.rs`, WP-M)

Health, regeneration, targeting, the Kingdoms weapon script protocol,
projectiles, damage, death. Code: `crates/tak-sim/src/combat.rs` (tick,
state) and `combat/defs.rs` (stats, FBI conversion, formulas; every guessed
number is a named constant or one small function there).

Tags: **Known** (observed in the retail data), **Believed** (TA-derived or
inferred, not contradicted), **Chosen** (our pick where the original is
unknown; deterministic, easy to change), **Unknown**. "TA:" marks findings
from Nanolathe's MIT research docs on the *original TA* executable
(`research/retail-executable-spec/06-weapons-projectiles-damage-and-effects.md`,
cited as "NL §n"); they are not confirmed for Kingdoms.

## Data (FBI → `CombatDef`)

`UnitDef.combat` (serde-defaulted and hashed; empty for synthetic defs, which
then stay outside combat: never targeted, damaged or killed).

| FBI | Sim | Tag |
|---|---|---|
| `maxdamage` | hit points; 0 = outside combat | Known key |
| `healtime` | hit points regenerated **per second** (16.16), applied every tick with a remainder, always while damaged | Believed (see below) |
| `sightdistance` | how far an idle unit looks for enemies to chase | Believed |
| `maneuverleashlength` | how far from its starting point an idle unit chases (falls back to sight) | Believed (TA "maneuver" stance) |
| `damagecategory` | lowercase row looked up in `[DAMAGE]` | Believed (`content.md`) |
| footprint | body radius = 8 × max(footprintx, footprintz) | Chosen (TA uses the model bounding box, NL §9.3) |
| `[WEAPONn]` | weapon slot n−1 (the script's weapon index) | Known order |
| `type` | `Melee`, `Ballistic`, `Line of Sight`, `Guided`, `Remote Effect`/`wandering` (case-insensitive); unknown → ballistic | Known values |
| `range`, `minrange` | world units, centre to centre | Believed (TA pixels) |
| `reloadtime` | ticks = trunc(s × 30), min 1 (16 raw steps added back to undo 16.16 rounding, so `1.7` → 51) | TA (NL §4.2), Believed |
| `weaponvelocity` | world units per second | Believed (TA) |
| `areaofeffect` | blast diameter; R = aoe / 2 | TA (NL §9.3) |
| `edgeeffectiveness` | blast-edge factor | TA |
| `aimtolerance` | max facing error (angle units) for firing; default 0x1000 | Believed / default Chosen |
| `noairweapon` | slot cannot select `canfly` units; independent of range | Known flag, Believed semantics |
| `manapershot` | > 0 marks a spell: fired only by `CastSpell` | Chosen |
| `damagetype` | damage kind: none 1, `fire` 2, `explosion` 3, `paralyzer` 4 | Believed (script branches, below) |
| `[DAMAGE]` | `default` × multiplier of the target's category, truncated | Believed (`content.md`) |
| `[EXPLODEAS]` | blast at the dying unit's position when it is removed | Believed |

**healtime.** TA reads it as a per-second worker rate for self-repair (1 hp
per 8 ticks once non-zero, NL / 05 "healtime, the only consumer"). Kingdoms
values are decimals (`0.4125` swordsman, `0.0417` monarchs, `3.4375`
critters, `600` gods, `0.001` mummy), so we keep the TA reading (rate per
second) but apply it finely. Gods regenerate fast, undead barely; plausible
but unverified. The inverse reading (seconds per hit point) fits the mummy
less well. Regeneration has no mana cost (Kingdoms has no energy).

**Melee reach (Chosen).** 45 of 48 melee weapons author `range = 300` (the
goblin 50, the god 1000), far beyond the visible sword reach; swordsmen walk
up to their target in play. Melee hits need body contact:
`radius_a + radius_b + 16` (`melee_reach`). The authored range is not used
for melee, and melee units look for enemies only within `sightdistance`.

## Script callbacks (argument lists)

From the bytecode of all 204 scripts (`tak-cli disasm`): leading
`alloc-local` count, which locals are read before written, and how values
flow into ports. Weapon indexes are **0-based** (Known: `araat` branches on
weapon 0/1 for its two archers, `QueryWeapon` picks a muzzle per index).

| Callback | Arguments | Use in scripts | Confidence |
|---|---|---|---|
| `AimWeapon(heading, pitch, weapon)` | heading relative to the unit's facing, signed angle units; pitch signed angle units; weapon index | turrets `turn ... y-axis` to `heading − 0x8000` and `wait-for-turn`, then `set 22 weapon` (129 of 142); infantry just wait for the current attack to finish | heading/weapon Known from usage; pitch Believed (never read in the corpus samples) |
| `FireWeapon(weapon)` | weapon index | starts `attackN(weapon)`; the animation writes `set 23 weapon` at the strike/release frame unless `TargetCleared` set its abort static | Known |
| `TargetCleared(weapon)` | weapon index | `set 21 weapon`, sets an abort static, signals the aim thread | Known |
| `QueryWeapon(piece, weapon)` | in/out muzzle piece, weapon index | writes the piece into local 0 | Known (not called yet: piece positions are not resolved) |
| `SweetSpot(piece)`, `QueryBlood(piece)` | in/out piece | | Known (not called) |
| `SetMaxReloadTime(ms)` | longest reload in ms | scripts store `2 × ms` as the `RestoreAfterDelay` time | Believed (TA); called once, the tick after spawn so `Create` has initialised its statics |
| `HitByWeapon(kind, x, z, damage)` | damage kind, hit direction vector, damage dealt | `aracan` rocks by `x·damage/500`; `cregod` ignores kinds 2 and 4 and turns its blood piece to `ATAN(z, x)` (port 14) | kind/damage Believed; vector sense Chosen: (forward, right) toward the source in the unit frame, length 500 |
| `Dying(kind)` | damage kind of the killing blow | kind 3 skips the death animation; otherwise plays it; ends with `set 26 1` (128 scripts) or `set 31 1` (38 monarch/god/lodestone scripts) | Believed |
| `Killed(severity, corpse, kind)` | severity 1..100, in/out corpse type (0 none, 1 normal), kind | kind 3 explodes every piece and leaves no corpse, kind 2 emits fire for 1.4 s, else corpse 1 | Believed (TA `Killed(severity, corpsetype)` plus a Kingdoms kind) |
| `TurnDirection(flag)` | 1 while turning? | ORed into the moving static by `MoveWatcher` | Unknown; not called |

## The weapon protocol (per slot, per tick)

Every script with `FireWeapon` (142) writes both port 22 and port 23
(**Known**), so the engine side is:

1. **Target.** Order target if any (Attack: every non-spell slot; Cast: the
   spell slot). Otherwise, unless fire state is Hold: keep the current unit
   target while it stays a valid enemy in reach, else the nearest enemy in
   reach, ties to the lower slot (ReturnFire: only the last unit that hurt
   us). Changing or losing a target that was being aimed or fired at calls
   `TargetCleared(w)`.
2. **Aim.** Write port 22 = −1, call `AimWeapon(rel heading, pitch, w)`.
   Aimed once the script writes 22 = w (often after a `wait-for-turn`, on a
   later tick). Re-called every 15 ticks until aimed (Chosen; each call
   signals the old aim thread, as in TA). Units without a script aim at once.
3. **Fire** when aimed, reload 0, in reach, and (mobile units only) facing
   the target within `aimtolerance`. Spells pay mana first (all or nothing;
   **TODO(WP-N merge)**: `economy::spend_unit_mana`). Write 23 = −1, call
   `FireWeapon(w)`, start the reload, clear the aim.
4. **Launch** when the script writes 23 = w (the strike frame; one tick of
   latency because scripts run after combat). No write within 90 ticks
   (Chosen) means the attack was abandoned. Units without a script launch at
   once.

Standing units turn their body toward their first target at `turnrate`
(infantry `AimWeapon` turns nothing). Moving units (player orders) still
fire at targets inside their tolerance.

Differences from TA: TA scans for targets on a per-player time slice and
prefers categories (NL §3.2); we pick the nearest every tick. TA leads
moving targets unless `dontleadtargets`; we aim at the current position.
TA has three fixed slots with `Primary/Secondary/Tertiary` callbacks;
Kingdoms passes the index instead (Known).

## Shots and damage

| Kind | Flight | Tag |
|---|---|---|
| Melee | instant hit on the target unit if within reach + 16 at the launch frame | Chosen |
| Remote Effect, wandering | instant, at the target point | Chosen |
| Line of Sight, Ballistic | straight to the target point at `velocity / 30` per tick, no homing; instant without a velocity; moves in its launch tick (TA, NL §1.1) | Chosen (no arc: ground travel only) |
| Guided | as above, destination follows the target unit | Believed |

**Impact** (TA routing, NL §9.1): if `areaofeffect <= 16` and a unit's body
covers the point (the aimed unit first, else the lowest slot), that unit
alone takes full damage. Otherwise every combat unit whose body edge is
within R (`d = distance − radius`, accepted if `d < R`) takes

    damage = trunc(base × ((1 − e)·(d/R − 1)² + e)),   base = [DAMAGE] row for its category

computed in integers (`area_damage`). The shooter is never hurt by its own
shot; allies are (TA, NL §9.3). No veterancy, armor auras (`ADJUSTARMOR`)
or paralysis yet.

## Death

Health ≤ 0: severity = clamp((overkill × 100 / max + percent before the
hit) / 2, 1, 100) (TA, NL death preamble; TA uses the percentage sampled
every 30 ticks, we use the one before the hit). The unit stops, drops its
orders, can no longer be targeted (port 4 reads 0), and `Dying(kind)` runs.
When it writes port 26 or 31 (or returns at once, or after 150 ticks,
Chosen), `Killed(severity, 0, kind)` runs, its corpse answer is recorded in
`CombatState.corpses` (with position, heading and definition) and the unit is
removed through `lifecycle::remove_unit`; `[EXPLODEAS]` goes off at that
moment. Then the corpse feature is spawned (next section).

## Frame health (WP-P)

Units under construction ("frames") take part in combat: they can be
targeted and damaged from the tick they are placed, but never act.

| Rule | Value | Tag |
|---|---|---|
| Starting health | **0** | TA-Known (Nanolathe `05` "Nanoframe allocation": a new nanoframe has remaining fraction 1 and health 0) |
| Gain per build step | `trunc(max × old_remaining) − trunc(max × new_remaining)`, remaining = `(total − done) / total`, capped at `maxdamage` | TA-Known ("Health gain and fractional carry": difference of truncations, so the steps sum to exactly `maxdamage`) |
| Damage to a frame | plain health loss; build progress is untouched, so a damaged frame completes below full health | TA-Known (the work step and damage write different fields) |
| Health 0 or less | the frame dies like any unit (`Dying`, `Killed`, corpse) | TA-Known (TA's health check is on damage) |
| Regeneration (`healtime`) | only once complete | Believed |
| Destroyed frame | the builder's order ends (`StopBuilding`, order dropped); a factory drops its whole queue (WP-O) | Believed |

Code: `combat::build_progress` (called by construction after every work
step), `combat::frame_health`. A frame first seen by combat without a work
step gets the health its progress has earned. Consequence players know: a
fresh frame dies to almost anything, so building under fire fails (the
WP-P real-map run had to keep the Taros monarch out of fireball range of
the keep's yard, or the first swordsman frame died at once and the keep
dropped its queue). Confidence: high for TA; the Kingdoms scripts' port 4
reads (health percent) during construction are consistent with it.

## Corpses and features (WP-P)

Module `tak_sim::features`; state `World::features` (hashed); static data
`Scenario::features` (`FeatureSetup`: feature definitions, placed map
features, the corpse of each unit definition).

- **Placed features**: every TNT feature-grid entry whose definition matters
  to gameplay (`blocking`, `reclaimable`, `damage`, `energy` or `sacredsite`)
  is a live feature, with health = `damage`. The content passability rule's
  top-left/center anchoring is retained. Content conversion transfers its
  static `FEATURE` stamps to counted live ownership only after the complete
  placed set is represented; capacity exhaustion fails closed. Other terrain
  flags and unmanaged cells remain untouched. Authored Scenario terrain is
  never normalized: an explicit static `FEATURE` remains an independent blocker.
- **Corpses**: after `Killed`, corpse answer 1 spawns the FBI `corpse`;
  higher answers follow up to four `featuredead` links (Believed, TA corpse
  depth). Answer 0 or no `corpse` leaves nothing. The footprint is centred
  and snapped like a structure (Chosen; `corpseadjustx/z` not applied), with
  the unit's heading and health = `damage`.
- **Blocking**: live blocking placed features and corpses each own one
  counted obstacle over their clipped footprint. Removal/restamping affects
  neither overlapping live features, structure yards nor slope/water. Layers
  built later see the same counts; existing layers and active searches are
  invalidated immediately. `S` placement keeps its previous placed-map-feature
  exemption, but overlapping corpses/external obstacles still refuse the site.
  Content `features_block = false` disables only placed-feature occupancy.
- **Decay**: a corpse with `decompose_time` vanishes that many seconds after
  it appeared (Believed). Runs at the end of the combat lane.
- `CombatState.corpses` (the unit death log) is kept as before.

### Weapon-impact clearing (bounded first slice)

Existing ground attacks and weapon explosions can damage and destroy features.
This is **not** the builder's Clear/reclaim action and awards **no mana**.
`reclaimable` does not confer damage immunity. Positive-health features are
eligible unless `indestructible`; zero/negative damage is ignored. FBI
`unitsonly` now reaches the sim and excludes all feature damage, including splash.

The following are **Chosen**, centralized in `features::impact_hits` and
`features::apply_damage`, rather than claims of exact retail behavior:

- `areaofeffect <= 16`: preserve the existing direct-unit priority; otherwise
  hit the first eligible feature in stable slot order whose half-open footprint
  contains the impact point. Zero-health/indestructible scenery does not shield
  an overlapping eligible feature. No automatic feature targeting is added.
- Larger blasts: distance is to the nearest point of the feature footprint;
  use the existing integer quadratic `area_damage` falloff within the radius.
  Feature damage uses the weapon's authored **default**, not an invented mapping
  from feature `category` to unit damage multipliers. Falloff intermediates are
  wide enough for validated extreme authored values.
- Snapshot `(FeatureId, damage)` hits before applying them. A lethal hit retires
  the old generation and spawns at most one `featuredead` successor. Discard
  overkill: the same impact cannot damage its own successor. Separate impacts
  in the same tick remain ordered and may hit later generations.
- Successors keep origin/heading, start at their own full `damage` and reset
  decay age. Their footprints snap around the original stable replacement
  centre, so odd/even footprint cycles cannot drift across the map. Each new
  generation causes the existing client feature renderer to replace its art.
- Weapon successors inherit any raw Clear earnings already claimed by the
  replacement lineage, including storage overflow. This Chosen conservation
  cap prevents partial Clear/weapon cycles from resetting mana entitlement;
  work progress itself resets for the new lifetime (see construction rules).

**Unknown/deferred:** retail feature hit volumes/density and blast falloff,
feature-category damage modifiers, projectile interception/occlusion, burning
and `featureburnt`, original reclaim timing/payout/damaged value/concurrency and
`featurereclamate`, resurrection. The separate builder Clear implementation has
explicitly Chosen rules in [construction.md](construction.md#builder-clear--reclaim);
no exact original formula is inferred from `damage` or `energy`. Ground attacks continue until stopped, as before; a melee
unit aiming deep inside a large blocking footprint may not reach the point.

Replay checkpoints use **rules version 7**; versions 1–6 are rejected rather
than silently changing their future combat or obstacle ownership. Existing
weapon-command JSON encoding is unchanged; Clear is an additive command variant.
Synthetic coverage is in `feature_damage.rs` (including cross-process command replay) and `feature_content.rs`; `tak-headless/tests/feature_damage_checkpoint.rs`
checks partial-damage/successor/cleared checkpoint continuations against fresh
command replay. No retail assets are included in fixtures. Retail graphical behavior still requires the user's
licensed install and a graphics-capable runtime.

## Orders

- `Attack` (unit or ground): replace drops the queue; append queues
  `Order::Attack` and starts it when it reaches the front. Units close
  through `pathfinding::go_to`, re-issued when the target moves 16 units from
  the last goal, and stop when any weapon is in reach. Unit attacks end when
  the target dies; ground attacks run until replaced.
- `Guard`: stay within 96 units of the guarded unit (Chosen), chase enemies
  within sight that are within the leash of the guarded unit. A mobile
  builder guarding something under construction assists instead (WP-P,
  `docs/construction.md`); while it assists it neither follows nor chases.
- `CastSpell { spell: SpellId(slot) }`: fire spell weapon `slot` once
  (targetless: on the caster). Mana via the economy (TODO above).
- Idle units at FireAtWill (or ReturnFire against their attacker) chase the
  nearest enemy within sight (ranged: also within weapon range) while it is
  within `maneuverleashlength` of where the chase began, then walk back.
- A player `Move` (replace) or `Stop` cancels the combat order.
- `go_to` replaces the whole queue (pathfinding stub), so a queued order
  behind an attack is lost when the unit starts closing.

## Results

- Synthetic (`tests/combat_damage.rs`): melee 360 × 1.5 (`human`) = 540 per
  hit every 51 ticks, the target dies on the second hit and is removed;
  a 300 px/s ballistic shot lands 20 steps after launch with full damage on
  the target and TA falloff on a neighbour; Attack orders close distance;
  Hold fire holds; idle chase; regeneration.
- Determinism: `tests/combat_determinism.rs` re-runs its own binary as a
  child process and compares per-tick hashes of a 4-v-4 brawl and (with
  data) the real skirmish. All state is hashed; no floats; no RNG use.
- Real data (`tests/combat_skirmish.rs`, Adamantine Gate, original COB): 5
  ARASWORD vs 5 ZONWOLF2 close in and fight to a winner in 263 ticks
  (swordsmen win at about half health); 5 ARASWORD vs 5 ZONTROLL: trolls win
  without loss in about 360 ticks; 5 ARAARCH vs 5 ZONTER: Zhon win. Zero VM
  errors, zero runaway loops; ports 22, 23, 26/31 written as expected.
- WP-P (`tests/open_items.rs`, synthetic): frames start at 0 health and
  gain it to exactly `maxdamage` on the completion tick (6 hp after the
  first monarch tick on a 1000-hp, 65-s wall); 300 damage mid-build leaves
  the wall complete at 700; a fresh frame dies to a 50-damage hit; a 4x4
  blocking wreck blocks the layer and a walker routes around it; a body
  decomposes after 30 ticks. Real data (`tests/open_items_real_map.rs`):
  see `docs/construction.md`.
- **Runaway loops re-tested** under real combat (each unit attacks a troll,
  is ordered to walk off mid-attack, re-attacks): 10 of 11 no longer run
  away (ARAPULT, MONDEV, MONGHOUL, NPCAYLA, NPCPOW2, VERBAL, VERLIHR, VERPULT,
  ZONGIANT, ZONHURT). `MONPIRAN:swim` still trips the cap once; the piranha
  is a water unit run on land in this test. The cap stays.

## Unknown / open

- Exact `healtime` meaning; melee reach; whether only a *selected* weapon
  fires (the readme's "Select weapon 1..3" keys) instead of every non-spell
  weapon; whether spells are ever auto-cast.
- Launch origin (muzzle piece via `QueryWeapon`) and arcs; target leading;
  veterancy (port 32, `veteranlevel`), auras, paralyzer, turn-to-stone,
  mind control.
- Ports 21 (aim aborted ack, unused by the engine), 26/31 exact meanings.
- Corpse placement offset (`corpseadjustx/z`), `decompose_time` units,
  whether features can be shot.

## Ground-only weapon targeting (October 2026 fidelity pass)

**Known**: the owned base and Crusades definitions have 58 weapon slots with
`noairweapon`; the content parser already preserves that flag. **Believed**:
these slots cannot select `canfly` units. The sim now preserves the flag and
checks it separately from range, for explicit attacks/casts, autonomous fire,
Return Fire, Guard chases and AttackMove engagements. Mixed-slot units may use
an unrestricted slot against flyers while their ground-only slot stays idle.
An incompatible explicit order ends without spending spell mana or chasing.
Script-delayed launches recheck compatibility. Ground-point orders remain
valid; this change does not infer new airborne splash-immunity rules.

The synthetic `air_targeting` integration tests cover those paths, delayed
script signals and cross-process hashes; its owned-data test checks every
base/Crusades weapon's mapping and reports how many flagged slots it exercised.
Older scenarios without the field deserialize as unrestricted and retain their
serialized shape/hashes. Owned scenarios whose weapon restrictions changed get
a different scenario fingerprint; old owned replays/checkpoints are not evidence
for the corrected rules.
