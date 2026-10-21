//! Stick shaping, cursor motion and aim assist (friction and soft snap).
//! Pure functions over [`tak_math`] types.

use tak_math::{Fx, Vec2};

use crate::tuning::Tuning;
use crate::world::fx_int;

/// Raw axis full scale.
pub const AXIS_MAX: i32 = 32767;

/// Raw stick axes to a vector with a radial deadzone, rescaled so the
/// deadzone edge maps to 0 and full deflection to length 1 (`+y` up).
/// Full deflection along one axis gives exactly `(±1, 0)` or `(0, ±1)`.
pub fn stick_vector(x: i16, y: i16, deadzone: i32) -> Vec2 {
    stick_vector_zones(x, y, deadzone, 0)
}

/// [`stick_vector`] with an outer deadzone as well: deflection within
/// `outer` raw units of full scale counts as full.
pub fn stick_vector_zones(x: i16, y: i16, inner: i32, outer: i32) -> Vec2 {
    let ax = |v: i16| Fx::from_ratio(i32::from(v).max(-AXIS_MAX), AXIS_MAX).unwrap_or(Fx::ZERO);
    let v = Vec2::new(ax(x), ax(y));
    let len = v.length();
    let inner = inner.clamp(0, AXIS_MAX - 2);
    let full = (AXIS_MAX - outer.max(0)).clamp(inner + 1, AXIS_MAX);
    let dz = Fx::from_ratio(inner, AXIS_MAX).unwrap_or(Fx::ZERO);
    let top = Fx::from_ratio(full, AXIS_MAX).unwrap_or(Fx::ONE);
    if len <= dz {
        return Vec2::ZERO;
    }
    let shaped = (len - dz)
        .checked_div(top - dz)
        .unwrap_or(Fx::ONE)
        .min(Fx::ONE);
    let factor = shaped.checked_div(len).unwrap_or(Fx::ZERO);
    v.scale(factor)
}

/// `rate_per_s * dt_ms / 1000` as `Fx`, saturating.
pub fn per_second(rate_per_s: i32, dt_ms: u32) -> Fx {
    // At most 2^31 * 2^32 * 2^16 * 2: fits in i128 with room to spare.
    let num = i128::from(rate_per_s) * i128::from(dt_ms) * i128::from(Fx::ONE_RAW);
    let q = (2 * num + 1000).div_euclid(2000);
    Fx::from_raw(q.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32)
}

