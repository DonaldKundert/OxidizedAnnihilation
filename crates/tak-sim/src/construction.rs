//! Construction (WP-O): build orders, frames (units under construction),
//! mobile builders, assisting, and factory production queues.
//!
//! - `Build` commands are routed here by
//!   [`crate::systems::apply_commands`] ([`apply_command`]): checked
//!   against the builder's build menu, then queued as [`Order::Build`].
//! - [`tick`] runs right after commands: frames are held still, then every
//!   builder works in slot order, then finished builds are wrapped up.
//! - New units come from [`crate::lifecycle::spawn_unit`], mana from
//!   [`crate::economy::drain`], builders walk with
//!   [`crate::pathfinding::go_to`].
//!
//! Formulas and their confidence: [`progress`] and `docs/construction.md`.
//! Other lanes should treat a unit with `!build.is_complete()` as inert:
//! it does not move, aim, fire, or count as a finished unit.

mod build_def;
pub mod content;
pub mod placement;
pub mod progress;
pub mod reclaim;
pub mod repair;
pub use reclaim::{ClearJob, apply_clear_command, can_clear, can_clear_target};

pub use build_def::{BuildDef, MAX_BUILD_OPTIONS, YARD_CHARS, build_def_from_fbi, expand_yardmap};
pub use progress::Frame;

use tak_commands::{
    FactoryRallyOrder, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Angle, Fx, Vec2};
use tak_script::port;

use crate::hash::StateHasher;
use crate::lifecycle;
use crate::script::ScriptHost;
use crate::world::{MAX_ORDERS, Order, World};
use crate::{WorldPos, economy, pathfinding};

/// Heading of new structures and factory products: south, toward the
/// camera (**Chosen**; TA samples a random angle around its `buildangle`,
/// which needs the renderer's model orientation to mean anything).
pub const BUILD_HEADING: Angle = Angle::QUARTER;

/// Ticks before a factory looks at a blocked exit spot again (**Known**
/// for TA, Nanolathe "Silent blocked revalidation").
pub const EXIT_RETRY_TICKS: u32 = 15;

/// How far past its factory's footprint a product without rally orders
/// walks, in world units (**Chosen**; TA's product release is Unknown even
/// in Nanolathe's research).
pub const EXIT_CLEARANCE: i32 = 24;

/// A mobile builder's current job: the [`Order::Build`] at the front of
/// its queue (behind the `Move`s it inserted to walk there).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuilderJob {
    /// What is being built.
    pub def: UnitDefId,
    /// Snapped site centre.
    pub site: WorldPos,
    /// The frame, once placed.
    pub target: Option<UnitId>,
    /// The builder was sent walking to the site.
    pub walking: bool,
    /// `StartBuilding` was called and `StopBuilding` is still owed.
    pub building: bool,
}

/// A factory's production state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FactoryState {
    /// The product on the pad.
    pub product: Option<UnitId>,
    /// The active product came from the repeat recipe, not a finite order.
    /// Retained when the recipe changes or is disabled until release/cancel.
    pub product_from_repeat: bool,
    /// `Activate` was called and `Deactivate` is still owed.
    pub active: bool,
    /// Do not look at the exit spot before this tick.
    pub retry_at: Option<Tick>,
    /// An explicit queue edit removed the active entry. Cleanup must occur
    /// before a different queued product can advance, even within one tick.
    pub cancel_product: bool,
}

/// Per-unit construction state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitBuild {
    /// `Some` while the unit is under construction.
    pub frame: Option<Frame>,
    /// Mobile builders: the current job.
    pub job: Option<BuilderJob>,
    /// Factories: production.
    pub factory: FactoryState,
    /// Explicit standing-order recipe, separate from production. `None`
    /// retains legacy queued-Move rally behavior for old command logs;
    /// `Some([])` explicitly clears it. The first Patrol needs one extra
    /// return leg; any Guard recipe reserves a yard-exit leg. Both are
    /// included in the shared `MAX_ORDERS` admission bound.
    pub factory_rally: Option<Vec<FactoryRallyOrder>>,
    /// Continuous recipe used only when no finite production remains.
    /// No orders are allocated for repeats; active products retain origin.
    pub factory_repeat: Option<UnitDefId>,
    /// Mobile builders: the frame it helps build because it guards that
    /// frame, its builder or its factory (WP-P).
    pub assist: Option<Assist>,
    /// Explicit queued Heal action, independent of Guard assistance.
    pub heal: Option<HealJob>,
    /// Explicit feature Clear interval and owned temporary steering.
    pub clear: Option<ClearJob>,
    /// Target-owned fractional repair and production-mana credit.
    pub repair: Option<repair::RepairCredit>,
}

/// A mobile builder assisting through `Guard` (see [`assist_target`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assist {
    /// The frame being built.
    pub target: UnitId,
    /// Last attempted steering goal. Retained after path failure so an
    /// unreachable unchanged target is not retried every tick.
    pub walk_to: Option<WorldPos>,
    /// The front Move is still the temporary assist walk, not a player order.
    pub walking: bool,
    /// `StartBuilding` was called and `StopBuilding` is still owed.
    pub building: bool,
}

/// An explicit Heal action and its temporary-movement ownership token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealJob {
    /// Shared work interval and owned temporary steering.
    pub work: Assist,
    /// Replacing orders invalidate the interval even for the same target.
    pub cancelled: bool,
}

impl UnitBuild {
    /// `false` while the unit is under construction. Incomplete units must
    /// not move, aim, fire or build.
    pub fn is_complete(&self) -> bool {
        self.frame.is_none()
    }

    /// Percent still to build for scripts (port 17): 100 for a fresh
    /// frame, 0 once complete.
    pub fn percent_left(&self) -> i32 {
        self.frame.map_or(0, |f| f.percent_left())
    }

    /// Built fraction as `(done, total)` work units; `(1, 1)` when
    /// complete. Combat can scale a frame's health with it (TA:
    /// `health = maxdamage * done / total`, truncated).
    pub fn built_fraction(&self) -> (i64, i64) {
        self.frame.map_or((1, 1), |f| (f.done, f.total))
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"B");
        match self.frame {
            None => h.u8(0),
            Some(f) => {
                h.u8(1);
                h.u64(f.done as u64);
                h.u64(f.total as u64);
                h.u64(f.paid as u64);
            }
        }
        match self.job {
            None => h.u8(0),
            Some(j) => {
                h.u8(1);
                h.unit_def(j.def);
                h.vec2(j.site);
                h.option(j.target, StateHasher::unit_id);
                h.bool(j.walking);
                h.bool(j.building);
            }
        }
        let f = &self.factory;
        h.option(f.product, StateHasher::unit_id);
        h.bool(f.active);
        h.option(f.retry_at, StateHasher::tick);
        h.option(self.assist, |h, a| {
            h.unit_id(a.target);
            h.option(a.walk_to, StateHasher::vec2);
            h.bool(a.walking);
            h.bool(a.building);
        });
        // No suffix for units that have never used Heal, retaining legacy
        // no-Heal replay hashes and old demo compatibility.
        if let Some(j) = self.heal {
            h.bytes(b"HEAL");
            h.unit_id(j.work.target);
            h.option(j.work.walk_to, StateHasher::vec2);
            h.bool(j.work.walking);
            h.bool(j.work.building);
            h.bool(j.cancelled);
        }
        if let Some(j) = self.clear {
            h.bytes(b"CLEAR");
            j.hash(h);
        }
        if let Some(r) = self.repair {
            h.bytes(b"REPAIR");
            r.hash(h);
        }
        // Additive commands alone enable this state. Omit empty legacy state
        // so existing replay/checkpoint hashes remain byte-for-byte stable.
        if let Some(rally) = &self.factory_rally {
            h.bytes(b"FACTORY_RALLY");
            h.len(rally.len());
            for order in rally {
                match *order {
                    FactoryRallyOrder::Move { to } => {
                        h.u8(0);
                        h.vec2(to);
                    }
                    FactoryRallyOrder::Patrol { to } => {
                        h.u8(1);
                        h.vec2(to);
                    }
                    FactoryRallyOrder::Guard { target } => {
                        h.u8(2);
                        h.unit_id(target);
                    }
                }
            }
        }
        if self.factory.cancel_product {
            h.bytes(b"FACTORY_CANCEL_PRODUCT");
        }
        if let Some(def) = self.factory_repeat {
            h.bytes(b"FACTORY_REPEAT");
            h.unit_def(def);
        }
        if self.factory.product_from_repeat {
            h.bytes(b"FACTORY_PRODUCT_FROM_REPEAT");
        }
    }
}

