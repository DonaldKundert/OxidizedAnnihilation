//! WP-S controller feel: tuning, presets, cursor shaping, aim assist,
//! camera motion, second-tap gestures, local/global select, context wheels,
//! recording, and the feel lab's verdict on the defaults.

mod common;

use common::{TestWorld, WorldSpec, pos, run, uid};
use proptest::prelude::*;
use tak_commands::{ClientCommand, Scope, SimCommand, Target, UnitSet};
use tak_input::lab::{LabConfig, default_presets, run_lab};
use tak_input::*;
use tak_math::{Fx, Vec2};

fn ev(t: u32, kind: EventKind) -> InputEvent {
    InputEvent::new(t, kind)
}
fn bdown(t: u32, b: Button) -> InputEvent {
    ev(t, EventKind::ButtonDown { button: b })
}
fn bup(t: u32, b: Button) -> InputEvent {
    ev(t, EventKind::ButtonUp { button: b })
}
fn axis(t: u32, a: Axis, v: i16) -> InputEvent {
    ev(t, EventKind::Axis { axis: a, value: v })
}
fn frame(t: u32) -> InputEvent {
    ev(t, EventKind::Frame)
}

fn world() -> TestWorld {
    let spec: WorldSpec = serde_json::from_str(
        r#"{"local_player": 0, "enemies": [1], "units": [
            {"index": 1, "owner": 0, "def": 10, "x": 640, "y": 480},
            {"index": 2, "owner": 0, "def": 10, "x": 700, "y": 480},
            {"index": 3, "owner": 0, "def": 11, "x": 900, "y": 480},
            {"index": 4, "owner": 1, "def": 50, "x": 640, "y": 300},
            {"index": 5, "owner": 0, "def": 10, "x": 3000, "y": 3000, "spells": [7]},
            {"index": 6, "owner": 1, "def": 50, "x": 672, "y": 300}
        ]}"#,
    )
    .unwrap();
    TestWorld::new(spec)
}

fn console() -> Profile {
    Profile::wp_s_controller()
}

// ---- tuning and presets -----------------------------------------------------

#[test]
fn presets_are_distinct_named_and_default_is_console() {
    assert_eq!(Tuning::default(), Tuning::console());
    let all: Vec<Tuning> = PRESETS.iter().map(|n| Tuning::preset(n).unwrap()).collect();
    for (i, a) in all.iter().enumerate() {
        assert_eq!(a.preset_name(), Some(PRESETS[i]));
        for b in &all[i + 1..] {
            assert_ne!(a, b);
        }
    }
    assert_eq!(Tuning::preset("WP-J"), Some(Tuning::wp_j()));
    assert_eq!(Tuning::preset("nope"), None);
}

