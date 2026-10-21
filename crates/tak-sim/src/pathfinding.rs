//! Pathfinding and movement support (WP-L). Full description, every
//! Believed/Chosen value and the comparison with original TA:
//! `docs/pathfinding.md`.
//!
//! - **Layers** ([`grid`]): per movement class (footprint plus terrain
//!   limits), one tier per footprint anchor on the 16-px attribute-cell
//!   grid: blocked, steep (passable, costs more) or clear. Stamped once per
//!   match, on the first tick. Flyers and maps without terrain have none.
//! - **Search** ([`search`]): deterministic A* with TA's integer costs,
//!   one search at a time, under a fixed per-tick step budget shared by
//!   every unit (requests are admitted in slot order from a rotating
//!   cursor). Suspended searches resume next tick.
//! - **Following**: the route is smoothed by grid line of sight into a few
//!   waypoints; [`crate::systems::movement`] steers along them. Stalled
//!   units replan, at most every [`REPATH_INTERVAL`] ticks, and give up
//!   after [`MAX_REPLANS`].
//! - Unreachable goals: the unit walks to the reachable point nearest the
//!   goal and drops the order with [`Outcome::Unreachable`].
//!
//! Other systems ask for movement through [`go_to`].

mod grid;
mod search;

use tak_commands::{PlayerId, QueueMode, SimCommand, UnitDefId, UnitId, WorldPos};
use tak_math::{Fx, Vec2};

use crate::defs::UnitDef;
use crate::hash::StateHasher;
use crate::map::terrain;
use crate::script::ScriptHost;
use crate::systems::ARRIVAL_TOLERANCE;
use crate::world::{MAX_ORDERS, Order, Unit, World};

pub use grid::{CellRect, ClassGrid, ClassKey};

// --- Tunables: every number lives here. Tags: docs/pathfinding.md. ---

/// Search work per tick, shared by all units (TA-derived: 1333).
pub const STEPS_PER_TICK: u32 = 1333;
/// Charged when a request is admitted (TA-derived: 100).
pub const ADMISSION_COST: u32 = 100;
/// A unit re-requests a path at most this often (TA-derived: 60 ticks).
pub const REPATH_INTERVAL: u32 = 60;
/// Replans without progress before an order is abandoned (Chosen).
pub const MAX_REPLANS: u8 = 3;
/// Cost of a cardinal and a diagonal step (TA-derived: 16, 22).
pub const STEP_COST: [u32; 2] = [16, 22];
/// Penalty by direction change in eighths of a turn (TA-derived).
pub const TURN_PENALTY: [u32; 8] = [0, 40, 60, 80, 100, 80, 60, 40];
/// Extra cost of stepping onto a steep-tier anchor (TA-derived: 30).
pub const STEEP_COST: u32 = 30;
/// Turning again within this many straight steps costs
/// [`SHORT_RUN_PENALTY`] more (TA-derived: 5, 75).
pub const SHORT_RUN: u8 = 5;
/// See [`SHORT_RUN`].
pub const SHORT_RUN_PENALTY: u32 = 75;
/// Octile goal radius subtracted in the heuristic (TA-derived: 4).
pub const HEURISTIC_RADIUS: u32 = 4;
/// Heuristic weight as a ratio (TA-derived: base 1.5 times a per-player
/// tier of 6, 3 or 1, so 9, 4.5 or 1.5).
pub const HEURISTIC_WEIGHT: (u32, u32) = (9, 2);
/// A waypoint is consumed within this distance (TA-derived: 5 world
/// units, inclusive).
pub const WAYPOINT_RADIUS: i32 = 5;
/// Pure-pursuit lookahead: beyond this distance from its waypoint a unit
/// aims at a point dragged back along the incoming leg (TA-derived: 80).
pub const LOOKAHEAD: Fx = Fx::from_int(80);
/// A stalled unit this close to its goal counts as arrived (Chosen; lets
/// a group share one destination).
pub const CROWD_RADIUS: Fx = Fx::from_int(48);
/// Most separation push per unit per tick (Chosen).
pub const SEPARATION_MAX_PUSH: Fx = Fx::ONE;
/// Default `GROUND2` limits for units without movement data (Known:
/// `moveinfo.tdf` CLASS0).
const DEFAULT_MAX_SLOPE: u8 = 30;
const DEFAULT_MAX_WATER_DEPTH: i32 = 20;
/// TA's movement-class template for keys a class omits (TA-derived:
/// unlimited slopes and depths).
const TEMPLATE_SLOPE: i32 = 255;
const TEMPLATE_MAX_DEPTH: i32 = 10_000;
const TEMPLATE_MIN_DEPTH: i32 = -10_000;

