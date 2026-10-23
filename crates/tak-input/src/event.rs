//! Device-neutral input events.
//!
//! A platform adapter (a future `gilrs` or Bevy adapter, behind a feature or
//! in the client crate) translates raw device input into these. Conventions
//! the adapter must follow:
//!
//! - `t` is milliseconds on a monotonic clock, non-decreasing across events.
//! - Emit axes from one device poll consecutively with the same timestamp.
//!   Wheel steering treats those updates as one coherent stick sample.
//! - Stick axes are `-32767..=32767` with **`+y` up** (flip if the device
//!   reports `+y` down). `-32768` is treated as `-32767`.
//! - Triggers are reported as buttons ([`Button::Lt`], [`Button::Rt`]) once
//!   they cross the adapter's threshold; their analog value may additionally
//!   be sent as an axis, which the mapper records but does not need.
//! - Pointer positions are screen pixels, origin top-left, `+y` down.
//! - Keys are lowercase names: `"a"`..`"z"`, `"0"`..`"9"`, `"f1"`..`"f12"`,
//!   `"shift"`, `"ctrl"`, `"alt"`, `"escape"`, `"space"`, `"home"`,
//!   `"period"`, ... (left/right variants folded into one name).
//! - Send a [`EventKind::Frame`] once per rendered frame; magnetism and
//!   brush painting update on frames.

use serde::{Deserialize, Serialize};

/// A gamepad or mouse button. Gamepad names follow the Xbox layout (A is the
/// bottom face button).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    /// Bottom face button.
    A,
    /// Right face button.
    B,
    /// Left face button.
    X,
    /// Top face button.
    Y,
    /// Left bumper.
    Lb,
    /// Right bumper.
    Rb,
    /// Left trigger (digital).
    Lt,
    /// Right trigger (digital).
    Rt,
    /// Left stick click.
    L3,
    /// Right stick click.
    R3,
    /// D-pad up.
    DpadUp,
    /// D-pad down.
    DpadDown,
    /// D-pad left.
    DpadLeft,
    /// D-pad right.
    DpadRight,
    /// View / Back / Select.
    View,
    /// Menu / Start.
    Menu,
    /// Left mouse button.
    MouseLeft,
    /// Right mouse button.
    MouseRight,
    /// Middle mouse button.
    MouseMiddle,
}

/// A keyboard key by lowercase name (see the module docs).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Key(pub String);

impl Key {
    /// Construct from a name.
    pub fn new(name: &str) -> Key {
        Key(name.to_owned())
    }
}

/// An analog axis.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// Left stick horizontal, `+` right.
    LeftX,
    /// Left stick vertical, `+` up.
    LeftY,
    /// Right stick horizontal, `+` right.
    RightX,
    /// Right stick vertical, `+` up.
    RightY,
    /// Left trigger, `0..=32767`.
    LeftTrigger,
    /// Right trigger, `0..=32767`.
    RightTrigger,
}

/// What happened.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    /// A button went down.
    ButtonDown {
        /// Which button.
        button: Button,
    },
    /// A button came up.
    ButtonUp {
        /// Which button.
        button: Button,
    },
    /// A key went down (adapters drop OS auto-repeat).
    KeyDown {
        /// Which key.
        key: Key,
    },
    /// A key came up.
    KeyUp {
        /// Which key.
        key: Key,
    },
    /// An axis changed.
    Axis {
        /// Which axis.
        axis: Axis,
        /// New value, `-32767..=32767`.
        value: i16,
    },
    /// The pointer moved (absolute screen pixels).
    PointerMove {
        /// Pixels from the left edge.
        x: i32,
        /// Pixels from the top edge.
        y: i32,
    },
    /// Pointer over a map surface. Screen coordinates still describe the
    /// gesture, but orders use the exact world point, even off-camera.
    /// Unlike a terrain pointer, this does not trigger edge panning.
    MapPointer {
        /// Pixels from the left edge.
        x: i32,
        /// Pixels from the top edge.
        y: i32,
        /// Ground point represented by this map pixel.
        world: tak_commands::WorldPos,
    },
    /// Mouse wheel: thousandths of a notch, positive away from the user
    /// (zoom in).
    Scroll {
        /// Notches times 1000.
        milli: i32,
    },
    /// A frame boundary: time passed with no other input.
    Frame,
}

/// A timestamped event. JSON: `{"t": 120, "type": "button_down", "button": "a"}`.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct InputEvent {
    /// Milliseconds on the adapter's monotonic clock.
    pub t: u32,
    /// The event.
    #[serde(flatten)]
    pub kind: EventKind,
}

impl InputEvent {
    /// Construct.
    pub fn new(t: u32, kind: EventKind) -> InputEvent {
        InputEvent { t, kind }
    }
}

/// A button or key: anything a binding can name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    /// `{"button": "a"}`
    Button(Button),
    /// `{"key": "shift"}`
    Key(Key),
}