#[test]
fn every_field_has_metadata_and_get_set_nudge_work() {
    let t = Tuning::console();
    let v = serde_json::to_value(t).unwrap();
    let names: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(names.len(), TUNING_FIELDS.len());
    for n in &names {
        let m = tuning_field(n).unwrap_or_else(|| panic!("no metadata for {n}"));
        let cur = t.get(n).unwrap();
        assert!(
            cur >= m.min && cur <= m.max,
            "{n} = {cur} outside {}..{}",
            m.min,
            m.max
        );
    }
    let mut t = Tuning::console();
    assert!(t.set("cursor_speed", 1234));
    assert_eq!(t.cursor_speed, 1234);
    assert!(t.set("cursor_speed", 1_000_000)); // clamped
    assert_eq!(t.cursor_speed, 4000);
    assert!(t.nudge("cursor_speed", -2));
    assert_eq!(t.cursor_speed, 3900);
    let was = t.rumble;
    assert!(t.nudge("rumble", 1));
    assert_eq!(t.rumble, !was);
    assert!(!t.set("no_such_field", 1));
    // A profile may name any subset; unknown names are rejected.
    let p = Profile::from_json(r#"{"name": "x", "bindings": [], "tuning": {"friction_pct": 40}}"#)
        .unwrap();
    assert_eq!(p.tuning.friction_pct, 40);
    assert_eq!(p.tuning.cursor_speed, Tuning::console().cursor_speed);
    assert!(
        Profile::from_json(r#"{"name": "x", "bindings": [], "tuning": {"bogus": 1}}"#).is_err()
    );
}

#[test]
fn default_profiles_round_trip() {
    for p in [
        Profile::default_controller(),
        Profile::default_combined(),
        Profile::classic_tak(),
        Profile::wp_s_combined(),
        Profile::wp_j_controller(),
    ] {
        p.validate().unwrap();
        let back = Profile::from_json(&p.to_json_pretty()).unwrap();
        assert_eq!(back, p);
    }
}

// ---- stick and cursor ------------------------------------------------------

#[test]
fn outer_deadzone_reaches_full_early() {
    let full = stick_vector_zones(32767 - 1500, 0, 7000, 2000);
    assert_eq!(full, Vec2::new(Fx::ONE, Fx::ZERO));
    let inner = stick_vector_zones(6000, 0, 7000, 2000);
    assert_eq!(inner, Vec2::ZERO);
    // Without an outer deadzone this is the WP-J function.
    assert_eq!(
        stick_vector_zones(20000, 0, 8000, 0),
        stick_vector(20000, 0, 8000)
    );
}

proptest! {
    #[test]
    fn curve_is_monotonic_and_exact_at_ends(e in 50i32..=400, a in 0i32..=65536, b in 0i32..=65536) {
        prop_assert_eq!(curve(Fx::ZERO, e), Fx::ZERO);
        prop_assert!((curve(Fx::ONE, e) - Fx::ONE).abs() <= Fx::from_raw(2));
        let (lo, hi) = (a.min(b), a.max(b));
        prop_assert!(curve(Fx::from_raw(lo), e) <= curve(Fx::from_raw(hi), e) + Fx::from_raw(1));
    }

    #[test]
    fn approach_is_frame_rate_independent(dt in 1u32..60, hl in 10u32..500) {
        // Two steps of dt close the same gap as one of 2 dt.
        let one = approach_frac(dt, hl);
        let two = approach_frac(2 * dt, hl);
        let rem = Fx::ONE - one;
        let composed = Fx::ONE - rem * rem;
        prop_assert!((composed - two).abs() <= Fx::from_raw(700), "{composed:?} vs {two:?}");
    }
}

#[test]
fn approach_half_life_is_a_half() {
    assert_eq!(approach_frac(100, 100), Fx::HALF);
    assert_eq!(approach_frac(0, 100), Fx::ZERO);
    assert_eq!(approach_frac(5, 0), Fx::ONE);
    assert_eq!(approach_frac(100_000, 1), Fx::ONE);
}

#[test]
fn cursor_speed_follows_zoom_curve_and_precision() {
    let t = Tuning {
        cursor_curve_pct: 100,
        cursor_accel_pct: 0,
        ..Tuning::console()
    };
    let right = Vec2::new(Fx::ONE, Fx::ZERO);
    let at = |zoom: Fx, precision: bool| {
        cursor_motion(
            right,
            100,
            &Motion {
                precision,
                world_per_px: zoom,
                ..Motion::neutral()
            },
            &t,
        )
        .x
    };
    let reference = at(Fx::HALF, false);
    assert_eq!(reference, Fx::from_int(110));
    // Zoomed out twice as far: half the pixels, the same ground.
    assert_eq!(at(Fx::ONE, false), Fx::from_int(55));
    // Precision is a multiplier on the same path.
    assert_eq!(at(Fx::HALF, true), Fx::from_ratio(385, 10).unwrap());
    // A squared curve: half deflection moves a quarter as fast.
    let sq = Tuning {
        cursor_curve_pct: 200,
        ..t
    };
    let half = cursor_motion(Vec2::new(Fx::HALF, Fx::ZERO), 100, &Motion::neutral(), &sq);
    assert_eq!(half.x, Fx::from_ratio(110, 4).unwrap());
}

#[test]
fn cursor_motion_is_frame_rate_independent() {
    // The same stick held for 600 ms, delivered at 60 and at 20 frames per
    // second, ends at the same place (within a pixel), friction included.
    let w = world();
    let end = |step: u32| {
        let mut m = Mapper::new(console());
        let mut evs = vec![axis(0, Axis::LeftX, 20000)];
        let mut t = 0;
        while t < 600 {
            t += step;
            evs.push(frame(t));
        }
        run(&mut m, &w, &evs);
        m.cursor_screen()
    };
    let a = end(20);
    let b = end(50);
    assert!(a.distance(b) <= Fx::ONE, "{a:?} vs {b:?}");
}

#[test]
fn acceleration_ramps_at_full_deflection() {
    let w = world();
    let mut t = console().tuning;
    t.cursor_accel_pct = 100;
    t.cursor_accel_ms = 200;
    t.friction_pct = 100;
    let mut p = console();
    p.tuning = t;
    let speed_after = |hold: u32| {
        let mut m = Mapper::new(p.clone());
        m.warp_cursor(pos(10, 10));
        run(&mut m, &w, &[axis(0, Axis::LeftX, 32767), frame(hold)]);
        let a = m.cursor_screen().x;
        run(&mut m, &w, &[frame(hold + 16)]);
        m.cursor_screen().x - a
    };
    let early = speed_after(16);
    let late = speed_after(400);
    assert!(
        late > early + early.checked_div_int(2).unwrap(),
        "{early:?} {late:?}"
    );
}

// ---- aim assist ------------------------------------------------------------

#[test]
fn friction_slows_over_targets() {
    let t = Tuning::console();
    let targets = [pos(100, 100)];
    assert_eq!(
        friction(pos(100, 100), &targets, &t),
        Fx::from_ratio(t.friction_pct, 100).unwrap()
    );
    assert_eq!(friction(pos(300, 100), &targets, &t), Fx::ONE);
    let mid = friction(pos(113, 100), &targets, &t);
    assert!(mid > Fx::from_ratio(t.friction_pct, 100).unwrap() && mid < Fx::ONE);
    let off = Tuning {
        friction_pct: 100,
        ..t
    };
    assert_eq!(friction(pos(100, 100), &targets, &off), Fx::ONE);
}

#[test]
fn soft_snap_hysteresis_keeps_the_current_target() {
    let t = Tuning {
        snap_hysteresis_px: 10,
        magnet_snap_px: 0,
        ..Tuning::console()
    };
    let targets = [pos(100, 100), pos(130, 100)];
    // Slightly nearer the second, but the first is current: stays.
    let (_, hit) = soft_snap(pos(116, 100), &targets, Some(0), 16, &t);
    assert_eq!(hit, Some(0));
    // Without a current target the nearest wins.
    let (_, hit) = soft_snap(pos(116, 100), &targets, None, 16, &t);
    assert_eq!(hit, Some(1));
    // Much nearer the second: switches.
    let (_, hit) = soft_snap(pos(127, 100), &targets, Some(0), 16, &t);
    assert_eq!(hit, Some(1));
}

#[test]
fn assist_targets_follow_context() {
    let w = world();
    // Nothing selected: the cursor between an own unit (1) and an enemy
    // (4), nearer the enemy, snaps to the own unit (enemies are not
    // targets).
    let mut m = Mapper::new(console());
    m.warp_cursor(pos(640, 420));
    run(&mut m, &w, &[frame(0), frame(16)]);
    assert_eq!(
        m.snap_target(),
        None,
        "own unit is 60 px away, outside the radius"
    );
    m.warp_cursor(pos(640, 340));
    run(
        &mut m,
        &w,
        &(1..40).map(|i| frame(16 * i)).collect::<Vec<_>>(),
    );
    assert_eq!(
        m.snap_target(),
        None,
        "enemy is not a target with nothing selected"
    );
    m.warp_cursor(pos(640, 450));
    run(
        &mut m,
        &w,
        &(40..80).map(|i| frame(16 * i)).collect::<Vec<_>>(),
    );
    assert_eq!(m.snap_target(), Some(uid(1)));
    assert_eq!(m.cursor_screen(), pos(640, 480));
    // Select it; now enemies are the targets.
    run(&mut m, &w, &[bdown(2000, Button::A), bup(2050, Button::A)]);
    assert_eq!(m.selection(), &UnitSet::single(uid(1)));
    m.warp_cursor(pos(645, 330));
    run(
        &mut m,
        &w,
        &(140..200).map(|i| frame(16 * i)).collect::<Vec<_>>(),
    );
    assert_eq!(m.snap_target(), Some(uid(4)));
    assert_eq!(m.cursor_screen(), pos(640, 300));
    assert!(m.take_feedback().contains(&Feedback::Snap));
    // Precision turns assist off.
    m.warp_cursor(pos(645, 330));
    run(
        &mut m,
        &w,
        &[
            bdown(4000, Button::Rt),
            frame(4016),
            frame(4100),
            frame(4200),
        ],
    );
    assert_eq!(m.cursor_screen(), pos(645, 330));
}

// ---- gestures and selection -----------------------------------------------

#[test]
fn second_tap_selects_same_type_without_delaying_the_first() {
    let w = world();
    let mut m = Mapper::new(console());
    m.warp_cursor(pos(640, 480));
    let out = run(&mut m, &w, &[bdown(0, Button::A), bup(50, Button::A)]);
    // The first tap fires at once, on release.
    assert_eq!(
        out,
        vec![Output::Client {
            command: ClientCommand::SelectAt {
                pos: pos(640, 480),
                additive: false
            }
        }]
    );
    let out = run(&mut m, &w, &[bdown(150, Button::A), bup(200, Button::A)]);
    assert_eq!(
        out,
        vec![Output::Client {
            command: ClientCommand::SelectSameType {
                scope: Scope::OnScreen
            }
        }]
    );
    assert_eq!(
        m.selection(),
        &[uid(1), uid(2)].into_iter().collect::<UnitSet>()
    );
    // Too late for a second tap: a plain select again.
    let out = run(&mut m, &w, &[bdown(900, Button::A), bup(950, Button::A)]);
    assert!(matches!(
        out.as_slice(),
        [Output::Client {
            command: ClientCommand::SelectAt { .. }
        }]
    ));
}

#[test]
fn local_and_global_select() {
    let w = world();
    let mut m = Mapper::new(console());
    let out = run(&mut m, &w, &[bdown(0, Button::Y), bup(50, Button::Y)]);
    let local: UnitSet = [uid(1), uid(2), uid(3)].into_iter().collect();
    assert_eq!(
        out,
        vec![Output::SelectAll {
            scope: Scope::OnScreen,
            units: local.clone()
        }]
    );
    let out = run(&mut m, &w, &[bdown(150, Button::Y), bup(200, Button::Y)]);
    let global: UnitSet = [uid(1), uid(2), uid(3), uid(5)].into_iter().collect();
    assert_eq!(
        out,
        vec![Output::SelectAll {
            scope: Scope::Map,
            units: global.clone()
        }]
    );
    assert_eq!(m.selection(), &global);
    assert!(m.selection_flash().is_some());
}

// ---- wheels ----------------------------------------------------------------

#[test]
fn command_wheel_is_built_from_the_selection() {
    let w = world();
    let mut m = Mapper::new(console());
    // Nothing selected: the idle wheel.
    run(&mut m, &w, &[bdown(0, Button::Lt)]);
    assert_eq!(m.wheel().unwrap().slots, m.profile().idle_wheel);
    run(&mut m, &w, &[bup(20, Button::Lt)]);
    // Unit 1 has no spells: no cast slot.
    m.warp_cursor(pos(640, 480));
    run(&mut m, &w, &[bdown(100, Button::A), bup(150, Button::A)]);
    run(&mut m, &w, &[bdown(600, Button::Lt)]);
    let slots = &m.wheel().unwrap().slots;
    assert!(!slots.contains(&Action::CastSpellMode));
    assert!(slots.contains(&Action::Stop));
}

fn key(t: u32, k: &str, down: bool) -> InputEvent {
    let key = Key::new(k);
    ev(
        t,
        if down {
            EventKind::KeyDown { key }
        } else {
            EventKind::KeyUp { key }
        },
    )
}

/// Modifier `m` + key `k`, then release both, from `t`.
fn chord(t: u32, m: &str, k: &str) -> Vec<InputEvent> {
    vec![
        key(t, m, true),
        key(t + 10, k, true),
        key(t + 20, k, false),
        key(t + 30, m, false),
    ]
}

#[test]
fn every_spell_has_a_wheel_slot_and_key_and_mana_gates_cast_mode() {
    let spec: WorldSpec = serde_json::from_str(
        r#"{"local_player": 0, "enemies": [1], "units": [
            {"index": 1, "owner": 0, "def": 10, "x": 640, "y": 480,
             "spells": [1, 2, 3], "no_mana": [3]}
        ]}"#,
    )
    .unwrap();
    let w = TestWorld::new(spec);
    let mut m = Mapper::new(Profile::wp_s_combined());
    m.warp_cursor(pos(640, 480));
    run(&mut m, &w, &[bdown(0, Button::A), bup(50, Button::A)]);
    assert_eq!(m.selection().len(), 1);
    run(&mut m, &w, &[bdown(600, Button::Lt)]);
    let slots = m.wheel().unwrap().slots.clone();
    let casts: Vec<Action> = slots
        .iter()
        .copied()
        .filter(|a| matches!(a, Action::CastSpell { .. }))
        .collect();
    assert_eq!(
        casts,
        (0..3)
            .map(|index| Action::CastSpell { index })
            .collect::<Vec<_>>()
    );
    assert!(!slots.contains(&Action::CastSpellMode));
    run(&mut m, &w, &[bdown(700, Button::B), bup(720, Button::B)]);
    assert!(m.wheel().is_none());
    // Ctrl+F2: the second spell. Ctrl+F3: refused, no mana.
    run(&mut m, &w, &chord(1000, "ctrl", "f2"));
    assert_eq!(m.mode(), Mode::CastSpell(tak_commands::SpellId(2)));
    run(
        &mut m,
        &w,
        &[key(1100, "escape", true), key(1110, "escape", false)],
    );
    assert_eq!(m.mode(), Mode::None);
    run(&mut m, &w, &chord(1200, "ctrl", "f3"));
    assert_eq!(m.mode(), Mode::None);
    // C: the first spell.
    run(&mut m, &w, &[key(1300, "c", true), key(1310, "c", false)]);
    assert_eq!(m.mode(), Mode::CastSpell(tak_commands::SpellId(1)));
}