/// Pathing-relevant unit data, from the FBI and its movement class.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Motion {
    /// FBI `canfly`: ignores terrain and is never pathed.
    pub canfly: bool,
    /// FBI `canhover` or `floater`: no water slowdown without a
    /// `watermultiplier`.
    pub hover: bool,
    /// Pathing footprint in cells; 0 means the unit's FBI footprint.
    pub footprint_x: u8,
    /// See `footprint_x`.
    pub footprint_z: u8,
    /// Land cells whose corner spread exceeds this are blocked.
    pub max_slope: u8,
    /// Spread above this (and not above `max_slope`) is the steep tier.
    pub bad_slope: u8,
    /// `max_slope` for cells below sea level.
    pub max_water_slope: u8,
    /// `bad_slope` for cells below sea level.
    pub bad_water_slope: u8,
    /// Blocked where the lowest corner is deeper than this below sea level.
    pub max_water_depth: i32,
    /// Blocked where the highest corner is above `sea - min_water_depth`
    /// (boats).
    pub min_water_depth: i32,
    /// FBI `roadmultiplier`: speed factor on paved cells.
    pub road_multiplier: Option<Fx>,
    /// FBI `watermultiplier`: speed factor in water.
    pub water_multiplier: Option<Fx>,
}

impl Default for Motion {
    /// A `GROUND2` walker using its FBI footprint.
    fn default() -> Motion {
        Motion {
            canfly: false,
            hover: false,
            footprint_x: 0,
            footprint_z: 0,
            max_slope: DEFAULT_MAX_SLOPE,
            bad_slope: DEFAULT_MAX_SLOPE >> 1,
            max_water_slope: DEFAULT_MAX_SLOPE,
            bad_water_slope: DEFAULT_MAX_SLOPE >> 1,
            max_water_depth: DEFAULT_MAX_WATER_DEPTH,
            min_water_depth: TEMPLATE_MIN_DEPTH,
            road_multiplier: None,
            water_multiplier: None,
        }
    }
}

impl Motion {
    /// From an FBI and its resolved movement class. TA rules
    /// (Nanolathe `02` §5, `04` §6.1): a resolved class supplies the
    /// footprint and limits; otherwise the FBI's own keys do. Omitted keys
    /// take TA's template (slopes 255, depths ±10000), `badslope` defaults
    /// to half the max, then the max water slope caps the max slope and
    /// each max caps its bad value.
    pub fn from_fbi(u: &tak_content::UnitDef, class: Option<&tak_content::MoveClassDef>) -> Motion {
        let extra = |k: &str| u.extra.get(k).and_then(|v| v.trim().parse::<i32>().ok());
        let (fx, fz, ms, bs, mws, bws, mwd, mnd) = match class {
            Some(c) => (
                c.footprint_x.unwrap_or(0),
                c.footprint_z.unwrap_or(0),
                c.max_slope,
                c.bad_slope,
                c.max_water_slope,
                c.bad_water_slope,
                c.max_water_depth,
                c.min_water_depth,
            ),
            None => (
                0,
                0,
                u.motion.maxslope,
                extra("badslope"),
                extra("maxwaterslope"),
                extra("badwaterslope"),
                u.motion.maxwaterdepth,
                extra("minwaterdepth"),
            ),
        };
        let byte = |v: i32| v.clamp(0, 255) as u8;
        let ms = byte(ms.unwrap_or(TEMPLATE_SLOPE));
        let mws = byte(mws.unwrap_or(TEMPLATE_SLOPE));
        let bs = bs.map_or(ms >> 1, byte);
        let bws = bws.map_or(mws >> 1, byte);
        let ms = ms.min(mws);
        Motion {
            canfly: u.has_flag("canfly"),
            hover: u.has_flag("canhover") || u.has_flag("floater"),
            footprint_x: fx,
            footprint_z: fz,
            max_slope: ms,
            bad_slope: bs.min(ms),
            max_water_slope: mws,
            bad_water_slope: bws.min(mws),
            max_water_depth: mwd.unwrap_or(TEMPLATE_MAX_DEPTH),
            min_water_depth: mnd.unwrap_or(TEMPLATE_MIN_DEPTH),
            road_multiplier: u.motion.roadmultiplier,
            water_multiplier: u.motion.watermultiplier,
        }
    }

