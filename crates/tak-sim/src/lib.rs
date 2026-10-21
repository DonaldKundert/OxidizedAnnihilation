//! World state, fixed 30 Hz tick, systems, state hashing. Never reads the
//! clock, the filesystem, or a thread-local RNG.
//!
//! - [`Scenario`]: seed, [`MapInfo`] stand-in, [`UnitDef`] stand-ins and
//!   player slots (JSON, versioned).
//! - [`World`]: players, a generational unit arena, the seeded RNG.
//! - [`Sim`]: steps the world one tick at a time; the systems and their
//!   order are documented in [`systems`]. The only input is
//!   [`tak_commands::SimCommand`]s via [`tak_commands::CommandLog`].
//! - [`StateHasher`]: xxHash64 over a canonical little-endian
//!   serialization, computed after every tick.
//! - [`ScriptHost`]: the hook a COB VM implements.
//!
//! Coordinates: see [`map`]. Headings: see [`Unit`].
//!
//! Integer arithmetic on sim state uses `Fx` operators (wrapping by
//! definition) or explicit `wrapping_*`/`saturating_*`/`checked_*` calls, so
//! debug and release builds behave identically.

#![deny(clippy::float_arithmetic, clippy::float_cmp)]

pub mod defs;
pub mod hash;
pub mod map;
pub mod scenario;
pub mod script;
mod sim;
pub mod systems;
pub mod testing;
pub mod world;

pub use defs::{UnitDef, UnitDefs};
pub use hash::StateHasher;
pub use map::{HeightGrid, MapInfo, StartPos};
pub use scenario::{PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario, ScenarioError};
pub use script::{NoScripts, ScriptCtx, ScriptHost};
pub use sim::{Sim, run_scenario};
pub use tak_commands::{PlayerId, Tick, UnitDefId, UnitId, WorldPos};
pub use world::{Order, Player, Unit, World};

/// Simulation rate. Game speed changes ticks per wall-second, never this.
pub const TICKS_PER_SECOND: u32 = 30;
