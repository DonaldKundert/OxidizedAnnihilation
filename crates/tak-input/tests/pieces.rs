//! Unit tests for the pure pieces (stick shaping, magnetism, brush, wheel,
//! context resolution), profile validation, and smaller mapper behaviours.

mod common;

use common::{TestWorld, WorldSpec, pos, run, uid};
use proptest::prelude::*;
use tak_commands::{ClientCommand, QueueMode, SimCommand, Target, UnitDefId, UnitSet};
use tak_input::*;
use tak_math::{Angle, Fx, Vec2};

fn t() -> Tuning {
    Tuning::wp_j()
}

fn ev(t: u32, kind: EventKind) -> InputEvent {
    InputEvent::new(t, kind)
}
fn bdown(t: u32, b: Button) -> InputEvent {
    ev(t, EventKind::ButtonDown { button: b })
}
fn bup(t: u32, b: Button) -> InputEvent {
    ev(t, EventKind::ButtonUp { button: b })
}

fn small_world() -> TestWorld {
    let spec: WorldSpec = serde_json::from_str(
        r#"{"local_player": 0, "enemies": [1], "units": [
            {"index": 1, "owner": 0, "def": 10, "x": 640, "y": 480},
            {"index": 2, "owner": 0, "def": 10, "x": 700, "y": 480},
            {"index": 3, "owner": 0, "def": 11, "x": 900, "y": 480},
            {"index": 4, "owner": 1, "def": 50, "x": 640, "y": 200},
            {"index": 5, "owner": 0, "def": 10, "x": 100, "y": 100, "selectable": false}
        ]}"#,
    )
    .unwrap();
    TestWorld::new(spec)
}

// ---- stick and cursor ------------------------------------------------------

#[test]
fn stick_full_axis_is_exact_and_deadzone_is_zero() {
    assert_eq!(stick_vector(32767, 0, 8000), Vec2::new(Fx::ONE, Fx::ZERO));
    assert_eq!(
        stick_vector(0, -32767, 8000),
        Vec2::new(Fx::ZERO, Fx::NEG_ONE)
    );
    assert_eq!(
        stick_vector(-32768, 0, 8000),
        Vec2::new(Fx::NEG_ONE, Fx::ZERO)
    );
    assert_eq!(stick_vector(5000, -5000, 8000), Vec2::ZERO);
    assert_eq!(stick_vector(0, 0, 0), Vec2::ZERO);
}

proptest! {
    #[test]
    fn stick_vector_is_bounded_and_keeps_direction(x in any::<i16>(), y in any::<i16>()) {
        let v = stick_vector(x, y, 8000);
        prop_assert!(v.length() <= Fx::ONE + Fx::from_raw(4));
        if v != Vec2::ZERO {
            prop_assert!(x == 0 || (v.x.raw() >= 0) == (x > 0) || v.x == Fx::ZERO);
            prop_assert!(y == 0 || (v.y.raw() >= 0) == (y > 0) || v.y == Fx::ZERO);
        }
    }
}

#[test]
fn cursor_step_speeds() {
    let right = Vec2::new(Fx::ONE, Fx::ZERO);
    assert_eq!(cursor_step(right, 50, false, &t()), pos(45, 0));
    assert_eq!(
        cursor_step(right, 50, true, &t()),
        Vec2::new(Fx::from_ratio(45, 4).unwrap(), Fx::ZERO)
    );
    // Stick up moves the cursor up the screen (-y).
    assert_eq!(
        cursor_step(Vec2::new(Fx::ZERO, Fx::ONE), 100, false, &t()),
        pos(0, -90)
    );
    assert_eq!(per_second(1000, 1), Fx::ONE);
    assert_eq!(per_second(i32::MAX, u32::MAX), Fx::MAX);
}

// ---- magnetism -------------------------------------------------------------

