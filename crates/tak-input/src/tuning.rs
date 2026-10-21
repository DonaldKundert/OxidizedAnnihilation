//! Feel constants. All integers (milliseconds, screen pixels, world units, raw
//! axis units, percentages, thousandths) or booleans, so a profile can
//! override them in readable JSON and nothing here needs floating point.
//! Converted to [`Fx`](tak_math::Fx) at use.
//!
//! Every value that shapes how input feels lives here, never in code:
//! [`TUNING_FIELDS`] lists them with a step, a range and a one-line
//! description, which is what the client's live tuning overlay shows.
//! [`Tuning::preset`] gives the named presets (`console`, `assisted`,
//! `precise`, and `wp_j`, the original WP-J values kept for comparison).

use serde::{Deserialize, Serialize};

/// Timing, cursor, aim assist, brush, wheel, camera and feedback tuning.
/// Every field has a default, so a profile's `"tuning"` object may set any
/// subset. See [`TUNING_FIELDS`] for what each one does.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tuning {
    // ---- gestures ----
    /// A press held this long becomes a hold (fires `hold` bindings).
    pub hold_ms: u32,
    /// A second press within this long of a release is a double tap.
    pub double_tap_ms: u32,

    // ---- stick and cursor ----
    /// Inner radial stick deadzone, raw axis units (of 32767).
    pub stick_deadzone: i32,
    /// Outer deadzone: deflection within this of full scale counts as full,
    /// raw axis units (0: none).
    pub stick_outer_deadzone: i32,
    /// Cursor speed at full deflection, screen pixels per second (at the
    /// reference zoom, see `cursor_zoom_pct`).
    pub cursor_speed: i32,
    /// Response curve exponent in hundredths (100: linear, 200: squared).
    /// Larger values give finer control near the centre.
    pub cursor_curve_pct: i32,
    /// Extra speed, percent, after holding near-full deflection for
    /// `cursor_accel_ms` (0: no acceleration).
    pub cursor_accel_pct: i32,
    /// Time at near-full deflection to reach the full acceleration bonus.
    pub cursor_accel_ms: u32,
    /// Deflection, percent of full, that counts as "near full" for the
    /// acceleration ramp.
    pub cursor_accel_threshold_pct: i32,
    /// How much the cursor speed follows the zoom, percent: 0 keeps screen
    /// speed constant, 100 keeps ground speed constant (the cursor covers
    /// the same ground per second at any zoom).
    pub cursor_zoom_pct: i32,
    /// Reference zoom for `cursor_zoom_pct` and `pan_zoom_pct`, thousandths
    /// of a world unit per pixel.
    pub zoom_ref_milli: i32,
    /// Precision mode (RT) multiplies cursor speed by this percentage.
    pub precision_pct: i32,

    // ---- aim assist ----
    /// Context target priority: enemies when own units are selected, own
    /// units when nothing is selected, frames under construction in build
    /// mode. Off: every pickable unit is an assist target (WP-J).
    pub assist_priority: bool,
    /// Friction: cursor speed at a target's centre, percent (100: off).
    /// Rises linearly to 100 at `friction_radius_px`.
    pub friction_pct: i32,
    /// Friction reaches this far from a target, screen pixels.
    pub friction_radius_px: i32,
    /// Soft snap radius: targets this close (screen pixels) pull the cursor
    /// while the stick is near idle.
    pub magnet_radius_px: i32,
    /// Within this distance the cursor locks onto the target, screen pixels.
    pub magnet_snap_px: i32,
    /// Linear pull, fraction of the remaining offset per second (used when
    /// `snap_half_life_ms` is 0; the WP-J rule, frame-rate dependent).
    pub magnet_pull_per_s: i32,
    /// Exponential pull: time to close half the remaining offset (0: use
    /// `magnet_pull_per_s`). Frame-rate independent.
    pub snap_half_life_ms: u32,
    /// Snap acts while stick deflection (after the deadzone) is at most
    /// this percentage (0: only with the stick fully idle).
    pub snap_idle_pct: i32,
    /// Hysteresis: a new snap target must be this much nearer (screen
    /// pixels) than the current one to take over.
    pub snap_hysteresis_px: i32,

    // ---- brush and drag ----
    /// Brush radius when it activates, world units.
    pub brush_min: i32,
    /// Brush radius growth while held, world units per second (average
    /// over the growth from `brush_min` to `brush_max`).
    pub brush_growth_per_s: i32,
    /// Largest brush radius, world units.
    pub brush_max: i32,
    /// Brush growth curve exponent in hundredths (100: linear; above 100
    /// grows slowly first, so small brushes are easier to hit).
    pub brush_curve_pct: i32,
    /// A mouse drag shorter than this (screen pixels) is a click.
    pub drag_min_px: i32,

    // ---- wheels ----
    /// Stick deflection needed to highlight a wheel slot, raw axis units.
    pub wheel_deadzone: i32,
    /// Most slots a wheel shows.
    pub wheel_max_slots: u8,
    /// Angle hysteresis, degrees: the highlight stays on a slot until the
    /// stick is this far past the slot's edge.
    pub wheel_hysteresis_deg: i32,
    /// Off: releasing the wheel button confirms (flick and release). On:
    /// the wheel stays open after release; A (or the context button)
    /// confirms, B or the wheel button again cancels.
    pub wheel_confirm_on_press: bool,
    /// Build the command wheel from what the selection can do (AoE IV
    /// style); with nothing selected show the profile's idle wheel.
    pub wheel_context: bool,
    /// Returning the stick to centre clears the highlight (cancels on
    /// release). Off keeps the last slot, so a flick and release works.
    pub wheel_recentre_clears: bool,

    // ---- camera ----
    /// Camera pan speed at full deflection, world units per second (at the
    /// reference zoom, see `pan_zoom_pct`).
    pub pan_speed: i32,
    /// How much pan speed follows the zoom, percent: 100 pans the same
    /// screen distance per second at any zoom.
    pub pan_zoom_pct: i32,
    /// Pan smoothing: time for the pan velocity to close half the gap to
    /// the stick's (0: instant).
    pub pan_smooth_ms: u32,
    /// Edge pan: the stick cursor pushing within this many pixels of the
    /// screen edge pans the camera (0: off).
    pub edge_pan_px: i32,
    /// Edge pan speed at full push, world units per second (at the
    /// reference zoom, scaled like `pan_speed`).
    pub edge_pan_speed: i32,
    /// Edge pan ramps from 0 to full speed over this long.
    pub edge_pan_ramp_ms: u32,
    /// Edge pan with the mouse pointer too.
    pub edge_pan_mouse: bool,
    /// Closest zoom, thousandths of a world unit per pixel.
    pub zoom_min_milli: i32,
    /// Farthest zoom, thousandths of a world unit per pixel.
    pub zoom_max_milli: i32,
    /// Zoom change per mouse-wheel notch, percent.
    pub zoom_step_pct: i32,
    /// Zoom speed with the zoom modifier (RB) + right stick at full
    /// deflection, percent per second.
    pub zoom_stick_pct_per_s: i32,
    /// Zoom smoothing: time to close half the gap to the target zoom
    /// (0: instant).
    pub zoom_smooth_ms: u32,
    /// Recentre and follow glide: time to close half the distance (0: jump).
    pub glide_ms: u32,
    /// Panning with the right stick or zooming keeps the stick cursor on the
    /// same ground point, so it stays on its target.
    pub cursor_world_lock: bool,

    // ---- feedback ----
    /// Selection rings flash this long after the selection changes (0: off).
    pub select_flash_ms: u32,
    /// Controller rumble (snap, selection, damage). Off by default.
    pub rumble: bool,
    /// Rumble strength on snapping to a target, percent.
    pub rumble_snap_pct: i32,
    /// Rumble strength on a selection change, percent.
    pub rumble_select_pct: i32,
    /// Rumble strength when own units take damage, percent.
    pub rumble_damage_pct: i32,
    /// Rumble pulse length.
    pub rumble_ms: u32,
}

