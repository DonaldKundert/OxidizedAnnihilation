//! Signed 16.16 fixed point.
//!
//! # Overflow policy
//!
//! The arithmetic operators (`+`, `-`, `*`, unary `-`) **wrap** on overflow, in
//! every build profile. Rust's default integer operators panic in debug builds
//! and wrap in release builds; a simulation that behaves differently between
//! the two is not deterministic, so `Fx` never uses them on its raw value.
//! Callers that care about overflow use the explicit `checked_*` (returns
//! `None`) or `saturating_*` (clamps to [`Fx::MIN`], [`Fx::MAX`]) methods.
//! Division has no operator: it can fail on a zero divisor, so it is only
//! available as [`Fx::checked_div`] and [`Fx::saturating_div`].
//!
//! # Rounding policy
//!
//! Every operation that drops fractional bits rounds **to nearest, ties toward
//! positive infinity** (add half an ulp, then floor). This is cheap (one add
//! and an arithmetic shift), unbiased except at exact ties, and identical on
//! every platform because it is pure integer arithmetic. Methods that round
//! differently say so in their names (`to_int_floor`, `to_int_ceil`).

use core::fmt;
use core::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// Signed 16.16 fixed-point number: an `i32` whose low 16 bits are the
/// fraction. Range is `[-32768, 32768 - 2^-16]`, resolution `2^-16`.
///
/// Serializes as its raw `i32` (exact, no decimal conversion).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Fx(i32);

/// Round a value with `shift` extra fractional bits to nearest, ties toward
/// positive infinity.
#[inline]
const fn round_shift_i64(v: i64, shift: u32) -> i64 {
    (v + (1i64 << (shift - 1))) >> shift
}

#[inline]
const fn round_shift_i128(v: i128, shift: u32) -> i128 {
    (v + (1i128 << (shift - 1))) >> shift
}

#[inline]
const fn saturate_i64(v: i64) -> i32 {
    if v > i32::MAX as i64 {
        i32::MAX
    } else if v < i32::MIN as i64 {
        i32::MIN
    } else {
        v as i32
    }
}

#[inline]
pub(crate) const fn saturate_i128(v: i128) -> i32 {
    if v > i32::MAX as i128 {
        i32::MAX
    } else if v < i32::MIN as i128 {
        i32::MIN
    } else {
        v as i32
    }
}

/// Integer square root of a `u128`, rounded to nearest (ties cannot occur for
/// integers). Bit-by-bit method: deterministic, no division.
pub(crate) const fn isqrt_round_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    // Floor square root, digit by digit in base 4.
    let mut rem = n;
    let mut root: u128 = 0;
    let mut bit: u128 = 1u128 << ((127 - n.leading_zeros()) & !1);
    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    // root = floor(sqrt(n)), rem = n - root^2. (root + 1/2)^2 = root^2 + root + 1/4,
    // so n is closer to (root+1)^2 exactly when rem > root.
    if rem > root { root + 1 } else { root }
}

impl Fx {
    /// Number of fractional bits.
    pub const FRAC_BITS: u32 = 16;
    /// Raw value of `1.0`.
    pub const ONE_RAW: i32 = 1 << 16;
    /// `0.0`
    pub const ZERO: Fx = Fx(0);
    /// `1.0`
    pub const ONE: Fx = Fx(1 << 16);
    /// `0.5`
    pub const HALF: Fx = Fx(1 << 15);
    /// `-1.0`
    pub const NEG_ONE: Fx = Fx(-(1 << 16));
    /// Smallest positive value, `2^-16`.
    pub const EPSILON: Fx = Fx(1);
    /// Largest value, `32768 - 2^-16`.
    pub const MAX: Fx = Fx(i32::MAX);
    /// Smallest value, `-32768`.
    pub const MIN: Fx = Fx(i32::MIN);

    /// Wraps a raw 16.16 bit pattern.
    #[inline]
    pub const fn from_raw(raw: i32) -> Fx {
        Fx(raw)
    }