/// Mana owed back to a player (a cancelled factory product). The economy
/// lane credits these with [`ConstructionState::take_refunds`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refund {
    /// Who gets it.
    pub player: PlayerId,
    /// Mana, raw 16.16.
    pub mana: i64,
}

/// A build order dropped because its site was refused (WP-P): what the
/// player sees as "cannot build here" and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejection {
    /// Tick of the refusal.
    pub tick: Tick,
    /// The builder whose order was dropped.
    pub builder: UnitId,
    /// Its owner.
    pub player: PlayerId,
    /// What it was to build.
    pub def: UnitDefId,
    /// The snapped site centre.
    pub site: WorldPos,
    /// Why ([`placement::SiteError::reason`] has the text).
    pub reason: placement::SiteError,
}

/// Most recent rejections kept in [`ConstructionState::rejections`].
pub const MAX_REJECTIONS: usize = 64;

/// World-level construction state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConstructionState {
    /// Refunds not yet credited by the economy.
    pub refunds: Vec<Refund>,
    /// Clear income, settled after this tick's live storage refresh.
    pub reclaim_credits: Vec<Refund>,
    /// The latest refused build orders, oldest first (at most
    /// [`MAX_REJECTIONS`]).
    pub rejections: Vec<Rejection>,
}

impl ConstructionState {
    /// Take the pending refunds (the economy calls this).
    pub fn take_refunds(&mut self) -> Vec<Refund> {
        std::mem::take(&mut self.refunds)
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"BUILD");
        h.len(self.refunds.len());
        for r in &self.refunds {
            h.player(r.player);
            h.u64(r.mana as u64);
        }
        if !self.reclaim_credits.is_empty() {
            h.bytes(b"RECLAIM_CREDITS");
            h.len(self.reclaim_credits.len());
            for r in &self.reclaim_credits {
                h.player(r.player);
                h.u64(r.mana as u64);
            }
        }
        h.len(self.rejections.len());
        for r in &self.rejections {
            h.tick(r.tick);
            h.unit_id(r.builder);
            h.player(r.player);
            h.unit_def(r.def);
            h.vec2(r.site);
            h.u8(r.reason as u8);
        }
    }

    fn reject(&mut self, r: Rejection) {
        if self.rejections.len() >= MAX_REJECTIONS {
            self.rejections.remove(0);
        }
        self.rejections.push(r);
    }
}

/// Whether `id` is a live, complete mobile builder eligible for explicit Heal.
/// Workertime alone never grants the capability (e.g. nonbuilder creatures).
pub fn can_heal(world: &World, id: UnitId) -> bool {
    world.unit(id).is_some_and(|u| {
        u.build.is_complete()
            && !u.combat.is_dying()
            && world
                .defs()
                .get(u.def)
                .is_some_and(|d| d.build.builder && d.build.mobile && d.build.workertime > 0)
    })
}

fn heal_target(world: &World, actor: UnitId, target: UnitId) -> bool {
    let (Some(a), Some(t)) = (world.unit(actor), world.unit(target)) else {
        return false;
    };
    actor != target
        && a.owner == t.owner
        && !t.combat.is_dying()
        && world
            .defs()
            .get(t.def)
            .is_some_and(|d| d.combat.max_damage > 0)
}

/// Admit an explicit owned-unit Heal order. Capability/target rejection leaves
/// the existing queue untouched. Full-health targets are accepted and finish
/// when the order reaches the front; queued targets may be damaged meanwhile.
pub fn apply_heal_command(world: &mut World, player: PlayerId, cmd: &SimCommand) {
    let SimCommand::Heal {
        units,
        target,
        queue,
    } = cmd
    else {
        return;
    };
    for &id in units {
        if !can_heal(world, id)
            || !heal_target(world, id, *target)
            || !world.unit(id).is_some_and(|u| u.owner == player)
        {
            continue;
        }
        if let Some(u) = world.unit_mut(id) {
            if *queue == QueueMode::Replace {
                u.cancel_player_orders();
            }
            if u.orders.len() < MAX_ORDERS {
                u.orders.push_back(Order::Heal { target: *target });
            }
        }
    }
}

/// An owned work walk, not an arbitrary player Move at the same coordinates.
/// Cancellation and every path/order completion must consume the ownership
/// flag before examining another waypoint.
pub(crate) fn is_work_move(u: &crate::world::Unit) -> bool {
    let owns = |a: Assist| {
        a.walking
            && a.walk_to
                .is_some_and(|to| u.orders.front() == Some(&Order::Move { to }))
    };
    u.build.assist.is_some_and(owns)
        || u.build.heal.is_some_and(|j| !j.cancelled && owns(j.work))
        || reclaim::is_work_move(u)
}

/// Consume temporary-work steering ownership before popping the front Move.
pub(crate) fn consume_work_move(u: &mut crate::world::Unit) -> bool {
    let front = u.orders.front().copied();
    let mut consumed = reclaim::consume_work_move(u);
    if let Some(a) = &mut u.build.assist {
        if a.walking
            && a.walk_to
                .is_some_and(|to| front == Some(Order::Move { to }))
        {
            a.walking = false;
            consumed = true;
        }
    }
    if let Some(j) = &mut u.build.heal {
        if j.work.walking
            && j.work
                .walk_to
                .is_some_and(|to| front == Some(Order::Move { to }))
        {
            j.work.walking = false;
            consumed = true;
        }
    }
    consumed
}

/// End an explicit Heal interval, including an owed StopBuilding. Called on
/// replacement, target loss, death and lifecycle removal, even without scripts.
pub(crate) fn cancel_heal(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    end_heal(world, scripts, id, false);
}