impl Default for Tuning {
    /// The `console` preset.
    fn default() -> Tuning {
        Tuning::console()
    }
}

/// Names accepted by [`Tuning::preset`], in menu order.
pub const PRESETS: [&str; 4] = ["console", "assisted", "precise", "wp_j"];

impl Tuning {
    /// The original WP-J values: linear stick, no acceleration, no friction,
    /// linear pull only with the stick idle, every unit a target, instant
    /// pan and zoom. Kept so the feel lab can compare and old tests pin it.
    pub fn wp_j() -> Tuning {
        Tuning {
            hold_ms: 250,
            double_tap_ms: 300,
            stick_deadzone: 8000,
            stick_outer_deadzone: 0,
            cursor_speed: 900,
            cursor_curve_pct: 100,
            cursor_accel_pct: 0,
            cursor_accel_ms: 400,
            cursor_accel_threshold_pct: 90,
            cursor_zoom_pct: 0,
            zoom_ref_milli: 500,
            precision_pct: 25,
            assist_priority: false,
            friction_pct: 100,
            friction_radius_px: 24,
            magnet_radius_px: 40,
            magnet_snap_px: 12,
            magnet_pull_per_s: 8,
            snap_half_life_ms: 0,
            snap_idle_pct: 0,
            snap_hysteresis_px: 0,
            brush_min: 16,
            brush_growth_per_s: 160,
            brush_max: 192,
            brush_curve_pct: 100,
            drag_min_px: 4,
            wheel_deadzone: 16000,
            wheel_max_slots: 16,
            wheel_hysteresis_deg: 0,
            wheel_confirm_on_press: false,
            wheel_context: false,
            wheel_recentre_clears: false,
            pan_speed: 1200,
            pan_zoom_pct: 0,
            pan_smooth_ms: 0,
            edge_pan_px: 0,
            edge_pan_speed: 900,
            edge_pan_ramp_ms: 300,
            edge_pan_mouse: false,
            zoom_min_milli: 150,
            zoom_max_milli: 4000,
            zoom_step_pct: 10,
            zoom_stick_pct_per_s: 150,
            zoom_smooth_ms: 0,
            glide_ms: 0,
            cursor_world_lock: false,
            select_flash_ms: 0,
            rumble: false,
            rumble_snap_pct: 20,
            rumble_select_pct: 30,
            rumble_damage_pct: 60,
            rumble_ms: 80,
        }
    }

