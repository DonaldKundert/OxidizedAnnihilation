//! Simulation commands (the only input to the sim) and client commands (local
//! UI actions that never reach the sim).

use serde::{Deserialize, Serialize};
use tak_math::Fx;

use crate::{ControlGroup, SpellId, UnitDefId, UnitId, WorldPos};

/// A set of units an order applies to: always sorted by [`UnitId`] and free
/// of duplicates, so two clients that select the same units in a different
/// order produce byte-identical commands. Deserialization re-normalizes, so
/// a hand-edited log cannot break the invariant.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize)]
#[serde(from = "Vec<UnitId>", into = "Vec<UnitId>")]
pub struct UnitSet(Vec<UnitId>);

impl UnitSet {
    /// An empty set.
    pub const fn new() -> UnitSet {
        UnitSet(Vec::new())
    }

    /// A set holding one unit.
    pub fn single(id: UnitId) -> UnitSet {
        UnitSet(vec![id])
    }

    /// Units in ascending `UnitId` order.
    pub fn as_slice(&self) -> &[UnitId] {
        &self.0
    }

    /// Iterate in ascending `UnitId` order.
    pub fn iter(&self) -> core::slice::Iter<'_, UnitId> {
        self.0.iter()
    }

    /// Number of units.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` if there are no units.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Membership test (binary search).
    pub fn contains(&self, id: UnitId) -> bool {
        self.0.binary_search(&id).is_ok()
    }

    /// Insert, keeping order. Returns `false` if already present.
    pub fn insert(&mut self, id: UnitId) -> bool {
        match self.0.binary_search(&id) {
            Ok(_) => false,
            Err(at) => {
                self.0.insert(at, id);
                true
            }
        }
    }

    /// Remove. Returns `false` if absent.
    pub fn remove(&mut self, id: UnitId) -> bool {
        match self.0.binary_search(&id) {
            Ok(at) => {
                self.0.remove(at);
                true
            }
            Err(_) => false,
        }
    }
}

impl From<Vec<UnitId>> for UnitSet {
    fn from(mut v: Vec<UnitId>) -> UnitSet {
        v.sort_unstable();
        v.dedup();
        UnitSet(v)
    }
}

impl From<UnitSet> for Vec<UnitId> {
    fn from(s: UnitSet) -> Vec<UnitId> {
        s.0
    }
}

impl FromIterator<UnitId> for UnitSet {
    fn from_iter<I: IntoIterator<Item = UnitId>>(iter: I) -> UnitSet {
        UnitSet::from(iter.into_iter().collect::<Vec<_>>())
    }
}

impl<'a> IntoIterator for &'a UnitSet {
    type Item = &'a UnitId;
    type IntoIter = core::slice::Iter<'a, UnitId>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// What an order is aimed at.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// A point on the ground.
    Ground(WorldPos),
    /// A unit (friendly or enemy, depending on the order).
    Unit(UnitId),
}

/// Automatic engagement policy. Legacy TA fire states keep their tokens.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum FireState {
    /// Never fire unless ordered to attack.
    HoldFire,
    /// Legacy retaliation against the last attacker, including idle pursuit.
    ReturnFire,
    /// Fire at and pursue enemies automatically (the legacy default).
    #[default]
    FireAtWill,
    /// Kingdoms Defensive: fire at enemies in reach, without automatic pursuit.
    /// Explicit movement, Attack and CastSpell orders still apply.
    Defensive,
}

/// Whether an order replaces the unit's queue or is appended to it
/// (shift-click).
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    /// Clear the current orders, then do this.
    #[default]
    Replace,
    /// Do this after the current orders.
    Append,
}

/// A standing order copied to each newly finished factory product. These
/// orders are separate from production and never move the factory itself.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FactoryRallyOrder {
    /// Move through a waypoint once.
    Move {
        /// Destination.
        to: WorldPos,
    },
    /// Begin or extend a patrol after preceding move waypoints. The first
    /// patrol also returns to the last move waypoint, or the product's release
    /// position when no move precedes it.
    Patrol {
        /// Patrol destination.
        to: WorldPos,
    },
    /// Guard a live, non-hostile unit after preceding waypoints. The exact
    /// generational target is checked again when each product finishes.
    Guard {
        /// Unit to escort or assist, including the producing factory.
        target: UnitId,
    },
}