#[test]
fn shift_f_keys_reach_build_options_past_f8() {
    let spec: WorldSpec = serde_json::from_str(
        r#"{"local_player": 0, "enemies": [1], "units": [
            {"index": 1, "owner": 0, "def": 10, "x": 640, "y": 480, "builder": "mobile",
             "build_options": [20, 21, 22, 23, 24, 25, 26, 27, 28]}
        ]}"#,
    )
    .unwrap();
    let w = TestWorld::new(spec);
    let mut m = Mapper::new(Profile::wp_s_combined());
    m.warp_cursor(pos(640, 480));
    run(&mut m, &w, &[bdown(0, Button::A), bup(50, Button::A)]);
    run(&mut m, &w, &[key(500, "f8", true), key(510, "f8", false)]);
    assert_eq!(m.mode(), Mode::Build(tak_commands::UnitDefId(27)));
    run(&mut m, &w, &chord(600, "shift", "f1"));
    assert_eq!(m.mode(), Mode::Build(tak_commands::UnitDefId(28)));
    // Shift+F2: no tenth option on this builder, so nothing changes.
    run(&mut m, &w, &chord(700, "shift", "f2"));
    assert_eq!(m.mode(), Mode::Build(tak_commands::UnitDefId(28)));
    // The build wheel lists all nine.
    run(&mut m, &w, &[bdown(800, Button::Lb)]);
    assert_eq!(m.wheel().unwrap().slots.len(), 9);
}