fn end_heal(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId, drop_order: bool) {
    let Some(u) = world.unit_mut(id) else { return };
    let Some(j) = u.build.heal.take() else { return };
    let a = j.work;
    if !j.cancelled
        && a.walking
        && a.walk_to
            .is_some_and(|to| u.orders.front() == Some(&Order::Move { to }))
    {
        u.orders.pop_front();
        u.path = Default::default();
    }
    if drop_order && !j.cancelled && u.orders.front() == Some(&Order::Heal { target: a.target }) {
        u.orders.pop_front();
    }
    if a.building {
        start(scripts, id, "StopBuilding");
    }
}

fn store_heal(world: &mut World, id: UnitId, work: Assist) {
    if let Some(u) = world.unit_mut(id) {
        u.build.heal = Some(HealJob {
            work,
            cancelled: false,
        });
    }
}

fn heal_done(world: &World, id: UnitId, target: UnitId) -> bool {
    if !heal_target(world, id, target) {
        return true;
    }
    let Some(t) = world.unit(target) else {
        return true;
    };
    if !t.build.is_complete() {
        return false;
    }
    world
        .defs()
        .get(t.def)
        .is_none_or(|d| d.build.buildtime <= 0)
        || (t.combat.ready && t.combat.health >= t.combat.max_health)
}

fn current_heal(u: &crate::world::Unit, j: HealJob) -> bool {
    !j.cancelled
        && u.combat.order.is_none()
        && (u.orders.front()
            == Some(&Order::Heal {
                target: j.work.target,
            })
            || (j.work.walking
                && j.work
                    .walk_to
                    .is_some_and(|to| u.orders.front() == Some(&Order::Move { to }))
                && u.orders.get(1)
                    == Some(&Order::Heal {
                        target: j.work.target,
                    })))
}

fn sync_heal_job(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    if world
        .unit(id)
        .is_some_and(|u| u.build.heal.is_some_and(|j| !current_heal(u, j)))
    {
        end_heal(world, scripts, id, false);
    }
}

fn heal_step(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(u) = world.unit(id) else { return };
    let current = u.build.heal.filter(|&j| current_heal(u, j));
    let Some(u) = world.unit(id) else { return };
    let mut a = match current {
        Some(j) => j.work,
        None => {
            // Never activate Heal through an earlier player Move, combat
            // order or construction order, even if coordinates happen to match.
            let Some(&Order::Heal { target }) = u.orders.front() else {
                return;
            };
            if u.combat.order.is_some() {
                return;
            }
            Assist {
                target,
                walk_to: None,
                walking: false,
                building: false,
            }
        }
    };
    store_heal(world, id, a);
    if !can_heal(world, id) || !heal_target(world, id, a.target) {
        end_heal(world, scripts, id, true);
        return;
    }
    crate::combat::ensure_health(world, a.target);
    if heal_done(world, id, a.target) {
        end_heal(world, scripts, id, true);
        return;
    }
    let (Some(u), Some(t)) = (world.unit(id), world.unit(a.target)) else {
        return;
    };
    let (Some(bd), Some(td)) = (world.defs().get(u.def), world.defs().get(t.def)) else {
        return;
    };
    let (owner, pos, turnrate, workertime) = (u.owner, t.pos, bd.turnrate, bd.build.workertime);
    let in_range = placement::in_range(u.pos, bd, pos, td);
    if !in_range {
        if a.building {
            start(scripts, id, "StopBuilding");
            a.building = false;
        }
        let goal = world.map().clamp(pos);
        if a.walk_to != Some(goal) {
            if let Some(u) = world.unit_mut(id) {
                if a.walking
                    && a.walk_to
                        .is_some_and(|to| u.orders.front() == Some(&Order::Move { to }))
                {
                    u.orders.pop_front();
                }
                // Steering may use one extra slot; accepted tails survive.
                u.orders.push_front(Order::Move { to: goal });
                u.path = Default::default();
            }
            a.walk_to = Some(goal);
            a.walking = true;
        } else if !a.walking {
            // The unchanged target was unreachable: finish, preserving the
            // tail, instead of asking pathfinding to retry every tick.
            store_heal(world, id, a);
            end_heal(world, scripts, id, true);
            return;
        }
        store_heal(world, id, a);
        return;
    }
    if a.walking {
        if let Some(u) = world.unit_mut(id) {
            if a.walk_to
                .is_some_and(|to| u.orders.front() == Some(&Order::Move { to }))
            {
                u.orders.pop_front();
            }
            u.path = Default::default();
            u.speed = Fx::ZERO;
        }
    }
    a.walking = false;
    a.walk_to = None;
    if let Some(u) = world.unit_mut(id) {
        u.speed = Fx::ZERO;
    }
    if !a.building {
        start(scripts, id, "StartBuilding");
        a.building = true;
    }
    store_heal(world, id, a);
    face(world, id, pos, turnrate);
    if world.unit(a.target).is_some_and(|u| !u.build.is_complete()) {
        work(world, a.target, owner, workertime);
    } else {
        repair_step(world, a.target, owner, workertime);
    }
}

fn repair_step(world: &mut World, target: UnitId, payer: PlayerId, workertime: i32) {
    let Some(t) = world.unit(target) else { return };
    let Some(d) = world.defs().get(t.def) else {
        return;
    };
    let (max, missing) = (
        t.combat.max_health,
        t.combat.max_health.saturating_sub(t.combat.health),
    );
    if d.build.buildtime <= 0 || missing <= 0 {
        return;
    }
    let (total, cost) = (
        progress::total_work(d.build.buildtime),
        progress::cost_raw(d.build.buildcost),
    );
    let mut credit = t.build.repair.unwrap_or_default();
    let gain = credit.step(
        max,
        missing,
        total,
        cost,
        progress::worker_step(workertime),
        |want| economy::drain(world, payer, want),
    );
    if let Some(t) = world.unit_mut(target) {
        t.build.repair = Some(credit);
    }
    crate::combat::apply_healing(world, target, gain);
}

fn heal_finish(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    if let Some(j) = world.unit(id).and_then(|u| u.build.heal) {
        if heal_done(world, id, j.work.target) {
            end_heal(world, scripts, id, true);
        }
    }
}

/// Apply a `Build` command. Each listed unit must be the player's, complete,
/// a builder, and have `def` in its build menu. Mobile builders (`bmcode`)
/// take `at: Some` (snapped to the footprint grid) and honour the queue
/// mode; factories take `at: None` and always append (**Believed**: TA's
/// factory buttons queue behind the active order). Anything else is
/// dropped silently.
pub fn apply_command(world: &mut World, player: PlayerId, cmd: &SimCommand) {
    let SimCommand::Build {
        units,
        def,
        at,
        queue,
    } = cmd
    else {
        return;
    };
    let Some(target) = world.defs().get(*def).cloned() else {
        return;
    };
    for &id in units {
        let Some(u) = world.unit(id).filter(|u| u.owner == player) else {
            continue;
        };
        if !u.build.is_complete() {
            continue;
        }
        let Some(bd) = world.defs().get(u.def) else {
            continue;
        };
        let b = &bd.build;
        if !b.builder || !b.canbuild.contains(def) {
            continue;
        }
        let order = match (*at, b.mobile) {
            (Some(p), true) => {
                let p = world.map().clamp(p);
                Order::Build {
                    def: *def,
                    at: Some(placement::site_center(&target, p)),
                }
            }
            (None, false) => Order::Build {
                def: *def,
                at: None,
            },
            _ => continue,
        };
        let replace = b.mobile && *queue == QueueMode::Replace;
        if let Some(u) = world.unit_mut(id) {
            if replace {
                u.cancel_player_orders();
            }
            if u.orders.len() < MAX_ORDERS {
                u.orders.push_back(order);
            }
        }
    }
}

