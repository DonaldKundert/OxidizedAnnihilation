//! Hook for unit behaviour scripts (COB). The sim calls a [`ScriptHost`] at
//! fixed points in the tick; the COB VM (WP-G, `tak-script`) will be wrapped
//! in an implementation of this trait. `tak-script` itself never depends on
//! `tak-sim`.
//!
//! Determinism contract for implementors: a host may only depend on what the
//! sim passes it (and its own state), must not read clocks, files or
//! thread-local randomness, and must feed all of its state into
//! [`ScriptHost::hash_state`] so script divergence shows up in the per-tick
//! hash.

use tak_commands::{PlayerId, Tick, UnitDefId, UnitId};
use tak_math::{Angle, Fx};

use crate::WorldPos;
use crate::hash::StateHasher;

/// Read-only snapshot of one unit, handed to script callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptCtx {
    /// Tick being simulated.
    pub tick: Tick,
    /// The unit.
    pub unit: UnitId,
    /// Its definition.
    pub def: UnitDefId,
    /// Its owner.
    pub owner: PlayerId,
    /// Ground position.
    pub pos: WorldPos,
    /// Terrain height at `pos`.
    pub ground_height: Fx,
    /// Facing.
    pub heading: Angle,
    /// Forward speed (world units per tick).
    pub speed: Fx,
    /// `true` while the unit has an active move order.
    pub moving: bool,
}

/// Unit script callbacks. Every method has a no-op default.
///
/// Call order within a tick: `on_create` for units spawned this tick (in
/// spawn order), then, after movement, `on_tick` for every live unit in
/// ascending `UnitId` slot order. `hash_state` runs once per tick, after the
/// world has been hashed.
pub trait ScriptHost {
    /// A unit was created (COB `Create`).
    fn on_create(&mut self, ctx: &ScriptCtx) {
        let _ = ctx;
    }

    /// Once per tick per live unit, after movement.
    fn on_tick(&mut self, ctx: &ScriptCtx) {
        let _ = ctx;
    }

    /// Feed all script state into the world hash.
    fn hash_state(&self, h: &mut StateHasher) {
        let _ = h;
    }
}

/// A host that runs no scripts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoScripts;

impl ScriptHost for NoScripts {}