/// `x * pct / 100`, saturating.
pub fn pct(x: Fx, pct: i32) -> Fx {
    let r = i64::from(x.raw()) * i64::from(pct) / 100;
    Fx::from_raw(r.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
}

/// A power curve on `m` in `[0, 1]` with exponent `exp_pct / 100`
/// (clamped to `50..=400`). Non-integer exponents blend the neighbouring
/// integer powers (`1.5`: half `m` and half `m²`; below 1, `√m` and `m`),
/// which keeps it integer-only, monotonic and exact at 0 and 1.
pub fn curve(m: Fx, exp_pct: i32) -> Fx {
    let m = m.clamp(Fx::ZERO, Fx::ONE);
    let e = exp_pct.clamp(50, 400);
    if e == 100 {
        return m;
    }
    if e < 100 {
        let frac = Fx::from_ratio(e - 50, 50).unwrap_or(Fx::ZERO);
        return m.sqrt().lerp(m, frac);
    }
    let n = e / 100;
    let frac = Fx::from_ratio(e % 100, 100).unwrap_or(Fx::ZERO);
    let mut lo = m;
    for _ in 1..n {
        lo *= m;
    }
    lo.lerp(lo * m, frac)
}

/// `0.5^(i/16)` in 16.16 for `i` in `0..=16`.
const HALF_POW: [i32; 17] = [
    65536, 62757, 60097, 57549, 55109, 52773, 50535, 48393, 46341, 44376, 42495, 40693, 38968,
    37316, 35734, 34219, 32768,
];

/// Fraction of a gap an exponential approach closes in `dt_ms` with the
/// given half-life: `1 - 0.5^(dt / half_life)`. A half-life of 0 closes it
/// at once. Frame-rate independent: two steps of `dt` close exactly what
/// one step of `2·dt` does (up to rounding).
pub fn approach_frac(dt_ms: u32, half_life_ms: u32) -> Fx {
    if half_life_ms == 0 {
        return Fx::ONE;
    }
    // dt / half-life in 1/16ths, linear between table entries.
    let x = u64::from(dt_ms) * 16 * 256 / u64::from(half_life_ms); // 1/4096ths of a half-life
    let whole = x / 4096;
    if whole >= 31 {
        return Fx::ONE;
    }
    let sixteenth = ((x % 4096) / 256) as usize;
    let rem = (x % 256) as i64;
    let a = i64::from(HALF_POW[sixteenth]);
    let b = i64::from(HALF_POW[sixteenth + 1]);
    let left = (a + (b - a) * rem / 256) >> whole;
    Fx::from_raw(Fx::ONE_RAW - left as i32)
}

/// Screen-space cursor displacement for a stick vector held for `dt_ms`,
/// linear response (the WP-J rule). Stick `+y` is up, screen `+y` is down.
pub fn cursor_step(stick: Vec2, dt_ms: u32, precision: bool, t: &Tuning) -> Vec2 {
    cursor_motion(
        stick,
        dt_ms,
        &Motion {
            precision,
            ..Motion::neutral()
        },
        &Tuning {
            cursor_curve_pct: 100,
            cursor_zoom_pct: 0,
            cursor_accel_pct: 0,
            ..*t
        },
    )
}

/// Conditions for one cursor step besides the stick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Motion {
    /// Precision mode held.
    pub precision: bool,
    /// Current zoom, world units per pixel.
    pub world_per_px: Fx,
    /// Acceleration ramp progress, `0..=1`.
    pub accel: Fx,
    /// Friction speed multiplier, `0..=1` ([`friction`]).
    pub friction: Fx,
}

impl Motion {
    /// No precision, reference zoom, no ramp, no friction.
    pub fn neutral() -> Motion {
        Motion {
            precision: false,
            world_per_px: Fx::ZERO,
            accel: Fx::ZERO,
            friction: Fx::ONE,
        }
    }
}

/// Cursor speed multiplier for the zoom: `1 + (ref / zoom - 1) * pct`,
/// clamped to `1/4..=4`. A zero zoom means "reference".
pub fn zoom_factor(world_per_px: Fx, t: &Tuning) -> Fx {
    if t.cursor_zoom_pct == 0 || world_per_px <= Fx::ZERO {
        return Fx::ONE;
    }
    let reference = Fx::from_ratio(t.zoom_ref_milli.max(1), 1000).unwrap_or(Fx::ONE);
    let ratio = reference.checked_div(world_per_px).unwrap_or(Fx::ONE);
    let k = Fx::ONE + pct(ratio - Fx::ONE, t.cursor_zoom_pct.clamp(0, 100));
    k.clamp(Fx::from_ratio(1, 4).unwrap_or(Fx::ONE), Fx::from_int(4))
}

/// Screen-space cursor displacement for a (deadzoned) stick held for
/// `dt_ms`: response curve, acceleration, zoom scaling, friction and the
/// precision multiplier, all as one speed multiplier on the stick
/// direction. With a linear curve and every modifier neutral this is
/// exactly `stick * cursor_speed * dt` (the WP-J rule).
pub fn cursor_motion(stick: Vec2, dt_ms: u32, m: &Motion, t: &Tuning) -> Vec2 {
    if stick == Vec2::ZERO {
        return Vec2::ZERO;
    }
    let mut speed = per_second(t.cursor_speed, dt_ms);
    let mut dir = stick;
    if t.cursor_curve_pct != 100 {
        let len = stick.length().min(Fx::ONE);
        let shaped = curve(len, t.cursor_curve_pct);
        dir = stick.scale(shaped.checked_div(len).unwrap_or(Fx::ZERO));
    }
    if t.cursor_accel_pct > 0 && m.accel > Fx::ZERO {
        speed = speed.saturating_add(pct(speed, t.cursor_accel_pct).saturating_mul(m.accel));
    }
    speed = speed.saturating_mul(zoom_factor(m.world_per_px, t));
    if m.friction < Fx::ONE {
        speed = speed.saturating_mul(m.friction.max(Fx::ZERO));
    }
    if m.precision {
        speed = pct(speed, t.precision_pct.clamp(1, 100));
    }
    let d = dir.scale(speed);
    Vec2::new(d.x, -d.y)
}