#[test]
fn wheel_hysteresis_holds_near_the_edge() {
    let mut wh = Wheel::open(WheelKind::Command, vec![Action::Stop; 8]);
    let dir = |deg: i32| Vec2::from_angle(tak_math::Angle::from_degrees(deg));
    wh.steer_with(dir(90), 10, false); // straight up: slot 0
    assert_eq!(wh.highlighted, Some(0));
    // 25 degrees clockwise of up is past slot 0's edge (22.5) but inside
    // the hysteresis.
    wh.steer_with(dir(65), 10, false);
    assert_eq!(wh.highlighted, Some(0));
    wh.steer_with(dir(55), 10, false);
    assert_eq!(wh.highlighted, Some(1));
    wh.steer_with(Vec2::ZERO, 10, true);
    assert_eq!(wh.highlighted, None);
}

#[test]
fn sticky_wheel_confirms_on_press() {
    let w = world();
    let mut p = console();
    p.tuning.wheel_confirm_on_press = true;
    let mut m = Mapper::new(p);
    m.warp_cursor(pos(640, 480));
    run(&mut m, &w, &[bdown(0, Button::A), bup(50, Button::A)]);
    // Open, release: stays open. Steer to slot 0 (stop), press A.
    run(
        &mut m,
        &w,
        &[
            bdown(600, Button::Lt),
            bup(650, Button::Lt),
            axis(700, Axis::LeftY, 32767),
        ],
    );
    assert_eq!(m.wheel().unwrap().highlighted, Some(0));
    let out = run(
        &mut m,
        &w,
        &[
            axis(750, Axis::LeftY, 0),
            bdown(800, Button::A),
            bup(850, Button::A),
        ],
    );
    assert!(m.wheel().is_none());
    assert_eq!(
        out.iter()
            .filter(|o| matches!(
                o,
                Output::Sim {
                    command: SimCommand::Stop { .. },
                    ..
                }
            ))
            .count(),
        1
    );
}

