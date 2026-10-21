//! Combat (WP-M). Status: stub. Health, weapons, targeting, damage, death.
//!
//! `Attack` and `Guard` commands are routed here by
//! [`crate::systems::apply_commands`].

mod defs;

pub use defs::{
    CombatDef, DEFAULT_AIM_TOLERANCE, DamageStats, MAX_WEAPONS, MELEE_MARGIN, WeaponKind,
    WeaponStats, area_damage, combat_def_from_fbi, kind, melee_reach, reload_ticks,
};

use crate::hash::StateHasher;
use crate::script::ScriptHost;
use crate::world::World;
use tak_commands::{PlayerId, SimCommand};

/// Per-unit combat state (health, reload timers, current target, ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitCombat {}

impl UnitCombat {
    /// Health in percent for scripts (port 4). Stub: always full.
    pub fn health_percent(&self) -> i32 {
        100
    }

    /// Drop the combat order and any chase (player `Move`/`Stop`).
    pub fn cancel_orders(&mut self) {}

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"C");
    }
}

/// World-level combat state (projectiles in flight, ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CombatState {}

impl CombatState {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"COMBAT");
    }
}

/// Apply an `Attack`, `Guard` or `CastSpell` command.
pub fn apply_command(_world: &mut World, _player: PlayerId, _cmd: &SimCommand) {}

/// Runs after movement, before the per-unit script tick.
pub fn tick(_world: &mut World, _scripts: &mut dyn ScriptHost) {}
