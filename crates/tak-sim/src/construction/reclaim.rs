//! Explicit builder Clear. Rules with uncertain retail semantics are centralized
//! in `total_work`, `in_range` and `work`; see docs/construction.md.
use tak_commands::{FeatureId, PlayerId, QueueMode, SimCommand, UnitId};
use tak_math::Fx;

use crate::features::{self, Feature, FeatureDef};
use crate::hash::StateHasher;
use crate::script::ScriptHost;
use crate::world::{MAX_ORDERS, Order, Unit, World};
use crate::{UnitDef, WorldPos};

/// One explicit Clear interval. The target owns progress, not this worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClearJob {
    /// Exact feature lifetime.
    pub target: FeatureId,
    /// Last attempted steering goal, retained to avoid endless failed retries.
    pub walk_to: Option<WorldPos>,
    /// The front Move is owned by this Clear, not by the player.
    pub walking: bool,
    /// StartBuilding has been sent and StopBuilding is owed.
    pub building: bool,
    /// Replacing commands invalidate the interval even for the same target.
    pub cancelled: bool,
}

impl ClearJob {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.u32(self.target.index);
        h.u32(self.target.generation);
        h.option(self.walk_to, StateHasher::vec2);
        h.bool(self.walking);
        h.bool(self.building);
        h.bool(self.cancelled);
    }
}

/// Authored capability on a complete live unit with positive work and range.
/// Neither a build menu nor `builder` is required: some authored Clear-capable
/// units are not builders. Stationary units can work within reach.
pub fn can_clear(world: &World, id: UnitId) -> bool {
    world.unit(id).is_some_and(|u| {
        u.build.is_complete()
            && !u.combat.is_dying()
            && world.defs().get(u.def).is_some_and(|d| {
                d.build.canreclaim && d.build.workertime > 0 && d.build.builddistance > 0
            })
    })
}

/// Reclaimability is independent of weapon health and indestructibility.
pub fn can_clear_target(world: &World, target: FeatureId) -> bool {
    world
        .features
        .get(target)
        .is_some_and(|f| world.features.def(f).is_some_and(|d| d.reclaimable))
}

/// Invalid actor/target rejection leaves existing queues untouched.
pub fn apply_clear_command(world: &mut World, player: PlayerId, cmd: &SimCommand) {
    let SimCommand::Clear {
        units,
        target,
        queue,
    } = cmd
    else {
        return;
    };
    if !can_clear_target(world, *target) {
        return;
    }
    for &id in units {
        if !can_clear(world, id) || !world.unit(id).is_some_and(|u| u.owner == player) {
            continue;
        }
        let u = world.unit_mut(id).expect("checked live");
        if *queue == QueueMode::Replace {
            u.cancel_player_orders();
        }
        if u.orders.len() < MAX_ORDERS {
            u.orders.push_back(Order::Clear { target: *target });
        }
    }
}

pub(crate) fn is_work_move(u: &Unit) -> bool {
    u.build.clear.is_some_and(|j| {
        !j.cancelled
            && j.walking
            && j.walk_to
                .is_some_and(|to| u.orders.front() == Some(&Order::Move { to }))
    })
}

pub(crate) fn consume_work_move(u: &mut Unit) -> bool {
    if is_work_move(u) {
        u.build.clear.as_mut().expect("checked").walking = false;
        true
    } else {
        false
    }
}

fn current(u: &Unit, j: ClearJob) -> bool {
    !j.cancelled
        && u.combat.order.is_none()
        && (u.orders.front() == Some(&Order::Clear { target: j.target })
            || (is_work_move(u) && u.orders.get(1) == Some(&Order::Clear { target: j.target })))
}

fn end(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId, drop_order: bool) {
    let Some(u) = world.unit_mut(id) else { return };
    let owned_move = is_work_move(u);
    let Some(j) = u.build.clear.take() else {
        return;
    };
    if owned_move {
        u.orders.pop_front();
        u.path = Default::default();
    }
    if drop_order && !j.cancelled && u.orders.front() == Some(&Order::Clear { target: j.target }) {
        u.orders.pop_front();
    }
    if j.building {
        super::start(scripts, id, "StopBuilding");
    }
}

/// Teardown on actor death, lifecycle removal or defeat.
pub(crate) fn cancel(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    end(world, scripts, id, false);
}

pub(crate) fn sync_job(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    if world
        .unit(id)
        .is_some_and(|u| u.build.clear.is_some_and(|j| !current(u, j)))
    {
        end(world, scripts, id, false);
    }
}

fn store(world: &mut World, id: UnitId, job: ClearJob) {
    if let Some(u) = world.unit_mut(id) {
        u.build.clear = Some(job);
    }
}

/// Chosen: use construction's footprint-padded, inclusive centre-distance
/// range test for features too. Not a claim about original model collision.
fn in_range(pos: WorldPos, builder: &UnitDef, f: &Feature, d: &FeatureDef) -> bool {
    let gap = pos
        .distance(f.center())
        .to_int_floor()
        .saturating_sub(super::placement::pad(
            builder.footprintx,
            builder.footprintz,
        ))
        .saturating_sub(super::placement::pad(d.footprint_x, d.footprint_z));
    gap <= builder.build.builddistance
}