    /// The layer key for a unit with this motion and FBI footprint.
    pub fn key(&self, def: &UnitDef) -> ClassKey {
        let pick = |m: u8, f: u8| if m > 0 { m } else { f.max(1) };
        ClassKey {
            fx: pick(self.footprint_x, def.footprintx),
            fz: pick(self.footprint_z, def.footprintz),
            max_slope: self.max_slope,
            bad_slope: self.bad_slope,
            max_water_slope: self.max_water_slope,
            bad_water_slope: self.bad_water_slope,
            max_water_depth: self.max_water_depth,
            min_water_depth: self.min_water_depth,
        }
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bool(self.canfly);
        h.bool(self.hover);
        h.u8(self.footprint_x);
        h.u8(self.footprint_z);
        h.u8(self.max_slope);
        h.u8(self.bad_slope);
        h.u8(self.max_water_slope);
        h.u8(self.bad_water_slope);
        h.i32(self.max_water_depth);
        h.i32(self.min_water_depth);
        h.option(self.road_multiplier, StateHasher::fx);
        h.option(self.water_multiplier, StateHasher::fx);
    }
}

/// How a unit's last move order ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    /// No order finished yet (or a new one is running).
    #[default]
    None,
    /// Reached its goal (within [`ARRIVAL_TOLERANCE`], or stalled within
    /// [`CROWD_RADIUS`] of it).
    Arrived,
    /// The goal could not be reached; the unit stopped at the reachable
    /// point nearest to it.
    Unreachable,
    /// No progress after [`MAX_REPLANS`] replans.
    GaveUp,
}

/// Where a unit's path stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathStatus {
    /// No move order at the front of the queue.
    #[default]
    Idle,
    /// Waiting for the planner (the unit brakes meanwhile).
    Pending,
    /// The planner's active search belongs to this unit.
    Searching,
    /// Steering along `waypoints`.
    Following,
}

/// Per-unit pathfinding state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitPath {
    /// The order target this path serves.
    pub goal: Option<WorldPos>,
    /// Planner state.
    pub status: PathStatus,
    /// Remaining waypoints; the first is being steered to, the last is
    /// the end of the route.
    pub waypoints: Vec<WorldPos>,
    /// The waypoint just left (start of the current leg).
    pub prev: WorldPos,
    /// The route ends short of the goal (goal unreachable).
    pub unreachable: bool,
    /// The last step was refused by terrain.
    pub blocked: bool,
    /// Tick of the last path request.
    pub last_request: u32,
    /// Closest distance to the current waypoint so far.
    pub best: Fx,
    /// Ticks without getting closer.
    pub stall: u16,
    /// Replans for this order.
    pub replans: u8,
    /// How the last order ended.
    pub outcome: Outcome,
}

impl UnitPath {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"P");
        h.option(self.goal, StateHasher::vec2);
        h.u8(self.status as u8);
        h.len(self.waypoints.len());
        for w in &self.waypoints {
            h.vec2(*w);
        }
        h.vec2(self.prev);
        h.bool(self.unreachable);
        h.bool(self.blocked);
        h.u32(self.last_request);
        h.fx(self.best);
        h.u16(self.stall);
        h.u8(self.replans);
        h.u8(self.outcome as u8);
    }

    fn reset(&mut self) {
        let (outcome, last_request) = (self.outcome, self.last_request);
        *self = UnitPath {
            outcome,
            last_request,
            ..UnitPath::default()
        };
    }

    fn follow(&mut self, pos: WorldPos, waypoints: Vec<WorldPos>, unreachable: bool) {
        self.status = PathStatus::Following;
        self.waypoints = waypoints;
        self.prev = pos;
        self.unreachable = unreachable;
        self.blocked = false;
        self.best = Fx::MAX;
        self.stall = 0;
    }
}

/// Planner counters (deterministic, hashed).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathStats {
    /// Steps spent on the last tick (never above [`STEPS_PER_TICK`]).
    pub steps_last_tick: u32,
    /// Most steps spent in one tick.
    pub max_steps_tick: u32,
    /// Requests admitted.
    pub requests: u32,
    /// Requests answered by a straight line (no A*).
    pub direct: u32,
    /// A* searches completed.
    pub searches: u32,
    /// Total heap pops.
    pub pops: u64,
    /// Requests whose goal was unreachable.
    pub unreachable: u32,
}

/// The active search and whose it is.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Active {
    unit: UnitId,
    grid: u16,
    goal_pos: WorldPos,
    prefix: Vec<WorldPos>,
    target: WorldPos,
    unreachable: bool,
    search: search::Search,
}

