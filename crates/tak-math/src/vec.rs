//! 2D and 3D vectors of [`Fx`].
//!
//! Component-wise `+`, `-` and unary `-` wrap, like [`Fx`]'s operators.
//! Products that combine components (`dot`, `length_squared`, `length`,
//! `normalize`) are accumulated exactly in 128-bit integers and rounded to
//! nearest (ties toward +inf) **once**, then saturated to the `Fx` range, so
//! they never overflow mid-computation and never lose more than half an ulp.

use core::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use crate::fx::{isqrt_round_u128, saturate_i128};
use crate::{Angle, Fx};

#[inline]
fn round16(v: i128) -> Fx {
    Fx::from_raw(saturate_i128((v + (1 << 15)) >> 16))
}

/// `round(c * 65536 / len)` for normalizing, `len > 0`.
#[inline]
fn div_round(c: i32, len: u128) -> Fx {
    let n = (c as i128) << 16;
    let d = len as i128;
    let q = (2 * n + d).div_euclid(2 * d);
    Fx::from_raw(saturate_i128(q))
}

macro_rules! impl_vec_ops {
    ($V:ident { $($f:ident),+ }) => {
        impl Add for $V {
            type Output = $V;
            /// Component-wise wrapping addition.
            #[inline]
            fn add(self, rhs: $V) -> $V { $V { $($f: self.$f + rhs.$f),+ } }
        }
        impl Sub for $V {
            type Output = $V;
            /// Component-wise wrapping subtraction.
            #[inline]
            fn sub(self, rhs: $V) -> $V { $V { $($f: self.$f - rhs.$f),+ } }
        }
        impl Neg for $V {
            type Output = $V;
            #[inline]
            fn neg(self) -> $V { $V { $($f: -self.$f),+ } }
        }
        impl AddAssign for $V {
            #[inline]
            fn add_assign(&mut self, rhs: $V) { *self = *self + rhs; }
        }
        impl SubAssign for $V {
            #[inline]
            fn sub_assign(&mut self, rhs: $V) { *self = *self - rhs; }
        }
        impl $V {
            /// The zero vector.
            pub const ZERO: $V = $V { $($f: Fx::ZERO),+ };

            /// Scale every component by `s` (rounded, wrapping, like `Fx * Fx`).
            #[inline]
            pub fn scale(self, s: Fx) -> $V { $V { $($f: self.$f * s),+ } }

            /// Dot product: exact sum of products, rounded once, saturated.
            #[inline]
            pub fn dot(self, rhs: $V) -> Fx {
                let sum: i128 = 0 $(+ self.$f.raw() as i128 * rhs.$f.raw() as i128)+;
                round16(sum)
            }

            /// Squared length (saturated; large vectors hit `Fx::MAX`).
            #[inline]
            pub fn length_squared(self) -> Fx { self.dot(self) }

            /// Euclidean length, rounded to nearest, saturated to `Fx::MAX`.
            /// Computed as the integer square root of the exact sum of squared
            /// raw components (which is already in 32.32, so its root is
            /// 16.16), so it is exact to half an ulp and never overflows.
            #[inline]
            pub fn length(self) -> Fx {
                Fx::from_raw(saturate_i128(isqrt_round_u128(self.raw_len_sq()) as i128))
            }

            #[inline]
            fn raw_len_sq(self) -> u128 {
                0 $(+ (self.$f.raw() as i128 * self.$f.raw() as i128) as u128)+
            }

            /// Distance to `other`, computed without wrapping (each component
            /// difference is taken in 64 bits).
            #[inline]
            pub fn distance(self, other: $V) -> Fx {
                let sq: u128 = 0 $(+ {
                    let d = self.$f.raw() as i128 - other.$f.raw() as i128;
                    (d * d) as u128
                })+;
                Fx::from_raw(saturate_i128(isqrt_round_u128(sq) as i128))
            }

            /// Unit vector in the same direction, each component rounded to
            /// nearest. `None` for the zero vector. The result's length is
            /// within a few ulps of `1`; it is not renormalized.
            #[inline]
            pub fn normalize(self) -> Option<$V> {
                let len = isqrt_round_u128(self.raw_len_sq());
                if len == 0 {
                    return None;
                }
                Some($V { $($f: div_round(self.$f.raw(), len)),+ })
            }
        }
    };
}