    /// The raw 16.16 bit pattern. Use `raw().to_le_bytes()` when hashing
    /// state, so the hash does not depend on host endianness.
    #[inline]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Integer to fixed point. Values outside `[-32768, 32767]` wrap (their
    /// high bits are lost); use [`Fx::checked_from_int`] to detect that.
    #[inline]
    pub const fn from_int(n: i32) -> Fx {
        Fx(n.wrapping_shl(16))
    }

    /// Integer to fixed point, `None` if `n` is outside `[-32768, 32767]`.
    #[inline]
    pub const fn checked_from_int(n: i32) -> Option<Fx> {
        if n >= -32768 && n <= 32767 {
            Some(Fx(n << 16))
        } else {
            None
        }
    }

    /// `num / den` as fixed point, rounded to nearest (ties toward +inf).
    /// `None` if `den == 0` or the result is out of range.
    #[inline]
    pub const fn from_ratio(num: i32, den: i32) -> Option<Fx> {
        Fx::div_raw_i64((num as i64) << 16, den as i64)
    }

    /// Integer part, rounded toward negative infinity (`-1.5 -> -2`).
    #[inline]
    pub const fn to_int_floor(self) -> i32 {
        self.0 >> 16
    }

    /// Integer part, rounded toward positive infinity (`1.25 -> 2`).
    #[inline]
    pub const fn to_int_ceil(self) -> i32 {
        ((self.0 as i64 + 0xFFFF) >> 16) as i32
    }

    /// Nearest integer, ties toward positive infinity (`0.5 -> 1`,
    /// `-0.5 -> 0`, `-1.5 -> -1`).
    #[inline]
    pub const fn to_int_round(self) -> i32 {
        round_shift_i64(self.0 as i64, 16) as i32
    }

    /// Fractional part, always in `[0, 1)` (so `frac(-1.25) = 0.75`, matching
    /// [`Fx::to_int_floor`]).
    #[inline]
    pub const fn frac(self) -> Fx {
        Fx(self.0 & 0xFFFF)
    }

    /// Wrapping addition (same as `+`).
    #[inline]
    pub const fn wrapping_add(self, rhs: Fx) -> Fx {
        Fx(self.0.wrapping_add(rhs.0))
    }

    /// Wrapping subtraction (same as `-`).
    #[inline]
    pub const fn wrapping_sub(self, rhs: Fx) -> Fx {
        Fx(self.0.wrapping_sub(rhs.0))
    }

    /// Checked addition.
    #[inline]
    pub const fn checked_add(self, rhs: Fx) -> Option<Fx> {
        match self.0.checked_add(rhs.0) {
            Some(v) => Some(Fx(v)),
            None => None,
        }
    }