/// Friction speed multiplier at `cursor` (screen pixels): `friction_pct`
/// over a target's centre, rising linearly to 1 at `friction_radius_px`;
/// the nearest target counts. 1 when friction is off or nothing is near.
pub fn friction(cursor: Vec2, targets: &[Vec2], t: &Tuning) -> Fx {
    if t.friction_pct >= 100 || t.friction_radius_px <= 0 {
        return Fx::ONE;
    }
    let r = fx_int(t.friction_radius_px);
    let Some(d) = targets
        .iter()
        .map(|&p| cursor.distance(p))
        .filter(|&d| d < r)
        .min()
    else {
        return Fx::ONE;
    };
    let floor = Fx::from_ratio(t.friction_pct.clamp(0, 100), 100).unwrap_or(Fx::ONE);
    let k = d.checked_div(r).unwrap_or(Fx::ONE);
    floor.lerp(Fx::ONE, k)
}

/// Cursor magnetism, applied on a frame while the stick is idle and
/// precision is off: find the nearest target within `magnet_radius_px`
/// (ties go to the earlier target, so pass targets sorted by unit id); within
/// `magnet_snap_px` snap onto it, otherwise move `min(1, pull * dt)` of the
/// way there. Returns the new cursor and the index of the target, if any.
///
/// All positions are screen pixels. This is the WP-J rule;
/// [`soft_snap`] adds hysteresis and the exponential pull.
pub fn magnetize(cursor: Vec2, targets: &[Vec2], dt_ms: u32, t: &Tuning) -> (Vec2, Option<usize>) {
    soft_snap(
        cursor,
        targets,
        None,
        dt_ms,
        &Tuning {
            snap_half_life_ms: 0,
            snap_hysteresis_px: 0,
            ..*t
        },
    )
}

/// Soft snap with hysteresis. `current` is the index of the target the
/// cursor was snapping to last frame (if still a target). The nearest
/// target within `magnet_radius_px` wins, except that `current` is kept
/// while it is within the radius plus `snap_hysteresis_px` and no other
/// target is nearer by more than `snap_hysteresis_px`. Within
/// `magnet_snap_px` the cursor locks on; otherwise it closes
/// [`approach_frac`] of the gap (or the linear WP-J pull when
/// `snap_half_life_ms` is 0).
pub fn soft_snap(
    cursor: Vec2,
    targets: &[Vec2],
    current: Option<usize>,
    dt_ms: u32,
    t: &Tuning,
) -> (Vec2, Option<usize>) {
    let radius = fx_int(t.magnet_radius_px);
    let hyst = fx_int(t.snap_hysteresis_px.max(0));
    let mut best: Option<(Fx, usize)> = None;
    for (i, &p) in targets.iter().enumerate() {
        let d = cursor.distance(p);
        if d <= radius && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    if let Some(c) = current.filter(|&c| c < targets.len() && hyst > Fx::ZERO) {
        let dc = cursor.distance(targets[c]);
        if dc <= radius.saturating_add(hyst)
            && best.is_none_or(|(bd, _)| bd.saturating_add(hyst) >= dc)
        {
            best = Some((dc, c));
        }
    }
    let Some((d, i)) = best else {
        return (cursor, None);
    };
    let target = targets[i];
    if d <= fx_int(t.magnet_snap_px) {
        return (target, Some(i));
    }
    let frac = if t.snap_half_life_ms > 0 {
        approach_frac(dt_ms, t.snap_half_life_ms)
    } else {
        per_second(t.magnet_pull_per_s, dt_ms).min(Fx::ONE)
    };
    if frac == Fx::ONE {
        return (target, Some(i));
    }
    (cursor + (target - cursor).scale(frac), Some(i))
}
