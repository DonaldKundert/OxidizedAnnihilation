//! Combat (WP-M): health, regeneration, targeting, the Kingdoms weapon
//! script protocol, projectiles, damage and death. Formulas, callback
//! argument lists and confidence tags: `docs/combat.md`.
//!
//! - **State** lives in [`UnitCombat`] (per unit) and [`CombatState`]
//!   (projectiles, death log); both are hashed every tick.
//! - **Health** is initialised lazily the first time [`tick`] sees a unit
//!   (from `maxdamage`); a definition with `maxdamage = 0` is outside combat.
//! - **Orders**: `Attack` (unit or ground), `Guard` and `CastSpell` arrive via
//!   [`apply_command`]; units close distance through
//!   [`crate::pathfinding::go_to`].
//! - **Weapons** follow the protocol the retail scripts implement: the
//!   engine calls `AimWeapon(heading, pitch, weapon)` and waits until the
//!   script writes the weapon index to port 22 (`WEAPON_READY`); it then
//!   calls `FireWeapon(weapon)` and launches the shot when the script writes
//!   the index to port 23 (`WEAPON_LAUNCH_NOW`, the strike frame of the
//!   attack animation). `TargetCleared(weapon)` tells the script to abandon
//!   the attack. Units without a script aim and launch at once.
//! - **Death**: `Dying(kind)` plays the death animation (the script writes
//!   port 26 or 31 when done), then `Killed(severity, corpse, kind)` picks
//!   the corpse and the unit is removed via [`crate::lifecycle::remove_unit`].
//!
//! Units are visited in slot order; projectiles in creation order.

mod defs;

pub use defs::{
    CombatDef, DEFAULT_AIM_TOLERANCE, DamageStats, MAX_WEAPONS, MELEE_MARGIN, WeaponKind,
    WeaponStats, area_damage, combat_def_from_fbi, kind, melee_reach, reload_ticks,
};

use std::collections::BTreeMap;

use tak_commands::{FireState, PlayerId, QueueMode, SimCommand, Target, UnitDefId, UnitId};
use tak_math::{Angle, Fx, Vec2};

use crate::WorldPos;
use crate::hash::StateHasher;
use crate::lifecycle;
use crate::script::ScriptHost;
use crate::world::{MAX_ORDERS, Order, World};

/// Kingdoms script ports used by combat (see `docs/combat.md`).
pub mod port {
    /// Script writes the weapon index when it acknowledges `TargetCleared`.
    pub const AIM_ABORTED: i32 = 21;
    /// Script writes the weapon index when `AimWeapon` has finished aiming.
    pub const WEAPON_READY: i32 = 22;
    /// Script writes the weapon index at the strike/release frame.
    pub const LAUNCH_NOW: i32 = 23;
    /// Written as 1 at the end of `Dying` (128 scripts).
    pub const DEATH_DONE: i32 = 26;
    /// Written as 1 by the monarch/god/lodestone `Dying` (38 scripts).
    pub const DEATH_DONE_ALT: i32 = 31;
    /// What the engine writes to ports 22/23 before asking again: no weapon
    /// index is negative, so the script's answer is unambiguous.
    pub const NONE: i32 = -1;
}

/// Ticks between repeated `AimWeapon` calls while a weapon is not yet aimed
/// (**Chosen**; each call signals the previous aim thread, as in TA).
pub const AIM_RETRY_TICKS: u32 = 15;
/// A fired weapon whose script never writes port 23 gives up after this many
/// ticks (the target was cleared mid-swing). **Chosen**.
pub const LAUNCH_TIMEOUT_TICKS: u32 = 90;
/// Death animations that never write port 26/31 end after this many ticks.
/// **Chosen**.
pub const DYING_TIMEOUT_TICKS: u32 = 150;
/// Re-issue a `go_to` toward a moving target when it has moved this far
/// from the last goal (world units). **Chosen**.
pub const REGOAL_DISTANCE: Fx = Fx::from_int(16);
/// A guard stays within this distance of the guarded unit. **Chosen**.
pub const GUARD_DISTANCE: Fx = Fx::from_int(96);
/// Projectiles older than this are dropped (30 s). **Chosen**.
pub const PROJECTILE_MAX_TICKS: u32 = 900;
/// Most projectiles in flight; further launches fizzle (TA's pool is 300).
pub const MAX_PROJECTILES: usize = 4096;
/// Corpses remembered for the feature system (oldest dropped first).
pub const MAX_CORPSES: usize = 1024;
/// Recent impacts kept for renderers (oldest dropped first).
pub const MAX_IMPACTS: usize = 256;
/// `HitByWeapon` direction vector length (**Chosen**).
pub const HIT_DIRECTION_SCALE: i32 = 500;

/// The active combat order of a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatOrder {
    /// Attack a unit (until it dies) or a ground point (until stopped).
    Attack(Target),
    /// Follow a friendly unit and fight enemies near it.
    Guard(UnitId),
    /// Fire spell weapon `slot` once at the target.
    Cast {
        /// Weapon slot (0-based, the script's weapon index).
        slot: u8,
        /// Where.
        target: Target,
    },
}

/// Aim progress of one weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Aim {
    /// Not aimed.
    #[default]
    Idle,
    /// `AimWeapon` called on this tick; waiting for port 22.
    Aiming(u32),
    /// The script reported the weapon aimed.
    Ready,
}

/// One weapon slot's runtime state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponState {
    /// Current target.
    pub target: Option<Target>,
    /// Ticks until the weapon may fire again.
    pub reload: u32,
    /// Aim progress.
    pub aim: Aim,
    /// `FireWeapon` was called on this tick; waiting for port 23.
    pub launch: Option<u32>,
}

/// A unit in its death animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dying {
    /// Tick the unit died.
    pub since: u32,
    /// `Killed` severity (1..=100).
    pub severity: i32,
    /// Damage kind of the killing blow ([`kind`]).
    pub kind: i32,
    /// The death animation has finished.
    pub done: bool,
}

/// Temporary steering for the durable front [`Order::AttackMove`].
/// No engagement is inserted into or removes anything from the order queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttackMoveState {
    /// Destination of the front order, used to reject stale steering.
    pub destination: WorldPos,
    /// Enemy being engaged, if any.
    pub target: Option<UnitId>,
    /// Where this engagement began, for the normal maneuver leash.
    pub origin: WorldPos,
    /// Current pathfinding goal, or `None` while fighting in range.
    pub goal: Option<WorldPos>,
    /// An unreachable or escaped enemy is ignored until it moves more than
    /// [`REGOAL_DISTANCE`], rather than re-requesting the same failed path.
    pub avoided: BTreeMap<UnitId, WorldPos>,
}

/// Per-unit combat state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitCombat {
    /// Health initialised from the definition.
    pub ready: bool,
    /// `SetMaxReloadTime` sent to the script.
    pub reload_sent: bool,
    /// Tick the unit was first seen.
    pub born: u32,
    /// Hit points.
    pub health: i32,
    /// `maxdamage` (0: outside combat).
    pub max_health: i32,
    /// Regeneration remainder (hit points × 30 × 65536).
    pub heal_acc: i64,
    /// Active combat order.
    pub order: Option<CombatOrder>,
    /// Engagement and steering for a front attack-move order.
    pub attack_move: Option<AttackMoveState>,
    /// Per weapon slot.
    pub weapons: Vec<WeaponState>,
    /// The last unit that damaged this one.
    pub last_attacker: Option<UnitId>,
    /// Set while the death animation plays.
    pub dying: Option<Dying>,
    /// The destination of the temporary front move combat last issued.
    pub goal: Option<WorldPos>,
    /// Where an idle chase started (the unit walks back when it ends).
    pub leash_origin: Option<WorldPos>,
    /// Units this one has killed.
    pub kills: u32,
}