    /// Checked subtraction.
    #[inline]
    pub const fn checked_sub(self, rhs: Fx) -> Option<Fx> {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Some(Fx(v)),
            None => None,
        }
    }

    /// Saturating addition.
    #[inline]
    pub const fn saturating_add(self, rhs: Fx) -> Fx {
        Fx(self.0.saturating_add(rhs.0))
    }

    /// Saturating subtraction.
    #[inline]
    pub const fn saturating_sub(self, rhs: Fx) -> Fx {
        Fx(self.0.saturating_sub(rhs.0))
    }

    /// Exact 32.32 product as `i64`.
    #[inline]
    const fn mul_wide(self, rhs: Fx) -> i64 {
        self.0 as i64 * rhs.0 as i64
    }

    /// Multiplication, rounded to nearest (ties toward +inf), wrapping on
    /// overflow (same as `*`). Error is at most half an ulp (`2^-17`).
    #[inline]
    pub const fn wrapping_mul(self, rhs: Fx) -> Fx {
        Fx(round_shift_i64(self.mul_wide(rhs), 16) as i32)
    }

    /// Multiplication, rounded to nearest, `None` on overflow.
    #[inline]
    pub const fn checked_mul(self, rhs: Fx) -> Option<Fx> {
        let v = round_shift_i64(self.mul_wide(rhs), 16);
        if v > i32::MAX as i64 || v < i32::MIN as i64 {
            None
        } else {
            Some(Fx(v as i32))
        }
    }

    /// Multiplication, rounded to nearest, clamped to the representable range.
    #[inline]
    pub const fn saturating_mul(self, rhs: Fx) -> Fx {
        Fx(saturate_i64(round_shift_i64(self.mul_wide(rhs), 16)))
    }

    /// Multiply by an integer, wrapping on overflow. Exact (no rounding).
    #[inline]
    pub const fn wrapping_mul_int(self, n: i32) -> Fx {
        Fx(self.0.wrapping_mul(n))
    }

    /// Multiply by an integer, `None` on overflow. Exact (no rounding).
    #[inline]
    pub const fn checked_mul_int(self, n: i32) -> Option<Fx> {
        match self.0.checked_mul(n) {
            Some(v) => Some(Fx(v)),
            None => None,
        }
    }

    /// `n / d` interpreted as a raw 16.16 result (callers pre-scale `n`),
    /// rounded to nearest, ties toward +inf. `None` on zero divisor or
    /// out-of-range result.
    #[inline]
    const fn div_raw_i64(n: i64, d: i64) -> Option<Fx> {
        if d == 0 {
            return None;
        }
        // q = floor((n + d/2) / d) for d > 0, computed as floor((2n + d) / 2d)
        // so the half is exact. Widened to i128 so 2n cannot overflow.
        let (n, d) = if d < 0 {
            (-(n as i128), -(d as i128))
        } else {
            (n as i128, d as i128)
        };
        let q = (2 * n + d).div_euclid(2 * d);
        if q > i32::MAX as i128 || q < i32::MIN as i128 {
            None
        } else {
            Some(Fx(q as i32))
        }
    }

    /// Division, rounded to nearest (ties toward +inf). `None` if `rhs` is
    /// zero or the quotient is out of range. Error is at most half an ulp.
    #[inline]
    pub const fn checked_div(self, rhs: Fx) -> Option<Fx> {
        Fx::div_raw_i64((self.0 as i64) << 16, rhs.0 as i64)
    }

    /// Division, rounded to nearest, clamped to the representable range.
    /// Dividing by zero gives [`Fx::MAX`] for a positive dividend,
    /// [`Fx::MIN`] for a negative one, and [`Fx::ZERO`] for `0 / 0`.
    #[inline]
    pub const fn saturating_div(self, rhs: Fx) -> Fx {
        match self.checked_div(rhs) {
            Some(v) => v,
            None => {
                // Either zero divisor or overflow; the sign of the true result
                // picks the bound.
                if self.0 == 0 {
                    Fx::ZERO
                } else if (self.0 < 0) == (rhs.0 < 0) {
                    Fx::MAX
                } else {
                    Fx::MIN
                }
            }
        }
    }

    /// Divide by an integer, rounded to nearest (ties toward +inf). `None` if
    /// `n == 0` or the result overflows (only `MIN / -1`).
    #[inline]
    pub const fn checked_div_int(self, n: i32) -> Option<Fx> {
        Fx::div_raw_i64(self.0 as i64, n as i64)
    }

    /// Wrapping negation (same as unary `-`; `-MIN == MIN`).
    #[inline]
    pub const fn wrapping_neg(self) -> Fx {
        Fx(self.0.wrapping_neg())
    }

    /// Absolute value, saturating (`abs(MIN) == MAX`).
    #[inline]
    pub const fn abs(self) -> Fx {
        Fx(self.0.saturating_abs())
    }

    /// Absolute value as an unsigned raw value; exact for every input.
    #[inline]
    pub const fn unsigned_abs_raw(self) -> u32 {
        self.0.unsigned_abs()
    }

    /// `-1`, `0` or `1` as fixed point.
    #[inline]
    pub const fn signum(self) -> Fx {
        Fx::from_int(self.0.signum())
    }

    /// `true` if strictly negative.
    #[inline]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// The smaller of two values.
    #[inline]
    pub const fn min(self, other: Fx) -> Fx {
        if self.0 <= other.0 { self } else { other }
    }

    /// The larger of two values.
    #[inline]
    pub const fn max(self, other: Fx) -> Fx {
        if self.0 >= other.0 { self } else { other }
    }

    /// Clamp into `[lo, hi]`. If `lo > hi` the result is `lo` (no panic).
    #[inline]
    pub const fn clamp(self, lo: Fx, hi: Fx) -> Fx {
        if self.0 < lo.0 {
            lo
        } else if self.0 > hi.0 {
            hi
        } else {
            self
        }
    }

    /// Square root, rounded to nearest. `None` for negative input.
    ///
    /// Computed as the integer square root of `raw << 16` with a bit-by-bit
    /// algorithm, so it is exact to within half an ulp and identical on every
    /// platform.
    #[inline]
    pub const fn checked_sqrt(self) -> Option<Fx> {
        if self.0 < 0 {
            return None;
        }
        // raw << 16 < 2^47, so the root is below 2^24 and fits in i32.
        let r = isqrt_round_u128((self.0 as u128) << 16);
        Some(Fx(r as i32))
    }

    /// Square root, rounded to nearest. Negative input returns
    /// [`Fx::ZERO`] (the sim should never take the root of a negative, and
    /// panicking would be worse than a defined value).
    #[inline]
    pub const fn sqrt(self) -> Fx {
        match self.checked_sqrt() {
            Some(v) => v,
            None => Fx::ZERO,
        }
    }

    /// Linear interpolation `self + (other - self) * t`, computed in 64 bits
    /// and rounded to nearest once. Result is saturated to the range.
    #[inline]
    pub const fn lerp(self, other: Fx, t: Fx) -> Fx {
        let diff = other.0 as i64 - self.0 as i64;
        let step = round_shift_i128(diff as i128 * t.0 as i128, 16);
        Fx(saturate_i128(self.0 as i128 + step))
    }
}

