//! The tick's systems. [`crate::Sim::step`] runs them in this order:
//!
//! 1. [`spawn_start_units`] (tick 0 only): one monarch per player, in player
//!    order, at its start position, facing the map centre.
//! 2. [`apply_commands`]: this tick's commands in `(tick, player, seq)` order.
//! 3. [`orders`]: pop reached move waypoints.
//! 4. [`movement`]: steer, accelerate/brake and advance every unit.
//! 5. Scripts (`on_tick`), in the sim.
//! 6. Advance the tick counter and hash.
//!
//! Units are always visited in ascending slot order.

use tak_commands::{LoggedCommand, PlayerId, QueueMode, SimCommand};
use tak_math::{Angle, Fx, Vec2};

use crate::defs::UnitDef;
use crate::map::MapInfo;
use crate::world::{MAX_ORDERS, Order, Unit, World};
use crate::{UnitId, WorldPos};

/// A move waypoint counts as reached within this distance (world units).
pub const ARRIVAL_TOLERANCE: Fx = Fx::from_int(2);

/// While the heading is off the target bearing by more than this, the unit
/// brakes and turns (in place, once stopped) instead of driving on, which
/// rules out orbiting a target inside its turning circle. 45 degrees.
pub const ALIGN_THRESHOLD: u16 = 0x2000;

/// Spawn each player's monarch. Returns the new ids in spawn order.
pub fn spawn_start_units(world: &mut World) -> Vec<UnitId> {
    let center = world.map().center();
    let mut spawned = Vec::new();
    for i in 0..world.players.len() {
        let p = world.players[i].clone();
        let d = center - p.start;
        let heading = if d == Vec2::ZERO {
            Angle::QUARTER
        } else {
            d.angle()
        };
        if let Some(id) = world.spawn(p.monarch_def, p.id, p.start, heading) {
            world.players[i].monarch = Some(id);
            spawned.push(id);
        }
    }
    spawned
}

/// Apply the commands for the current tick. Entries for other ticks, unknown
/// players, and unit ids that are stale or owned by someone else are dropped
/// silently. Orders the skeleton does not implement yet (attack, build,
/// guard, patrol, spells) are ignored entirely and do not touch the queue.
pub fn apply_commands(world: &mut World, commands: &[LoggedCommand]) {
    for c in commands {
        if c.tick != world.tick || !world.has_player(c.player) {
            continue;
        }
        apply_one(world, c.player, &c.command);
    }
}

fn apply_one(world: &mut World, player: PlayerId, cmd: &SimCommand) {
    match cmd {
        SimCommand::Move { units, to, queue } => {
            let to = world.map().clamp(*to);
            for &id in units {
                if let Some(u) = world.unit_mut_owned(id, player) {
                    if *queue == QueueMode::Replace {
                        u.orders.clear();
                    }
                    if u.orders.len() < MAX_ORDERS {
                        u.orders.push_back(Order::Move { to });
                    }
                }
            }
        }
        SimCommand::Stop { units } => {
            for &id in units {
                if let Some(u) = world.unit_mut_owned(id, player) {
                    u.orders.clear();
                }
            }
        }
        SimCommand::SetFireState { units, state } => {
            for &id in units {
                if let Some(u) = world.unit_mut_owned(id, player) {
                    u.fire_state = *state;
                }
            }
        }
        SimCommand::Attack { .. }
        | SimCommand::Build { .. }
        | SimCommand::Guard { .. }
        | SimCommand::Patrol { .. }
        | SimCommand::CastSpell { .. } => {}
    }
}

/// Pop every reached waypoint at the front of each queue. Reaching the last
/// one stops the unit dead (its speed there is already at most about
/// `sqrt(2 * brakerate * tolerance)` thanks to the approach braking).
pub fn orders(world: &mut World) {
    for id in world.unit_ids() {
        if let Some(u) = world.unit_mut(id) {
            order_step(u);
        }
    }
}

fn order_step(u: &mut Unit) {
    while let Some(Order::Move { to }) = u.orders.front() {
        if u.pos.distance(*to) > ARRIVAL_TOLERANCE {
            break;
        }
        u.orders.pop_front();
        if u.orders.is_empty() {
            u.speed = Fx::ZERO;
        }
    }
}

/// Move every unit one tick.
pub fn movement(world: &mut World) {
    for id in world.unit_ids() {
        if let Some((u, def, map)) = world.unit_and_def_mut(id) {
            move_unit(u, def, map);
        }
    }
}

/// One unit, one tick. Straight-line driving, no pathfinding:
///
/// - Turn toward the active waypoint by at most `turnrate`.
/// - Target speed is `maxvelocity`, or zero while misaligned by more than
///   [`ALIGN_THRESHOLD`]. On the last waypoint it is also capped at
///   `sqrt(2 * brakerate * distance)` so the unit brakes to arrive, with a
///   floor of `min(acceleration, maxvelocity)` so it never stalls short.
/// - Speed moves toward the target by `acceleration` (up) or `brakerate`
///   (down).
/// - If the unit is aligned and this tick's travel reaches the waypoint, it
///   lands exactly on it; otherwise it advances along its heading.
/// - Idle units brake to a halt along their heading.
///
/// Positions are clamped to the map.
pub fn move_unit(u: &mut Unit, def: &UnitDef, map: &MapInfo) {
    let Some(&Order::Move { to }) = u.orders.front() else {
        u.speed = u.speed.saturating_sub(def.brakerate).max(Fx::ZERO);
        advance(u, map);
        return;
    };
    let last_leg = u.orders.len() == 1;
    let delta = to - u.pos;
    let dist = u.pos.distance(to);

    let err = i32::from(u.heading.delta_to(delta.angle()));
    let tr = i32::from(def.turnrate);
    let turn = err.clamp(-tr, tr);
    u.heading = Angle::from_raw(u.heading.raw().wrapping_add(turn as i16 as u16));
    let aligned = (err - turn).unsigned_abs() <= u32::from(ALIGN_THRESHOLD);

    let want = if !aligned {
        Fx::ZERO
    } else if last_leg {
        let braking = def
            .brakerate
            .saturating_mul(dist)
            .saturating_add(def.brakerate.saturating_mul(dist))
            .sqrt();
        let floor = def.acceleration.min(def.maxvelocity);
        def.maxvelocity.min(braking.max(floor))
    } else {
        def.maxvelocity
    };
    u.speed = if u.speed < want {
        u.speed.saturating_add(def.acceleration).min(want)
    } else {
        u.speed.saturating_sub(def.brakerate).max(want)
    };

    if aligned && u.speed >= dist {
        u.pos = to;
    } else {
        advance(u, map);
    }
}

fn advance(u: &mut Unit, map: &MapInfo) {
    if u.speed == Fx::ZERO {
        return;
    }
    let step: WorldPos = Vec2::from_angle(u.heading).scale(u.speed);
    u.pos = map.clamp(u.pos + step);
}
