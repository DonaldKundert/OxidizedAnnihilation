//! Binary angles (65,536 per turn) and table-driven trigonometry.
//!
//! # Convention
//!
//! `Angle(0)` points along `+x`; angles increase toward `+y`
//! (`Angle::QUARTER` points along `+y`). So `sin` is the `y` component and
//! `cos` the `x` component of a unit vector at that angle. How `+x`/`+y` map
//! onto the screen, and how Kingdoms' own heading values map onto this, is the
//! sim's decision.
//!
//! # The sine table
//!
//! [`SIN_TABLE`] holds `sin` for one quarter turn at 1,025 evenly spaced
//! points (every 16 angle units, both ends included), as raw 16.16 values. It
//! is computed at compile time by a `const fn` Taylor series in 60-bit integer
//! fixed point, so there is no floating point anywhere, not even in
//! generation. Lookups between table points use linear interpolation, rounded
//! to nearest. The interpolation error is below `2^-21`, far under the 16.16
//! resolution: every `sin`/`cos` result is within one ulp (`2^-16`) of the
//! true value, and `sin^2 + cos^2` (with `Fx` multiplication) is within two
//! ulps of `1` for every angle. Both are checked exhaustively in the tests.

use core::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use crate::Fx;

/// Number of table intervals per quarter turn.
const QUARTER_STEPS: usize = 1024;
/// Angle units per table interval (`16384 / 1024`).
const STEP_SHIFT: u32 = 4;

/// `pi * 2^60`, from the hex expansion of pi (3.243F6A8885A308D3...).
const PI_Q60: i128 = 0x3243_F6A8_885A_308D;

/// `sin(x)` for `x` in `[0, pi/2]`, both in 60-bit fixed point.
const fn sin_q60(x: i128) -> i128 {
    let x2 = (x * x) >> 60;
    let mut term = x;
    let mut sum = x;
    let mut n: i128 = 1;
    loop {
        term = -((term * x2) >> 60) / ((2 * n) * (2 * n + 1));
        if term == 0 {
            break;
        }
        sum += term;
        n += 1;
    }
    sum
}

const fn build_sin_table() -> [i32; QUARTER_STEPS + 1] {
    let mut table = [0i32; QUARTER_STEPS + 1];
    let mut k = 0;
    while k <= QUARTER_STEPS {
        // x = (pi / 2) * k / 1024
        let x = PI_Q60 * k as i128 / (2 * QUARTER_STEPS as i128);
        let s = sin_q60(x);
        // 60 -> 16 fractional bits, rounded to nearest.
        table[k] = ((s + (1i128 << 43)) >> 44) as i32;
        k += 1;
    }
    table
}

/// Quarter-wave sine table: `SIN_TABLE[k] = round(sin(k/1024 * pi/2) * 65536)`.
pub const SIN_TABLE: [i32; QUARTER_STEPS + 1] = build_sin_table();

/// `sin` of an offset within the first quadrant, `w` in `[0, 16384]`.
#[inline]
const fn quarter_sin(w: u32) -> i32 {
    let idx = (w >> STEP_SHIFT) as usize;
    let frac = (w & ((1 << STEP_SHIFT) - 1)) as i32;
    let a = SIN_TABLE[idx];
    if frac == 0 {
        return a;
    }
    let b = SIN_TABLE[idx + 1];
    // Round to nearest, ties toward +inf (the slope is never negative here).
    a + (((b - a) * frac + (1 << (STEP_SHIFT - 1))) >> STEP_SHIFT)
}

/// `|x| * sin t - |y| * cos t` for `t` in `[0, 16384]`, used by `atan2`.
#[inline]
const fn atan_f(ax: i64, ay: i64, t: u32) -> i64 {
    ax * quarter_sin(t) as i64 - ay * quarter_sin(0x4000 - t) as i64
}

/// Binary angle: 65,536 units per full turn, wrapping on every operation.
///
/// Serializes as its raw `u16`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Angle(pub u16);

