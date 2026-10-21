//! Pathfinding (WP-L). Status: stub. Units drive straight at their
//! waypoints until this lands.
//!
//! Other systems ask for movement through [`go_to`]; how the unit gets there
//! is this module's business.

use crate::hash::StateHasher;
use crate::script::ScriptHost;
use crate::world::{Order, World};
use tak_commands::{PlayerId, SimCommand, UnitId, WorldPos};

/// Per-unit pathfinding state (current path, replan timers, ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitPath {}

impl UnitPath {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"P");
    }
}

/// World-level pathfinding state (cached grids, request queues, ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathState {}

impl PathState {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"PATH");
    }
}

/// Apply a `Patrol` command.
pub fn apply_command(_world: &mut World, _player: PlayerId, _cmd: &SimCommand) {}

/// Runs after orders, before movement.
pub fn tick(_world: &mut World, _scripts: &mut dyn ScriptHost) {}

/// Send a unit toward a point, replacing its current orders. Used by combat
/// (closing to range) and construction (walking to a site).
pub fn go_to(world: &mut World, unit: UnitId, to: WorldPos) {
    let to = world.map().clamp(to);
    if let Some(u) = world.unit_mut(unit) {
        u.orders.clear();
        u.orders.push_back(Order::Move { to });
    }
}
