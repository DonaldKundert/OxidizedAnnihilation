//! Exhaustive checks of the integer trig against a floating-point reference.
//! Floats appear only here, in a test, never in the crate's runtime path.

use std::f64::consts::PI;

use tak_math::{Angle, Fx, SIN_TABLE};

fn ref_sin_raw(a: u16) -> f64 {
    (a as f64 / 65536.0 * 2.0 * PI).sin() * 65536.0
}

#[test]
fn table_matches_float_reference() {
    for (k, &v) in SIN_TABLE.iter().enumerate() {
        let exact = (k as f64 / 1024.0 * PI / 2.0).sin() * 65536.0;
        let rounded = exact.round() as i32;
        let near_tie = (exact.fract().abs() - 0.5).abs() < 1e-6;
        assert!(
            v == rounded || (near_tie && (v - rounded).abs() <= 1),
            "k={k} table={v} ref={exact}"
        );
    }
}

/// Every angle's sin and cos is within 1 raw unit (2^-16) of the true value.
#[test]
fn sin_cos_accuracy_all_angles() {
    let mut max_err = 0.0f64;
    for a in 0..=u16::MAX {
        let s = Angle(a).sin().raw() as f64;
        let c = Angle(a).cos().raw() as f64;
        let rs = ref_sin_raw(a);
        let rc = ref_sin_raw(a.wrapping_add(0x4000));
        max_err = max_err.max((s - rs).abs()).max((c - rc).abs());
    }
    assert!(max_err <= 1.0, "max sin/cos error {max_err} raw units");
}

/// sin^2 + cos^2 is within 2 raw units of 1.0 for every angle (with `Fx`
/// rounded multiplication).
#[test]
fn pythagorean_identity_all_angles() {
    let mut worst = 0;
    for a in 0..=u16::MAX {
        let (s, c) = Angle(a).sin_cos();
        let sum = s * s + c * c;
        worst = worst.max((sum.raw() - Fx::ONE.raw()).abs());
    }
    assert!(worst <= 2, "worst |sin^2+cos^2-1| = {worst} raw units");
}

/// atan2(sin a, cos a) == a exactly, for every angle.
#[test]
fn atan2_inverts_sin_cos_all_angles() {
    let mut worst = 0i32;
    for a in 0..=u16::MAX {
        let a = Angle(a);
        let back = Angle::atan2(a.sin(), a.cos());
        worst = worst.max((a.delta_to(back) as i32).abs());
    }
    assert!(worst == 0, "worst atan2 round trip error {worst} units");
}

/// atan2 on integer grid vectors agrees with the float reference to within
/// one unit.
#[test]
fn atan2_matches_float_on_grid() {
    for y in (-200..=200).step_by(7) {
        for x in (-200..=200).step_by(5) {
            if x == 0 && y == 0 {
                continue;
            }
            let got = Angle::atan2(Fx::from_int(y), Fx::from_int(x));
            let want = ((y as f64).atan2(x as f64) / (2.0 * PI) * 65536.0).round() as i64;
            let want = Angle(want.rem_euclid(65536) as u16);
            let d = (want.delta_to(got) as i32).abs();
            assert!(d <= 1, "atan2({y}, {x}) = {got:?}, want {want:?}");
        }
    }
}