    /// "Console": Halo Wars-like defaults, chosen with the feel lab (see
    /// `docs/controller.md`).
    pub fn console() -> Tuning {
        Tuning {
            stick_deadzone: 7000,
            stick_outer_deadzone: 2000,
            cursor_speed: 1100,
            cursor_curve_pct: 110,
            cursor_accel_pct: 0,
            cursor_accel_ms: 350,
            cursor_zoom_pct: 100,
            precision_pct: 35,
            assist_priority: true,
            friction_pct: 30,
            friction_radius_px: 26,
            magnet_radius_px: 44,
            magnet_snap_px: 8,
            snap_half_life_ms: 45,
            snap_idle_pct: 15,
            snap_hysteresis_px: 10,
            brush_growth_per_s: 140,
            brush_curve_pct: 80,
            wheel_hysteresis_deg: 8,
            wheel_context: true,
            pan_zoom_pct: 100,
            pan_smooth_ms: 60,
            edge_pan_px: 24,
            edge_pan_speed: 1000,
            zoom_smooth_ms: 70,
            glide_ms: 90,
            cursor_world_lock: true,
            select_flash_ms: 250,
            ..Tuning::wp_j()
        }
    }

    /// "Assisted": stronger snap and friction, slower cursor.
    pub fn assisted() -> Tuning {
        Tuning {
            cursor_speed: 900,
            friction_pct: 20,
            friction_radius_px: 34,
            magnet_radius_px: 56,
            magnet_snap_px: 12,
            snap_half_life_ms: 30,
            snap_idle_pct: 25,
            snap_hysteresis_px: 14,
            brush_growth_per_s: 120,
            ..Tuning::console()
        }
    }