pub(crate) fn step(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(u) = world.unit(id) else { return };
    // An appended Clear waits for an active repeated product to detach.
    // Factory production never advances alongside a Clear work interval.
    if u.build.factory.product.is_some() {
        return;
    }
    let mut j = match u.build.clear.filter(|&j| current(u, j)) {
        Some(j) => j,
        None => {
            let Some(&Order::Clear { target }) = u.orders.front() else {
                return;
            };
            if u.combat.order.is_some() {
                return;
            }
            ClearJob {
                target,
                walk_to: None,
                walking: false,
                building: false,
                cancelled: false,
            }
        }
    };
    store(world, id, j);
    if !can_clear(world, id) || !can_clear_target(world, j.target) {
        end(world, scripts, id, true);
        return;
    }
    let u = world.unit(id).expect("checked live");
    let f = world.features.get(j.target).expect("checked target");
    let bd = world.defs().get(u.def).expect("valid unit def");
    let fd = world.features.def(f).expect("valid feature def");
    let (owner, pos, turnrate, workertime) =
        (u.owner, f.center(), bd.turnrate, bd.build.workertime);
    if !in_range(u.pos, bd, f, fd) {
        if j.building {
            super::start(scripts, id, "StopBuilding");
            j.building = false;
        }
        // bmcode controls mobile builders; no implicit movement for factories.
        if !bd.build.mobile || bd.maxvelocity <= Fx::ZERO {
            store(world, id, j);
            end(world, scripts, id, true);
            return;
        }
        let goal = world.map().clamp(pos);
        if j.walk_to != Some(goal) {
            let u = world.unit_mut(id).expect("live");
            if is_work_move(u) {
                u.orders.pop_front();
            }
            // Temporary steering may use one extra slot without dropping tails.
            u.orders.push_front(Order::Move { to: goal });
            u.path = Default::default();
            j.walk_to = Some(goal);
            j.walking = true;
        } else if !j.walking {
            store(world, id, j);
            end(world, scripts, id, true);
            return;
        }
        store(world, id, j);
        return;
    }
    let u = world.unit_mut(id).expect("live");
    if is_work_move(u) {
        u.orders.pop_front();
    }
    u.path = Default::default();
    u.speed = Fx::ZERO;
    j.walking = false;
    j.walk_to = None;
    if !j.building {
        super::start(scripts, id, "StartBuilding");
        j.building = true;
    }
    store(world, id, j);
    super::face(world, id, pos, turnrate);
    work(world, j.target, owner, workertime);
}

pub(crate) fn finish(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    if world
        .unit(id)
        .and_then(|u| u.build.clear)
        .is_some_and(|j| !can_clear_target(world, j.target))
    {
        end(world, scripts, id, true);
    }
}

/// Chosen fallback: work proportional to the greater of authored damage and
/// energy, at least one. Zero-yield obstacles still require clearing work.
/// Exact Kingdoms timing is not established.
/// Raw 16.16 workertime-ticks; one worker adds `workertime << 16` per tick.
pub fn total_work(d: &FeatureDef) -> i64 {
    super::progress::total_work(i64::from(d.damage.max(d.energy).max(1)) << 16)
}

fn work(world: &mut World, target: FeatureId, owner: PlayerId, workertime: i32) {
    let Some(f) = world.features.get(target) else {
        return;
    };
    let Some(d) = world.features.def(f) else {
        return;
    };
    let (total, energy) = (total_work(d), i64::from(d.energy) << 16);
    let before = f.reclaim_done;
    let done = (before + super::progress::worker_step(workertime)).min(total);
    // Chosen: shared cumulative rounding, awarded to the owner contributing
    // each actual slice. Cancellation/overwork cannot create another bounty.
    let owed = |work: i64| (i128::from(energy) * i128::from(work) / i128::from(total)) as i64;
    // Chosen resource conservation across weapon replacements. A successor
    // may offer only the difference above earnings already consumed by its
    // lineage; self/cyclic featuredead graphs cannot mint an unbounded bounty.
    // Count attempted earnings, not storage acceptance, so overflow is lost.
    let gain = (owed(done) - owed(before)).min((energy - f.reclaim_claimed).max(0));
    let claimed = f.reclaim_claimed + gain;
    world.features.set_reclaim_progress(target, done, claimed);
    if gain > 0 {
        // Storage is refreshed later this tick, including tick-zero spawning
        // and storage units finished by construction. Never lose that first
        // slice against the economy's stale initial zero capacity.
        world.construction.reclaim_credits.push(super::Refund {
            player: owner,
            mana: gain,
        });
    }
    if done == total {
        // Retail featurereclamate is almost always a self-reference; its
        // meaning is unverified. Never recreate a resource-bearing lifetime.
        features::remove(world, target);
    }
}
