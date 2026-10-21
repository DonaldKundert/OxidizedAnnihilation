//! Property tests. All checks are integer-exact (no float reference needed):
//! errors are measured against exact `i128` arithmetic.

use proptest::prelude::*;
use tak_math::{Angle, Fx, Rng, Vec2, Vec3};

fn fx() -> impl Strategy<Value = Fx> {
    any::<i32>().prop_map(Fx::from_raw)
}

/// Values with magnitude in [1, 32767].
fn fx_big() -> impl Strategy<Value = Fx> {
    (Fx::ONE.raw()..=i32::MAX, any::<bool>())
        .prop_map(|(r, neg)| Fx::from_raw(if neg { -r } else { r }))
}

proptest! {
    /// Multiplication error is at most half an ulp: |q * 2^16 - a*b| <= 2^15.
    #[test]
    fn mul_error_half_ulp(a in fx(), b in fx()) {
        let exact = a.raw() as i128 * b.raw() as i128;
        match a.checked_mul(b) {
            Some(q) => prop_assert!((q.raw() as i128 * 65536 - exact).abs() <= 32768),
            None => {
                let q = (exact + 32768) >> 16;
                prop_assert!(q > i32::MAX as i128 || q < i32::MIN as i128);
            }
        }
    }

    /// Division error is at most half an ulp: |q*b - a*2^16| <= |b|/2.
    #[test]
    fn div_error_half_ulp(a in fx(), b in fx()) {
        match a.checked_div(b) {
            Some(q) => {
                let lhs = q.raw() as i128 * b.raw() as i128 - ((a.raw() as i128) << 16);
                prop_assert!(2 * lhs.abs() <= (b.raw() as i128).abs());
            }
            None => {
                if b != Fx::ZERO {
                    // Then the true quotient must be out of range.
                    let q = ((a.raw() as i128) << 16) / b.raw() as i128;
                    prop_assert!(q.abs() >= i32::MAX as i128);
                }
            }
        }
    }

    /// (a * b) / b == a within one ulp when |b| >= 1 and nothing overflows.
    #[test]
    fn mul_div_round_trip(a in fx(), b in fx_big()) {
        if let Some(p) = a.checked_mul(b) {
            let back = p.checked_div(b).expect("|b| >= 1 cannot overflow");
            prop_assert!((back.raw() as i64 - a.raw() as i64).abs() <= 1, "{a:?} {b:?} {back:?}");
        }
    }

    /// (a / b) * b == a within |b|/2 + 1 ulps.
    #[test]
    fn div_mul_round_trip(a in fx(), b in fx()) {
        if let Some(q) = a.checked_div(b) {
            if let Some(back) = q.checked_mul(b) {
                let tol = (b.raw() as i64).abs() / (2 * 65536) + 1;
                prop_assert!((back.raw() as i64 - a.raw() as i64).abs() <= tol);
            }
        }
    }

    /// Saturating ops agree with checked ops where those succeed, and clamp
    /// to the bounds otherwise.
    #[test]
    fn saturating_matches_checked(a in fx(), b in fx()) {
        let s = a.saturating_mul(b);
        match a.checked_mul(b) {
            Some(c) => prop_assert_eq!(s, c),
            None => prop_assert!(s == Fx::MAX || s == Fx::MIN),
        }
        let s = a.saturating_add(b);
        match a.checked_add(b) {
            Some(c) => { prop_assert_eq!(s, c); prop_assert_eq!(a + b, c); }
            None => prop_assert!(s == Fx::MAX || s == Fx::MIN),
        }
    }

    /// sqrt is the nearest representable root: |r^2 - x| <= r (in raw^2 units).
    #[test]
    fn sqrt_nearest(x in 0..=i32::MAX) {
        let r = Fx::from_raw(x).sqrt().raw() as i128;
        let n = (x as i128) << 16;
        prop_assert!((r * r - n).abs() <= r);
    }

    /// Integer conversions round-trip and rounding modes bracket the value.
    #[test]
    fn int_conversions(a in fx(), n in -32768i32..=32767) {
        prop_assert_eq!(Fx::from_int(n).to_int_floor(), n);
        prop_assert_eq!(Fx::from_int(n).to_int_round(), n);
        let f = a.to_int_floor() as i64;
        let c = a.to_int_ceil() as i64;
        let r = a.to_int_round() as i64;
        prop_assert!(f << 16 <= a.raw() as i64 && (a.raw() as i64) < (f + 1) << 16);
        prop_assert!(c - f <= 1 && (c << 16) >= a.raw() as i64);
        prop_assert!(r == f || r == f + 1);
    }

    /// sin^2 + cos^2 == 1 within 2 raw units (2^-16 each).
    #[test]
    fn pythagorean(a in any::<u16>()) {
        let (s, c) = Angle(a).sin_cos();
        prop_assert!(((s * s + c * c).raw() - Fx::ONE.raw()).abs() <= 2);
    }

    /// atan2(sin a, cos a) == a exactly; within one unit for scaled vectors.
    #[test]
    fn atan2_round_trip(a in any::<u16>(), k in 1i32..=20_000) {
        let a = Angle(a);
        let (s, c) = a.sin_cos();
        prop_assert_eq!(Angle::atan2(s, c), a);
        // Scaling by an integer keeps the direction (exact, unless it wraps).
        if let (Some(ks), Some(kc)) = (s.checked_mul_int(k), c.checked_mul_int(k)) {
            prop_assert!(a.delta_to(Angle::atan2(ks, kc)).abs() <= 1);
        }
    }

    /// Angle arithmetic wraps and sin/cos respect the angle-sum symmetries.
    #[test]
    fn angle_symmetry(a in any::<u16>(), b in any::<u16>()) {
        let (a, b) = (Angle(a), Angle(b));
        prop_assert_eq!(a + b - b, a);
        prop_assert_eq!((-a).sin(), -a.sin());
        prop_assert_eq!((a + Angle::QUARTER).sin(), a.cos());
    }

    /// Normalized vectors have length 1 within 2 raw units when the input is
    /// not tiny (tiny inputs have too few bits of direction).
    #[test]
    fn normalize_unit(x in fx(), y in fx(), z in fx()) {
        let v = Vec3::new(x, y, z);
        if v.length() >= Fx::from_int(1) || v.length() == Fx::MAX {
            let n = v.normalize().unwrap();
            prop_assert!((n.length().raw() - Fx::ONE.raw()).abs() <= 2, "{v:?} -> {n:?}");
        }
        let w = Vec2::new(x, y);
        if w.length() >= Fx::from_int(1) {
            let n = w.normalize().unwrap();
            prop_assert!((n.length().raw() - Fx::ONE.raw()).abs() <= 2);
            prop_assert!(w.angle().delta_to(n.angle()).abs() <= 1);
        }
    }

    /// Same seed and stream, same sequence; draws stay in range.
    #[test]
    fn rng_reproducible(seed in any::<u64>(), stream in any::<u64>(), lo in any::<u32>(), span in 1u32..=u32::MAX) {
        let mut a = Rng::with_stream(seed, stream);
        let mut b = Rng::with_stream(seed, stream);
        let hi = lo.saturating_add(span);
        for _ in 0..64 {
            prop_assert_eq!(a.next_u32(), b.next_u32());
            let x = a.range(lo, hi);
            prop_assert_eq!(x, b.range(lo, hi));
            if hi > lo {
                prop_assert!(lo <= x && x < hi);
            }
        }
        prop_assert_eq!(a, b);
    }

    /// Serialized RNG state resumes the same sequence.
    #[test]
    fn rng_serde_resume(seed in any::<u64>(), skip in 0usize..100) {
        let mut a = Rng::new(seed);
        for _ in 0..skip { a.next_u32(); }
        let json = serde_json::to_string(&a).unwrap();
        let mut b: Rng = serde_json::from_str(&json).unwrap();
        for _ in 0..16 { prop_assert_eq!(a.next_u32(), b.next_u32()); }
    }

    /// Fx, Angle and vectors serialize to their raw integers and round-trip.
    #[test]
    fn serde_round_trip(x in fx(), y in fx(), a in any::<u16>()) {
        let v = Vec2::new(x, y);
        let back: Vec2 = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
        prop_assert_eq!(back, v);
        prop_assert_eq!(serde_json::to_string(&x).unwrap(), x.raw().to_string());
        let back: Angle = serde_json::from_str(&serde_json::to_string(&Angle(a)).unwrap()).unwrap();
        prop_assert_eq!(back, Angle(a));
    }
}

/// `below` is unbiased: with a bound of 3 over many draws, each bucket is
/// near a third (a loose sanity check; the method is exact by construction).
#[test]
fn rng_below_spread() {
    let mut r = Rng::new(1);
    let mut counts = [0u32; 3];
    for _ in 0..30_000 {
        counts[r.below(3) as usize] += 1;
    }
    for c in counts {
        assert!((9_500..=10_500).contains(&c), "{counts:?}");
    }
}
