//! Construction (WP-O). Status: stub. Builders, build sites, build
//! progress, factories and their queues.
//!
//! `Build` commands are routed here by [`crate::systems::apply_commands`].
//! New units go through [`crate::lifecycle::spawn_unit`]; mana through
//! [`crate::economy::drain`]; builders walk with
//! [`crate::pathfinding::go_to`].

use crate::hash::StateHasher;
use crate::script::ScriptHost;
use crate::world::World;
use tak_commands::{PlayerId, SimCommand};

/// Per-unit construction state (build progress if under construction,
/// current build target if a builder, factory queue, ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitBuild {}

impl UnitBuild {
    /// Percent still to build for scripts (port 17). Stub: complete.
    pub fn percent_left(&self) -> i32 {
        0
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"B");
    }
}

/// World-level construction state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConstructionState {}

impl ConstructionState {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"BUILD");
    }
}

/// Apply a `Build` command.
pub fn apply_command(_world: &mut World, _player: PlayerId, _cmd: &SimCommand) {}

/// Runs right after commands, before economy.
pub fn tick(_world: &mut World, _scripts: &mut dyn ScriptHost) {}
