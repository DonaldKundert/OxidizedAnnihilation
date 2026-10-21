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

pub use build_def::{BuildDef, MAX_BUILD_OPTIONS, YARD_CHARS, build_def_from_fbi, expand_yardmap};
pub use progress::Frame;

use tak_commands::{PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId};
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
    /// `Activate` was called and `Deactivate` is still owed.
    pub active: bool,
    /// Do not look at the exit spot before this tick.
    pub retry_at: Option<Tick>,
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

/// World-level construction state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConstructionState {
    /// Refunds not yet credited by the economy.
    pub refunds: Vec<Refund>,
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
                u.orders.clear();
            }
            if u.orders.len() < MAX_ORDERS {
                u.orders.push_back(order);
            }
        }
    }
}

/// Runs right after commands, before economy.
///
/// 1. Frames are held: orders cleared, speed zero.
/// 2. Every complete builder, in slot order, advances its job (walk, place,
///    work) or its factory queue. Work is visible at once to later
///    builders, so the lowest-slot builder to finish a frame wins.
/// 3. Builders and factories whose frame completed this tick call
///    `StopBuilding` and move on; factory products walk out.
pub fn tick(world: &mut World, scripts: &mut dyn ScriptHost) {
    let ids = world.unit_ids();
    for &id in &ids {
        if let Some(u) = world.unit_mut(id) {
            if !u.build.is_complete() {
                u.orders.clear();
                u.speed = Fx::ZERO;
            }
        }
    }
    for &id in &ids {
        match builder_kind(world, id) {
            Some(true) => mobile_step(world, scripts, id),
            Some(false) => factory_step(world, scripts, id),
            None => {}
        }
    }
    for &id in &ids {
        match builder_kind(world, id) {
            Some(true) => mobile_finish(world, scripts, id),
            Some(false) => factory_finish(world, scripts, id),
            None => {}
        }
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
    let added = frame.commit(cost, to, want, granted);
    if let Some(u) = world.unit_mut(target) {
        u.build.frame = (!frame.is_complete()).then_some(frame);
    }
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
            for o in rest {
                if u.orders.len() < MAX_ORDERS {
                    u.orders.push_back(o);
                }
            }
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
    let target = match target {
        Some(t) => t,
        None => match unit_at_site(world, job.def, owner, job.site) {
            // Assist an existing frame (or find it already finished).
            Some(t) => t,
            None => {
                if placement::check_site(world, job.def, job.site, None).is_err() {
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
    orders.iter().enumerate().find_map(|(i, o)| match *o {
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

fn factory_step(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) {
    let Some(u) = world.unit(id) else { return };
    let owner = u.owner;
    let mut f = u.build.factory;
    let next = first_production(&u.orders);
    let Some(fd) = world.defs().get(u.def) else {
        return;
    };
    let workertime = fd.build.workertime;

    if let Some(p) = f.product {
        let Some(pu) = world.unit(p) else {
            // Product destroyed: TA cancels all the factory's orders.
            if let Some(u) = world.unit_mut(id) {
                u.orders.clear();
            }
            factory_idle(world, scripts, id, true);
            return;
        };
        if next.is_none() {
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

    let Some((_, def)) = next else {
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
    let rally: Vec<WorldPos> = u
        .orders
        .iter()
        .filter_map(|o| match *o {
            Order::Move { to } => Some(to),
            _ => None,
        })
        .collect();
    let (fpos, fdef) = (u.pos, u.def);
    if let Some(u) = world.unit_mut(id) {
        if let Some((i, _)) = first_production(&u.orders) {
            u.orders.remove(i);
        }
    }
    start(scripts, id, "StopBuilding");
    let mut f2 = f;
    f2.product = None;
    set_factory(world, id, f2);
    if world
        .unit(id)
        .is_some_and(|u| first_production(&u.orders).is_none())
    {
        factory_idle(world, scripts, id, false);
    }
    release(world, p, fpos, fdef, &rally);
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