// ---- camera ----------------------------------------------------------------

#[test]
fn scroll_zooms_smoothly_within_limits() {
    let w = world();
    let mut m = Mapper::new(console());
    let mut cam = *m.camera();
    cam.world_per_px = Fx::HALF;
    m.set_camera(cam);
    run(
        &mut m,
        &w,
        &[frame(0), ev(1, EventKind::Scroll { milli: 1000 })],
    );
    // Smoothed: part of the way after one frame, all of it later.
    run(&mut m, &w, &[frame(16)]);
    let z1 = m.take_camera_zoom().unwrap();
    assert!(
        z1 < Fx::HALF && z1 > Fx::from_ratio(45, 100).unwrap(),
        "{z1:?}"
    );
    for i in 2..60 {
        run(&mut m, &w, &[frame(16 * i)]);
        if let Some(z) = m.take_camera_zoom() {
            cam.world_per_px = z;
            m.set_camera(cam);
        }
    }
    assert!((m.camera().world_per_px - Fx::from_ratio(45, 100).unwrap()).abs() <= Fx::from_raw(2));
    // Far zoom clamps.
    for i in 0..60 {
        run(
            &mut m,
            &w,
            &[ev(2000 + i, EventKind::Scroll { milli: -5000 })],
        );
    }
    for i in 0..200 {
        run(&mut m, &w, &[frame(3000 + 16 * i)]);
        if let Some(z) = m.take_camera_zoom() {
            cam.world_per_px = z;
            m.set_camera(cam);
        }
    }
    assert_eq!(m.camera().world_per_px, Fx::from_int(4));
}

