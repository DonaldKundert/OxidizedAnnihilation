//! The tick's systems. [`crate::Sim::step`] runs them in this order:
//!
//! 1. [`spawn_start_units`] (tick 0 only): one monarch per player, in player
//!    order, at its start position, facing the map centre.
//! 2. [`apply_commands`]: this tick's commands in `(tick, player, seq)` order.
//!    Attack/Guard/CastSpell go to [`crate::combat`], Build to
//!    [`crate::construction`], Patrol to [`crate::pathfinding`].
//! 3. [`crate::construction::tick`]: build progress, factories.
//! 4. [`crate::economy::tick`]: mana income and storage.
//! 5. [`orders`]: pop reached move waypoints.
//! 6. [`crate::pathfinding::tick`]: plan and follow paths.
//! 7. [`movement`]: steer, accelerate/brake and advance every unit.
//! 8. [`crate::combat::tick`]: targeting, weapons, damage, death.
//! 9. Scripts (`on_tick`), in the sim.
//! 10. Advance the tick counter and hash.
//!
//! Lanes 3, 4, 6 and 8 each own their module; they talk to each other only
//! through the small APIs those modules expose (`economy::drain`,
//! `pathfinding::go_to`) and through [`crate::lifecycle`].
//!
//! Units are always visited in ascending slot order.

use tak_commands::{LoggedCommand, PlayerId, QueueMode, SimCommand};
use tak_math::{Angle, Fx, Vec2};

use crate::defs::UnitDef;
use crate::map::MapInfo;
use crate::pathfinding::{LOOKAHEAD, WAYPOINT_RADIUS};
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
                        u.combat.cancel_orders();
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
                    u.combat.cancel_orders();
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
        SimCommand::Attack { .. } | SimCommand::Guard { .. } | SimCommand::CastSpell { .. } => {
            crate::combat::apply_command(world, player, cmd);
        }
        SimCommand::Build { .. } => crate::construction::apply_command(world, player, cmd),
        SimCommand::Patrol { .. } => crate::pathfinding::apply_command(world, player, cmd),
    }
}

/// Pop every reached waypoint at the front of each queue. Reaching the last
/// one stops the unit dead (the approach braking has it slow by then). A
/// reached patrol leg moves to the back of the queue instead (at most one
/// per unit per tick).
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
        u.path = Default::default();
        u.path.outcome = crate::pathfinding::Outcome::Arrived;
        if u.orders.is_empty() {
            u.speed = Fx::ZERO;
        }
    }
    if let Some(&o @ Order::Patrol { to }) = u.orders.front() {
        if u.pos.distance(to) <= ARRIVAL_TOLERANCE {
            u.orders.pop_front();
            u.orders.push_back(o);
        }
    }
}

/// How terrain constrains one unit's steps.
#[derive(Clone, Copy)]
enum Terrain<'a> {
    /// Flyers, and every unit on maps without terrain.
    Free,
    /// The unit's movement-class layer.
    Grid(&'a crate::pathfinding::ClassGrid),
    /// Layers not built yet (movement run outside [`crate::Sim::step`]):
    /// the map's own centre-cell test.
    Legacy,
}

/// Move every unit one tick along its path, then separate overlapping
/// units ([`crate::pathfinding`]).
pub fn movement(world: &mut World) {
    let pathing = std::mem::take(&mut world.pathing);
    for id in world.unit_ids() {
        let Some(def_id) = world.unit(id).map(|u| u.def) else {
            continue;
        };
        let terr = match pathing.grid_for(def_id) {
            Some(g) => Terrain::Grid(g),
            None if pathing.is_built() => Terrain::Free,
            None => {
                let fly = world.defs().get(def_id).is_some_and(|d| d.motion.canfly);
                if fly { Terrain::Free } else { Terrain::Legacy }
            }
        };
        if let Some((u, def, map)) = world.unit_and_def_mut(id) {
            drive(u, def, map, terr);
            u.sample_terrain(map);
        }
    }
    crate::pathfinding::separate(world, &pathing);
    world.pathing = pathing;
}

/// One unit, one tick, steering straight at its front move order (no
/// pathfinding; the map's centre-cell blocking test). [`movement`] is the
/// system; this is kept for tools that drive a lone unit.
pub fn move_unit(u: &mut Unit, def: &UnitDef, map: &MapInfo) {
    drive(u, def, map, Terrain::Legacy);
    u.sample_terrain(map);
}