/// Whether `id` can accept explicit factory production/rally controls.
pub fn can_factory_control(world: &World, id: UnitId) -> bool {
    world.unit(id).is_some_and(|u| {
        u.build.is_complete()
            && !u.combat.is_dying()
            && world
                .defs()
                .get(u.def)
                .is_some_and(|d| d.build.builder && !d.build.mobile)
    })
}

/// Whether an eligible factory may record this exact Guard target. The
/// factory itself and allies are allowed; stale, dying or hostile targets
/// are not. Commanding-player ownership is checked separately at admission.
/// Each product rechecks the relationship against its own owner at release.
pub fn can_factory_guard_target(world: &World, factory: UnitId, target: UnitId) -> bool {
    can_factory_control(world, factory)
        && world
            .unit(factory)
            .is_some_and(|u| valid_guard_target(world, u.owner, target))
}

fn valid_guard_target(world: &World, owner: PlayerId, target: UnitId) -> bool {
    world.unit(target).is_some_and(|u| {
        !u.combat.is_dying()
            && (u.combat.max_health == 0 || u.combat.health > 0)
            && !world.are_hostile(owner, u.owner)
    })
}

/// Matching finite entries plus an active repeated product, if any.
/// The standing repeat recipe itself is not a finite entry.
/// Ineligible, dying, unfinished or stale factories report zero.
pub fn production_count(world: &World, id: UnitId, def: UnitDefId) -> u32 {
    if !can_factory_control(world, id) {
        return 0;
    }
    world.unit(id).map_or(0, |u| {
        let finite = u
            .orders
            .iter()
            .filter(|&&order| order == (Order::Build { def, at: None }))
            .count() as u32;
        let repeated = u.build.factory.product_from_repeat
            && !u.build.factory.cancel_product
            && u.build
                .factory
                .product
                .and_then(|p| world.unit(p))
                .is_some_and(|p| p.def == def);
        finite + u32::from(repeated)
    })
}

/// The continuous recipe of an eligible factory. Disabled or ineligible
/// factories report `None`; an active product can outlive this recipe.
pub fn factory_repeat(world: &World, id: UnitId) -> Option<UnitDefId> {
    can_factory_control(world, id)
        .then(|| world.unit(id).and_then(|u| u.build.factory_repeat))
        .flatten()
}

/// Apply explicit standing-order, repeat, or production-removal controls.
/// Production controls never edit standing rally orders. Tail-first removal
/// is **Chosen**: the manual does not specify removal priority.
pub fn apply_factory_command(world: &mut World, player: PlayerId, cmd: &SimCommand) {
    if !matches!(
        cmd,
        SimCommand::SetFactoryRally { .. }
            | SimCommand::CancelProduction { .. }
            | SimCommand::SetFactoryRepeat { .. }
    ) {
        return;
    }
    for &id in cmd.units() {
        if !can_factory_control(world, id) || !world.unit(id).is_some_and(|u| u.owner == player) {
            continue;
        }
        match cmd {
            SimCommand::SetFactoryRally { order, queue, .. } => {
                // Validate before Replace can clear a paid factory's previous
                // recipe, including its legacy None fallback state.
                if let Some(FactoryRallyOrder::Guard { target }) = order {
                    if !can_factory_guard_target(world, id, *target) {
                        continue;
                    }
                }
                let order = order.map(|order| match order {
                    FactoryRallyOrder::Move { to } => FactoryRallyOrder::Move {
                        to: world.map().clamp(to),
                    },
                    FactoryRallyOrder::Patrol { to } => FactoryRallyOrder::Patrol {
                        to: world.map().clamp(to),
                    },
                    FactoryRallyOrder::Guard { target } => FactoryRallyOrder::Guard { target },
                });
                let Some(u) = world.unit_mut(id) else {
                    continue;
                };
                let rally = u.build.factory_rally.get_or_insert_with(Vec::new);
                if *queue == QueueMode::Replace || order.is_none() {
                    rally.clear();
                }
                if let Some(order) = order {
                    let has_patrol = rally
                        .iter()
                        .any(|o| matches!(o, FactoryRallyOrder::Patrol { .. }))
                        || matches!(order, FactoryRallyOrder::Patrol { .. });
                    let has_guard = rally
                        .iter()
                        .any(|o| matches!(o, FactoryRallyOrder::Guard { .. }))
                        || matches!(order, FactoryRallyOrder::Guard { .. });
                    // Reserve the automatic return leg so a full recipe can
                    // never become a one-way patrol on a newly built unit.
                    // A Guard recipe also reserves one ordinary yard exit,
                    // even if today's first target/route will not need it.
                    let cost = rally.len() + 1 + usize::from(has_patrol) + usize::from(has_guard);
                    if cost <= MAX_ORDERS {
                        rally.push(order);
                    }
                }
            }
            SimCommand::SetFactoryRepeat { def, .. } => {
                if let Some(def) = def {
                    let valid = world.defs().get(*def).is_some()
                        && world
                            .unit(id)
                            .and_then(|u| world.defs().get(u.def))
                            .is_some_and(|d| d.build.canbuild.contains(def));
                    if !valid {
                        continue;
                    }
                }
                if let Some(u) = world.unit_mut(id) {
                    u.build.factory_repeat = *def;
                }
            }
            SimCommand::CancelProduction { def, count, .. } => {
                if *count == 0 {
                    continue;
                }
                let active_repeat_matches = world.unit(id).is_some_and(|u| {
                    u.build.factory.product_from_repeat
                        && !u.build.factory.cancel_product
                        && u.build
                            .factory
                            .product
                            .and_then(|p| world.unit(p))
                            .is_some_and(|p| p.def == *def)
                });
                let Some(u) = world.unit_mut(id) else {
                    continue;
                };
                if u.build.factory_repeat == Some(*def) {
                    u.build.factory_repeat = None;
                }
                let active = if u.build.factory.product.is_some()
                    && !u.build.factory.product_from_repeat
                    && !u.build.factory.cancel_product
                {
                    first_production(&u.orders).map(|(i, _)| i)
                } else {
                    None
                };
                let mut remaining = *count;
                // Descending indices remain valid after each removal. The
                // active entry is the first production entry, hence last.
                for i in (0..u.orders.len()).rev() {
                    if remaining == 0 {
                        break;
                    }
                    if u.orders[i]
                        == (Order::Build {
                            def: *def,
                            at: None,
                        })
                    {
                        u.orders.remove(i);
                        remaining -= 1;
                        if active == Some(i) {
                            u.build.factory.cancel_product = true;
                        }
                    }
                }
                // A repeated product owns no finite entry. Its cancellation
                // comes after all matching finite work, even if queued later.
                if remaining > 0 && active_repeat_matches {
                    u.build.factory.cancel_product = true;
                }
            }
            _ => unreachable!("factory command checked above"),
        }
    }
}

