//! COB v6 script VM (WP-G, milestone M6). Parsing and disassembly live in
//! `tak_formats::cob`; this crate runs the bytecode.
//!
//! - [`Program`]: a parsed script, decoded once, shared by every unit of a type.
//! - [`Instance`]: one unit's statics, piece animations and threads.
//!   [`Instance::start`] / [`Instance::call`] are the entry points;
//!   [`Instance::tick`] advances one 30 Hz tick.
//! - [`Engine`]: what the VM asks of the world (ports, effects, sounds,
//!   random numbers). The sim implements it; this crate does not depend on
//!   `tak-sim`. [`HeadlessEngine`] is a stand-alone implementation for tools
//!   and tests.
//!
//! All state is integer: positions are 16.16 [`tak_math::Fx`], angles are
//! [`tak_math::Angle`]. Semantics, timing and open questions:
//! `docs/formats/cob-vm.md`.

#![deny(clippy::float_arithmetic)]

pub mod anim;
pub mod engine;
mod hash;
pub mod vm;

pub use anim::{PieceAnim, PieceFlags, PieceTransform, TICKS_PER_SECOND};
pub use engine::{Effect, Engine, HeadlessEngine, port};
pub use hash::Fnv64;
pub use vm::{
    CallOutcome, Instance, Limits, Program, StartError, ThreadError, ThreadId, VmError, sleep_ticks,
};