/// Steering, after TA's ground mover (Nanolathe `04` §8.1, R-MOV-01;
/// `docs/pathfinding.md`):
///
/// - Consume the current waypoint within [`WAYPOINT_RADIUS`] (one per
///   tick). Aim at it, or, more than [`LOOKAHEAD`] away, at a point dragged
///   back along the incoming leg (pure pursuit).
/// - Turn toward the aim by at most `turnrate`.
/// - Accelerate only if the aim is more than twice the distance covered
///   while finishing the turn (`|err| * speed / turnrate`) and the point
///   after it (the end, on the last leg) is beyond braking distance
///   (`speed² / 2 brakerate`); otherwise brake. Speed is capped at
///   `maxvelocity` times the terrain factor
///   ([`crate::pathfinding::speed_factor`]).
/// - Kept from WP-I: when the unit is within [`ALIGN_THRESHOLD`] of its
///   bearing and this tick's travel reaches the waypoint, it lands exactly
///   on it; non-last orders flow through their end at speed.
/// - A step the terrain refuses is not taken: speed is capped at half
///   `maxvelocity` (TA) and the path is marked blocked; the planner replans
///   if the unit stalls.
/// - Idle units brake to a halt along their heading. Waiting for a path
///   counts as idle.
fn drive(u: &mut Unit, def: &UnitDef, map: &MapInfo, terr: Terrain) {
    let Some(to) = (match u.orders.front() {
        Some(Order::Move { to }) | Some(Order::Patrol { to }) => Some(*to),
        _ => None,
    }) else {
        u.speed = u.speed.saturating_sub(def.brakerate).max(Fx::ZERO);
        advance(u, map, terr);
        return;
    };
    let following = u.path.goal == Some(to)
        && u.path.status == crate::pathfinding::PathStatus::Following
        && !u.path.waypoints.is_empty();
    if !following {
        if !matches!(terr, Terrain::Legacy) {
            u.speed = u.speed.saturating_sub(def.brakerate).max(Fx::ZERO);
            advance(u, map, terr);
            return;
        }
        u.path.waypoints = vec![to];
        u.path.prev = u.pos;
    }

    // Consume a reached waypoint (never the last).
    if u.path.waypoints.len() >= 2 && within(u.pos, u.path.waypoints[0], WAYPOINT_RADIUS) {
        u.path.prev = u.path.waypoints.remove(0);
        u.path.best = Fx::MAX;
    }
    let wp = u.path.waypoints[0];
    let final_wp = u.path.waypoints.len() == 1;
    let next = u.path.waypoints.get(1).copied().unwrap_or(wp);
    let last_leg =
        final_wp && u.orders.len() == 1 && matches!(u.orders.front(), Some(Order::Move { .. }));

    // Pure-pursuit aim point.
    let dist = u.pos.distance(wp);
    let mut aim = wp;
    if dist > LOOKAHEAD {
        let leg = wp - u.path.prev;
        let len = leg.length();
        if len >= Fx::ONE {
            let t = (dist - LOOKAHEAD).min(len);
            aim = wp - leg.scale(t.saturating_div(len));
        }
    }

    let delta = aim - u.pos;
    let err = if delta == Vec2::ZERO {
        0
    } else {
        i32::from(u.heading.delta_to(delta.angle()))
    };
    let tr = i32::from(def.turnrate);
    let turn = err.clamp(-tr, tr);
    u.heading = Angle::from_raw(u.heading.raw().wrapping_add(turn as i16 as u16));
    let aligned = (err - turn).unsigned_abs() <= u32::from(ALIGN_THRESHOLD);

    // Accelerate or brake (TA's two tests).
    let speed = i64::from(u.speed.raw());
    let turn_dist: i64 = match (err, tr) {
        (0, _) => 0,
        (_, 0) => i64::MAX / 4,
        _ => i64::from(err.unsigned_abs()) * speed / i64::from(tr),
    };
    let stop_dist = speed * speed / (2 * i64::from(def.brakerate.raw()).max(1));
    let aim_dist = i64::from(u.pos.distance(aim).raw());
    let after = if last_leg || !final_wp {
        i64::from(u.pos.distance(next).raw())
    } else {
        i64::MAX
    };
    let cap = def
        .maxvelocity
        .saturating_mul(crate::pathfinding::speed_factor(u, def))
        .min(crate::defs::MAX_VELOCITY_CAP);
    u.speed = if aim_dist > 2 * turn_dist && after > stop_dist {
        u.speed.saturating_add(def.acceleration).min(cap)
    } else {
        u.speed.saturating_sub(def.brakerate).max(Fx::ZERO)
    };
    if u.speed > cap {
        u.speed = cap;
    }

    let moved = if aligned && u.speed >= dist {
        let ok = step_to(u, map, wp, terr);
        if ok && !final_wp {
            u.path.prev = u.path.waypoints.remove(0);
            u.path.best = Fx::MAX;
        }
        ok
    } else {
        advance(u, map, terr)
    };
    u.path.blocked = !moved;
    if !moved {
        let half = def.maxvelocity.saturating_mul(Fx::HALF);
        u.speed = u.speed.min(half);
    }
}

/// `true` if `a` is within `r` whole world units of `b` (TA compares the
/// integer parts, squared, inclusive).
fn within(a: WorldPos, b: WorldPos, r: i32) -> bool {
    let dx = i64::from(a.x.to_int_floor()) - i64::from(b.x.to_int_floor());
    let dy = i64::from(a.y.to_int_floor()) - i64::from(b.y.to_int_floor());
    dx * dx + dy * dy <= i64::from(r) * i64::from(r)
}

/// Advance along the heading; `false` if terrain refused the step.
fn advance(u: &mut Unit, map: &MapInfo, terr: Terrain) -> bool {
    if u.speed == Fx::ZERO {
        return true;
    }
    let step: WorldPos = Vec2::from_angle(u.heading).scale(u.speed);
    let next = map.clamp(u.pos + step);
    step_to(u, map, next, terr)
}

/// Move to `to` unless terrain refuses it. A refused idle step stops the
/// unit dead.
fn step_to(u: &mut Unit, map: &MapInfo, to: WorldPos, terr: Terrain) -> bool {
    let ok = match terr {
        Terrain::Free => true,
        Terrain::Grid(g) => g.step_allowed(u.pos, to),
        Terrain::Legacy => !map.blocks_step(u.pos, to),
    };
    if !ok {
        if u.orders.is_empty() {
            u.speed = Fx::ZERO;
        }
        return false;
    }
    u.pos = to;
    true
}