/// Runs right after commands, before economy.
///
/// 1. Frames are held: orders cleared, speed zero.
/// 2. Explicitly cancelled factory products are removed before any helper can
///    finish them. Other cancellation keeps its legacy worker ordering.
/// 3. Every complete builder, in slot order, advances its job (walk, place,
///    work) or its factory queue. Work is visible at once to later
///    builders, so the lowest-slot builder to finish a frame wins.
/// 4. Builders and factories whose frame completed this tick call
///    `StopBuilding` and move on; factory products walk out.
pub fn tick(world: &mut World, scripts: &mut dyn ScriptHost) {
    let ids: Vec<_> = world
        .unit_ids()
        .into_iter()
        .filter(|&id| {
            world
                .unit(id)
                .is_some_and(|u| !world.is_inert_player(u.owner))
        })
        .collect();
    for &id in &ids {
        if let Some(u) = world.unit_mut(id) {
            if u.build.is_complete() && u.combat.ready && u.combat.health >= u.combat.max_health {
                if let Some(r) = &mut u.build.repair {
                    r.hp_remainder = 0;
                }
            }
            if !u.build.is_complete() {
                u.orders.clear();
                u.speed = Fx::ZERO;
            }
        }
    }
    // A lower-slot Guard/Heal helper may otherwise finish a cancelled frame
    // before its factory gets a turn, erasing Frame::paid and bypassing the
    // refund/removal. Resolve only the new explicit cancellation here, so
    // legacy command logs keep their existing per-worker cancellation order.
    let cancelled: Vec<_> = ids
        .iter()
        .copied()
        .filter(|&id| {
            world
                .unit(id)
                .is_some_and(|u| u.build.factory.cancel_product)
        })
        .collect();
    for &id in &cancelled {
        factory_step(world, scripts, id);
    }
    for &id in &ids {
        // Retire the old explicit interval before another action starts its
        // callbacks; a replacing Build must not be stopped by stale Heal.
        sync_heal_job(world, scripts, id);
        reclaim::sync_job(world, scripts, id);
        match builder_kind(world, id) {
            Some(true) => {
                mobile_step(world, scripts, id);
                assist_step(world, scripts, id);
            }
            // As with ordinary cancellation, the next product starts on
            // the following tick, not during the cancellation/refund tick.
            Some(false) if cancelled.binary_search(&id).is_err() => {
                factory_step(world, scripts, id);
            }
            Some(false) | None => {}
        }
        // Conversely, Build/Guard cleanup precedes a newly activated Heal.
        heal_step(world, scripts, id);
        reclaim::step(world, scripts, id);
    }
    for &id in &ids {
        heal_finish(world, scripts, id);
        reclaim::finish(world, scripts, id);
        match builder_kind(world, id) {
            Some(true) => {
                mobile_finish(world, scripts, id);
                assist_finish(world, scripts, id);
            }
            Some(false) => factory_finish(world, scripts, id),
            None => {}
        }
    }
}

/// The frame a builder guarding `guarded` helps build (TA: a builder
/// ordered to guard assists): `guarded` itself while it is under
/// construction; else the frame `guarded` is building (a mobile builder's
/// job or assist, one level deep); else a factory's product on the pad.
/// Same owner only (no alliances yet). `None` when there is nothing to
/// build.
pub fn assist_target(world: &World, owner: PlayerId, guarded: UnitId) -> Option<UnitId> {
    let g = world.unit(guarded).filter(|g| g.owner == owner)?;
    if g.combat.is_dying() {
        return None;
    }
    let frame = |t: UnitId| {
        world
            .unit(t)
            .filter(|u| u.owner == owner && !u.build.is_complete() && !u.combat.is_dying())
            .map(|_| t)
    };
    if !g.build.is_complete() {
        return Some(guarded);
    }
    g.build
        .job
        .and_then(|j| j.target)
        .and_then(frame)
        .or_else(|| g.build.assist.and_then(|a| frame(a.target)))
        .or_else(|| g.build.factory.product.and_then(frame))
}

/// Guard-assist for a mobile builder without a build job (WP-P): walk into
/// build range of the [`assist_target`] and work on it with this builder's
/// `workertime`, paid by its owner through [`economy::drain`], with
/// `StartBuilding`/`StopBuilding` as for a build order. A player order
/// (anything but the guard) suspends it.
fn assist_step(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(u) = world.unit(id) else { return };
    let owner = u.owner;
    let mut assist = u.build.assist;
    let guarded = match u.combat.order {
        Some(crate::combat::CombatOrder::Guard(g)) if u.build.job.is_none() => Some(g),
        _ => None,
    };
    let walk_to = assist.filter(|a| a.walking).and_then(|a| a.walk_to);
    let mut own_walk = walk_to.is_some_and(|w| u.orders.front() == Some(&Order::Move { to: w }));
    let mut combat_walk = crate::combat::is_our_move(u);
    let busy = guarded.is_none() && !u.orders.is_empty() && !own_walk && !combat_walk;
    let target = guarded
        .filter(|_| !busy)
        .and_then(|g| assist_target(world, owner, g));

    // Lost or changed target: stop working on (and walking to) the old one.
    if let Some(a) = assist {
        if Some(a.target) != target {
            if a.building {
                start(scripts, id, "StopBuilding");
            }
            if own_walk {
                if let Some(u) = world.unit_mut(id) {
                    u.orders.pop_front();
                    if combat_walk {
                        u.combat.goal = None;
                    }
                }
                // This exact temporary move has been consumed. Do not infer
                // ownership again from coordinates: a queued player move may
                // name the same point. A new target needs fresh steering.
                own_walk = false;
                combat_walk = false;
            }
            assist = None;
        }
    }
    let Some(t) = target else {
        set_assist(world, id, None);
        return;
    };
    let mut a = assist.unwrap_or(Assist {
        target: t,
        walk_to: None,
        walking: false,
        building: false,
    });
    let (Some(u), Some(tu)) = (world.unit(id), world.unit(t)) else {
        return;
    };
    let tpos = tu.pos;
    let (Some(bd), Some(td)) = (world.defs().get(u.def), world.defs().get(tu.def)) else {
        return;
    };
    let in_range = placement::in_range(u.pos, bd, tpos, td);
    let (workertime, turnrate) = (bd.build.workertime, bd.turnrate);
    if !in_range {
        // Walk over; a walk that ended out of range is not repeated unless
        // the target moved (unreachable frames are left alone).
        let goal = world.map().clamp(tpos);
        if !own_walk && a.walk_to != Some(goal) {
            if let Some(u) = world.unit_mut(id) {
                if combat_walk {
                    u.orders.pop_front();
                }
                u.orders.push_front(Order::Move { to: goal });
                u.combat.goal = None;
            }
            a.walk_to = Some(goal);
            a.walking = true;
        }
        if a.building {
            start(scripts, id, "StopBuilding");
            a.building = false;
        }
        set_assist(world, id, Some(a));
        return;
    }
    if own_walk || combat_walk {
        if let Some(u) = world.unit_mut(id) {
            u.orders.pop_front();
            u.combat.goal = None;
        }
    }
    a.walk_to = None;
    a.walking = false;
    if !a.building {
        start(scripts, id, "StartBuilding");
        a.building = true;
    }
    set_assist(world, id, Some(a));
    face(world, id, tpos, turnrate);
    work(world, t, owner, workertime);
}