#[test]
fn magnetism_snaps_pulls_and_ignores_far_targets() {
    let targets = [pos(100, 100), pos(300, 100)];
    // Inside snap distance: exactly onto the target.
    assert_eq!(
        magnetize(pos(110, 100), &targets, 16, &t()),
        (pos(100, 100), Some(0))
    );
    // Inside the radius: pulled 0.4 of the way at 50 ms (8 per second).
    let (c, hit) = magnetize(pos(130, 100), &targets, 50, &t());
    assert_eq!(hit, Some(0));
    // 0.4 is not exact in 16.16; allow a few ulps.
    assert!((c.x - Fx::from_int(118)).abs() <= Fx::from_raw(64), "{c:?}");
    assert_eq!(c.y, Fx::from_int(100));
    // A long frame pulls all the way.
    assert_eq!(
        magnetize(pos(130, 100), &targets, 200, &t()).0,
        pos(100, 100)
    );
    // Outside the radius: untouched.
    assert_eq!(
        magnetize(pos(200, 100), &targets, 50, &t()),
        (pos(200, 100), None)
    );
    // Nearer target wins; exact ties go to the earlier one.
    assert_eq!(magnetize(pos(280, 100), &targets, 50, &t()).1, Some(1));
    let tied = [pos(0, 0), pos(20, 0)];
    assert_eq!(magnetize(pos(10, 0), &tied, 50, &t()).1, Some(0));
}

proptest! {
    #[test]
    fn magnetism_never_moves_away(cx in 0i32..400, cy in 0i32..400, dt in 0u32..200) {
        let targets = [pos(100, 100), pos(250, 300)];
        let c = pos(cx, cy);
        let (out, hit) = magnetize(c, &targets, dt, &t());
        match hit {
            None => prop_assert_eq!(out, c),
            Some(i) => prop_assert!(out.distance(targets[i]) <= c.distance(targets[i])),
        }
    }
}

// ---- brush -----------------------------------------------------------------

#[test]
fn brush_grows_and_caps() {
    assert_eq!(brush_radius(0, &t()), Fx::from_int(16));
    assert_eq!(brush_radius(500, &t()), Fx::from_int(96));
    assert_eq!(brush_radius(10_000, &t()), Fx::from_int(192));
    assert_eq!(brush_radius(u32::MAX, &t()), Fx::from_int(192));
}

#[test]
fn drag_circle_spans_the_drag() {
    let (c, r) = drag_circle(pos(100, 100), pos(140, 130));
    assert_eq!(c, pos(120, 115));
    assert_eq!(r, Fx::from_int(25));
}

#[test]
fn circle_selects_own_selectable_units_only() {
    let w = small_world();
    let got = units_in_circle(&w, pos(670, 480), Fx::from_int(40));
    assert_eq!(got, [uid(1), uid(2)].into_iter().collect::<UnitSet>());
    // The enemy and the unselectable unit are never picked.
    assert!(units_in_circle(&w, pos(640, 200), Fx::from_int(10)).is_empty());
    assert!(units_in_circle(&w, pos(100, 100), Fx::from_int(10)).is_empty());
}

// ---- wheel -----------------------------------------------------------------

#[test]
fn wheel_slots_run_clockwise_from_top() {
    let up = Vec2::new(Fx::ZERO, Fx::ONE);
    let right = Vec2::new(Fx::ONE, Fx::ZERO);
    let down = Vec2::new(Fx::ZERO, Fx::NEG_ONE);
    let left = Vec2::new(Fx::NEG_ONE, Fx::ZERO);
    assert_eq!(slot_for(up, 8), Some(0));
    assert_eq!(slot_for(right, 8), Some(2));
    assert_eq!(slot_for(down, 8), Some(4));
    assert_eq!(slot_for(left, 8), Some(6));
    assert_eq!(slot_for(left, 4), Some(3));
    assert_eq!(slot_for(Vec2::ZERO, 8), None);
    assert_eq!(slot_for(up, 0), None);
    assert_eq!(slot_for(down, 1), Some(0));
}

proptest! {
    #[test]
    fn slot_direction_inverts_slot_for(n in 1usize..=16, k in 0usize..16) {
        let k = k % n;
        let v = Vec2::from_angle(slot_direction(k, n));
        prop_assert_eq!(slot_for(v, n), Some(k));
    }

    #[test]
    fn any_direction_has_a_slot(raw in any::<u16>(), n in 1usize..=16) {
        let s = slot_for(Vec2::from_angle(Angle::from_raw(raw)), n);
        prop_assert!(s.is_some_and(|s| s < n));
    }
}

#[test]
fn wheel_keeps_highlight_on_flick_and_cancels_without_one() {
    let mut w = Wheel::open(WheelKind::Command, vec![Action::Stop, Action::PatrolMode]);
    assert_eq!(w.clone().confirm(), None);
    w.steer(Vec2::new(Fx::ZERO, Fx::NEG_ONE)); // down: slot 1 of 2
    w.steer(Vec2::ZERO); // stick back to centre
    assert_eq!(w.highlighted, Some(1));
    assert_eq!(w.confirm(), Some(Action::PatrolMode));
}