/// World-level pathfinding state.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PathState {
    built: bool,
    grids: Vec<ClassGrid>,
    /// Layer per unit definition (`None`: flyer, or no terrain).
    def_grid: Vec<Option<u16>>,
    active: Option<Active>,
    scratch: search::Scratch,
    /// Next slot the planner polls.
    cursor: u32,
    /// Counters.
    pub stats: PathStats,
    /// Dynamic obstacles: blockers per attribute cell, row-major over the
    /// map's terrain grid (empty until the first [`add_obstacle`]).
    obstacles: Vec<u16>,
}

impl core::fmt::Debug for PathState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PathState")
            .field("built", &self.built)
            .field(
                "grids",
                &self.grids.iter().map(|g| g.key).collect::<Vec<_>>(),
            )
            .field(
                "active",
                &self.active.as_ref().map(|a| (a.unit, a.search.pops)),
            )
            .field("cursor", &self.cursor)
            .field("stats", &self.stats)
            .finish()
    }
}

impl PathState {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"PATH");
        // Layers derive from the map and defs (already in the static
        // digest); their keys and the def mapping are enough.
        h.bool(self.built);
        h.len(self.grids.len());
        for g in &self.grids {
            let k = g.key;
            h.bytes(&[
                k.fx,
                k.fz,
                k.max_slope,
                k.bad_slope,
                k.max_water_slope,
                k.bad_water_slope,
            ]);
            h.i32(k.max_water_depth);
            h.i32(k.min_water_depth);
        }
        h.len(self.def_grid.len());
        for g in &self.def_grid {
            h.option(*g, StateHasher::u16);
        }
        // A search's working set is a pure function of its seed and pop
        // count, so those identify it.
        h.option(self.active.as_ref(), |h, a| {
            h.unit_id(a.unit);
            h.u16(a.grid);
            h.vec2(a.goal_pos);
            h.len(a.prefix.len());
            for p in &a.prefix {
                h.vec2(*p);
            }
            h.vec2(a.target);
            h.bool(a.unreachable);
            h.u32(a.search.start);
            h.u32(a.search.goal);
            h.u32(a.search.pops);
        });
        h.u32(self.cursor);
        let s = &self.stats;
        h.u32(s.steps_last_tick);
        h.u32(s.max_steps_tick);
        h.u32(s.requests);
        h.u32(s.direct);
        h.u32(s.searches);
        h.u64(s.pops);
        h.u32(s.unreachable);
        // Sparse: (index, count) of every blocked cell.
        let n = self.obstacles.iter().filter(|&&c| c > 0).count();
        h.len(n);
        for (i, &c) in self.obstacles.iter().enumerate() {
            if c > 0 {
                h.u32(i as u32);
                h.u16(c);
            }
        }
    }

    /// Dynamic blockers on attribute cell `(cx, cz)` (0 outside the map or
    /// on maps without terrain).
    pub fn obstacles_at(&self, cx: i32, cz: i32, cols: u32) -> u16 {
        if cx < 0 || cz < 0 || cx as u32 >= cols {
            return 0;
        }
        let i = cz as usize * cols as usize + cx as usize;
        self.obstacles.get(i).copied().unwrap_or(0)
    }

    /// The layer for a unit definition, once built (`None` for flyers and
    /// maps without terrain, which move freely).
    pub fn grid_for(&self, def: UnitDefId) -> Option<&ClassGrid> {
        let g = (*self.def_grid.get(def.0 as usize)?)?;
        self.grids.get(usize::from(g))
    }

    /// `true` once the layers exist (after the first pathfinding tick).
    pub fn is_built(&self) -> bool {
        self.built
    }

    fn ensure_built(&mut self, world: &World) {
        if self.built {
            return;
        }
        self.built = true;
        let Some(t) = world.map().terrain.as_ref() else {
            self.def_grid = vec![None; world.defs().len()];
            return;
        };
        for def in world.defs().as_slice() {
            if def.motion.canfly {
                self.def_grid.push(None);
                continue;
            }
            let key = def.motion.key(def);
            let i = match self.grids.iter().position(|g| g.key == key) {
                Some(i) => i,
                None => {
                    self.grids
                        .push(ClassGrid::build_with(key, world.map(), t, &self.obstacles));
                    self.grids.len() - 1
                }
            };
            self.def_grid.push(Some(i as u16));
        }
    }
}