#[test]
fn edge_pan_ramps_when_the_cursor_pushes_the_edge() {
    let w = world();
    let mut m = Mapper::new(console());
    m.warp_cursor(pos(1279, 480));
    let mut evs = vec![axis(0, Axis::LeftX, 32767)];
    for i in 1..=10 {
        evs.push(frame(16 * i));
    }
    run(&mut m, &w, &evs);
    let early = m.take_camera_pan();
    let mut evs = Vec::new();
    for i in 11..=40 {
        evs.push(frame(16 * i));
    }
    run(&mut m, &w, &evs);
    let late = m.take_camera_pan();
    assert!(early.x > Fx::ZERO && early.y == Fx::ZERO);
    // 30 frames at full speed move more than 3x the 10 ramping frames.
    assert!(
        late.x > early.x.checked_mul_int(3).unwrap(),
        "{early:?} {late:?}"
    );
    // The WP-J tuning has no edge pan.
    let mut m = Mapper::new(Profile::wp_j_controller());
    m.warp_cursor(pos(1279, 480));
    run(
        &mut m,
        &w,
        &[axis(0, Axis::LeftX, 32767), frame(16), frame(500)],
    );
    assert_eq!(m.take_camera_pan(), Vec2::ZERO);
}

#[test]
fn right_stick_pan_keeps_the_cursor_on_its_ground_point() {
    let w = world();
    let mut m = Mapper::new(console());
    let mut cam = *m.camera();
    m.warp_cursor(pos(640, 480));
    let before = m.cursor_world();
    let mut evs = vec![axis(0, Axis::RightX, 32767)];
    for i in 1..=10 {
        evs.push(frame(16 * i));
    }
    run(&mut m, &w, &evs);
    let pan = m.take_camera_pan();
    assert!(pan.x > Fx::ZERO);
    cam.origin += pan;
    m.set_camera(cam);
    assert!(m.cursor_world().distance(before) <= Fx::ONE);
    assert!(m.cursor_screen().x < Fx::from_int(640));
}