// ---- context ---------------------------------------------------------------

#[test]
fn context_resolution() {
    let w = small_world();
    let sel: UnitSet = [uid(1), uid(2)].into_iter().collect();
    let enemy = w.unit(uid(4));
    let friend = w.unit(uid(3));
    let ground = pos(500, 500);
    let r =
        |h: Option<&UnitInfo>, mode| resolve_context(&w, &sel, ground, h, mode, QueueMode::Replace);
    assert!(matches!(
        r(enemy.as_ref(), Mode::None),
        Some(SimCommand::Attack { target: Target::Unit(id), .. }) if id == uid(4)
    ));
    assert!(matches!(
        r(friend.as_ref(), Mode::None),
        Some(SimCommand::Guard { target, .. }) if target == uid(3)
    ));
    assert!(matches!(r(None, Mode::None), Some(SimCommand::Move { to, .. }) if to == ground));
    // Hovering a unit of the selection itself is a move.
    assert!(matches!(
        r(w.unit(uid(1)).as_ref(), Mode::None),
        Some(SimCommand::Move { .. })
    ));
    assert!(matches!(
        r(enemy.as_ref(), Mode::Patrol),
        Some(SimCommand::Patrol { .. })
    ));
    assert_eq!(r(enemy.as_ref(), Mode::Guard), None);
    assert_eq!(r(None, Mode::Guard), None);
    // No builders in this selection can place def 20.
    assert_eq!(r(None, Mode::Build(UnitDefId(20))), None);
    assert_eq!(
        resolve_context(
            &w,
            &UnitSet::new(),
            ground,
            None,
            Mode::None,
            QueueMode::Replace
        ),
        None
    );
}

#[test]
fn pick_prefers_nearest_selectable() {
    let w = small_world();
    assert_eq!(pick_unit(&w, pos(645, 480)).map(|u| u.id), Some(uid(1)));
    assert_eq!(pick_unit(&w, pos(660, 480)), None);
    assert_eq!(pick_unit(&w, pos(100, 100)), None); // unselectable
}

// ---- profiles --------------------------------------------------------------

#[test]
fn default_profiles_load_and_round_trip() {
    for p in [
        Profile::default_controller(),
        Profile::default_mouse_keyboard(),
    ] {
        let back = Profile::from_json(&p.to_json_pretty()).unwrap();
        assert_eq!(back, p);
    }
    assert_eq!(Profile::default_controller().command_wheel.len(), 8);
}