/// Apply a `Patrol` command. Replace: the unit patrols between its
/// position and `to`. Append: `to` joins the patrol route; if the queue
/// had no patrol legs yet, a return leg to where the queue ends is added
/// too. Legs rotate to the back of the queue when reached (Chosen).
pub fn apply_command(world: &mut World, player: PlayerId, cmd: &SimCommand) {
    let SimCommand::Patrol { units, to, queue } = cmd else {
        return;
    };
    let to = world.map().clamp(*to);
    for &id in units {
        let Some(u) = world.unit_mut_owned(id, player) else {
            continue;
        };
        if *queue == QueueMode::Replace {
            u.orders.clear();
        }
        let had_patrol = u.orders.iter().any(|o| matches!(o, Order::Patrol { .. }));
        let back_to = u
            .orders
            .iter()
            .rev()
            .find_map(|o| match o {
                Order::Move { to } | Order::Patrol { to } => Some(*to),
                _ => None,
            })
            .unwrap_or(u.pos);
        if u.orders.len() < MAX_ORDERS {
            u.orders.push_back(Order::Patrol { to });
        }
        if !had_patrol && u.orders.len() < MAX_ORDERS {
            u.orders.push_back(Order::Patrol { to: back_to });
        }
    }
}

/// Send a unit toward a point, replacing its current orders. Used by combat
/// (closing to range) and construction (walking to a site). The planner
/// picks the new goal up on its next tick.
pub fn go_to(world: &mut World, unit: UnitId, to: WorldPos) {
    let to = world.map().clamp(to);
    if let Some(u) = world.unit_mut(unit) {
        u.orders.clear();
        u.orders.push_back(Order::Move { to });
    }
}

// --- Dynamic obstacles (WP-P) ---
//
// The hook other lanes use for things that block movement after the map
// is loaded (blocking features such as wrecks; later, structures). Each
// call adds or removes one blocker on every cell of a rectangle; a cell is
// blocked while its count is non-zero, exactly like a `FEATURE` cell of
// the map. Every built layer is re-stamped around the rectangle at once,
// so movement sees the change in the same tick; layers built later include
// it. Units already standing there can still walk out (the step rule lets
// a unit on a blocked anchor move to any point whose own cell is open);
// followers whose path crosses it stall and replan.

/// Add one blocker to every cell of `r` (clipped to the map). No effect on
/// maps without a terrain grid (where nothing blocks).
pub fn add_obstacle(world: &mut World, r: CellRect) {
    change_obstacle(world, r, true);
}

/// Remove one blocker from every cell of `r` (the inverse of
/// [`add_obstacle`]; counts never go below zero).
pub fn remove_obstacle(world: &mut World, r: CellRect) {
    change_obstacle(world, r, false);
}

/// `true` if attribute cell `(cx, cz)` holds a dynamic obstacle.
pub fn is_obstacle(world: &World, cx: i32, cz: i32) -> bool {
    let cols = world.map().terrain.as_ref().map_or(0, |t| t.cols);
    world.pathing.obstacles_at(cx, cz, cols) > 0
}

fn change_obstacle(world: &mut World, r: CellRect, add: bool) {
    let Some(t) = world.map().terrain.as_ref() else {
        return;
    };
    let (cols, rows) = (t.cols, t.rows);
    let (x0, x1, z0, z1) = r.clip(cols, rows);
    if x0 >= x1 || z0 >= z1 {
        return;
    }
    let mut ps = std::mem::take(&mut world.pathing);
    let n = cols as usize * rows as usize;
    if ps.obstacles.len() != n {
        ps.obstacles = vec![0; n];
    }
    for z in z0..z1 {
        for x in x0..x1 {
            let c = &mut ps.obstacles[z as usize * cols as usize + x as usize];
            *c = if add {
                c.saturating_add(1)
            } else {
                c.saturating_sub(1)
            };
        }
    }
    if let Some(t) = world.map().terrain.as_ref() {
        let obstacles = std::mem::take(&mut ps.obstacles);
        for g in &mut ps.grids {
            g.restamp(world.map(), t, &obstacles, r);
        }
        ps.obstacles = obstacles;
    }
    world.pathing = ps;
}

/// Runs after orders, before movement: builds the layers on the first
/// call, tracks each unit's goal, detects stalls, finishes unreachable
/// orders and runs the planner under the step budget.
pub fn tick(world: &mut World, _scripts: &mut dyn ScriptHost) {
    let mut ps = std::mem::take(&mut world.pathing);
    ps.ensure_built(world);
    let now = world.tick().0;
    for id in world.unit_ids() {
        sync_unit(world, &ps, id, now);
    }
    plan(world, &mut ps, now);
    world.pathing = ps;
}

/// The move target at the front of a unit's queue.
fn front_target(u: &Unit) -> Option<WorldPos> {
    match u.orders.front() {
        Some(Order::Move { to }) | Some(Order::Patrol { to }) => Some(*to),
        _ => None,
    }
}