/// 2D vector of [`Fx`]. Field order is the serialization and `Ord` order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Vec2 {
    /// X component.
    pub x: Fx,
    /// Y component.
    pub y: Fx,
}

/// 3D vector of [`Fx`]. Field order is the serialization and `Ord` order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Vec3 {
    /// X component.
    pub x: Fx,
    /// Y component.
    pub y: Fx,
    /// Z component.
    pub z: Fx,
}

impl_vec_ops!(Vec2 { x, y });
impl_vec_ops!(Vec3 { x, y, z });

impl Vec2 {
    /// Construct from components.
    #[inline]
    pub const fn new(x: Fx, y: Fx) -> Vec2 {
        Vec2 { x, y }
    }

    /// Construct from whole numbers (wrapping like [`Fx::from_int`]).
    #[inline]
    pub const fn from_ints(x: i32, y: i32) -> Vec2 {
        Vec2 {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
        }
    }

    /// Unit vector `(cos a, sin a)`.
    #[inline]
    pub const fn from_angle(a: Angle) -> Vec2 {
        Vec2 {
            x: a.cos(),
            y: a.sin(),
        }
    }

    /// Direction of this vector, via [`Angle::atan2`]. Zero vector gives
    /// [`Angle::ZERO`].
    #[inline]
    pub const fn angle(self) -> Angle {
        Angle::atan2(self.y, self.x)
    }

    /// Lift into 3D with the given `z`.
    #[inline]
    pub const fn extend(self, z: Fx) -> Vec3 {
        Vec3 {
            x: self.x,
            y: self.y,
            z,
        }
    }
}

impl Vec3 {
    /// Construct from components.
    #[inline]
    pub const fn new(x: Fx, y: Fx, z: Fx) -> Vec3 {
        Vec3 { x, y, z }
    }

    /// Construct from whole numbers (wrapping like [`Fx::from_int`]).
    #[inline]
    pub const fn from_ints(x: i32, y: i32, z: i32) -> Vec3 {
        Vec3 {
            x: Fx::from_int(x),
            y: Fx::from_int(y),
            z: Fx::from_int(z),
        }
    }

    /// Drop `z`.
    #[inline]
    pub const fn xy(self) -> Vec2 {
        Vec2 {
            x: self.x,
            y: self.y,
        }
    }

    /// Cross product; each component is an exact difference of products,
    /// rounded once and saturated.
    #[inline]
    pub fn cross(self, rhs: Vec3) -> Vec3 {
        let m = |a: Fx, b: Fx| a.raw() as i128 * b.raw() as i128;
        Vec3 {
            x: round16(m(self.y, rhs.z) - m(self.z, rhs.y)),
            y: round16(m(self.z, rhs.x) - m(self.x, rhs.z)),
            z: round16(m(self.x, rhs.y) - m(self.y, rhs.x)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        let v = Vec2::from_ints(3, 4);
        assert_eq!(v.length(), Fx::from_int(5));
        assert_eq!(v.length_squared(), Fx::from_int(25));
        assert_eq!(v.dot(Vec2::from_ints(1, 1)), Fx::from_int(7));
        let n = v.normalize().unwrap();
        assert_eq!(
            n,
            Vec2::new(Fx::from_ratio(3, 5).unwrap(), Fx::from_ratio(4, 5).unwrap())
        );
        assert_eq!(Vec2::ZERO.normalize(), None);
        assert_eq!(Vec3::from_ints(2, 3, 6).length(), Fx::from_int(7));
        // Huge vector: length saturates instead of overflowing.
        let big = Vec2::new(Fx::MAX, Fx::MAX);
        assert_eq!(big.length(), Fx::MAX);
        assert_eq!(
            Vec2::new(Fx::MIN, Fx::ZERO).normalize(),
            Some(Vec2::new(Fx::NEG_ONE, Fx::ZERO))
        );
        assert_eq!(
            Vec2::new(Fx::MIN, Fx::ZERO).distance(Vec2::new(Fx::MAX, Fx::ZERO)),
            Fx::MAX
        );
        let x = Vec3::from_ints(1, 0, 0);
        let y = Vec3::from_ints(0, 1, 0);
        assert_eq!(x.cross(y), Vec3::from_ints(0, 0, 1));
        assert_eq!(Vec2::from_angle(Angle::QUARTER), Vec2::from_ints(0, 1));
    }
}