#[test]
fn glide_recentres_over_frames() {
    let w = world();
    let mut m = Mapper::new(console());
    let mut cam = *m.camera();
    m.glide_to(pos(2000, 2000));
    let goal = pos(2000, 2000);
    let centre = |c: &Camera| c.screen_to_world(c.center_screen());
    run(&mut m, &w, &[frame(0), frame(16)]);
    cam.origin += m.take_camera_pan();
    m.set_camera(cam);
    assert!(centre(&cam).distance(goal) > Fx::from_int(100));
    for i in 2..120 {
        run(&mut m, &w, &[frame(16 * i)]);
        cam.origin += m.take_camera_pan();
        m.set_camera(cam);
    }
    assert!(centre(&cam).distance(goal) <= Fx::from_int(2));
}

#[test]
fn follow_selection_tracks_and_stops_on_pan() {
    let w = world();
    let mut m = Mapper::new(console());
    m.warp_cursor(pos(900, 480));
    run(&mut m, &w, &[bdown(0, Button::A), bup(50, Button::A)]);
    run(&mut m, &w, &[bdown(600, Button::R3), frame(1000)]);
    assert!(m.following());
    run(
        &mut m,
        &w,
        &[
            bup(1010, Button::R3),
            axis(1020, Axis::RightX, 32767),
            frame(1040),
        ],
    );
    assert!(!m.following());
}

// ---- cursor kinds ------------------------------------------------------------

#[test]
fn cursor_kind_follows_context() {
    let w = world();
    let mut m = Mapper::new(console());
    m.warp_cursor(pos(640, 480));
    assert_eq!(m.cursor_kind(&w), CursorKind::Select);
    m.warp_cursor(pos(200, 200));
    assert_eq!(m.cursor_kind(&w), CursorKind::Normal);
    m.warp_cursor(pos(640, 480));
    run(&mut m, &w, &[bdown(0, Button::A), bup(50, Button::A)]);
    m.warp_cursor(pos(640, 300));
    assert_eq!(m.cursor_kind(&w), CursorKind::Attack);
    m.warp_cursor(pos(700, 480));
    assert_eq!(m.cursor_kind(&w), CursorKind::Guard);
    m.warp_cursor(pos(200, 200));
    assert_eq!(m.cursor_kind(&w), CursorKind::Move);
}

// ---- recording -------------------------------------------------------------

