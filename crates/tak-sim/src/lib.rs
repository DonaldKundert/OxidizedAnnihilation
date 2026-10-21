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
//! - [`ScriptHost`]: the hook a COB VM implements; [`CobScripts`] is the
//!   real one (`tak-script` underneath).
//! - [`content`]: real maps and FBI units into a [`Scenario`], unit
//!   conversions, and [`content::load_match`] (reads an install through
//!   `tak-vfs`; the only place the crate touches files, and only when a
//!   caller asks it to, before the match starts).
//!
//! Coordinates: see [`map`]. Headings: see [`Unit`]. Units, conversions and
//! script ports: `docs/sim.md`.
//!
//! Integer arithmetic on sim state uses `Fx` operators (wrapping by
//! definition) or explicit `wrapping_*`/`saturating_*`/`checked_*` calls, so
//! debug and release builds behave identically.

#![deny(clippy::float_arithmetic, clippy::float_cmp)]

pub mod cob;
pub mod content;
pub mod defs;
pub mod hash;
pub mod map;
pub mod scenario;
pub mod script;
mod sim;
pub mod systems;
pub mod testing;
pub mod world;

pub use cob::{CobScripts, ScriptType};
pub use content::{Match, PlayerSetup, load_match};
pub use defs::{UnitDef, UnitDefs};
pub use hash::StateHasher;
pub use map::{HeightGrid, MapInfo, StartPos, TerrainGrid};
pub use scenario::{PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario, ScenarioError};
pub use script::{NoScripts, ScriptCtx, ScriptHost, ScriptStats};
pub use sim::{Sim, run_scenario};
pub use tak_commands::{PlayerId, Tick, UnitDefId, UnitId, WorldPos};
pub use world::{Order, Player, Unit, World};

/// Simulation rate. Game speed changes ticks per wall-second, never this.
pub const TICKS_PER_SECOND: u32 = 30;