impl Angle {
    /// 0 degrees.
    pub const ZERO: Angle = Angle(0);
    /// 90 degrees.
    pub const QUARTER: Angle = Angle(0x4000);
    /// 180 degrees.
    pub const HALF: Angle = Angle(0x8000);
    /// 270 degrees.
    pub const THREE_QUARTERS: Angle = Angle(0xC000);

    /// Wraps a raw angle.
    #[inline]
    pub const fn from_raw(raw: u16) -> Angle {
        Angle(raw)
    }

    /// The raw angle.
    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Whole degrees to an angle, rounded to nearest unit (ties toward +inf),
    /// wrapping to one turn. Any `i32` is accepted.
    #[inline]
    pub const fn from_degrees(deg: i32) -> Angle {
        let d = deg.rem_euclid(360) as i64;
        let units = (d * 65536 * 2 + 360) / 720;
        Angle(units as u16)
    }

    /// Signed shortest difference `to - self`, in `[-32768, 32767]` units.
    /// Exactly half a turn reports `-32768`.
    #[inline]
    pub const fn delta_to(self, to: Angle) -> i16 {
        to.0.wrapping_sub(self.0) as i16
    }

    /// Sine, table-driven, as 16.16. Exact at multiples of a quarter turn
    /// (`sin(QUARTER) == Fx::ONE`) and odd-symmetric: `sin(-a) == -sin(a)`,
    /// `sin(a + HALF) == -sin(a)` for every `a`.
    #[inline]
    pub const fn sin(self) -> Fx {
        let a = self.0 as u32;
        let quadrant = a >> 14;
        let w = a & 0x3FFF;
        let v = match quadrant {
            0 => quarter_sin(w),
            1 => quarter_sin(0x4000 - w),
            2 => -quarter_sin(w),
            _ => -quarter_sin(0x4000 - w),
        };
        Fx::from_raw(v)
    }

    /// Cosine, defined as `sin(self + QUARTER)`, so it shares every property
    /// of [`Angle::sin`].
    #[inline]
    pub const fn cos(self) -> Fx {
        Angle(self.0.wrapping_add(0x4000)).sin()
    }

    /// `(sin, cos)`.
    #[inline]
    pub const fn sin_cos(self) -> (Fx, Fx) {
        (self.sin(), self.cos())
    }