impl Add for Fx {
    type Output = Fx;
    /// Wrapping addition.
    #[inline]
    fn add(self, rhs: Fx) -> Fx {
        self.wrapping_add(rhs)
    }
}

impl Sub for Fx {
    type Output = Fx;
    /// Wrapping subtraction.
    #[inline]
    fn sub(self, rhs: Fx) -> Fx {
        self.wrapping_sub(rhs)
    }
}

impl Mul for Fx {
    type Output = Fx;
    /// Rounded-to-nearest, wrapping multiplication.
    #[inline]
    fn mul(self, rhs: Fx) -> Fx {
        self.wrapping_mul(rhs)
    }
}

impl Neg for Fx {
    type Output = Fx;
    /// Wrapping negation.
    #[inline]
    fn neg(self) -> Fx {
        self.wrapping_neg()
    }
}

impl AddAssign for Fx {
    #[inline]
    fn add_assign(&mut self, rhs: Fx) {
        *self = *self + rhs;
    }
}

impl SubAssign for Fx {
    #[inline]
    fn sub_assign(&mut self, rhs: Fx) {
        *self = *self - rhs;
    }
}

impl MulAssign for Fx {
    #[inline]
    fn mul_assign(&mut self, rhs: Fx) {
        *self = *self * rhs;
    }
}