fn set_assist(world: &mut World, id: UnitId, a: Option<Assist>) {
    if let Some(u) = world.unit_mut(id) {
        u.build.assist = a;
    }
}

/// An assisted frame that completed (or vanished) this tick: `StopBuilding`
/// now; the guard picks up the next frame (e.g. the factory's next
/// product) on a later tick.
fn assist_finish(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(a) = world.unit(id).and_then(|u| u.build.assist) else {
        return;
    };
    let done = world.unit(a.target).is_none_or(|u| u.build.is_complete());
    if done && a.building {
        start(scripts, id, "StopBuilding");
        set_assist(world, id, None);
    }
}

/// `Some(mobile)` for a live, complete builder.
fn builder_kind(world: &World, id: UnitId) -> Option<bool> {
    let u = world.unit(id)?;
    let d = world.defs().get(u.def)?;
    (d.build.builder && u.build.is_complete()).then_some(d.build.mobile)
}

fn start(scripts: &mut dyn ScriptHost, id: UnitId, name: &str) {
    scripts.start_script(id, name, &[]);
}

/// Spawn a frame of `def` for `owner`.
fn spawn_frame(
    world: &mut World,
    scripts: &mut dyn ScriptHost,
    def: UnitDefId,
    owner: PlayerId,
    at: WorldPos,
) -> Option<UnitId> {
    let total = progress::total_work(world.defs().get(def)?.build.buildtime);
    let id = lifecycle::spawn_unit(world, scripts, def, owner, at, BUILD_HEADING)?;
    // `Create` first runs on the next script tick, so it already sees the
    // frame (port 17 = 100).
    if let Some(u) = world.unit_mut(id) {
        u.build.frame = Some(Frame::new(total));
    }
    Some(id)
}

/// One builder's work on a frame this tick, paid by `payer`. Returns
/// whether any work was done.
fn work(world: &mut World, target: UnitId, payer: PlayerId, workertime: i32) -> bool {
    let Some(u) = world.unit(target) else {
        return false;
    };
    let Some(mut frame) = u.build.frame else {
        return false;
    };
    let Some(cost) = world
        .defs()
        .get(u.def)
        .map(|d| progress::cost_raw(d.build.buildcost))
    else {
        return false;
    };
    let step = progress::worker_step(workertime);
    if step == 0 {
        return false;
    }
    let (to, want) = frame.plan(cost, step);
    let granted = if want > Fx::ZERO {
        economy::drain(world, payer, want)
    } else {
        Fx::ZERO
    };
    let before = frame.done;
    let added = frame.commit(cost, to, want, granted);
    if let Some(u) = world.unit_mut(target) {
        u.build.frame = (!frame.is_complete()).then_some(frame);
    }
    // Frame health follows progress (combat owns health).
    crate::combat::build_progress(world, target, before, frame.done, frame.total);
    added > 0
}

/// Index of the front `Build` order behind only `Move`s, if any.
fn front_build(
    orders: &std::collections::VecDeque<Order>,
) -> Option<(usize, UnitDefId, Option<WorldPos>)> {
    for (i, o) in orders.iter().enumerate() {
        match *o {
            Order::Move { .. } => continue,
            Order::Build { def, at } => return Some((i, def, at)),
            _ => return None,
        }
    }
    None
}

/// End a mobile builder's job: `StopBuilding` if owed, and optionally drop
/// its `Build` order (and the walk `Move`s in front of it).
fn end_job(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId, drop_order: bool) {
    let Some(u) = world.unit_mut(id) else { return };
    let Some(job) = u.build.job.take() else {
        return;
    };
    if drop_order {
        if let Some((i, _, _)) = front_build(&u.orders) {
            u.orders.drain(..=i);
        }
    }
    if job.building {
        start(scripts, id, "StopBuilding");
    }
}

/// A frame of `def` owned by `owner` standing exactly on `site`.
fn unit_at_site(world: &World, def: UnitDefId, owner: PlayerId, site: WorldPos) -> Option<UnitId> {
    world
        .units()
        .find(|(_, u)| u.def == def && u.owner == owner && u.pos == site)
        .map(|(id, _)| id)
}

fn mobile_step(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(u) = world.unit(id) else { return };
    // A queued Build behind an active Attack/Guard is not active yet,
    // even when combat has no temporary Move at the front while firing.
    if u.combat.order.is_some() {
        if u.build.job.is_some() {
            end_job(world, scripts, id, false);
        }
        return;
    }
    let owner = u.owner;
    let front = front_build(&u.orders);
    let job = u.build.job;

    // Is the current job still the front Build order?
    let current = match (job, front) {
        (Some(j), Some((i, def, Some(at)))) if def == j.def && at == j.site => {
            (i == 0 || j.walking).then_some(j)
        }
        _ => None,
    };
    let mut job = match current {
        Some(j) => j,
        None => {
            if job.is_some() {
                // Replaced or stopped: the frame stays where it is.
                end_job(world, scripts, id, false);
            }
            let Some(u) = world.unit(id) else { return };
            let Some(Order::Build {
                def,
                at: Some(site),
            }) = u.orders.front().copied()
            else {
                return;
            };
            BuilderJob {
                def,
                site,
                target: None,
                walking: false,
                building: false,
            }
        }
    };

    let (Some(u), defs) = (world.unit(id), world.defs()) else {
        return;
    };
    let (Some(bd), Some(td)) = (defs.get(u.def), defs.get(job.def)) else {
        return;
    };
    let in_range = placement::in_range(u.pos, bd, job.site, td);
    let leading_moves = front_build(&u.orders).map_or(0, |(i, _, _)| i);
    let workertime = bd.build.workertime;
    let turnrate = bd.turnrate;

    if !in_range {
        if leading_moves > 0 {
            store_job(world, id, job);
            return;
        }
        if job.walking {
            // Arrived (or was halted) and still out of range: give up.
            store_job(world, id, job);
            end_job(world, scripts, id, true);
            return;
        }
        let rest: Vec<Order> = world
            .unit(id)
            .map_or(Vec::new(), |u| u.orders.iter().copied().collect());
        pathfinding::go_to(world, id, job.site);
        if let Some(u) = world.unit_mut(id) {
            // The temporary steering move uses one extra slot; every
            // previously accepted player order must survive it.
            u.orders.extend(rest);
        }
        job.walking = true;
        store_job(world, id, job);
        return;
    }

    // In range: stop walking.
    if let Some(u) = world.unit_mut(id) {
        u.orders.drain(..leading_moves);
    }

    let target = job.target.filter(|&t| world.unit(t).is_some());
    if job.target.is_some() && target.is_none() {
        // The frame was destroyed: the order ends (**Believed**, TA drops
        // a build order whose target dies; rebuilding needs a new order).
        store_job(world, id, job);
        end_job(world, scripts, id, true);
        return;
    }
    let target = match target {
        Some(t) => t,
        None => match unit_at_site(world, job.def, owner, job.site) {
            // Assist an existing frame (or find it already finished).
            Some(t) => t,
            None => {
                if let Err(reason) = placement::check_site(world, job.def, job.site, None) {
                    let tick = world.tick();
                    world.construction.reject(Rejection {
                        tick,
                        builder: id,
                        player: owner,
                        def: job.def,
                        site: job.site,
                        reason,
                    });
                    store_job(world, id, job);
                    end_job(world, scripts, id, true);
                    return;
                }
                let Some(t) = spawn_frame(world, scripts, job.def, owner, job.site) else {
                    store_job(world, id, job);
                    end_job(world, scripts, id, true);
                    return;
                };
                t
            }
        },
    };
    job.target = Some(target);
    if !job.building {
        start(scripts, id, "StartBuilding");
        job.building = true;
    }
    store_job(world, id, job);
    face(world, id, job.site, turnrate);
    work(world, target, owner, workertime);
}

