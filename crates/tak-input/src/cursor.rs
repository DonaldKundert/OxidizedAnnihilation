//! Stick shaping, cursor motion and cursor magnetism. Pure functions over
//! [`tak_math`] types.

use tak_math::{Fx, Vec2};

use crate::tuning::Tuning;
use crate::world::fx_int;

/// Raw axis full scale.
pub const AXIS_MAX: i32 = 32767;

/// Raw stick axes to a vector with a radial deadzone, rescaled so the
/// deadzone edge maps to 0 and full deflection to length 1 (`+y` up).
/// Full deflection along one axis gives exactly `(±1, 0)` or `(0, ±1)`.
pub fn stick_vector(x: i16, y: i16, deadzone: i32) -> Vec2 {
    let ax = |v: i16| Fx::from_ratio(i32::from(v).max(-AXIS_MAX), AXIS_MAX).unwrap_or(Fx::ZERO);
    let v = Vec2::new(ax(x), ax(y));
    let len = v.length();
    let dz = Fx::from_ratio(deadzone.clamp(0, AXIS_MAX - 1), AXIS_MAX).unwrap_or(Fx::ZERO);
    if len <= dz {
        return Vec2::ZERO;
    }
    let shaped = (len - dz)
        .checked_div(Fx::ONE - dz)
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

/// Screen-space cursor displacement for a stick vector held for `dt_ms`.
/// Stick `+y` is up, screen `+y` is down.
pub fn cursor_step(stick: Vec2, dt_ms: u32, precision: bool, t: &Tuning) -> Vec2 {
    let mut speed = per_second(t.cursor_speed, dt_ms);
    if precision {
        speed = speed
            .checked_div_int(t.precision_divisor.max(1))
            .unwrap_or(speed);
    }
    let d = stick.scale(speed);
    Vec2::new(d.x, -d.y)
}

/// Cursor magnetism, applied on a frame while the stick is idle and
/// precision is off: find the nearest target within `magnet_radius_px`
/// (ties go to the earlier target, so pass targets sorted by unit id); within
/// `magnet_snap_px` snap onto it, otherwise move `min(1, pull * dt)` of the
/// way there. Returns the new cursor and the index of the target, if any.
///
/// All positions are screen pixels.
pub fn magnetize(cursor: Vec2, targets: &[Vec2], dt_ms: u32, t: &Tuning) -> (Vec2, Option<usize>) {
    let radius = fx_int(t.magnet_radius_px);
    let mut best: Option<(Fx, usize)> = None;
    for (i, &p) in targets.iter().enumerate() {
        let d = cursor.distance(p);
        if d <= radius && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    let Some((d, i)) = best else {
        return (cursor, None);
    };
    let target = targets[i];
    if d <= fx_int(t.magnet_snap_px) {
        return (target, Some(i));
    }
    let frac = per_second(t.magnet_pull_per_s, dt_ms).min(Fx::ONE);
    if frac == Fx::ONE {
        return (target, Some(i));
    }
    (cursor + (target - cursor).scale(frac), Some(i))
}