impl UnitCombat {
    /// Health in percent for scripts (port 4): `health × 100 / maxdamage`,
    /// truncated (TA). Units outside combat read 100; dying units 0.
    pub fn health_percent(&self) -> i32 {
        if self.dying.is_some() {
            return 0;
        }
        if self.max_health <= 0 || !self.ready {
            return 100;
        }
        let p = i64::from(self.health.max(0)) * 100 / i64::from(self.max_health);
        p.clamp(0, 100) as i32
    }

    /// `true` once the unit has died (it is playing its death animation).
    pub fn is_dying(&self) -> bool {
        self.dying.is_some()
    }

    /// Drop the combat order and any chase (player `Move`/`Stop`).
    pub fn cancel_orders(&mut self) {
        self.order = None;
        self.attack_move = None;
        self.goal = None;
        self.leash_origin = None;
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"C");
        h.bool(self.ready);
        h.bool(self.reload_sent);
        h.u32(self.born);
        h.i32(self.health);
        h.i32(self.max_health);
        h.u64(self.heal_acc as u64);
        h.option(self.order, |h, o| match o {
            CombatOrder::Attack(t) => {
                h.u8(0);
                hash_target(h, t);
            }
            CombatOrder::Guard(u) => {
                h.u8(1);
                h.unit_id(u);
            }
            CombatOrder::Cast { slot, target } => {
                h.u8(2);
                h.u8(slot);
                hash_target(h, target);
            }
        });
        // Preserve the serialization of worlds that never use this new
        // command. Active attack-move state has its own explicit domain.
        if let Some(a) = &self.attack_move {
            h.bytes(b"ATTACKMOVE");
            h.vec2(a.destination);
            h.option(a.target, StateHasher::unit_id);
            h.vec2(a.origin);
            h.option(a.goal, StateHasher::vec2);
            h.len(a.avoided.len());
            for (&id, &pos) in &a.avoided {
                h.unit_id(id);
                h.vec2(pos);
            }
        }
        h.len(self.weapons.len());
        for w in &self.weapons {
            h.option(w.target, hash_target);
            h.u32(w.reload);
            match w.aim {
                Aim::Idle => h.u8(0),
                Aim::Aiming(t) => {
                    h.u8(1);
                    h.u32(t);
                }
                Aim::Ready => h.u8(2),
            }
            h.option(w.launch, StateHasher::u32);
        }
        h.option(self.last_attacker, StateHasher::unit_id);
        h.option(self.dying, |h, d| {
            h.u32(d.since);
            h.i32(d.severity);
            h.i32(d.kind);
            h.bool(d.done);
        });
        h.option(self.goal, StateHasher::vec2);
        h.option(self.leash_origin, StateHasher::vec2);
        h.u32(self.kills);
    }
}

fn hash_target(h: &mut StateHasher, t: Target) {
    match t {
        Target::Ground(p) => {
            h.u8(0);
            h.vec2(p);
        }
        Target::Unit(id) => {
            h.u8(1);
            h.unit_id(id);
        }
    }
}

/// Which weapon a projectile carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponRef {
    /// `weapons[slot]` of the definition.
    Slot(UnitDefId, u8),
    /// The definition's `explode_as`.
    Explosion(UnitDefId),
}

impl WeaponRef {
    fn hash(self, h: &mut StateHasher) {
        match self {
            WeaponRef::Slot(d, s) => {
                h.u8(0);
                h.unit_def(d);
                h.u8(s);
            }
            WeaponRef::Explosion(d) => {
                h.u8(1);
                h.unit_def(d);
            }
        }
    }
}

/// A shot in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Projectile {
    /// Who fired (excluded from its own blast).
    pub shooter: UnitId,
    /// Shooter's owner.
    pub owner: PlayerId,
    /// Weapon.
    pub weapon: WeaponRef,
    /// Current ground position.
    pub pos: WorldPos,
    /// Where it lands.
    pub dest: WorldPos,
    /// The unit aimed at (homed on by guided weapons).
    pub target: Option<UnitId>,
    /// World units per tick.
    pub speed: Fx,
    /// Homes on `target`.
    pub guided: bool,
    /// Ticks in flight.
    pub age: u32,
}

/// A unit that died and left (or did not leave) a corpse, for the feature
/// system to pick up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corpse {
    /// Tick of removal.
    pub tick: u32,
    /// Definition of the dead unit (its FBI `corpse` names the feature).
    pub def: UnitDefId,
    /// Where.
    pub pos: WorldPos,
    /// Facing.
    pub heading: Angle,
    /// `Killed`'s corpse answer: 0 none, 1 the normal corpse, higher values
    /// a more wrecked variant (TA corpse depth; Believed).
    pub kind: i32,
}

/// A weapon going off at a point (a projectile landing or an instant hit),
/// recorded for renderers to draw its explosion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Impact {
    /// Tick it happened.
    pub tick: u32,
    /// The weapon.
    pub weapon: WeaponRef,
    /// Where.
    pub point: WorldPos,
    /// Who fired, if anyone.
    pub shooter: Option<UnitId>,
}

/// World-level combat state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatState {
    /// Shots in flight, in creation order.
    pub projectiles: Vec<Projectile>,
    /// Units killed so far.
    pub deaths: u64,
    /// Recent corpses (capped at [`MAX_CORPSES`]).
    pub corpses: Vec<Corpse>,
    /// Recent impacts, oldest first (capped at [`MAX_IMPACTS`]). Output
    /// only, for renderers: the sim never reads it, so it is not hashed.
    pub impacts: Vec<Impact>,
}

impl CombatState {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"COMBAT");
        h.len(self.projectiles.len());
        for p in &self.projectiles {
            h.unit_id(p.shooter);
            h.player(p.owner);
            p.weapon.hash(h);
            h.vec2(p.pos);
            h.vec2(p.dest);
            h.option(p.target, StateHasher::unit_id);
            h.fx(p.speed);
            h.bool(p.guided);
            h.u32(p.age);
        }
        h.u64(self.deaths);
        h.len(self.corpses.len());
        for c in &self.corpses {
            h.u32(c.tick);
            h.unit_def(c.def);
            h.vec2(c.pos);
            h.angle(c.heading);
            h.i32(c.kind);
        }
    }
}

// ---------------------------------------------------------------------------
// Commands