fn store_job(world: &mut World, id: UnitId, job: BuilderJob) {
    if let Some(u) = world.unit_mut(id) {
        u.build.job = Some(job);
    }
}

/// Turn a stationary builder toward its site by at most `turnrate`.
fn face(world: &mut World, id: UnitId, site: WorldPos, turnrate: u16) {
    let Some(u) = world.unit_mut(id) else { return };
    let d = site - u.pos;
    if d == Vec2::ZERO {
        return;
    }
    let err = i32::from(u.heading.delta_to(d.angle()));
    let tr = i32::from(turnrate);
    let turn = err.clamp(-tr, tr);
    u.heading = Angle::from_raw(u.heading.raw().wrapping_add(turn as i16 as u16));
}

fn mobile_finish(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(job) = world.unit(id).and_then(|u| u.build.job) else {
        return;
    };
    let Some(t) = job.target else { return };
    let done = world.unit(t).is_some_and(|u| u.build.is_complete());
    if done {
        end_job(world, scripts, id, true);
    }
}

/// Index and def of a factory's first production order.
fn first_production(orders: &std::collections::VecDeque<Order>) -> Option<(usize, UnitDefId)> {
    orders
        .iter()
        .take_while(|o| !matches!(o, Order::Clear { .. }))
        .enumerate()
        .find_map(|(i, o)| match *o {
            Order::Build { def, at: None } => Some((i, def)),
            _ => None,
        })
}

fn set_factory(world: &mut World, id: UnitId, f: FactoryState) {
    if let Some(u) = world.unit_mut(id) {
        u.build.factory = f;
    }
}

/// Lower every edge: `StopBuilding` (if a product was attached) then
/// `Deactivate`.
fn factory_idle(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId, had_product: bool) {
    let Some(mut f) = world.unit(id).map(|u| u.build.factory) else {
        return;
    };
    if had_product {
        start(scripts, id, "StopBuilding");
    }
    if f.active {
        start(scripts, id, "Deactivate");
    }
    f = FactoryState::default();
    set_factory(world, id, f);
}

/// End continuous production as its factory dies. Resolve a repeated frame
/// immediately: a chain explosion can remove the factory within this combat
/// pass, before the next construction tick. Legacy finite production keeps
/// its historical death handling and hashes.
pub(crate) fn cancel_repeat_on_death(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(u) = world.unit_mut(id) else { return };
    u.build.factory_repeat = None;
    if u.build.factory.product_from_repeat && u.build.factory.product.is_some() {
        u.build.factory.cancel_product = true;
        factory_step(world, scripts, id);
    }
}

fn factory_step(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(u) = world.unit(id) else { return };
    let owner = u.owner;
    let mut f = u.build.factory;
    let repeat_controlled = f.product_from_repeat || u.build.factory_repeat.is_some();
    let finite = first_production(&u.orders);
    let next = finite.map(|(_, def)| (def, false)).or_else(|| {
        u.build
            .factory_repeat
            .filter(|_| !u.orders.iter().any(|o| matches!(o, Order::Clear { .. })))
            .map(|def| (def, true))
    });
    let Some(fd) = world.defs().get(u.def) else {
        return;
    };
    let workertime = fd.build.workertime;

    if let Some(p) = f.product {
        let pu = match world.unit(p) {
            Some(pu) if !repeat_controlled || !pu.combat.is_dying() => pu,
            _ => {
                // Product destroyed: TA cancels all the factory's orders.
                // Repeat-controlled work must stop at lethal damage, before
                // one final construction step can detach a dying product.
                // Leave its death animation to combat; never refund a loss.
                if let Some(u) = world.unit_mut(id) {
                    u.orders.clear();
                    u.build.factory_repeat = None;
                }
                factory_idle(world, scripts, id, true);
                return;
            }
        };
        if (!f.product_from_repeat && finite.is_none()) || f.cancel_product {
            // Cancelled (Stop or a replacing order): refund what was spent
            // and remove the frame (**Known** TA cancel-current).
            if let Some(frame) = pu.build.frame {
                world.construction.refunds.push(Refund {
                    player: owner,
                    mana: frame.paid,
                });
                lifecycle::remove_unit(world, scripts, p);
            }
            factory_idle(world, scripts, id, true);
            return;
        }
        work(world, p, owner, workertime);
        return;
    }

    let Some((def, from_repeat)) = next else {
        if f.active {
            factory_idle(world, scripts, id, false);
        }
        return;
    };
    if !f.active {
        start(scripts, id, "Activate");
        f.active = true;
        set_factory(world, id, f);
    }
    // The yard-door handshake belongs to the script: wait for it to set
    // INBUILDSTANCE (units without a script skip the wait).
    if scripts.port_value(id, port::INBUILDSTANCE) == Some(0) {
        return;
    }
    if f.retry_at.is_some_and(|t| world.tick() < t) {
        return;
    }
    let exit = exit_spot(world, scripts, id);
    if !placement::exit_clear(world, def, exit, id) {
        f.retry_at = Some(Tick(world.tick().0.saturating_add(EXIT_RETRY_TICKS)));
        set_factory(world, id, f);
        return;
    }
    let fire_state = world.unit(id).map(|u| u.fire_state);
    let Some(p) = spawn_frame(world, scripts, def, owner, exit) else {
        f.retry_at = Some(Tick(world.tick().0.saturating_add(EXIT_RETRY_TICKS)));
        set_factory(world, id, f);
        return;
    };
    if let (Some(fs), Some(pu)) = (fire_state, world.unit_mut(p)) {
        pu.fire_state = fs;
    }
    f.product = Some(p);
    f.product_from_repeat = from_repeat;
    f.retry_at = None;
    set_factory(world, id, f);
    start(scripts, id, "StartBuilding");
    work(world, p, owner, workertime);
}