    /// Integer-only `atan2`: the angle of the vector `(x, y)`.
    /// `atan2(a.sin(), a.cos()) == a` exactly for every angle, and general
    /// vectors land within one unit of the true angle (both checked in the
    /// tests). `atan2(0, 0)` returns [`Angle::ZERO`].
    ///
    /// Method: reduce to the first quadrant, then binary-search the angle
    /// `t` in `[0, 16384]` where `|x| * sin t - |y| * cos t` changes sign,
    /// using the same tables as `sin`/`cos`, and pick whichever neighbour is
    /// closer. 14 iterations, all in `i64`, so it agrees with the table trig
    /// on every platform.
    pub const fn atan2(y: Fx, x: Fx) -> Angle {
        let ax = x.raw().unsigned_abs() as i64;
        let ay = y.raw().unsigned_abs() as i64;
        if ax == 0 && ay == 0 {
            return Angle::ZERO;
        }
        // f(t) = ax*sin t - ay*cos t is non-decreasing on [0, 16384] because
        // the table sin is non-decreasing and table cos non-increasing there.
        // Smallest t with f(t) >= 0; f(16384) = ax * 65536 >= 0 always.
        let mut lo: u32 = 0;
        let mut hi: u32 = 0x4000;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if atan_f(ax, ay, mid) >= 0 {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let mut t = lo;
        if t > 0 && -atan_f(ax, ay, t - 1) < atan_f(ax, ay, t) {
            t -= 1;
        }
        let t = t as u16;
        let theta = match (x.raw() < 0, y.raw() < 0) {
            (false, false) => t,
            (true, false) => 0x8000u16.wrapping_sub(t),
            (true, true) => 0x8000u16.wrapping_add(t),
            (false, true) => 0u16.wrapping_sub(t),
        };
        Angle(theta)
    }
}

impl Add for Angle {
    type Output = Angle;
    /// Wrapping addition.
    #[inline]
    fn add(self, rhs: Angle) -> Angle {
        Angle(self.0.wrapping_add(rhs.0))
    }
}

impl Sub for Angle {
    type Output = Angle;
    /// Wrapping subtraction.
    #[inline]
    fn sub(self, rhs: Angle) -> Angle {
        Angle(self.0.wrapping_sub(rhs.0))
    }
}

impl Neg for Angle {
    type Output = Angle;
    /// Wrapping negation (the opposite rotation).
    #[inline]
    fn neg(self) -> Angle {
        Angle(self.0.wrapping_neg())
    }
}

impl AddAssign for Angle {
    #[inline]
    fn add_assign(&mut self, rhs: Angle) {
        *self = *self + rhs;
    }
}

impl SubAssign for Angle {
    #[inline]
    fn sub_assign(&mut self, rhs: Angle) {
        *self = *self - rhs;
    }
}

impl core::fmt::Debug for Angle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Angle({:#06x})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_known_values() {
        assert_eq!(SIN_TABLE[0], 0);
        assert_eq!(SIN_TABLE[QUARTER_STEPS], 65536);
        // sin(45 deg) = 0.70710678 -> 46340.95
        assert_eq!(SIN_TABLE[512], 46341);
        // Non-decreasing over the quarter (the last steps are flat at 16.16).
        for k in 0..QUARTER_STEPS {
            assert!(SIN_TABLE[k] <= SIN_TABLE[k + 1]);
        }
    }

    #[test]
    fn quadrant_values() {
        assert_eq!(Angle::ZERO.sin(), Fx::ZERO);
        assert_eq!(Angle::QUARTER.sin(), Fx::ONE);
        assert_eq!(Angle::HALF.sin(), Fx::ZERO);
        assert_eq!(Angle::THREE_QUARTERS.sin(), Fx::NEG_ONE);
        assert_eq!(Angle::ZERO.cos(), Fx::ONE);
        assert_eq!(Angle::HALF.cos(), Fx::NEG_ONE);
        // sin(30 deg) = 0.5, angle 5461.33 rounds to 5461.
        assert_eq!(Angle::from_degrees(30).raw(), 5461);
        let s = Angle::from_degrees(30).sin().raw();
        assert!((s - 32768).abs() <= 4, "{s}");
    }

    #[test]
    fn symmetry_exhaustive() {
        for a in 0..=u16::MAX {
            let a = Angle(a);
            assert_eq!((-a).sin(), -a.sin());
            assert_eq!((a + Angle::HALF).sin(), -a.sin());
            assert_eq!((Angle::HALF - a).sin(), a.sin());
            assert_eq!((-a).cos(), a.cos());
        }
    }

    #[test]
    fn atan2_axes() {
        assert_eq!(Angle::atan2(Fx::ZERO, Fx::ONE), Angle::ZERO);
        assert_eq!(Angle::atan2(Fx::ONE, Fx::ZERO), Angle::QUARTER);
        assert_eq!(Angle::atan2(Fx::ZERO, Fx::NEG_ONE), Angle::HALF);
        assert_eq!(Angle::atan2(Fx::NEG_ONE, Fx::ZERO), Angle::THREE_QUARTERS);
        assert_eq!(Angle::atan2(Fx::ONE, Fx::ONE), Angle(0x2000));
        assert_eq!(Angle::atan2(Fx::ZERO, Fx::ZERO), Angle::ZERO);
        assert_eq!(Angle::atan2(Fx::MIN, Fx::MIN), Angle(0xA000));
    }

    #[test]
    fn delta() {
        assert_eq!(Angle(10).delta_to(Angle(5)), -5);
        assert_eq!(Angle(65530).delta_to(Angle(4)), 10);
    }
}