    /// "Precise": light assist, faster cursor.
    pub fn precise() -> Tuning {
        Tuning {
            cursor_speed: 1400,
            cursor_curve_pct: 100,
            precision_pct: 30,
            friction_pct: 45,
            friction_radius_px: 22,
            magnet_radius_px: 24,
            magnet_snap_px: 6,
            snap_half_life_ms: 60,
            snap_idle_pct: 5,
            snap_hysteresis_px: 6,
            brush_growth_per_s: 170,
            ..Tuning::console()
        }
    }

    /// A named preset (see [`PRESETS`]).
    pub fn preset(name: &str) -> Option<Tuning> {
        match name.to_ascii_lowercase().replace('-', "_").as_str() {
            "console" => Some(Tuning::console()),
            "assisted" => Some(Tuning::assisted()),
            "precise" => Some(Tuning::precise()),
            "wp_j" | "wpj" | "legacy" => Some(Tuning::wp_j()),
            _ => None,
        }
    }

    /// The preset these values equal, if any.
    pub fn preset_name(&self) -> Option<&'static str> {
        PRESETS
            .iter()
            .copied()
            .find(|n| Tuning::preset(n).as_ref() == Some(self))
    }

    /// A field's value by name (booleans as 0/1).
    pub fn get(&self, name: &str) -> Option<i64> {
        let v = serde_json::to_value(self).ok()?;
        match v.get(name)? {
            serde_json::Value::Bool(b) => Some(i64::from(*b)),
            n => n.as_i64(),
        }
    }

    /// Set a field by name, clamped to its range in [`TUNING_FIELDS`]
    /// (booleans: nonzero is on). Returns `false` for an unknown name.
    pub fn set(&mut self, name: &str, value: i64) -> bool {
        let Some(meta) = field(name) else {
            return false;
        };
        let Ok(serde_json::Value::Object(mut map)) = serde_json::to_value(*self) else {
            return false;
        };
        let v = value.clamp(meta.min, meta.max);
        let new = if meta.boolean {
            serde_json::Value::Bool(v != 0)
        } else {
            serde_json::Value::from(v)
        };
        map.insert(name.to_owned(), new);
        match serde_json::from_value(serde_json::Value::Object(map)) {
            Ok(t) => {
                *self = t;
                true
            }
            Err(_) => false,
        }
    }

    /// Step a field by `steps` of its step size (booleans toggle).
    pub fn nudge(&mut self, name: &str, steps: i64) -> bool {
        let (Some(meta), Some(cur)) = (field(name), self.get(name)) else {
            return false;
        };
        if meta.boolean {
            return steps == 0 || self.set(name, 1 - cur);
        }
        self.set(name, cur.saturating_add(steps.saturating_mul(meta.step)))
    }
}

/// One tunable value, for the overlay and the docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuningField {
    /// JSON name.
    pub name: &'static str,
    /// Group heading.
    pub group: &'static str,
    /// Unit shown after the value.
    pub unit: &'static str,
    /// Overlay step.
    pub step: i64,
    /// Smallest allowed value.
    pub min: i64,
    /// Largest allowed value.
    pub max: i64,
    /// On/off value.
    pub boolean: bool,
    /// What it does, one line.
    pub help: &'static str,
}

const fn f(
    name: &'static str,
    group: &'static str,
    unit: &'static str,
    step: i64,
    min: i64,
    max: i64,
    help: &'static str,
) -> TuningField {
    TuningField {
        name,
        group,
        unit,
        step,
        min,
        max,
        boolean: false,
        help,
    }
}

const fn b(name: &'static str, group: &'static str, help: &'static str) -> TuningField {
    TuningField {
        name,
        group,
        unit: "",
        step: 1,
        min: 0,
        max: 1,
        boolean: true,
        help,
    }
}