/// Where a factory puts its product: the `QueryBuildInfo` piece's static
/// model position if the script and model answer (**Known** TA: the query
/// answers a piece through `locals[0]`, pre-set to -1), else the centre of
/// the yard map's `c`/`C` cells, else the factory centre (**Chosen**).
fn exit_spot(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) -> WorldPos {
    let piece = lifecycle::call_script(world, scripts, id, "QueryBuildInfo", &[-1])
        .and_then(|r| r.locals.first().copied())
        .filter(|&p| p >= 0);
    let Some(u) = world.unit(id) else {
        return WorldPos::ZERO;
    };
    let (pos, heading) = (u.pos, u.heading);
    if let Some(o) = piece.and_then(|p| scripts.piece_origin(id, p)) {
        return world.map().clamp(model_to_world(pos, heading, o));
    }
    let Some(d) = world.defs().get(u.def) else {
        return pos;
    };
    let (mut sx, mut sz, mut n) = (0i32, 0i32, 0i32);
    for cz in 0..u32::from(d.footprintz) {
        for cx in 0..u32::from(d.footprintx) {
            if matches!(d.build.yard_cell(cx, cz, d.footprintx), b'c' | b'C') {
                sx += 2 * cx as i32 + 1;
                sz += 2 * cz as i32 + 1;
                n += 1;
            }
        }
    }
    if n == 0 {
        return pos;
    }
    let half = placement::SQUARE / 2;
    let off = Vec2::new(
        Fx::from_ratio(sx * half, n).unwrap_or(Fx::ZERO),
        Fx::from_ratio(sz * half, n).unwrap_or(Fx::ZERO),
    ) - Vec2::from_ints(
        i32::from(d.footprintx) * half,
        i32::from(d.footprintz) * half,
    );
    world.map().clamp(pos + off)
}

/// A model-space point (raw 16.16 `[x, y, z]`, 1 model unit = 1 world
/// unit, **Believed**) to the ground plane of a unit at `pos` facing
/// `heading`: model `-Z` is forward (as in `world.rs`), model `+X` is the
/// unit's right (**Chosen**).
pub fn model_to_world(pos: WorldPos, heading: Angle, o: [i32; 3]) -> WorldPos {
    let fwd = Vec2::from_angle(heading);
    let right = Vec2::new(-fwd.y, fwd.x);
    pos + fwd.scale(Fx::from_raw(o[2]).wrapping_neg()) + right.scale(Fx::from_raw(o[0]))
}

fn factory_finish(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(u) = world.unit(id) else { return };
    let f = u.build.factory;
    let Some(p) = f.product else { return };
    if !world.unit(p).is_some_and(|pu| pu.build.is_complete()) {
        return;
    }
    let standing = u.build.factory_rally.clone();
    let rally: Vec<WorldPos> = u
        .orders
        .iter()
        .filter_map(|o| match *o {
            Order::Move { to } => Some(to),
            _ => None,
        })
        .collect();
    let (fpos, fdef) = (u.pos, u.def);
    if !f.product_from_repeat {
        if let Some(u) = world.unit_mut(id) {
            if let Some((i, _)) = first_production(&u.orders) {
                u.orders.remove(i);
            }
        }
    }
    start(scripts, id, "StopBuilding");
    let mut f2 = f;
    f2.product = None;
    f2.product_from_repeat = false;
    set_factory(world, id, f2);
    if world
        .unit(id)
        .is_some_and(|u| first_production(&u.orders).is_none() && u.build.factory_repeat.is_none())
    {
        factory_idle(world, scripts, id, false);
    }
    match standing {
        Some(orders) if !orders.is_empty() => release_standing(world, p, fpos, fdef, &orders),
        Some(_) => release(world, p, fpos, fdef, &[]),
        None => release(world, p, fpos, fdef, &rally),
    }
}

/// Snapshot the factory's recipe at completion, not at build start. Patrol
/// uses the same return-leg rule as direct unit commands, so Move→Move→Patrol
/// traverses the two moves once, then patrols from the last move onward.
fn release_standing(
    world: &mut World,
    p: UnitId,
    fpos: WorldPos,
    fdef: UnitDefId,
    orders: &[FactoryRallyOrder],
) {
    let Some(owner) = world.unit(p).map(|u| u.owner) else {
        return;
    };
    // Only this new target-bearing recipe path filters or adds egress.
    // Move/Patrol-only routes retain their exact original behavior/hashes.
    let effective: Vec<_> = orders
        .iter()
        .copied()
        .filter(|order| match *order {
            FactoryRallyOrder::Guard { target } => {
                target != p && valid_guard_target(world, owner, target)
            }
            _ => true,
        })
        .collect();
    if effective.is_empty() || matches!(effective.first(), Some(FactoryRallyOrder::Guard { .. })) {
        // Chosen: always leave the pad before a leading Guard, even when
        // its target is far away now. A nearby/self-factory target or one
        // that dies immediately after release must not suppress egress.
        // This is the existing bounded pathfinding behavior, not a special
        // movement/combat lifetime rule; impassable exits may still fail.
        release(world, p, fpos, fdef, &[]);
    }
    for order in effective {
        match order {
            FactoryRallyOrder::Move { to } => {
                if let Some(u) = world.unit_mut(p) {
                    if u.orders.len() < MAX_ORDERS {
                        u.orders.push_back(Order::Move { to });
                    }
                }
            }
            FactoryRallyOrder::Patrol { to } => pathfinding::apply_command(
                world,
                owner,
                &SimCommand::Patrol {
                    units: UnitSet::single(p),
                    to,
                    queue: QueueMode::Append,
                },
            ),
            FactoryRallyOrder::Guard { target } => crate::combat::apply_command(
                world,
                owner,
                &SimCommand::Guard {
                    units: UnitSet::single(p),
                    target,
                    queue: QueueMode::Append,
                },
            ),
        }
    }
}

/// Send a finished product on its way: the factory's `Move` orders as a
/// rally (**Known** TA: the product copies the factory's queued moves), or
/// out of the yard past the footprint edge (**Chosen**).
fn release(world: &mut World, p: UnitId, fpos: WorldPos, fdef: UnitDefId, rally: &[WorldPos]) {
    if let Some((&first, rest)) = rally.split_first() {
        pathfinding::go_to(world, p, first);
        if let Some(u) = world.unit_mut(p) {
            for &to in rest {
                if u.orders.len() < MAX_ORDERS {
                    u.orders.push_back(Order::Move { to });
                }
            }
        }
        return;
    }
    let (Some(pu), Some(fd)) = (world.unit(p), world.defs().get(fdef)) else {
        return;
    };
    let d = pu.pos - fpos;
    let hx = i32::from(fd.footprintx) * placement::SQUARE / 2 + EXIT_CLEARANCE;
    let hz = i32::from(fd.footprintz) * placement::SQUARE / 2 + EXIT_CLEARANCE;
    let out = if d.x.abs() > d.y.abs() {
        let sx = if d.x < Fx::ZERO { -hx } else { hx };
        Vec2::new(fpos.x + Fx::from_int(sx), pu.pos.y)
    } else {
        // Straight out of the yard; south when the product sits on the
        // centre (the side TA structures face).
        let sz = if d.y < Fx::ZERO { -hz } else { hz };
        Vec2::new(pu.pos.x, fpos.y + Fx::from_int(sz))
    };
    pathfinding::go_to(world, p, out);
}

/// Stop surviving builders after the default defeat sweep, without refunding
/// or advancing their remaining frames. They no longer own active work.
pub(crate) fn halt_defeated_unit(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    cancel_heal(world, scripts, id);
    reclaim::cancel(world, scripts, id);
    let Some(u) = world.unit_mut(id) else { return };
    let building = u.build.job.take().is_some_and(|j| j.building)
        | u.build.assist.take().is_some_and(|a| a.building);
    let product = u.build.factory.product.is_some();
    u.build.factory_repeat = None;
    if building {
        start(scripts, id, "StopBuilding");
    }
    factory_idle(world, scripts, id, product);
}
