//! Hook for unit behaviour scripts (COB). The sim calls a [`ScriptHost`] at
//! fixed points in the tick. [`crate::cob::CobScripts`] wraps the COB VM
//! (`tak-script`) in an implementation of this trait; `tak-script` itself
//! never depends on `tak-sim`.
//!
//! Determinism contract for implementors: a host may only depend on what the
//! sim passes it (the [`ScriptCtx`] snapshot, the world RNG) and its own
//! state, must not read clocks, files or thread-local randomness, and must
//! feed all of its state into [`ScriptHost::hash_state`] so script
//! divergence shows up in the per-tick hash. [`ScriptStats`] are diagnostics
//! and are not hashed.

use std::collections::BTreeMap;

use tak_commands::{FireState, PlayerId, Tick, UnitDefId, UnitId};
use tak_math::{Angle, Fx, Rng};

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
    /// Terrain flags at `pos` ([`crate::map::terrain`]).
    pub terrain: u8,
    /// Facing.
    pub heading: Angle,
    /// Forward speed (world units per tick).
    pub speed: Fx,
    /// `true` while the unit has an active move order.
    pub moving: bool,
    /// Fire state toggle.
    pub fire_state: FireState,
    /// Health in percent (port 4), from the combat lane.
    pub health_percent: i32,
    /// Percent still to build (port 17), from the construction lane.
    pub build_percent_left: i32,
}

/// Unit script callbacks. Every method has a default.
///
/// Call order within a tick: [`ScriptHost::create_unit`] for units spawned
/// this tick (in spawn order), then, after movement,
/// [`ScriptHost::tick_unit`] for every live unit in ascending `UnitId` slot
/// order. `hash_state` runs once per tick, after the world has been hashed.
///
/// `create_unit`/`tick_unit` receive the world RNG (the only randomness a
/// script may use); by default they forward to `on_create`/`on_tick`, so
/// simple hosts only implement those.
pub trait ScriptHost {
    /// A unit was created (COB `Create`).
    fn on_create(&mut self, ctx: &ScriptCtx) {
        let _ = ctx;
    }

    /// Once per tick per live unit, after movement.
    fn on_tick(&mut self, ctx: &ScriptCtx) {
        let _ = ctx;
    }

    /// What the sim calls on creation. Defaults to [`ScriptHost::on_create`].
    fn create_unit(&mut self, ctx: &ScriptCtx, rng: &mut Rng) {
        let _ = rng;
        self.on_create(ctx);
    }

    /// What the sim calls every tick. Defaults to [`ScriptHost::on_tick`].
    fn tick_unit(&mut self, ctx: &ScriptCtx, rng: &mut Rng) {
        let _ = rng;
        self.on_tick(ctx);
    }

    /// Run a script function on a unit now (COB `call-script`) and return
    /// its result. `None` if the unit has no script or no such function, or
    /// if the call blocked (it then keeps running as a thread). Default: no
    /// scripts.
    fn call_script(
        &mut self,
        _ctx: &ScriptCtx,
        _rng: &mut Rng,
        _name: &str,
        _args: &[i32],
    ) -> Option<ScriptReturn> {
        None
    }

    /// Start a script function on a unit as a new thread (COB
    /// `start-script`); it first runs on the unit's next tick. Returns
    /// whether it started. Default: no scripts.
    fn start_script(&mut self, _unit: UnitId, _name: &str, _args: &[i32]) -> bool {
        false
    }

    /// Forget a unit's script (it was removed from the world).
    fn remove_unit(&mut self, _unit: UnitId) {}

    /// Feed all script state into the world hash.
    fn hash_state(&self, h: &mut StateHasher) {
        let _ = h;
    }

    /// Diagnostics for tools (not hashed). Empty by default.
    fn stats(&self) -> ScriptStats {
        ScriptStats::default()
    }
}

/// What a completed [`ScriptHost::call_script`] returned: the `return`
/// value and the function's locals (query callbacks such as `QueryWeapon`
/// write their answer into `locals[0]`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptReturn {
    /// Value of the `return` instruction.
    pub value: i32,
    /// The function's locals when it returned.
    pub locals: Vec<i32>,
}

/// A host that runs no scripts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoScripts;

impl ScriptHost for NoScripts {}

/// Script diagnostics gathered by a host. Never hashed: they describe the
/// run, they are not part of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptStats {
    /// Units with a running script instance.
    pub units: u64,
    /// Unit types whose script could not be loaded (`name: reason`).
    pub missing: Vec<String>,
    /// COB pieces with no model piece of the same name, summed over types.
    pub unmapped_pieces: u64,
    /// Every port read (`get-unit-value` and `get`), by port.
    pub reads: BTreeMap<i32, u64>,
    /// Reads of ports the host does not implement (they return 0).
    pub unknown_reads: BTreeMap<i32, u64>,
    /// Every `set`, by port.
    pub writes: BTreeMap<i32, u64>,
    /// Writes to ports with no known meaning (stored, otherwise unused).
    pub unknown_writes: BTreeMap<i32, u64>,
    /// Reads of the speed port (29) that answered "moving" (`> 5`).
    pub speed_reads_moving: u64,
    /// Unit-ticks in which the unit's pose changed.
    pub pose_ticks: u64,
    /// Unit-ticks in which the pose changed while the unit was moving.
    pub pose_ticks_moving: u64,
    /// Threads stopped by the per-tick instruction cap, by `UNIT:function`.
    pub runaway: BTreeMap<String, u64>,
    /// Other thread errors, by `UNIT:function: error`.
    pub errors: BTreeMap<String, u64>,
    /// Effects requested (sfx, explode, sound, mission command).
    pub effects: u64,
}

impl ScriptStats {
    /// Plain-text report, one item per line.
    pub fn report(&self) -> String {
        let mut s = String::new();
        let map = |m: &BTreeMap<i32, u64>| {
            m.iter()
                .map(|(k, v)| format!("{k}x{v}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        s.push_str(&format!("script units: {}\n", self.units));
        for m in &self.missing {
            s.push_str(&format!("missing script: {m}\n"));
        }
        s.push_str(&format!("unmapped pieces: {}\n", self.unmapped_pieces));
        s.push_str(&format!("port reads: {}\n", map(&self.reads)));
        s.push_str(&format!(
            "unknown port reads: {}\n",
            map(&self.unknown_reads)
        ));
        s.push_str(&format!("port writes: {}\n", map(&self.writes)));
        s.push_str(&format!(
            "unknown port writes: {}\n",
            map(&self.unknown_writes)
        ));
        s.push_str(&format!(
            "speed reads answering moving: {}\n",
            self.speed_reads_moving
        ));
        s.push_str(&format!(
            "pose-changing unit-ticks: {} ({} while moving)\n",
            self.pose_ticks, self.pose_ticks_moving
        ));
        let total: u64 = self.runaway.values().sum();
        s.push_str(&format!("runaway loops (instruction cap): {total}\n"));
        for (k, v) in &self.runaway {
            s.push_str(&format!("  {k}: {v}\n"));
        }
        let total: u64 = self.errors.values().sum();
        s.push_str(&format!("other thread errors: {total}\n"));
        for (k, v) in &self.errors {
            s.push_str(&format!("  {k}: {v}\n"));
        }
        s.push_str(&format!("effects: {}\n", self.effects));
        s
    }
}