/// Every [`Tuning`] field in overlay order.
pub const TUNING_FIELDS: &[TuningField] = &[
    f(
        "cursor_speed",
        "Cursor",
        "px/s",
        50,
        100,
        4000,
        "Cursor speed at full stick",
    ),
    f(
        "cursor_curve_pct",
        "Cursor",
        "%",
        10,
        50,
        400,
        "Response curve exponent x100 (100 linear)",
    ),
    f(
        "stick_deadzone",
        "Cursor",
        "raw",
        500,
        0,
        20000,
        "Inner stick deadzone",
    ),
    f(
        "stick_outer_deadzone",
        "Cursor",
        "raw",
        500,
        0,
        12000,
        "Outer deadzone: this near full counts as full",
    ),
    f(
        "cursor_accel_pct",
        "Cursor",
        "%",
        10,
        0,
        300,
        "Extra speed after holding full stick",
    ),
    f(
        "cursor_accel_ms",
        "Cursor",
        "ms",
        50,
        0,
        3000,
        "Time to reach the full acceleration bonus",
    ),
    f(
        "cursor_accel_threshold_pct",
        "Cursor",
        "%",
        5,
        50,
        100,
        "Deflection that counts as full for acceleration",
    ),
    f(
        "cursor_zoom_pct",
        "Cursor",
        "%",
        10,
        0,
        100,
        "Cursor speed follows zoom (100: same ground speed)",
    ),
    f(
        "precision_pct",
        "Cursor",
        "%",
        5,
        5,
        100,
        "Precision (RT) speed multiplier",
    ),
    b(
        "assist_priority",
        "Aim assist",
        "Context targets: enemies with units selected, own units otherwise",
    ),
    f(
        "friction_pct",
        "Aim assist",
        "%",
        5,
        5,
        100,
        "Cursor speed over a target's centre (100: off)",
    ),
    f(
        "friction_radius_px",
        "Aim assist",
        "px",
        2,
        0,
        120,
        "Friction reach around a target",
    ),
    f(
        "magnet_radius_px",
        "Aim assist",
        "px",
        2,
        0,
        160,
        "Soft snap radius",
    ),
    f(
        "magnet_snap_px",
        "Aim assist",
        "px",
        1,
        0,
        60,
        "Lock-on distance",
    ),
    f(
        "snap_half_life_ms",
        "Aim assist",
        "ms",
        5,
        0,
        1000,
        "Snap pull half-life (0: linear WP-J pull)",
    ),
    f(
        "magnet_pull_per_s",
        "Aim assist",
        "/s",
        1,
        0,
        100,
        "Linear pull per second (when half-life is 0)",
    ),
    f(
        "snap_idle_pct",
        "Aim assist",
        "%",
        5,
        0,
        100,
        "Snap acts below this stick deflection",
    ),
    f(
        "snap_hysteresis_px",
        "Aim assist",
        "px",
        1,
        0,
        60,
        "A new target must be this much nearer to take over",
    ),
    f(
        "hold_ms",
        "Buttons",
        "ms",
        10,
        50,
        2000,
        "Press longer than this is a hold",
    ),
    f(
        "double_tap_ms",
        "Buttons",
        "ms",
        10,
        50,
        2000,
        "Second press within this is a double tap",
    ),
    f(
        "brush_min",
        "Brush",
        "wu",
        2,
        0,
        400,
        "Brush start radius (world units)",
    ),
    f(
        "brush_growth_per_s",
        "Brush",
        "wu/s",
        10,
        10,
        2000,
        "Brush growth speed",
    ),
    f(
        "brush_max",
        "Brush",
        "wu",
        8,
        8,
        1000,
        "Brush largest radius",
    ),
    f(
        "brush_curve_pct",
        "Brush",
        "%",
        10,
        50,
        400,
        "Brush growth curve x100 (above 100: slow start)",
    ),
    f(
        "drag_min_px",
        "Brush",
        "px",
        1,
        0,
        50,
        "Mouse drag shorter than this is a click",
    ),
    f(
        "wheel_deadzone",
        "Wheels",
        "raw",
        1000,
        1000,
        30000,
        "Stick deflection to highlight a slot",
    ),
    f(
        "wheel_max_slots",
        "Wheels",
        "",
        1,
        1,
        24,
        "Most slots on a wheel",
    ),
    f(
        "wheel_hysteresis_deg",
        "Wheels",
        "deg",
        1,
        0,
        45,
        "Stay on a slot until this far past its edge",
    ),
    b(
        "wheel_confirm_on_press",
        "Wheels",
        "Wheel stays open; A confirms (off: release confirms)",
    ),
    b(
        "wheel_context",
        "Wheels",
        "Wheel shows what the selection can do",
    ),
    b(
        "wheel_recentre_clears",
        "Wheels",
        "Stick back to centre clears the highlight",
    ),
    f(
        "pan_speed",
        "Camera",
        "wu/s",
        50,
        100,
        6000,
        "Right-stick pan speed",
    ),
    f(
        "pan_zoom_pct",
        "Camera",
        "%",
        10,
        0,
        100,
        "Pan speed follows zoom (100: same screen speed)",
    ),
    f(
        "pan_smooth_ms",
        "Camera",
        "ms",
        10,
        0,
        1000,
        "Pan smoothing half-life (0: instant)",
    ),
    f(
        "edge_pan_px",
        "Camera",
        "px",
        2,
        0,
        200,
        "Edge pan margin for the stick cursor (0: off)",
    ),
    f(
        "edge_pan_speed",
        "Camera",
        "wu/s",
        50,
        0,
        6000,
        "Edge pan speed",
    ),
    f(
        "edge_pan_ramp_ms",
        "Camera",
        "ms",
        25,
        0,
        3000,
        "Edge pan ramp-up time",
    ),
    b("edge_pan_mouse", "Camera", "Edge pan with the mouse too"),
    f(
        "zoom_min_milli",
        "Camera",
        "/1000",
        10,
        50,
        4000,
        "Closest zoom (world units per pixel x1000)",
    ),
    f(
        "zoom_max_milli",
        "Camera",
        "/1000",
        100,
        100,
        16000,
        "Farthest zoom",
    ),
    f(
        "zoom_step_pct",
        "Camera",
        "%",
        1,
        1,
        50,
        "Zoom per mouse-wheel notch",
    ),
    f(
        "zoom_stick_pct_per_s",
        "Camera",
        "%/s",
        10,
        10,
        1000,
        "RB + right stick zoom speed",
    ),
    f(
        "zoom_smooth_ms",
        "Camera",
        "ms",
        10,
        0,
        1000,
        "Zoom smoothing half-life (0: instant)",
    ),
    f(
        "glide_ms",
        "Camera",
        "ms",
        10,
        0,
        2000,
        "Recentre / follow glide half-life (0: jump)",
    ),
    b(
        "cursor_world_lock",
        "Camera",
        "Panning and zooming keep the cursor on its ground point",
    ),
    f(
        "zoom_ref_milli",
        "Camera",
        "/1000",
        50,
        50,
        4000,
        "Reference zoom for the zoom-scaled speeds",
    ),
    f(
        "select_flash_ms",
        "Feedback",
        "ms",
        25,
        0,
        2000,
        "Selection ring flash length",
    ),
    b(
        "rumble",
        "Feedback",
        "Controller rumble on snap, selection and damage",
    ),
    f(
        "rumble_snap_pct",
        "Feedback",
        "%",
        5,
        0,
        100,
        "Rumble strength on snap",
    ),
    f(
        "rumble_select_pct",
        "Feedback",
        "%",
        5,
        0,
        100,
        "Rumble strength on selection",
    ),
    f(
        "rumble_damage_pct",
        "Feedback",
        "%",
        5,
        0,
        100,
        "Rumble strength on damage",
    ),
    f(
        "rumble_ms",
        "Feedback",
        "ms",
        10,
        10,
        1000,
        "Rumble pulse length",
    ),
];

/// Metadata for one field.
pub fn field(name: &str) -> Option<&'static TuningField> {
    TUNING_FIELDS.iter().find(|m| m.name == name)
}