#[test]
fn profile_validation_rejects_bad_bindings() {
    let bad = |bindings: &str| {
        Profile::from_json(&format!(r#"{{"name": "x", "bindings": [{bindings}]}}"#)).unwrap_err()
    };
    assert!(matches!(
        bad(
            r#"{"input": {"button": "a"}, "with": {"button": "a"}, "gesture": "press", "action": {"type": "stop"}}"#
        ),
        ProfileError::SelfChord(0)
    ));
    assert!(matches!(
        bad(r#"{"input": {"button": "a"}, "gesture": "tap", "action": {"type": "brush"}}"#),
        ProfileError::HeldOnInstantGesture(0, Action::Brush)
    ));
    assert!(matches!(
        bad(
            r#"{"input": {"key": "s"}, "gesture": "press", "action": {"type": "stop"}},
               {"input": {"key": "s"}, "gesture": "press", "action": {"type": "cancel"}}"#
        ),
        ProfileError::Duplicate { index: 1, .. }
    ));
    assert!(matches!(
        bad(
            r#"{"input": {"key": "1"}, "gesture": "press", "action": {"type": "recall_group", "group": 10}}"#
        ),
        ProfileError::Json(_)
    ));
    assert!(matches!(
        Profile::from_json(
            r#"{"name": "x", "bindings": [], "command_wheel": [{"type": "precision"}]}"#
        )
        .unwrap_err(),
        ProfileError::HeldInWheel(0, Action::Precision)
    ));
    assert!(matches!(
        Profile::from_json(
            r#"{"name": "x", "tuning": {"wheel_max_slots": 1}, "bindings": [], "command_wheel": [{"type": "stop"}, {"type": "stop"}]}"#
        )
        .unwrap_err(),
        ProfileError::WheelTooLarge(2, 1)
    ));
    assert!(Profile::from_json("not json").is_err());
}

#[test]
fn event_json_format() {
    let e: InputEvent =
        serde_json::from_str(r#"{"t": 120, "type": "button_down", "button": "a"}"#).unwrap();
    assert_eq!(e, bdown(120, Button::A));
    let e: InputEvent =
        serde_json::from_str(r#"{"t": 5, "type": "axis", "axis": "left_x", "value": -32767}"#)
            .unwrap();
    assert_eq!(
        e.kind,
        EventKind::Axis {
            axis: Axis::LeftX,
            value: -32767
        }
    );
    let e: InputEvent =
        serde_json::from_str(r#"{"t": 5, "type": "key_down", "key": "shift"}"#).unwrap();
    assert_eq!(
        e.kind,
        EventKind::KeyDown {
            key: Key::new("shift")
        }
    );
    assert!(
        serde_json::from_str::<InputEvent>(
            r#"{"t": 5, "type": "axis", "axis": "left_x", "value": 40000}"#
        )
        .is_err()
    );
}

// ---- mapper behaviours -----------------------------------------------------

fn sims(out: &[Output]) -> Vec<SimCommand> {
    out.iter()
        .filter_map(|o| match o {
            Output::Sim { command, .. } => Some(command.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn right_stick_pans_camera() {
    let w = small_world();
    let mut m = Mapper::new(Profile::wp_j_controller());
    run(
        &mut m,
        &w,
        &[
            ev(
                0,
                EventKind::Axis {
                    axis: Axis::RightY,
                    value: 32767,
                },
            ),
            ev(100, EventKind::Frame),
            ev(
                100,
                EventKind::Axis {
                    axis: Axis::RightY,
                    value: 0,
                },
            ),
        ],
    );
    // 1200 world units/s for 100 ms, stick up = world -y.
    assert_eq!(m.take_camera_pan(), pos(0, -120));
    assert_eq!(m.take_camera_pan(), Vec2::ZERO);
    assert_eq!(m.cursor_screen(), pos(640, 480));
}

#[test]
fn pending_single_tap_resolves_before_another_input() {
    // Y (single tap pending) then X quickly: the same-type selection must
    // happen before the order, not after it.
    let w = small_world();
    let mut m = Mapper::new(Profile::wp_j_controller());
    let out = run(
        &mut m,
        &w,
        &[
            bdown(0, Button::A),
            bup(50, Button::A), // select u1 (cursor starts on it)
            bdown(100, Button::Y),
            bup(150, Button::Y),
            bdown(200, Button::X),
        ],
    );
    let kinds: Vec<String> = out.iter().map(|o| format!("{o:?}")).collect();
    assert_eq!(out.len(), 3, "{kinds:?}");
    assert!(matches!(
        out[1],
        Output::Client {
            command: ClientCommand::SelectSameType { .. }
        }
    ));
    // u1 and u2 share def 10 and are both on screen.
    assert!(
        matches!(&out[2], Output::Sim { command: SimCommand::Move { units, .. }, .. }
        if units.len() == 2)
    );
}

#[test]
fn chord_modifier_suppresses_its_own_tap() {
    // Mouse/keyboard: ctrl + 3 assigns; releasing ctrl does nothing else, and
    // a bare 3 later recalls.
    let w = small_world();
    let mut m = Mapper::new(common::wp_j_mk());
    let k = |t, down: bool, key: &str| {
        ev(
            t,
            if down {
                EventKind::KeyDown { key: Key::new(key) }
            } else {
                EventKind::KeyUp { key: Key::new(key) }
            },
        )
    };
    let out = run(
        &mut m,
        &w,
        &[
            ev(0, EventKind::PointerMove { x: 700, y: 480 }),
            bdown(10, Button::MouseLeft),
            bup(20, Button::MouseLeft),
            k(30, true, "ctrl"),
            k(40, true, "3"),
            k(50, false, "3"),
            k(60, false, "ctrl"),
            k(70, true, "escape"),
            k(80, true, "3"),
        ],
    );
    let cmds: Vec<String> = out
        .iter()
        .map(|o| serde_json::to_string(o).unwrap())
        .collect();
    assert_eq!(cmds.len(), 4, "{cmds:#?}");
    assert!(cmds[1].contains("assign_group"));
    assert_eq!(
        out[2],
        Output::Client {
            command: ClientCommand::ClearSelection
        }
    );
    assert!(cmds[3].contains("recall_group"));
    assert_eq!(m.selection(), &UnitSet::single(uid(2)));
}

#[test]
fn mouse_drag_selects_a_circle_and_click_selects_one() {
    let w = small_world();
    let mut m = Mapper::new(common::wp_j_mk());
    let out = run(
        &mut m,
        &w,
        &[
            ev(0, EventKind::PointerMove { x: 620, y: 480 }),
            bdown(10, Button::MouseLeft),
            ev(20, EventKind::PointerMove { x: 720, y: 480 }),
            bup(30, Button::MouseLeft),
        ],
    );
    assert_eq!(
        out,
        vec![Output::Client {
            command: ClientCommand::SelectBrush {
                center: pos(670, 480),
                radius: Fx::from_int(50),
                additive: false
            }
        }]
    );
    assert_eq!(m.selection().len(), 2);
    // Shift-click on u3 adds it.
    let out = run(
        &mut m,
        &w,
        &[
            ev(40, EventKind::PointerMove { x: 902, y: 481 }),
            ev(
                50,
                EventKind::KeyDown {
                    key: Key::new("shift"),
                },
            ),
            bdown(60, Button::MouseLeft),
            bup(70, Button::MouseLeft),
            ev(
                80,
                EventKind::KeyUp {
                    key: Key::new("shift"),
                },
            ),
        ],
    );
    assert!(matches!(
        out[0],
        Output::Client {
            command: ClientCommand::SelectAt { additive: true, .. }
        }
    ));
    assert_eq!(m.selection().len(), 3);
}

#[test]
fn moving_brush_paints_additively() {
    let w = small_world();
    let mut m = Mapper::new(Profile::wp_j_controller());
    // Hold A on u1 (brush radius 16 at activation), then sweep right 90 px
    // over u2 at 700.
    let out = run(
        &mut m,
        &w,
        &[
            bdown(0, Button::A),
            ev(250, EventKind::Frame),
            ev(
                250,
                EventKind::Axis {
                    axis: Axis::LeftX,
                    value: 32767,
                },
            ),
            ev(300, EventKind::Frame),
            ev(
                300,
                EventKind::Axis {
                    axis: Axis::LeftX,
                    value: 0,
                },
            ),
            bup(300, Button::A),
        ],
    );
    let stamps: Vec<(Vec2, bool)> = out
        .iter()
        .map(|o| match o {
            Output::Client {
                command:
                    ClientCommand::SelectBrush {
                        center, additive, ..
                    },
            } => (*center, *additive),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        stamps,
        vec![
            (pos(640, 480), false),
            (pos(685, 480), true),
            (pos(685, 480), true)
        ]
    );
    assert_eq!(
        m.selection(),
        &[uid(1), uid(2)].into_iter().collect::<UnitSet>()
    );
}

#[test]
fn cursor_is_clamped_to_viewport() {
    let w = small_world();
    let mut m = Mapper::new(Profile::wp_j_controller());
    run(
        &mut m,
        &w,
        &[
            ev(
                0,
                EventKind::Axis {
                    axis: Axis::LeftX,
                    value: -32767,
                },
            ),
            ev(5000, EventKind::Frame),
        ],
    );
    assert_eq!(m.cursor_screen(), pos(0, 480));
    // A huge gap between events saturates instead of wrapping.
    run(
        &mut m,
        &w,
        &[
            ev(
                5000,
                EventKind::Axis {
                    axis: Axis::LeftX,
                    value: 32767,
                },
            ),
            ev(4_000_000, EventKind::Frame),
        ],
    );
    assert_eq!(m.cursor_screen(), pos(1280, 480));
}

#[test]
fn orders_skip_dead_units() {
    let mut spec: WorldSpec = small_world().spec;
    let w = TestWorld::new(spec.clone());
    let mut m = Mapper::new(common::wp_j_mk());
    run(
        &mut m,
        &w,
        &[
            ev(0, EventKind::PointerMove { x: 600, y: 480 }),
            bdown(10, Button::MouseLeft),
            ev(20, EventKind::PointerMove { x: 740, y: 480 }),
            bup(30, Button::MouseLeft),
        ],
    );
    assert_eq!(m.selection().len(), 2);
    spec.units.retain(|u| u.index != 2); // u2 dies
    let w2 = TestWorld::new(spec);
    let out = run(
        &mut m,
        &w2,
        &[ev(40, EventKind::KeyDown { key: Key::new("s") })],
    );
    assert_eq!(
        sims(&out),
        vec![SimCommand::Stop {
            units: UnitSet::single(uid(1))
        }]
    );
}