#[test]
fn recording_round_trips_and_gives_ghosts() {
    let w = world();
    let mut m = Mapper::new(console());
    let mut rec = Recorder::new(serde_json::json!({"map": "test"}), &m);
    let script: Vec<(u32, Vec<EventKind>)> = vec![
        (
            0,
            vec![EventKind::Axis {
                axis: Axis::LeftY,
                value: -32767,
            }],
        ),
        (
            20,
            vec![EventKind::Axis {
                axis: Axis::LeftY,
                value: 0,
            }],
        ),
        (200, vec![]),
        (300, vec![EventKind::ButtonDown { button: Button::A }]),
        (350, vec![EventKind::ButtonUp { button: Button::A }]),
        (400, vec![]),
        (500, vec![]),
    ];
    for (t, evs) in &script {
        let mut outs = Vec::new();
        for e in evs.iter().chain([&EventKind::Frame]) {
            outs.extend(m.handle(&ev(*t, e.clone()), &w, tak_commands::Tick(t / 33)));
        }
        let fb = m.take_feedback();
        rec.frame(
            &m,
            *t,
            t / 33,
            FrameIo {
                events: evs,
                outputs: &outs,
                submitted: &[],
                feedback: &fb,
            },
        );
    }
    let r = rec.finish();
    // The last two frames changed nothing and were not stored.
    assert!(r.frames.len() < script.len());
    assert_eq!(r.frames.last().unwrap().t, 350);
    let back = Recording::from_json(&r.to_json()).unwrap();
    assert_eq!(back, r);
    let g = back.ghost_at(15).unwrap();
    assert_eq!(g.selection, UnitSet::single(uid(1)));
    assert_eq!(g.cursor_world, pos(640, 480));
    assert!(back.ghost_at(0).unwrap().selection.is_empty());
    let s = back.summary();
    assert_eq!(s.selections, 1);
    assert!(s.events >= 4);
    assert!(
        Recording::from_json(
            r#"{"version": 99, "profile": {"name": "x", "bindings": []}, "frames": []}"#
        )
        .is_err()
    );
}

// ---- WP-J behaviour is kept under the wp_j preset ---------------------------

#[test]
fn wp_j_tuning_reproduces_the_wp_j_cursor() {
    let w = world();
    let mut m = Mapper::new(Profile::wp_j_controller());
    run(
        &mut m,
        &w,
        &[
            axis(0, Axis::LeftX, 32767),
            frame(50),
            axis(50, Axis::LeftX, 0),
        ],
    );
    assert_eq!(m.cursor_screen(), pos(640 + 45, 480));
    let _ = Target::Ground(pos(0, 0));
}

// ---- the feel lab's verdict ------------------------------------------------

#[test]
fn feel_lab_console_beats_wp_j() {
    let cfg = LabConfig {
        trials: 24,
        ..LabConfig::default()
    };
    let presets: Vec<_> = default_presets()
        .into_iter()
        .filter(|(n, _)| n == "wp_j" || n == "console")
        .collect();
    let rows = run_lab(&presets, &cfg);
    let (wpj, con) = (&rows[0], &rows[1]);
    assert!(con.acquire_ms < wpj.acquire_ms, "{rows:?}");
    assert!(
        con.wrong_permille + con.miss_permille < wpj.wrong_permille + wpj.miss_permille,
        "{rows:?}"
    );
    assert!(con.wrong_permille <= wpj.wrong_permille, "{rows:?}");
    // No brush regression (allow 1 point of noise at this trial count).
    assert!(
        con.brush_f1_permille + 10 >= wpj.brush_f1_permille,
        "{rows:?}"
    );
}

#[test]
fn feel_lab_reticle_matches_near_aim_and_wins_far() {
    let cfg = LabConfig {
        trials: 24,
        ..LabConfig::default()
    };
    let presets: Vec<_> = default_presets()
        .into_iter()
        .filter(|(n, _)| n == "console" || n == "console reticle")
        .collect();
    let rows = run_lab(&presets, &cfg);
    let (free, ret) = (&rows[0], &rows[1]);
    // Same speed law and assist: on-screen aiming is a wash.
    assert!(ret.acquire_ms <= free.acquire_ms + 10, "{rows:?}");
    assert!(
        ret.wrong_permille + ret.miss_permille <= free.wrong_permille + free.miss_permille + 10,
        "{rows:?}"
    );
    // Off-screen targets: no edge-pan ramp, no cursor stuck at the edge.
    assert!(ret.far_ms < free.far_ms, "{rows:?}");
    assert!(
        ret.far_errors_permille <= free.far_errors_permille,
        "{rows:?}"
    );
}
