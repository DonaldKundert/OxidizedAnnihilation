//! Deterministic math for the simulation: 16.16 fixed point ([`Fx`]), binary
//! angles with table-driven trig ([`Angle`]), fixed-point vectors
//! ([`Vec2`], [`Vec3`]) and a seeded PRNG ([`Rng`]).
//!
//! Nothing in this crate uses floating point, at runtime or when building the
//! trig table. `tests/no_floats.rs` scans the sources to keep it that way, and
//! clippy's `float_arithmetic` lint is denied below.
//!
//! Every operation is pure integer arithmetic with explicit overflow behavior
//! (wrapping, checked or saturating, never "panic in debug, wrap in
//! release"), so results are identical across platforms and build profiles.
//! Rounding choices are documented on each type and method; the default is
//! round to nearest, ties toward positive infinity.
//!
//! The `serde` feature (on by default) derives `Serialize`/`Deserialize`.
//! [`Fx`] and [`Angle`] serialize as their raw integers.

#![deny(clippy::float_arithmetic, clippy::float_cmp)]

mod angle;
mod fx;
mod rng;
mod vec;

pub use angle::{Angle, SIN_TABLE};
pub use fx::Fx;
pub use rng::{DEFAULT_STREAM, Rng};
pub use vec::{Vec2, Vec3};