/// Finish the front move order with `outcome` (a patrol leg rotates to
/// the back instead of being dropped).
fn finish(u: &mut Unit, outcome: Outcome) {
    match u.orders.pop_front() {
        Some(o @ Order::Patrol { .. }) => u.orders.push_back(o),
        Some(_) if u.orders.is_empty() => u.speed = Fx::ZERO,
        _ => {}
    }
    u.path.reset();
    u.path.outcome = outcome;
}

/// Ticks without progress before a unit counts as stalled: two seconds
/// plus the time to turn half a circle (Chosen).
fn stall_window(def: &UnitDef) -> u16 {
    let turn = if def.turnrate == 0 {
        540
    } else {
        32768 / u32::from(def.turnrate)
    };
    (60 + turn).min(600) as u16
}

fn sync_unit(world: &mut World, ps: &PathState, id: UnitId, now: u32) {
    let Some(def_id) = world.unit(id).map(|u| u.def) else {
        return;
    };
    let has_grid = ps.grid_for(def_id).is_some();
    let Some((u, def, _)) = world.unit_and_def_mut(id) else {
        return;
    };
    let Some(target) = front_target(u) else {
        if u.path.goal.is_some() || u.path.status != PathStatus::Idle {
            u.path.reset();
        }
        return;
    };
    if u.path.goal != Some(target) {
        let last_request = u.path.last_request;
        u.path = UnitPath {
            goal: Some(target),
            status: PathStatus::Pending,
            last_request,
            ..UnitPath::default()
        };
        if !has_grid {
            u.path.follow(u.pos, vec![target], false);
        }
        return;
    }
    if u.path.status != PathStatus::Following {
        return;
    }
    let Some(&end) = u.path.waypoints.last() else {
        u.path.status = PathStatus::Pending;
        return;
    };
    let last_wp = u.path.waypoints.len() == 1;
    if u.path.unreachable && last_wp && u.pos.distance(end) <= ARRIVAL_TOLERANCE {
        finish(u, Outcome::Unreachable);
        return;
    }
    let d = u.pos.distance(u.path.waypoints[0]);
    if d.saturating_add(Fx::ONE) <= u.path.best {
        u.path.best = d;
        u.path.stall = 0;
    } else {
        u.path.stall = u.path.stall.saturating_add(1);
    }
    if u.path.stall < stall_window(def) {
        return;
    }
    if last_wp && u.pos.distance(end) <= CROWD_RADIUS {
        let o = if u.path.unreachable {
            Outcome::Unreachable
        } else {
            Outcome::Arrived
        };
        finish(u, o);
    } else if !has_grid || u.path.replans >= MAX_REPLANS {
        finish(u, Outcome::GaveUp);
    } else if now >= u.path.last_request.saturating_add(REPATH_INTERVAL) {
        u.path.replans += 1;
        u.path.status = PathStatus::Pending;
        u.path.stall = 0;
        u.path.best = Fx::MAX;
    }
}

/// Heading to TA's eight direction sectors (N = 0, counter-clockwise).
fn heading_dir(u: &Unit) -> u8 {
    (0xC000u16.wrapping_sub(u.heading.raw()).wrapping_add(0x1000) >> 13) as u8 & 7
}

fn plan(world: &mut World, ps: &mut PathState, now: u32) {
    let mut budget = STEPS_PER_TICK;
    // Resume the active search if its unit still wants it.
    if let Some(mut a) = ps.active.take() {
        let valid = world.unit(a.unit).is_some_and(|u| {
            u.path.status == PathStatus::Searching && u.path.goal == Some(a.goal_pos)
        });
        if valid {
            run_search(world, ps, &mut a, &mut budget);
            if world
                .unit(a.unit)
                .is_some_and(|u| u.path.status == PathStatus::Searching)
            {
                ps.active = Some(a);
            }
        }
    }
    // Admit new requests in slot order from the cursor.
    let ids = world.unit_ids();
    if !ids.is_empty() {
        let start = ids.partition_point(|id| id.index < ps.cursor);
        let mut k = 0;
        while k < ids.len() && ps.active.is_none() && budget >= ADMISSION_COST {
            let id = ids[(start + k) % ids.len()];
            k += 1;
            let pending = world
                .unit(id)
                .is_some_and(|u| u.path.status == PathStatus::Pending);
            if !pending {
                continue;
            }
            budget -= ADMISSION_COST;
            ps.cursor = id.index.wrapping_add(1);
            ps.stats.requests += 1;
            start_request(world, ps, id, now, &mut budget);
        }
    }
    let spent = STEPS_PER_TICK - budget;
    ps.stats.steps_last_tick = spent;
    ps.stats.max_steps_tick = ps.stats.max_steps_tick.max(spent);
}

