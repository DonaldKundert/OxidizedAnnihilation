//! Economy: mana (WP-N). Status: stub. Mana income (monarch, lodestones),
//! storage, and spending.
//!
//! Other systems spend through [`drain`]. The stub grants everything, so
//! construction works before this lands.

use crate::hash::StateHasher;
use crate::script::ScriptHost;
use crate::world::World;
use tak_commands::PlayerId;
use tak_math::Fx;

/// Per-unit economy state (mana pool, income contribution, ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitEconomy {}

impl UnitEconomy {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"E");
    }
}

/// World-level economy state (per-player mana, lodestone ownership, ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EconomyState {}

impl EconomyState {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"ECON");
    }
}

/// Runs after construction, before orders.
pub fn tick(_world: &mut World, _scripts: &mut dyn ScriptHost) {}

/// Ask to spend `want` mana for `player` this tick. Returns how much was
/// granted (between zero and `want`). Callers scale their progress by the
/// granted fraction.
pub fn drain(_world: &mut World, _player: PlayerId, want: Fx) -> Fx {
    want
}