/// Decimal approximation with up to five fractional digits, truncated toward
/// zero (display only; never parsed back). Example: `Fx::HALF` shows `0.5`.
impl fmt::Display for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let neg = self.0 < 0;
        let abs = self.0.unsigned_abs();
        let int = abs >> 16;
        // Five decimal digits, truncated: 2^-16 ~ 0.0000153.
        let mut frac = ((abs & 0xFFFF) as u64 * 100_000) >> 16;
        if neg {
            f.write_str("-")?;
        }
        write!(f, "{int}")?;
        if frac != 0 {
            let mut digits = 5;
            while frac % 10 == 0 {
                frac /= 10;
                digits -= 1;
            }
            write!(f, ".{frac:0digits$}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fx({self} = {:#010x})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_round_trips_and_rounding() {
        assert_eq!(Fx::from_int(3).to_int_floor(), 3);
        assert_eq!(Fx::from_int(-3).to_int_floor(), -3);
        let m15 = Fx::from_raw(-(3 << 15)); // -1.5
        assert_eq!(m15.to_int_floor(), -2);
        assert_eq!(m15.to_int_ceil(), -1);
        assert_eq!(m15.to_int_round(), -1);
        assert_eq!(Fx::HALF.to_int_round(), 1);
        assert_eq!((-Fx::HALF).to_int_round(), 0);
        assert_eq!(Fx::from_raw(-(5 << 14)).frac(), Fx::from_raw(3 << 14)); // -1.25 -> 0.75
        assert_eq!(Fx::checked_from_int(32768), None);
        assert_eq!(Fx::checked_from_int(-32768), Some(Fx::MIN));
    }

    #[test]
    fn mul_div_basics() {
        let a = Fx::from_int(3);
        let b = Fx::HALF;
        assert_eq!(a * b, Fx::from_raw(3 << 15));
        assert_eq!(a.checked_div(b), Some(Fx::from_int(6)));
        assert_eq!(a.checked_div(Fx::ZERO), None);
        assert_eq!(Fx::MAX.checked_mul(Fx::from_int(2)), None);
        assert_eq!(Fx::MAX.saturating_mul(Fx::from_int(2)), Fx::MAX);
        assert_eq!(Fx::ONE.saturating_div(Fx::ZERO), Fx::MAX);
        assert_eq!(Fx::NEG_ONE.saturating_div(Fx::ZERO), Fx::MIN);
        assert_eq!(Fx::ZERO.saturating_div(Fx::ZERO), Fx::ZERO);
        assert_eq!(Fx::MIN.checked_div_int(-1), None);
        assert_eq!(Fx::from_ratio(1, 3), Some(Fx::from_raw(21845)));
        assert_eq!(Fx::from_ratio(2, 3), Some(Fx::from_raw(43691)));
        assert_eq!(Fx::from_ratio(-2, 3), Some(Fx::from_raw(-43691)));
        // Wrapping operators wrap in every profile.
        assert_eq!(Fx::MAX + Fx::EPSILON, Fx::MIN);
        assert_eq!(-Fx::MIN, Fx::MIN);
    }

    #[test]
    fn sqrt_values() {
        assert_eq!(Fx::from_int(4).sqrt(), Fx::from_int(2));
        assert_eq!(Fx::from_int(2).sqrt(), Fx::from_raw(92682)); // 1.41421 * 65536 = 92681.9
        assert_eq!(Fx::ZERO.sqrt(), Fx::ZERO);
        assert_eq!(Fx::NEG_ONE.checked_sqrt(), None);
        assert_eq!(Fx::MAX.sqrt().to_int_floor(), 181);
    }

    #[test]
    fn display() {
        assert_eq!(Fx::HALF.to_string(), "0.5");
        assert_eq!(Fx::from_int(-2).to_string(), "-2");
        assert_eq!(Fx::from_raw(-(5 << 14)).to_string(), "-1.25");
        assert_eq!(Fx::EPSILON.to_string(), "0.00001");
        assert_eq!(Fx::MIN.to_string(), "-32768");
        assert_eq!(format!("{:?}", Fx::ONE), "Fx(1 = 0x00010000)");
    }

    #[test]
    fn lerp_and_clamp() {
        let a = Fx::from_int(10);
        let b = Fx::from_int(20);
        assert_eq!(a.lerp(b, Fx::HALF), Fx::from_int(15));
        assert_eq!(Fx::from_int(30).clamp(a, b), b);
        assert_eq!(Fx::from_int(5).clamp(b, a), b);
    }
}