fn start_request(world: &mut World, ps: &mut PathState, id: UnitId, now: u32, budget: &mut u32) {
    let Some(u) = world.unit(id) else { return };
    let Some(gi) = ps.def_grid.get(u.def.0 as usize).copied().flatten() else {
        return;
    };
    let Some(goal_pos) = u.path.goal else { return };
    let (pos, dir) = (u.pos, heading_dir(u));
    let grid = &ps.grids[usize::from(gi)];

    // Start: escape an obstruction's margin to the nearest passable anchor.
    let mut prefix = Vec::new();
    let mut s = grid.anchor_clamped(pos);
    if grid.tier(s) == grid::BLOCKED {
        match grid.nearest(s, |g, a| g.tier(a) != grid::BLOCKED) {
            Some(n) => {
                s = n;
                prefix.push(grid.anchor_world(n));
            }
            None => {
                if let Some(u) = world.unit_mut(id) {
                    finish(u, Outcome::GaveUp);
                }
                return;
            }
        }
    }
    // Goal: if it is blocked or in another component, aim for the
    // reachable anchor nearest to it.
    let comp = grid.comp(s);
    let g = grid.anchor_clamped(goal_pos);
    let (goal, target, unreachable) = if grid.tier(g) != grid::BLOCKED && grid.comp(g) == comp {
        (g, goal_pos, false)
    } else {
        let n = grid.nearest(g, |gr, a| gr.comp(a) == comp).unwrap_or(s);
        (n, grid.clamp_into(goal_pos, n), true)
    };
    if unreachable {
        ps.stats.unreachable += 1;
    }
    // Straight line first.
    let from = prefix.last().copied().unwrap_or(pos);
    let (clear, cost) = grid.line_of_sight(from, target);
    *budget = budget.saturating_sub(cost);
    let Some(u) = world.unit_mut(id) else { return };
    u.path.last_request = now;
    if clear || s == goal {
        ps.stats.direct += 1;
        prefix.push(target);
        u.path.follow(pos, prefix, unreachable);
        return;
    }
    let (Some(si), Some(gx)) = (grid.index(s), grid.index(goal)) else {
        finish(u, Outcome::GaveUp);
        return;
    };
    u.path.status = PathStatus::Searching;
    let search = search::Search::new(grid, &mut ps.scratch, si, gx, dir);
    let mut a = Active {
        unit: id,
        grid: gi,
        goal_pos,
        prefix,
        target,
        unreachable,
        search,
    };
    run_search(world, ps, &mut a, budget);
    if world
        .unit(id)
        .is_some_and(|u| u.path.status == PathStatus::Searching)
    {
        ps.active = Some(a);
    }
}

fn run_search(world: &mut World, ps: &mut PathState, a: &mut Active, budget: &mut u32) {
    let grid = &ps.grids[usize::from(a.grid)];
    let (progress, spent) = a.search.run(grid, &mut ps.scratch, *budget);
    *budget = budget.saturating_sub(spent);
    ps.stats.pops += u64::from(spent);
    let Some(u) = world.unit_mut(a.unit) else {
        return;
    };
    match progress {
        search::Progress::Suspended => {}
        search::Progress::Exhausted => finish(u, Outcome::GaveUp),
        search::Progress::Found(cells) => {
            ps.stats.searches += 1;
            let start = a.prefix.last().copied().unwrap_or(u.pos);
            let mut nodes: Vec<WorldPos> = cells
                .iter()
                .skip(1)
                .take(cells.len().saturating_sub(2))
                .map(|&c| grid.anchor_world(grid.coords(c)))
                .collect();
            nodes.push(a.target);
            let mut wps = a.prefix.clone();
            wps.extend(smooth(grid, start, &nodes));
            let pos = u.pos;
            u.path.follow(pos, wps, a.unreachable);
        }
    }
}

/// Greedy line-of-sight smoothing: from each kept point, skip ahead to
/// the farthest route node still in a straight walkable line.
fn smooth(grid: &ClassGrid, start: WorldPos, nodes: &[WorldPos]) -> Vec<WorldPos> {
    let mut out = Vec::new();
    let mut anchor = start;
    let mut i = 0;
    while i < nodes.len() {
        let mut j = i;
        while j + 1 < nodes.len() && grid.line_of_sight(anchor, nodes[j + 1]).0 {
            j += 1;
        }
        out.push(nodes[j]);
        anchor = nodes[j];
        i = j + 1;
    }
    out
}