/// An order for the simulation. Every variant names the units it applies to;
/// the sim validates ownership (a player can only order their own units) and
/// silently drops IDs that are stale or not theirs.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SimCommand {
    /// Move to a ground point.
    Move {
        /// Units to move.
        units: UnitSet,
        /// Destination.
        to: WorldPos,
        /// Replace or append.
        queue: QueueMode,
    },
    /// Move to a ground point, stopping to engage enemies along the way.
    AttackMove {
        /// Units to move.
        units: UnitSet,
        /// Destination to resume after each engagement.
        to: WorldPos,
        /// Replace or append.
        queue: QueueMode,
    },
    /// Attack a unit, or force-fire at a ground point.
    Attack {
        /// Attackers.
        units: UnitSet,
        /// What to attack.
        target: Target,
        /// Replace or append.
        queue: QueueMode,
    },
    /// Build one unit of `def`. With `at: Some(pos)` the builders place it
    /// there (a construction order); with `at: None` it goes into each
    /// factory's production queue.
    Build {
        /// Builders or factories.
        units: UnitSet,
        /// What to build.
        def: UnitDefId,
        /// Placement, or `None` for factory production.
        at: Option<WorldPos>,
        /// Replace or append.
        queue: QueueMode,
    },
    /// Follow and protect (and assist, for builders) another unit.
    Guard {
        /// Guards.
        units: UnitSet,
        /// Unit to guard.
        target: UnitId,
        /// Replace or append.
        queue: QueueMode,
    },
    /// Heal an owned unit or assist its unfinished construction frame.
    Heal {
        /// Complete mobile builders.
        units: UnitSet,
        /// Owned unit to heal; self-targeting is not supported.
        target: UnitId,
        /// Replace or append.
        queue: QueueMode,
    },
    /// Patrol between the unit's position (or previous patrol point) and `to`.
    Patrol {
        /// Patrollers.
        units: UnitSet,
        /// Patrol waypoint.
        to: WorldPos,
        /// Replace or append.
        queue: QueueMode,
    },
    /// Cast a spell. `target` is `None` for untargeted spells.
    CastSpell {
        /// Casters.
        units: UnitSet,
        /// Which spell.
        spell: SpellId,
        /// Spell target, if any.
        target: Option<Target>,
        /// Replace or append.
        queue: QueueMode,
    },
    /// Clear all orders immediately.
    Stop {
        /// Units to stop.
        units: UnitSet,
    },
    /// Change fire state immediately (not queued).
    SetFireState {
        /// Units to change.
        units: UnitSet,
        /// New fire state.
        state: FireState,
    },
    /// Set standing orders without cancelling or reordering production.
    /// Ordinary `Move`/`Patrol` keep their historical semantics in old logs.
    SetFactoryRally {
        /// Owned, complete, live factories; other units are ignored.
        units: UnitSet,
        /// Standing order, or `None` to clear all standing orders.
        order: Option<FactoryRallyOrder>,
        /// Replace or append; ignored when `order` is `None`.
        queue: QueueMode,
    },
    /// Remove matching factory production entries, newest pending first.
    /// Also disables a matching repeat recipe without consuming `count`.
    /// The active product is cancelled only after pending matches run out.
    CancelProduction {
        /// Owned, complete, live factories; other units are ignored.
        units: UnitSet,
        /// Product type to remove.
        def: UnitDefId,
        /// Maximum entries to remove per factory; zero does nothing.
        count: u32,
    },
    /// Set continuous factory production after its finite queue runs out.
    /// Changing or disabling the recipe preserves the active paid product.
    SetFactoryRepeat {
        /// Owned, complete, live factories; other units are ignored.
        units: UnitSet,
        /// A product in the factory's build menu, or `None` to disable.
        def: Option<UnitDefId>,
    },
}

impl SimCommand {
    /// The units this command applies to.
    pub fn units(&self) -> &UnitSet {
        match self {
            SimCommand::Move { units, .. }
            | SimCommand::AttackMove { units, .. }
            | SimCommand::Attack { units, .. }
            | SimCommand::Build { units, .. }
            | SimCommand::SetFactoryRally { units, .. }
            | SimCommand::CancelProduction { units, .. }
            | SimCommand::SetFactoryRepeat { units, .. }
            | SimCommand::Guard { units, .. }
            | SimCommand::Heal { units, .. }
            | SimCommand::Patrol { units, .. }
            | SimCommand::CastSpell { units, .. }
            | SimCommand::Stop { units }
            | SimCommand::SetFireState { units, .. } => units,
        }
    }

    /// The queue mode, or `None` for immediate commands (`Stop`,
    /// `SetFireState`, `CancelProduction`, `SetFactoryRepeat`).
    pub fn queue(&self) -> Option<QueueMode> {
        match self {
            SimCommand::Move { queue, .. }
            | SimCommand::AttackMove { queue, .. }
            | SimCommand::Attack { queue, .. }
            | SimCommand::Build { queue, .. }
            | SimCommand::SetFactoryRally { queue, .. }
            | SimCommand::Guard { queue, .. }
            | SimCommand::Heal { queue, .. }
            | SimCommand::Patrol { queue, .. }
            | SimCommand::CastSpell { queue, .. } => Some(*queue),
            SimCommand::Stop { .. }
            | SimCommand::SetFireState { .. }
            | SimCommand::CancelProduction { .. }
            | SimCommand::SetFactoryRepeat { .. } => None,
        }
    }
}

/// Where a select-same-type sweep looks.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Units currently visible in the viewport.
    #[default]
    OnScreen,
    /// All of the player's units on the map.
    Map,
}

/// A client-side action: selection, camera and control groups. These read
/// sim state but never change it, so they are not part of the
/// [`CommandLog`](crate::CommandLog) and do not need to be deterministic. The input layer
/// resolves screen coordinates to [`WorldPos`] before building them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientCommand {
    /// Select the unit at a point (click).
    SelectAt {
        /// Picked ground point.
        pos: WorldPos,
        /// Add to the selection instead of replacing it.
        additive: bool,
    },
    /// Select own units within a circle (drag box or gamepad brush).
    SelectBrush {
        /// Brush centre.
        center: WorldPos,
        /// Brush radius in world units.
        radius: Fx,
        /// Add to the selection instead of replacing it.
        additive: bool,
    },
    /// Select every own unit sharing a type with the current selection.
    SelectSameType {
        /// Where to look.
        scope: Scope,
    },
    /// Store the current selection as a control group.
    AssignGroup {
        /// Group number.
        group: ControlGroup,
    },
    /// Replace the selection with a control group.
    RecallGroup {
        /// Group number.
        group: ControlGroup,
    },
    /// Move the camera to the most recent alert (unit under attack, etc.).
    JumpToAlert,
    /// Select the next idle builder and centre on it.
    CycleIdleBuilder,
    /// Centre the camera on the player's monarch.
    CenterOnMonarch,
    /// Clear the selection (B / Escape with nothing else to cancel).
    ClearSelection,
    /// Add a control group to the current selection without replacing it.
    RecallGroupAdd {
        /// Group number.
        group: ControlGroup,
    },
}
