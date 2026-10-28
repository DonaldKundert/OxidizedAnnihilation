//! The command layer: everything a player can ask for, with no knowledge of
//! input devices.
//!
//! - [`SimCommand`]: orders for the simulation. These are the **only** input
//!   to the sim, are recorded in the [`CommandLog`], and must be identical on
//!   every peer.
//! - [`ClientCommand`]: local selection, camera and control-group actions.
//!   They read sim state but never change it, and are not logged.
//! - [`CommandLog`]: tick-stamped [`SimCommand`]s in canonical
//!   `(tick, player, seq)` order, with a versioned JSON form.
//!
//! IDs: [`PlayerId`] (player slot), [`UnitId`] (generational arena handle),
//! [`UnitDefId`] and [`SpellId`] (opaque, assigned by the content layer),
//! [`Tick`] (30 per second).
//!
//! Positions are [`WorldPos`], a 2D [`tak_math::Vec2`] on the ground plane:
//! every order targets a map point or a unit, and the height at a point is
//! the terrain's, which the sim looks up. A 3D position can be added later
//! without changing the existing commands.

mod command;
mod ids;
mod log;

pub use command::{
    ClientCommand, FactoryRallyOrder, FireState, QueueMode, Scope, SimCommand, Target, UnitSet,
};
pub use ids::{ControlGroup, FeatureId, PlayerId, SpellId, Tick, UnitDefId, UnitId};
pub use log::{CommandLog, LOG_FORMAT_VERSION, LogError, LoggedCommand};

/// A ground-plane position in world units (16.16 fixed point). Which way
/// `+x`/`+y` point on the map, and the size of a world unit, are the sim's
/// decision; this crate only carries the value.
pub type WorldPos = tak_math::Vec2;