/// Apply an `Attack`, `Guard` or `CastSpell` command. Units must be owned by
/// `player`, alive and part of combat. `Replace` drops the unit's queue and
/// starts the order now; `Append` starts it now if the unit is idle and
/// otherwise queues it behind the current orders (it starts when it reaches
/// the front).
pub fn apply_command(world: &mut World, player: PlayerId, cmd: &SimCommand) {
    let (units, order, queue) = match cmd {
        SimCommand::Attack {
            units,
            target,
            queue,
        } => (units, CombatOrder::Attack(*target), *queue),
        SimCommand::Guard {
            units,
            target,
            queue,
        } => (units, CombatOrder::Guard(*target), *queue),
        SimCommand::CastSpell {
            units,
            spell,
            target,
            queue,
        } => {
            let Ok(slot) = u8::try_from(spell.0) else {
                return;
            };
            (
                units,
                CombatOrder::Cast {
                    slot,
                    // Targetless spells land on the caster (resolved below).
                    target: target.unwrap_or(Target::Ground(WorldPos::ZERO)),
                },
                *queue,
            )
        }
        _ => return,
    };
    let targetless = matches!(cmd, SimCommand::CastSpell { target: None, .. });
    let map = world.map().clone();
    for &id in units.iter() {
        let Some(u) = world.unit_mut_owned(id, player) else {
            continue;
        };
        if u.combat.is_dying() {
            continue;
        }
        let mut order = order;
        if let CombatOrder::Attack(Target::Ground(p)) = &mut order {
            *p = map.clamp(*p);
        }
        if let CombatOrder::Cast { target, .. } = &mut order {
            if targetless {
                *target = Target::Unit(id);
            } else if let Target::Ground(p) = target {
                *p = map.clamp(*p);
            }
        }
        if matches!(order, CombatOrder::Guard(g) if g == id) {
            continue;
        }
        let idle = u.orders.is_empty() && u.combat.order.is_none();
        if queue == QueueMode::Replace || idle {
            u.cancel_player_orders();
            u.combat.order = Some(order);
        } else if u.orders.len() < MAX_ORDERS {
            u.orders.push_back(match order {
                CombatOrder::Attack(target) => Order::Attack { target },
                CombatOrder::Guard(target) => Order::Guard { target },
                // Spells are not queueable in the world's order list; start
                // the cast when the current orders are done.
                CombatOrder::Cast { .. } => {
                    u.combat.order = Some(order);
                    continue;
                }
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tick

/// Runs after movement, before the per-unit script tick.
pub fn tick(world: &mut World, scripts: &mut dyn ScriptHost) {
    let now = world.tick().0;
    let ids = world.unit_ids();
    for &id in &ids {
        init_and_heal(world, scripts, id, now);
    }
    for &id in &ids {
        if world.unit(id).is_some_and(|u| u.combat.is_dying()) {
            progress_death(world, scripts, id, now);
        }
    }
    for &id in &ids {
        // Units still under construction (WP-O) can be hit but never act.
        let active = world.unit(id).is_some_and(|u| {
            !world.is_inert_player(u.owner)
                && u.combat.ready
                && u.combat.max_health > 0
                && !u.combat.is_dying()
                && u.build.is_complete()
        });
        if active {
            unit_step(world, scripts, id, now);
        }
    }
    step_projectiles(world, scripts, now);
    // Corpses decompose (WP-P features ride on the combat lane).
    crate::features::tick(world);
}

fn init_and_heal(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId, now: u32) {
    let Some(u) = world.unit(id) else { return };
    let Some(def) = world.defs().get(u.def) else {
        return;
    };
    let max = def.combat.max_damage;
    let heal = def.combat.heal_rate;
    let nweapons = def.combat.weapons.len();
    let reload_ms = def.combat.max_reload_ms();
    let u = world.unit_mut(id).expect("live");
    let (done, total) = u.build.built_fraction();
    let complete = u.build.is_complete();
    let c = &mut u.combat;
    if !c.ready {
        // A frame first seen here (not through `build_progress`) starts
        // with the health its progress has earned.
        let health = frame_health(max, total - done.min(total), total);
        init_state(c, max, nweapons, now, max - health);
        return;
    }
    // Frames do not regenerate: their health comes from building.
    if c.max_health <= 0 || c.dying.is_some() || !complete {
        return;
    }
    if c.health < c.max_health && heal > Fx::ZERO {
        let per_hp = i64::from(Fx::ONE_RAW) * i64::from(crate::TICKS_PER_SECOND);
        c.heal_acc += i64::from(heal.raw());
        let gain = c.heal_acc / per_hp;
        c.heal_acc %= per_hp;
        c.health = (i64::from(c.health) + gain).min(i64::from(c.max_health)) as i32;
    } else {
        c.heal_acc = 0;
    }
    if c.health >= c.max_health {
        // Active-repair work cannot be saved through a full-health interval,
        // even when projectiles damage this unit later in this same tick.
        if let Some(r) = &mut u.build.repair {
            r.hp_remainder = 0;
        }
    }
    // TA sends the longest reload once the script exists; after `Create`
    // has had a tick to initialise its statics.
    if !c.reload_sent && now > c.born {
        c.reload_sent = true;
        if scripts.has_function(id, "SetMaxReloadTime") {
            lifecycle::call_script(world, scripts, id, "SetMaxReloadTime", &[reload_ms]);
        }
    }
}

fn init_state(c: &mut UnitCombat, max: i32, nweapons: usize, now: u32, health: i32) {
    c.ready = true;
    c.born = now;
    c.health = health;
    c.max_health = max;
    c.weapons = vec![WeaponState::default(); nweapons];
    c.reload_sent = nweapons == 0;
}

/// `trunc(maxdamage × remaining / total)`: the health a frame has *not*
/// yet earned when `remaining` of `total` work is left. TA-Known (Nanolathe
/// `05` "Health gain and fractional carry"): health gained by a build step
/// is `trunc(max × old) − trunc(max × new)` over the remaining fraction.
pub fn frame_health(max: i32, remaining: i64, total: i64) -> i32 {
    if total <= 0 || max <= 0 {
        return 0;
    }
    let r = i128::from(remaining.clamp(0, total));
    (i128::from(max) * r / i128::from(total)) as i32
}

/// Construction calls this after every build step on a frame (WP-P frame
/// health, TA-Known): a new frame starts at **zero** health (TA allocates
/// nanoframes with health 0) and each step adds
/// `trunc(max × old_remaining) − trunc(max × new_remaining)`, capped at
/// `maxdamage`. Damage to a frame is plain health loss; its build progress
/// is untouched (TA: the work step and damage write separate fields), so
/// a damaged frame completes below full health and a frame whose health
/// reaches zero dies like any unit.
pub fn build_progress(
    world: &mut World,
    id: UnitId,
    done_before: i64,
    done_after: i64,
    total: i64,
) {
    let now = world.tick().0;
    let Some(u) = world.unit(id) else { return };
    let Some(def) = world.defs().get(u.def) else {
        return;
    };
    let max = def.combat.max_damage;
    let nweapons = def.combat.weapons.len();
    let Some(u) = world.unit_mut(id) else { return };
    let c = &mut u.combat;
    if !c.ready {
        init_state(c, max, nweapons, now, 0);
    }
    if c.max_health <= 0 || c.dying.is_some() {
        return;
    }
    let old = frame_health(max, total - done_before, total);
    let new = frame_health(max, total - done_after, total);
    let gain = i64::from(old) - i64::from(new);
    c.health = (i64::from(c.health) + gain).clamp(0, i64::from(c.max_health)) as i32;
}

/// Initialize health without passive regeneration or script side effects.
/// Construction/repair can run before the first combat tick.
pub(crate) fn ensure_health(world: &mut World, id: UnitId) {
    let now = world.tick().0;
    let Some(u) = world.unit(id) else { return };
    if u.combat.ready {
        return;
    }
    let Some(d) = world.defs().get(u.def) else {
        return;
    };
    let (max, weapons) = (d.combat.max_damage, d.combat.weapons.len());
    let (done, total) = u.build.built_fraction();
    let health = max - frame_health(max, total - done.min(total), total);
    if let Some(u) = world.unit_mut(id) {
        init_state(&mut u.combat, max, weapons, now, health);
    }
}

/// Positive healing only. Never resurrects, invokes combat callbacks, changes
/// passive-regeneration credit, last attacker, kills or construction progress.
/// Returns the actual HP restored, capped at the target's maximum.
pub fn apply_healing(world: &mut World, target: UnitId, amount: i32) -> i32 {
    if amount <= 0 {
        return 0;
    }
    ensure_health(world, target);
    let Some(u) = world.unit_mut(target) else {
        return 0;
    };
    if u.combat.is_dying()
        || !u.build.is_complete()
        || u.combat.max_health <= 0
        || u.combat.health <= 0
    {
        return 0;
    }
    let before = u.combat.health;
    u.combat.health = before.saturating_add(amount).min(u.combat.max_health);
    if u.combat.health >= u.combat.max_health {
        if let Some(r) = &mut u.build.repair {
            r.hp_remainder = 0;
        }
    }
    u.combat.health - before
}

/// Copy of the per-weapon numbers the tick needs (no allocation).
#[derive(Clone, Copy, Debug)]
struct Core {
    kind: WeaponKind,
    range: i32,
    min_range: i32,
    reload: u32,
    tolerance: u16,
    spell: bool,
    mana: i32,
    no_air_weapon: bool,
}

fn cores(world: &World, def: UnitDefId) -> Vec<Core> {
    world.defs().get(def).map_or_else(Vec::new, |d| {
        d.combat
            .weapons
            .iter()
            .map(|w| Core {
                kind: w.kind,
                range: w.range,
                min_range: w.min_range,
                reload: w.reload_ticks,
                tolerance: w.aim_tolerance.unwrap_or(DEFAULT_AIM_TOLERANCE),
                spell: w.is_spell(),
                mana: w.mana_per_shot,
                no_air_weapon: w.no_air_weapon,
            })
            .collect()
    })
}

fn radius_of(world: &World, id: UnitId) -> i32 {
    world
        .unit(id)
        .and_then(|u| world.defs().get(u.def))
        .map_or(0, |d| d.combat.radius)
}

/// A unit that can be targeted and damaged.
pub fn is_target(world: &World, id: UnitId) -> bool {
    world
        .unit(id)
        .is_some_and(|u| u.combat.ready && u.combat.max_health > 0 && !u.combat.is_dying())
}

fn target_pos(world: &World, t: Target) -> Option<WorldPos> {
    match t {
        Target::Ground(p) => Some(p),
        Target::Unit(id) => is_target(world, id).then(|| world.unit(id).map(|u| u.pos))?,
    }
}

/// Distance (world units, rounded down) within which weapon `w` of unit
/// `shooter` can hit `t`. Melee: body contact; others: authored range.
fn reach(world: &World, shooter: UnitId, core: &Core, t: Target) -> i32 {
    match core.kind {
        WeaponKind::Melee => {
            let tr = match t {
                Target::Unit(id) => radius_of(world, id),
                Target::Ground(_) => 0,
            };
            melee_reach(radius_of(world, shooter), tr)
        }
        _ => core.range,
    }
}

/// Weapon target compatibility is independent of range: incompatible targets
/// must not start a chase, even when outside the weapon's reach.
fn can_target(world: &World, no_air_weapon: bool, t: Target) -> bool {
    match t {
        Target::Ground(_) => true,
        Target::Unit(id) => world.unit(id).is_some_and(|u| {
            world
                .defs()
                .get(u.def)
                .is_some_and(|d| !no_air_weapon || !d.motion.canfly)
        }),
    }
}

fn in_reach(world: &World, shooter: UnitId, pos: WorldPos, core: &Core, t: Target) -> bool {
    if !can_target(world, core.no_air_weapon, t) {
        return false;
    }
    let Some(tp) = target_pos(world, t) else {
        return false;
    };
    let d = pos.distance(tp).to_int_floor();
    d <= reach(world, shooter, core, t) && (core.kind == WeaponKind::Melee || d >= core.min_range)
}

pub(crate) fn is_our_move(u: &crate::world::Unit) -> bool {
    match u.combat.goal {
        Some(g) => u.orders.front() == Some(&Order::Move { to: g }),
        None => false,
    }
}

fn move_toward(world: &mut World, id: UnitId, to: WorldPos) {
    let to = world.map().clamp(to);
    let Some(u) = world.unit(id) else { return };
    if world
        .defs()
        .get(u.def)
        .is_none_or(|d| d.maxvelocity <= Fx::ZERO)
    {
        return;
    }
    let fresh = is_our_move(u)
        && u.combat
            .goal
            .is_some_and(|g| g.distance(to) <= REGOAL_DISTANCE);
    if fresh {
        return;
    }
    if let Some(u) = world.unit_mut(id) {
        if is_our_move(u) {
            u.orders.pop_front();
        }
        // Temporary steering must not destroy the player's queued orders.
        u.orders.push_front(Order::Move { to });
        u.combat.goal = Some(to);
    }
}

fn halt_our_move(world: &mut World, id: UnitId) {
    if let Some(u) = world.unit_mut(id) {
        if is_our_move(u) {
            u.orders.pop_front();
        }
        u.combat.goal = None;
    }
}

/// Nearest enemy (combat-capable, not dying) satisfying `ok`, ties to the
/// lower slot (units are scanned in slot order with a strict `<`).
fn nearest_enemy(
    world: &World,
    owner: PlayerId,
    pos: WorldPos,
    mut ok: impl FnMut(UnitId, WorldPos, i32) -> bool,
) -> Option<UnitId> {
    let mut best: Option<(Fx, UnitId)> = None;
    for (oid, o) in world.units() {
        if !world.are_hostile(owner, o.owner)
            || !o.combat.ready
            || o.combat.max_health <= 0
            || o.combat.is_dying()
        {
            continue;
        }
        let d = pos.distance(o.pos);
        if !ok(oid, o.pos, d.to_int_floor()) {
            continue;
        }
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, oid));
        }
    }
    best.map(|(_, id)| id)
}

fn unit_step(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId, now: u32) {
    // Promote a queued attack/guard that reached the front of the queue.
    if let Some(u) = world.unit_mut(id) {
        match u
            .orders
            .front()
            .copied()
            .filter(|_| u.combat.order.is_none())
        {
            Some(Order::Attack { target }) => {
                u.orders.pop_front();
                u.combat.cancel_orders();
                u.combat.order = Some(CombatOrder::Attack(target));
            }
            Some(Order::Guard { target }) => {
                u.orders.pop_front();
                u.combat.cancel_orders();
                u.combat.order = Some(CombatOrder::Guard(target));
            }
            _ => {}
        }
        // A player order took over the unit: combat stops steering it.
        if u.combat.order.is_none() && !u.orders.is_empty() && !is_our_move(u) {
            u.combat.goal = None;
            u.combat.leash_origin = None;
        }
    }
    let Some(u) = world.unit(id) else { return };
    let (pos, owner, heading, fire_state, def_id) =
        (u.pos, u.owner, u.heading, u.fire_state, u.def);
    // Friendly fire still records its attacker for damage bookkeeping, but
    // only a hostile player can qualify for automatic ReturnFire targeting.
    let last_attacker = u.combat.last_attacker.filter(|&a| {
        is_target(world, a)
            && world
                .unit(a)
                .is_some_and(|a| world.are_hostile(owner, a.owner))
    });
    let mut order = u.combat.order;
    let Some(def) = world.defs().get(def_id) else {
        return;
    };
    let (turnrate, sight, leash, can_move) = (
        def.turnrate,
        def.combat.sight,
        def.combat.leash,
        def.maxvelocity > Fx::ZERO,
    );
    let cores = cores(world, def_id);

    // Validate the order.
    order = match order {
        Some(CombatOrder::Attack(t))
            if target_pos(world, t).is_none()
                || !cores
                    .iter()
                    .any(|c| !c.spell && can_target(world, c.no_air_weapon, t)) =>
        {
            None
        }
        Some(CombatOrder::Cast { slot, target })
            if target_pos(world, target).is_none()
                || !cores
                    .get(slot as usize)
                    .is_some_and(|c| can_target(world, c.no_air_weapon, target)) =>
        {
            None
        }
        Some(CombatOrder::Cast { slot, .. })
            if !cores.get(slot as usize).is_some_and(|c| c.spell) =>
        {
            None
        }
        Some(CombatOrder::Guard(g)) if world.unit(g).is_none_or(|g| g.combat.is_dying()) => None,
        o => o,
    };
    if order != world.unit(id).and_then(|u| u.combat.order) {
        if let Some(u) = world.unit_mut(id) {
            u.combat.order = order;
        }
        if order.is_none() {
            halt_our_move(world, id);
        }
    }
    let player_busy = world
        .unit(id)
        .is_some_and(|u| u.combat.order.is_none() && !u.orders.is_empty() && !is_our_move(u));
    let attack_move = match world.unit(id).and_then(|u| u.orders.front()).copied() {
        Some(Order::AttackMove { to }) if order.is_none() => Some(to),
        _ => None,
    };
    let engagement = if let Some(to) = attack_move {
        attack_move_step(
            world,
            id,
            to,
            sight,
            leash,
            &cores,
            fire_state,
            last_attacker,
        )
    } else {
        if let Some(u) = world.unit_mut(id) {
            u.combat.attack_move = None;
        }
        None
    };
    let forced: Option<(Target, Option<u8>)> = match order {
        Some(CombatOrder::Attack(t)) => Some((t, None)),
        Some(CombatOrder::Cast { slot, target }) => Some((target, Some(slot))),
        _ => engagement.map(|t| (Target::Unit(t), None)),
    };

    // 1. Pick a target per weapon.
    let autonomous = fire_state != FireState::HoldFire;
    let mut new_targets: Vec<Option<Target>> = Vec::with_capacity(cores.len());
    for (w, core) in cores.iter().enumerate() {
        let current = world
            .unit(id)
            .and_then(|u| u.combat.weapons.get(w))
            .and_then(|s| s.target);
        let t = match forced {
            Some((t, Some(slot))) => (slot as usize == w).then_some(t),
            Some((t, None)) if !core.spell => Some(t),
            _ if core.spell || !autonomous => None,
            _ => {
                let allowed =
                    |oid: UnitId| fire_state != FireState::ReturnFire || Some(oid) == last_attacker;
                let keep = current.filter(|&t| match t {
                    Target::Unit(oid) => {
                        allowed(oid)
                            && world
                                .unit(oid)
                                .is_some_and(|o| world.are_hostile(owner, o.owner))
                            && in_reach(world, id, pos, core, t)
                    }
                    Target::Ground(_) => false,
                });
                keep.or_else(|| {
                    nearest_enemy(world, owner, pos, |oid, _, _| {
                        allowed(oid) && in_reach(world, id, pos, core, Target::Unit(oid))
                    })
                    .map(Target::Unit)
                })
            }
        };
        new_targets.push(t.filter(|&t| can_target(world, core.no_air_weapon, t)));
    }

    // 2. Movement: close on the order target, guard, or chase.
    let can_hit_any = new_targets
        .iter()
        .zip(&cores)
        .any(|(t, c)| t.is_some_and(|t| in_reach(world, id, pos, c, t)));
    if can_move && !player_busy && attack_move.is_none() {
        match order {
            Some(CombatOrder::Attack(t)) | Some(CombatOrder::Cast { target: t, .. }) => {
                let slot = match order {
                    Some(CombatOrder::Cast { slot, .. }) => Some(slot as usize),
                    _ => None,
                };
                let hit = cores.iter().enumerate().any(|(w, c)| {
                    slot.map_or(!c.spell, |s| s == w) && in_reach(world, id, pos, c, t)
                });
                if hit {
                    halt_our_move(world, id);
                } else if let Some(tp) = target_pos(world, t) {
                    move_toward(world, id, tp);
                }
            }
            Some(CombatOrder::Guard(g)) => {
                let gpos = world.unit(g).map_or(pos, |g| g.pos);
                // A builder assisting through Guard (WP-P) stays at its
                // frame instead of following the guarded unit.
                let assisting = world.unit(id).is_some_and(|u| u.build.assist.is_some());
                let chase = chase_target(
                    world,
                    id,
                    owner,
                    pos,
                    gpos,
                    leash.max(GUARD_DISTANCE.to_int_floor()),
                    sight,
                    &cores,
                    fire_state,
                    last_attacker,
                );
                if !assisting {
                    step_chase(world, id, pos, chase, can_hit_any, || {
                        (pos.distance(gpos) > GUARD_DISTANCE).then_some(gpos)
                    });
                }
            }
            None if fire_state == FireState::Defensive => {
                // Never resume an old autonomous chase/return after switching
                // stance. Do not touch movement explicitly ordered by the player.
                halt_our_move(world, id);
                if let Some(u) = world.unit_mut(id) {
                    u.combat.leash_origin = None;
                }
            }
            None => {
                let origin = world
                    .unit(id)
                    .and_then(|u| u.combat.leash_origin)
                    .unwrap_or(pos);
                let chase = chase_target(
                    world,
                    id,
                    owner,
                    pos,
                    origin,
                    leash,
                    sight,
                    &cores,
                    fire_state,
                    last_attacker,
                );
                if chase.is_some() && !can_hit_any {
                    if let Some(u) = world.unit_mut(id) {
                        u.combat.leash_origin.get_or_insert(pos);
                    }
                }
                let back = world.unit(id).and_then(|u| u.combat.leash_origin);
                step_chase(world, id, pos, chase, can_hit_any, || {
                    let o = back?;
                    (pos.distance(o) > crate::systems::ARRIVAL_TOLERANCE).then_some(o)
                });
                if chase.is_none() {
                    if let Some(u) = world.unit_mut(id) {
                        // Back where the chase started: it is over.
                        if back.is_none_or(|o| pos.distance(o) <= crate::systems::ARRIVAL_TOLERANCE)
                        {
                            u.combat.leash_origin = None;
                        }
                    }
                }
            }
        }
    }

    // 3. Face the first target while standing.
    let standing = world.unit(id).is_some_and(|u| {
        u.orders.is_empty()
            || ((attack_move.is_some() || u.combat.order.is_some())
                && crate::pathfinding::front_target(u).is_none())
    });
    let face = new_targets.iter().flatten().next().copied();
    if standing && turnrate > 0 {
        if let Some(tp) = face.and_then(|t| target_pos(world, t)) {
            if tp != pos {
                let err = i32::from(heading.delta_to((tp - pos).angle()));
                let tr = i32::from(turnrate);
                let turn = err.clamp(-tr, tr);
                if let Some(u) = world.unit_mut(id) {
                    u.heading = Angle::from_raw(u.heading.raw().wrapping_add(turn as i16 as u16));
                }
            }
        }
    }

    // 4. Weapons.
    for (w, core) in cores.iter().enumerate() {
        weapon_step(
            world,
            scripts,
            id,
            w,
            core,
            new_targets[w],
            turnrate > 0,
            now,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn attack_move_step(
    world: &mut World,
    id: UnitId,
    to: WorldPos,
    sight: i32,
    leash: i32,
    cores: &[Core],
    fire_state: FireState,
    last_attacker: Option<UnitId>,
) -> Option<UnitId> {
    let u = world.unit(id)?;
    let (pos, owner) = (u.pos, u.owner);
    let mut state = world
        .unit_mut(id)?
        .combat
        .attack_move
        .take()
        .filter(|a| a.destination == to)
        .unwrap_or(AttackMoveState {
            destination: to,
            target: None,
            origin: pos,
            goal: Some(to),
            avoided: BTreeMap::new(),
        });
    let auto: Vec<&Core> = cores.iter().filter(|c| !c.spell).collect();
    let look = |target| {
        auto.iter()
            .filter(|c| c.kind != WeaponKind::Melee && can_target(world, c.no_air_weapon, target))
            .map(|c| c.range)
            .max()
            .unwrap_or(0)
            .max(sight)
    };
    let allowed = |oid: UnitId| {
        auto.iter().any(|c| can_target(world, c.no_air_weapon, Target::Unit(oid)))
            && world
                .unit(oid)
                .is_some_and(|u| world.are_hostile(owner, u.owner))
            && fire_state != FireState::HoldFire
            && (fire_state != FireState::ReturnFire || Some(oid) == last_attacker)
            // Defensive and legacy ReturnFire never divert from the route
            // to chase. Explicit Attack/Cast still override either state.
            && (fire_state == FireState::FireAtWill
                || auto.iter().any(|c| in_reach(world, id, pos, c, Target::Unit(oid))))
    };
    // Forget a failed location once its unit moves, dies, or its slot is reused.
    state.avoided.retain(|&oid, at| {
        is_target(world, oid)
            && world.unit(oid).is_some_and(|u| {
                world.are_hostile(owner, u.owner) && u.pos.distance(*at) <= REGOAL_DISTANCE
            })
    });
    if let Some(t) = state.target {
        let keep = allowed(t)
            && is_target(world, t)
            && (fire_state == FireState::Defensive
                || world.unit(t).is_some_and(|u| {
                    state.origin.distance(u.pos).to_int_floor() <= leash.max(look(Target::Unit(t)))
                }));
        if !keep {
            if is_target(world, t) && allowed(t) {
                if let Some(u) = world.unit(t) {
                    state.avoided.insert(t, u.pos);
                }
            }
            state.target = None;
        }
    }
    if state.target.is_none() {
        state.target = nearest_enemy(world, owner, pos, |oid, _, d| {
            allowed(oid)
                && (fire_state == FireState::Defensive
                    || (d <= look(Target::Unit(oid)) && !state.avoided.contains_key(&oid)))
        });
        if state.target.is_some() {
            state.origin = pos;
        }
    }
    state.goal = match state.target {
        Some(t)
            if auto
                .iter()
                .any(|c| in_reach(world, id, pos, c, Target::Unit(t))) =>
        {
            None
        }
        Some(t) => world.unit(t).map(|u| {
            // Retain the current path until the target meaningfully moves.
            state
                .goal
                .filter(|&g| g.distance(u.pos) <= REGOAL_DISTANCE)
                .unwrap_or(u.pos)
        }),
        None => Some(to),
    };
    let target = state.target;
    if let Some(u) = world.unit_mut(id) {
        u.combat.attack_move = Some(state);
    }
    target
}

#[allow(clippy::too_many_arguments)]
fn chase_target(
    world: &World,
    id: UnitId,
    owner: PlayerId,
    pos: WorldPos,
    origin: WorldPos,
    leash: i32,
    sight: i32,
    cores: &[Core],
    fire_state: FireState,
    last_attacker: Option<UnitId>,
) -> Option<UnitId> {
    let _ = id;
    let auto: Vec<&Core> = cores.iter().filter(|c| !c.spell).collect();
    if auto.is_empty() || matches!(fire_state, FireState::HoldFire | FireState::Defensive) {
        return None;
    }
    // Melee `range` is not a reach (see `melee_reach`); only ranged weapons
    // see past `sightdistance`.
    let look = |target| {
        auto.iter()
            .filter(|c| c.kind != WeaponKind::Melee && can_target(world, c.no_air_weapon, target))
            .map(|c| c.range)
            .max()
            .unwrap_or(0)
            .max(sight)
    };
    nearest_enemy(world, owner, pos, |oid, opos, d| {
        auto.iter()
            .any(|c| can_target(world, c.no_air_weapon, Target::Unit(oid)))
            && (fire_state != FireState::ReturnFire || Some(oid) == last_attacker)
            && d <= look(Target::Unit(oid))
            && origin.distance(opos).to_int_floor() <= leash
    })
}

fn step_chase(
    world: &mut World,
    id: UnitId,
    pos: WorldPos,
    chase: Option<UnitId>,
    can_hit_any: bool,
    otherwise: impl FnOnce() -> Option<WorldPos>,
) {
    let _ = pos;
    if can_hit_any {
        halt_our_move(world, id);
        return;
    }
    if let Some(tp) = chase.and_then(|c| world.unit(c)).map(|c| c.pos) {
        move_toward(world, id, tp);
        return;
    }
    match otherwise() {
        Some(p) => move_toward(world, id, p),
        None => halt_our_move(world, id),
    }
}

#[allow(clippy::too_many_arguments)]
fn weapon_step(
    world: &mut World,
    scripts: &mut dyn ScriptHost,
    id: UnitId,
    w: usize,
    core: &Core,
    target: Option<Target>,
    must_face: bool,
    now: u32,
) {
    let windex = w as i32;
    let Some(state) = world
        .unit(id)
        .and_then(|u| u.combat.weapons.get(w))
        .copied()
    else {
        return;
    };
    let mut st = state;
    st.reload = st.reload.saturating_sub(1);

    // Target change: tell the script to drop the old attack.
    if st.target.is_some() && st.target != target {
        if (st.aim != Aim::Idle || st.launch.is_some()) && scripts.has_function(id, "TargetCleared")
        {
            lifecycle::call_script(world, scripts, id, "TargetCleared", &[windex]);
        }
        st.aim = Aim::Idle;
    }
    st.target = target;
    let scripted = scripts.port(id, port::WEAPON_READY).is_some();

    // A pending launch: the script writes port 23 at the strike frame.
    if let Some(since) = st.launch {
        if scripts.port(id, port::LAUNCH_NOW) == Some(windex) {
            scripts.set_port(id, port::LAUNCH_NOW, port::NONE);
            st.launch = None;
            store(world, id, w, st);
            if let Some(t) = target {
                launch(world, scripts, id, w, t, now);
            }
            return;
        }
        if now.saturating_sub(since) >= LAUNCH_TIMEOUT_TICKS {
            st.launch = None;
        } else {
            store(world, id, w, st);
            return;
        }
    }
    let Some(t) = target else {
        store(world, id, w, st);
        return;
    };
    let Some((pos, heading, ground)) = world.unit(id).map(|u| (u.pos, u.heading, u.ground)) else {
        return;
    };
    let Some(tp) = target_pos(world, t) else {
        store(world, id, w, st);
        return;
    };

    // Aim.
    if st.aim != Aim::Ready {
        if let Aim::Aiming(_) = st.aim {
            if scripts.port(id, port::WEAPON_READY) == Some(windex) {
                st.aim = Aim::Ready;
            }
        }
        let retry = match st.aim {
            Aim::Idle => true,
            Aim::Aiming(since) => now.saturating_sub(since) >= AIM_RETRY_TICKS,
            Aim::Ready => false,
        };
        if retry {
            if scripted && scripts.has_function(id, "AimWeapon") {
                let rel = i32::from(heading.delta_to((tp - pos).angle()));
                let th = match t {
                    Target::Unit(o) => world.unit(o).map_or(ground, |o| o.ground),
                    Target::Ground(p) => world.map().height_at(p),
                };
                let flat = pos.distance(tp).max(Fx::ONE);
                let pitch = i32::from(Angle::atan2(th - ground, flat).raw() as i16);
                scripts.set_port(id, port::WEAPON_READY, port::NONE);
                store(world, id, w, st);
                lifecycle::call_script(world, scripts, id, "AimWeapon", &[rel, pitch, windex]);
                st = world
                    .unit(id)
                    .and_then(|u| u.combat.weapons.get(w))
                    .copied()
                    .unwrap_or(st);
                st.aim = if scripts.port(id, port::WEAPON_READY) == Some(windex) {
                    Aim::Ready
                } else {
                    Aim::Aiming(now)
                };
            } else {
                st.aim = Aim::Ready;
            }
        }
    }

    // Fire.
    let facing_ok = !must_face
        || tp == pos
        || heading.delta_to((tp - pos).angle()).unsigned_abs() <= core.tolerance;
    let ready =
        st.aim == Aim::Ready && st.reload == 0 && facing_ok && in_reach(world, id, pos, core, t);
    if !ready {
        store(world, id, w, st);
        return;
    }
    if core.mana > 0 && !spend_mana(world, id, core.mana) {
        store(world, id, w, st);
        return;
    }
    st.reload = core.reload;
    st.aim = Aim::Idle;
    if scripted && scripts.has_function(id, "FireWeapon") {
        scripts.set_port(id, port::LAUNCH_NOW, port::NONE);
        st.launch = Some(now);
        store(world, id, w, st);
        lifecycle::call_script(world, scripts, id, "FireWeapon", &[windex]);
        if scripts.port(id, port::LAUNCH_NOW) == Some(windex) {
            scripts.set_port(id, port::LAUNCH_NOW, port::NONE);
            if let Some(u) = world.unit_mut(id) {
                if let Some(s) = u.combat.weapons.get_mut(w) {
                    s.launch = None;
                }
            }
            launch(world, scripts, id, w, t, now);
        }
    } else {
        store(world, id, w, st);
        launch(world, scripts, id, w, t, now);
    }
}

/// Pay `manapershot` from the caster's own mana pool, all or nothing; fire is
/// held when it returns `false`.
///
/// Pay `mana` from the caster's own pool (WP-N: all or nothing, recharging
/// at `manarechargerate`). `false` means hold fire.
fn spend_mana(world: &mut World, caster: UnitId, mana: i32) -> bool {
    if mana <= 0 {
        return true;
    }
    let cost = Fx::from_int(mana.min(i32::from(i16::MAX)));
    crate::economy::spend_unit_mana(world, caster, cost)
}

fn store(world: &mut World, id: UnitId, w: usize, st: WeaponState) {
    if let Some(s) = world.unit_mut(id).and_then(|u| u.combat.weapons.get_mut(w)) {
        *s = st;
    }
}

/// The shot leaves the weapon (port 23, or at once without a script).
fn launch(
    world: &mut World,
    scripts: &mut dyn ScriptHost,
    id: UnitId,
    w: usize,
    t: Target,
    now: u32,
) {
    let Some(u) = world.unit(id) else { return };
    if u.combat.is_dying() {
        return;
    }
    let (pos, owner, def_id) = (u.pos, u.owner, u.def);
    // A one-shot cast is done once it launches.
    if matches!(u.combat.order, Some(CombatOrder::Cast { slot, .. }) if slot as usize == w) {
        if let Some(u) = world.unit_mut(id) {
            u.combat.order = None;
        }
    }
    let Some(ws) = world
        .defs()
        .get(def_id)
        .and_then(|d| d.combat.weapons.get(w))
    else {
        return;
    };
    if !can_target(world, ws.no_air_weapon, t) {
        return;
    }
    let (kind, speed) = (ws.kind, ws.speed_per_tick());
    let Some(tp) = target_pos(world, t) else {
        return;
    };
    let target_unit = match t {
        Target::Unit(o) => Some(o),
        Target::Ground(_) => None,
    };
    let weapon = WeaponRef::Slot(def_id, w as u8);
    match kind {
        WeaponKind::Melee => {
            let core = cores(world, def_id)[w];
            let slack = reach(world, id, &core, t).saturating_add(MELEE_MARGIN);
            if pos.distance(tp).to_int_floor() <= slack {
                impact(world, scripts, Some(id), weapon, tp, target_unit, now);
            }
        }
        WeaponKind::RemoteEffect => impact(world, scripts, Some(id), weapon, tp, target_unit, now),
        _ if speed <= Fx::ZERO || pos.distance(tp) <= speed => {
            impact(world, scripts, Some(id), weapon, tp, target_unit, now)
        }
        _ => {
            if world.combat.projectiles.len() < MAX_PROJECTILES {
                world.combat.projectiles.push(Projectile {
                    shooter: id,
                    owner,
                    weapon,
                    pos,
                    dest: tp,
                    target: target_unit,
                    speed,
                    guided: kind == WeaponKind::Guided,
                    age: 0,
                });
            }
        }
    }
}

fn step_projectiles(world: &mut World, scripts: &mut dyn ScriptHost, now: u32) {
    let mut arrived = Vec::new();
    let mut keep = Vec::with_capacity(world.combat.projectiles.len());
    let projectiles = std::mem::take(&mut world.combat.projectiles);
    for mut p in projectiles {
        if p.guided {
            if let Some(t) = p.target.filter(|&t| is_target(world, t)) {
                p.dest = world.unit(t).map_or(p.dest, |u| u.pos);
            }
        }
        p.age += 1;
        let d = p.pos.distance(p.dest);
        if d <= p.speed {
            p.pos = p.dest;
            arrived.push(p);
        } else if p.age < PROJECTILE_MAX_TICKS {
            if let Some(dir) = (p.dest - p.pos).normalize() {
                p.pos = world.map().clamp(p.pos + dir.scale(p.speed));
            }
            keep.push(p);
        }
    }
    world.combat.projectiles = keep;
    for p in arrived {
        // Direct hit only if the aimed unit is still where the shot lands
        // (resolved inside `impact`).
        impact(
            world,
            scripts,
            Some(p.shooter),
            p.weapon,
            p.dest,
            p.target,
            now,
        );
    }
}

/// A shot lands at `point`. TA routing (nanolathe retail spec §9.1): with
/// `areaofeffect <= 16` and a unit under the shot, only that unit takes full
/// damage; otherwise every unit whose body is within the blast radius takes
/// [`area_damage`]. The shooter is never hurt by its own shot; allies are.
fn impact(
    world: &mut World,
    scripts: &mut dyn ScriptHost,
    shooter: Option<UnitId>,
    weapon: WeaponRef,
    point: WorldPos,
    aimed: Option<UnitId>,
    now: u32,
) {
    if world.combat.impacts.len() >= MAX_IMPACTS {
        world.combat.impacts.remove(0);
    }
    world.combat.impacts.push(Impact {
        tick: now,
        weapon,
        point,
        shooter,
    });
    let (def_id, ws) = match weapon {
        WeaponRef::Slot(d, s) => (
            d,
            world
                .defs()
                .get(d)
                .and_then(|x| x.combat.weapons.get(s as usize)),
        ),
        WeaponRef::Explosion(d) => (
            d,
            world
                .defs()
                .get(d)
                .and_then(|x| x.combat.explode_as.as_ref()),
        ),
    };
    let _ = def_id;
    let Some(ws) = ws else { return };
    let (aoe, r, edge, dkind) = (
        ws.area_of_effect,
        ws.blast_radius(),
        ws.edge_effectiveness,
        ws.damage_kind,
    );
    let damage = ws.damage.clone();
    let units_only = ws.units_only;
    let body = |world: &World, id: UnitId| -> Option<Fx> {
        let u = world.unit(id)?;
        Some(u.pos.distance(point) - Fx::from_int(radius_of(world, id)))
    };
    // The unit under the shot: the aimed unit if the point is inside its
    // body, else the first (slot order) unit whose body covers the point.
    let direct = aimed
        .filter(|&a| {
            is_target(world, a)
                && Some(a) != shooter
                && body(world, a).is_some_and(|d| d <= Fx::ZERO)
        })
        .or_else(|| {
            world.units().map(|(oid, _)| oid).find(|&oid| {
                Some(oid) != shooter
                    && is_target(world, oid)
                    && body(world, oid).is_some_and(|d| d <= Fx::ZERO)
            })
        });
    let category = |world: &World, id: UnitId| -> Option<String> {
        world
            .unit(id)
            .and_then(|u| world.defs().get(u.def))
            .and_then(|d| d.combat.damage_category.clone())
    };
    let mut hits: Vec<(UnitId, i32)> = Vec::new();
    if aoe <= 16 {
        if let Some(v) = direct {
            hits.push((v, damage.against(category(world, v).as_deref())));
        }
    } else {
        for (oid, _) in world.units() {
            if Some(oid) == shooter || !is_target(world, oid) {
                continue;
            }
            let d = body(world, oid)
                .unwrap_or(Fx::MAX)
                .max(Fx::ZERO)
                .to_int_floor();
            if d < r {
                let base = damage.against(category(world, oid).as_deref());
                hits.push((oid, area_damage(base, d, r, edge)));
            }
        }
    }
    let feature_hits = if units_only {
        Vec::new()
    } else {
        crate::features::impact_hits(world, point, aoe, damage.default, edge, direct.is_some())
    };
    for (victim, amount) in hits {
        apply_damage(world, scripts, victim, amount, shooter, dkind, point, now);
    }
    for (feature, amount) in feature_hits {
        crate::features::apply_damage(world, feature, amount);
    }
}

/// Subtract health, notify the script (`HitByWeapon`), start the death when
/// health reaches zero.
#[allow(clippy::too_many_arguments)]
pub fn apply_damage(
    world: &mut World,
    scripts: &mut dyn ScriptHost,
    victim: UnitId,
    amount: i32,
    attacker: Option<UnitId>,
    dkind: i32,
    from: WorldPos,
    now: u32,
) {
    if !is_target(world, victim) || amount <= 0 {
        return;
    }
    let Some(u) = world.unit_mut(victim) else {
        return;
    };
    let before = u.combat.health_percent();
    u.combat.health = u.combat.health.saturating_sub(amount);
    if let Some(a) = attacker.filter(|&a| a != victim) {
        u.combat.last_attacker = Some(a);
    }
    let (health, max, pos, heading) = (u.combat.health, u.combat.max_health, u.pos, u.heading);
    if scripts.has_function(victim, "HitByWeapon") {
        // Direction of the hit in the unit's frame: (forward, right) × 500.
        let rel = if from == pos {
            Angle::ZERO
        } else {
            Angle::from_raw(heading.delta_to((from - pos).angle()) as u16)
        };
        let v = Vec2::from_angle(rel).scale(Fx::from_int(HIT_DIRECTION_SCALE));
        let args = [dkind, v.x.to_int_round(), v.y.to_int_round(), amount];
        lifecycle::call_script(world, scripts, victim, "HitByWeapon", &args);
    }
    if health <= 0 {
        // TA: (overkill % of max + percent before the hit) / 2, in 1..=100.
        let over = i64::from(health).unsigned_abs() * 100 / u64::from(max.max(1) as u32);
        let severity = ((over as i64 + i64::from(before)) / 2).clamp(1, 100) as i32;
        if let Some(a) = attacker.filter(|&a| a != victim) {
            if let Some(au) = world.unit_mut(a) {
                au.combat.kills = au.combat.kills.saturating_add(1);
            }
        }
        start_death(world, scripts, victim, severity, dkind, now);
    }
}

fn start_death(
    world: &mut World,
    scripts: &mut dyn ScriptHost,
    id: UnitId,
    severity: i32,
    dkind: i32,
    now: u32,
) {
    crate::construction::cancel_heal(world, scripts, id);
    crate::construction::cancel_repeat_on_death(world, scripts, id);
    let Some(u) = world.unit_mut(id) else { return };
    u.orders.clear();
    u.combat.cancel_orders();
    for w in &mut u.combat.weapons {
        *w = WeaponState::default();
    }
    u.combat.dying = Some(Dying {
        since: now,
        severity,
        kind: dkind,
        done: false,
    });
    world.combat.deaths += 1;
    let mut done = true;
    if scripts.has_function(id, "Dying") {
        scripts.set_port(id, port::DEATH_DONE, 0);
        scripts.set_port(id, port::DEATH_DONE_ALT, 0);
        // Completed at once (no animation) or blocked into a thread.
        done = lifecycle::call_script(world, scripts, id, "Dying", &[dkind]).is_some();
    }
    if let Some(d) = world.unit_mut(id).and_then(|u| u.combat.dying.as_mut()) {
        d.done = done;
    }
}

fn progress_death(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId, now: u32) {
    let Some(d) = world.unit(id).and_then(|u| u.combat.dying) else {
        return;
    };
    let done = d.done
        || scripts.port(id, port::DEATH_DONE) == Some(1)
        || scripts.port(id, port::DEATH_DONE_ALT) == Some(1)
        || now.saturating_sub(d.since) >= DYING_TIMEOUT_TICKS;
    if !done {
        return;
    }
    let mut corpse = 1;
    if scripts.has_function(id, "Killed") {
        if let Some(r) =
            lifecycle::call_script(world, scripts, id, "Killed", &[d.severity, 0, d.kind])
        {
            corpse = r.locals.get(1).copied().unwrap_or(1);
        }
    }
    let Some(u) = world.unit(id) else { return };
    let (def, pos, heading) = (u.def, u.pos, u.heading);
    let explodes = world
        .defs()
        .get(def)
        .is_some_and(|d| d.combat.explode_as.is_some());
    if explodes {
        impact(
            world,
            scripts,
            Some(id),
            WeaponRef::Explosion(def),
            pos,
            None,
            now,
        );
    }
    if world.combat.corpses.len() >= MAX_CORPSES {
        world.combat.corpses.remove(0);
    }
    world.combat.corpses.push(Corpse {
        tick: now,
        def,
        pos,
        heading,
        kind: corpse,
    });
    lifecycle::remove_unit(world, scripts, id);
    // The FBI `corpse` feature (WP-P), blocking movement if the
    // definition says so.
    crate::features::spawn_corpse(world, def, pos, heading, corpse);
}

/// Retire a defeated survivor's pending scripted attacks. Late strike signals
/// cannot launch because the combat lane also excludes inert owners.
pub(crate) fn halt_defeated_unit(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let slots: Vec<_> = world
        .unit(id)
        .map(|u| {
            u.combat
                .weapons
                .iter()
                .enumerate()
                .filter(|(_, w)| w.target.is_some() || w.launch.is_some())
                .map(|(slot, _)| slot as i32)
                .collect()
        })
        .unwrap_or_default();
    for slot in slots {
        lifecycle::call_script(world, scripts, id, "TargetCleared", &[slot]);
    }
    scripts.set_port(id, port::WEAPON_READY, port::NONE);
    scripts.set_port(id, port::LAUNCH_NOW, port::NONE);
    if let Some(u) = world.unit_mut(id) {
        u.combat.cancel_orders();
        u.combat.weapons.fill(WeaponState::default());
    }
}
