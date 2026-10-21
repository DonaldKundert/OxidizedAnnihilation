//! Feel constants. All integers (milliseconds, screen pixels, world units, raw
//! axis units), so a profile can override them in readable JSON and nothing
//! here needs floating point. Converted to [`Fx`](tak_math::Fx) at use.

use serde::{Deserialize, Serialize};

/// Timing, cursor, magnetism, brush, wheel and camera tuning. Every field has
/// a default, so a profile's `"tuning"` object may set any subset.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tuning {
    /// A press held this long becomes a hold (fires `hold` bindings).
    pub hold_ms: u32,
    /// A second press within this long of a release is a double tap.
    pub double_tap_ms: u32,
    /// Radial stick deadzone, raw axis units (of 32767).
    pub stick_deadzone: i32,
    /// Cursor speed at full deflection, screen pixels per second.
    pub cursor_speed: i32,
    /// Precision mode divides cursor speed by this.
    pub precision_divisor: i32,
    /// Magnetism attraction radius, screen pixels.
    pub magnet_radius_px: i32,
    /// Within this distance the cursor snaps onto the target, screen pixels.
    pub magnet_snap_px: i32,
    /// Fraction of the remaining offset pulled per second while the stick is
    /// idle (applied as `min(1, rate * dt)` each frame).
    pub magnet_pull_per_s: i32,
    /// Brush radius when it activates, world units.
    pub brush_min: i32,
    /// Brush radius growth while held, world units per second.
    pub brush_growth_per_s: i32,
    /// Largest brush radius, world units.
    pub brush_max: i32,
    /// A mouse drag shorter than this (screen pixels) is a click.
    pub drag_min_px: i32,
    /// Stick deflection needed to highlight a wheel slot, raw axis units.
    pub wheel_deadzone: i32,
    /// Most slots a wheel shows.
    pub wheel_max_slots: u8,
    /// Camera pan speed at full deflection, world units per second.
    pub pan_speed: i32,
}

impl Default for Tuning {
    fn default() -> Tuning {
        Tuning {
            hold_ms: 250,
            double_tap_ms: 300,
            stick_deadzone: 8000,
            cursor_speed: 900,
            precision_divisor: 4,
            magnet_radius_px: 40,
            magnet_snap_px: 12,
            magnet_pull_per_s: 8,
            brush_min: 16,
            brush_growth_per_s: 160,
            brush_max: 192,
            drag_min_px: 4,
            wheel_deadzone: 16000,
            wheel_max_slots: 16,
            pan_speed: 1200,
        }
    }
}