/// Speed factor for a unit on its current terrain: FBI `roadmultiplier`
/// on paved cells (Believed, Kingdoms key); in water, FBI
/// `watermultiplier`, or TA's half speed for non-hovering units without
/// one (TA-derived). Flyers are never slowed.
pub fn speed_factor(u: &Unit, def: &UnitDef) -> Fx {
    let m = &def.motion;
    if m.canfly {
        return Fx::ONE;
    }
    let mut f = Fx::ONE;
    if u.terrain & terrain::PAVED != 0 {
        f = f.saturating_mul(m.road_multiplier.unwrap_or(Fx::ONE));
    }
    if u.terrain & terrain::WATER != 0 {
        let w = m
            .water_multiplier
            .unwrap_or(if m.hover { Fx::ONE } else { Fx::HALF });
        f = f.saturating_mul(w);
    }
    f.max(Fx::ZERO)
}

/// Unit-unit separation, after movement: overlapping ground units (radius
/// half their footprint's larger side) are pushed apart by the overlap,
/// split evenly between two mobile units (an immobile one does not move),
/// at most [`SEPARATION_MAX_PUSH`] per tick, never into blocked terrain.
/// Pushes are computed from one snapshot and applied in slot order.
pub(crate) fn separate(world: &mut World, ps: &PathState) {
    struct E {
        bucket: (i32, i32),
        id: UnitId,
        pos: WorldPos,
        r: Fx,
        mobile: bool,
    }
    let mut es: Vec<E> = world
        .units()
        .filter_map(|(id, u)| {
            let def = world.defs().get(u.def)?;
            if def.motion.canfly {
                return None;
            }
            let k = def.motion.key(def);
            Some(E {
                bucket: (u.pos.y.to_int_floor() >> 6, u.pos.x.to_int_floor() >> 6),
                id,
                pos: u.pos,
                r: Fx::from_int(i32::from(k.fx.max(k.fz)) * 8),
                mobile: def.maxvelocity > Fx::ZERO,
            })
        })
        .collect();
    if es.len() < 2 {
        return;
    }
    es.sort_by_key(|e| (e.bucket, e.id.index));
    let mut push = vec![Vec2::ZERO; es.len()];
    for i in 0..es.len() {
        for by in es[i].bucket.0 - 1..=es[i].bucket.0 + 1 {
            for bx in es[i].bucket.1 - 1..=es[i].bucket.1 + 1 {
                let lo = es.partition_point(|e| e.bucket < (by, bx));
                let hi = es.partition_point(|e| e.bucket <= (by, bx));
                for j in lo..hi {
                    if es[j].id.index <= es[i].id.index || !(es[i].mobile || es[j].mobile) {
                        continue;
                    }
                    let d = es[i].pos.distance(es[j].pos);
                    let min = es[i].r.saturating_add(es[j].r);
                    if d >= min {
                        continue;
                    }
                    let dir = (es[j].pos - es[i].pos).normalize().unwrap_or_else(|| {
                        let a = (es[i].id.index.wrapping_mul(40503)
                            ^ es[j].id.index.wrapping_mul(9973))
                            as u16;
                        Vec2::from_angle(tak_math::Angle::from_raw(a))
                    });
                    let overlap = min - d;
                    let (si, sj) = match (es[i].mobile, es[j].mobile) {
                        (true, true) => (
                            overlap.saturating_mul(Fx::HALF),
                            overlap.saturating_mul(Fx::HALF),
                        ),
                        (true, false) => (overlap, Fx::ZERO),
                        _ => (Fx::ZERO, overlap),
                    };
                    push[i] -= dir.scale(si);
                    push[j] += dir.scale(sj);
                }
            }
        }
    }
    let mut moves: Vec<(UnitId, Vec2)> = es
        .iter()
        .zip(push)
        .filter(|(_, p)| *p != Vec2::ZERO)
        .map(|(e, p)| (e.id, p))
        .collect();
    moves.sort_by_key(|(id, _)| id.index);
    for (id, p) in moves {
        let p = match p.length() {
            l if l > SEPARATION_MAX_PUSH => p
                .normalize()
                .map_or(Vec2::ZERO, |n| n.scale(SEPARATION_MAX_PUSH)),
            _ => p,
        };
        let Some(def_id) = world.unit(id).map(|u| u.def) else {
            continue;
        };
        let grid = ps.grid_for(def_id);
        let Some((u, _, map)) = world.unit_and_def_mut(id) else {
            continue;
        };
        let to = map.clamp(u.pos + p);
        let ok = match grid {
            Some(g) => g.step_allowed(u.pos, to),
            None => !map.blocks_step(u.pos, to),
        };
        if ok {
            u.pos = to;
            u.sample_terrain(map);
        }
    }
}
