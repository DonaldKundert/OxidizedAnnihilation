//! Device events to commands, with controller play as a first-class mode.
//!
//! Pipeline:
//!
//! 1. A platform adapter (not in this crate) turns raw gamepad, mouse and
//!    keyboard input into device-neutral [`InputEvent`]s, timestamped in ms.
//! 2. A [`Profile`] (JSON) binds inputs and gestures (press, release, tap,
//!    hold, double tap, chord) to [`Action`]s. [`Profile::default_controller`]
//!    and [`Profile::default_mouse_keyboard`] produce the same orders for
//!    the same intents.
//! 3. The [`Mapper`] runs the gesture recogniser and the client-side state
//!    (virtual cursor, selection, control groups, wheels, brush, order mode),
//!    reading the world only through the read-only [`WorldView`] trait.
//! 4. Out comes an ordered stream of [`Output`]: [`ClientCommand`]s and
//!    [`SimCommand`]s stamped with the caller's tick.
//!
//! The geometry is pure functions over [`tak_math`] types, with no floating
//! point anywhere: [`stick_vector`], [`magnetize`], [`brush_radius`],
//! [`slot_for`], [`resolve_context`]. Tuning constants live in [`Tuning`].
//!
//! This crate never mutates sim state and does not depend on `tak-sim`.
//!
//! [`ClientCommand`]: tak_commands::ClientCommand
//! [`SimCommand`]: tak_commands::SimCommand

#![deny(clippy::float_arithmetic, clippy::float_cmp)]

mod binding;
mod brush;
mod context;
mod cursor;
mod event;
mod keys_tdf;
pub mod lab;
mod mapper;
mod record;
mod tuning;
mod wheel;
mod world;

pub use binding::{
    Action, Binding, CONTROL_SCHEMES, Category, GRID_KEYS, Gesture, Profile, ProfileError,
    ReservedOrder, Ring,
};
pub use brush::{brush_radius, drag_circle, units_in_circle};
pub use context::{
    Mode, builders_for, casters_for, healers_for, own_units, pick_unit, resolve_context,
};
pub use cursor::{
    AXIS_MAX, Motion, approach_frac, cursor_motion, cursor_step, curve, friction, magnetize, pct,
    per_second, soft_snap, stick_vector, stick_vector_zones, zoom_factor,
};
pub use event::{Axis, Button, EventKind, Input, InputEvent, Key};
pub use keys_tdf::{KeyLine, KeysError, KeysImport, LineStatus, parse_keys_tdf};
pub use mapper::{BOOKMARKS, CursorKind, Feedback, Mapper, Output, Refusal, SubSelect};
pub use record::{
    FrameIo, Ghost, RECORDING_VERSION, RecFrame, Recorder, Recording, RecordingError, Summary,
};
pub use tuning::{PRESETS, TUNING_FIELDS, Tuning, TuningField, field as tuning_field};
pub use wheel::{
    BLANK, PAGE, Wheel, WheelKind, context_slots, context_slots_fixed, expand_spells, is_blank,
    ring_slots, ring_usable, slot_direction, slot_for,
};
pub use world::{BuilderKind, Camera, UnitInfo, WorldView, units_sorted};
