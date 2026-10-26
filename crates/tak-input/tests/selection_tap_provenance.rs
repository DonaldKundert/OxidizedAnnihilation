//! Chosen controller/default-mouse gesture policy: only a successful point
//! selection arms like-type expansion on the same live owned handle. This is
//! not additional evidence for the original manual's mouse-selection rules.

mod common;

use common::{TestWorld, pos, uid};
use tak_commands::{
    ClientCommand, ControlGroup, QueueMode, Scope, SimCommand, Target, Tick, UnitDefId, UnitSet,
};
use tak_input::{
    Action, Axis, Button, EventKind, InputEvent, Mapper, Mode, Output, Profile, Ring, WheelKind,
    slot_direction,
};
use tak_math::Fx;

fn world() -> TestWorld {
    TestWorld::new(
        serde_json::from_str(
            r#"{
        "local_player":0,"enemies":[1],"units":[
            {"index":1,"owner":0,"def":10,"x":200,"y":200,"builder":"mobile","build_options":[20]},
            {"index":2,"owner":0,"def":10,"x":300,"y":200},
            {"index":3,"owner":0,"def":11,"x":400,"y":200},
            {"index":4,"owner":1,"def":10,"x":600,"y":200},
            {"index":5,"owner":2,"def":10,"x":700,"y":200},
            {"index":6,"owner":0,"def":10,"x":3000,"y":200},
            {"index":7,"owner":0,"def":10,"x":500,"y":200,"selectable":false}
        ]}"#,
        )
        .unwrap(),
    )
}

fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&id| uid(id)).collect()
}

fn event(m: &mut Mapper, w: &TestWorld, kind: EventKind) -> Vec<Output> {
    m.handle(&InputEvent::new(m.now() + 10, kind), w, Tick(1))
}

fn button(m: &mut Mapper, w: &TestWorld, b: Button, down: bool) -> Vec<Output> {
    event(
        m,
        w,
        if down {
            EventKind::ButtonDown { button: b }
        } else {
            EventKind::ButtonUp { button: b }
        },
    )
}

fn tap(m: &mut Mapper, w: &TestWorld, b: Button) -> Vec<Output> {
    let mut out = button(m, w, b, true);
    out.extend(button(m, w, b, false));
    out
}

fn point(m: &mut Mapper, w: &TestWorld, x: i32, y: i32) {
    event(m, w, EventKind::PointerMove { x, y });
}

fn click(m: &mut Mapper, w: &TestWorld, b: Button, x: i32, y: i32) -> Vec<Output> {
    point(m, w, x, y);
    tap(m, w, b)
}

fn selected(profile: Profile, w: &TestWorld) -> Mapper {
    let mut m = Mapper::new(profile);
    point(&mut m, w, 200, 200);
    m.perform(Action::Select, w, Tick(1));
    m
}

fn selection(x: i32, additive: bool) -> Vec<Output> {
    vec![Output::Client {
        command: ClientCommand::SelectAt {
            pos: pos(x, 200),
            additive,
        },
    }]
}

fn sticky_profile() -> Profile {
    let mut p = Profile::default_combined();
    p.tuning.wheel_confirm_on_press = true;
    p
}

fn profiles() -> [Profile; 3] {
    [
        Profile::default_combined(),
        Profile::original_tak(),
        Profile::classic_tak(),
    ]
}

#[test]
fn rapid_different_targets_are_fresh_selections_for_controller_and_mouse() {
    let w = world();
    for (profile, b) in [
        (Profile::default_combined(), Button::A),
        (Profile::default_combined(), Button::MouseLeft),
        (Profile::original_tak(), Button::MouseLeft),
    ] {
        let mut m = Mapper::new(profile);
        assert_eq!(click(&mut m, &w, b, 200, 200), selection(200, false));
        for x in [300, 400, 200] {
            assert_eq!(
                click(&mut m, &w, b, x, 200),
                selection(x, false),
                "{b:?} at {x}"
            );
        }
        assert_eq!(m.selection(), &units(&[1]));
    }
}

#[test]
fn same_target_double_tap_is_immediate_but_expired_tap_is_fresh() {
    let w = world();
    for (profile, b) in [
        (Profile::default_combined(), Button::A),
        (Profile::default_combined(), Button::MouseLeft),
        (Profile::original_tak(), Button::MouseLeft),
    ] {
        let mut m = Mapper::new(profile);
        assert_eq!(click(&mut m, &w, b, 200, 200), selection(200, false));
        assert_eq!(
            click(&mut m, &w, b, 201, 200),
            vec![Output::Client {
                command: ClientCommand::SelectSameType {
                    scope: Scope::OnScreen
                }
            }]
        );
        assert_eq!(m.selection(), &units(&[1, 2]));
        let t = m.now() + m.tuning().double_tap_ms + 1;
        m.handle(&InputEvent::new(t, EventKind::Frame), &w, Tick(1));
        assert_eq!(click(&mut m, &w, b, 200, 200), selection(200, false));
    }
}

#[test]
fn empty_foreign_unselectable_clicks_never_seed_same_type_selection() {
    let w = world();
    for b in [Button::A, Button::MouseLeft] {
        for x in [800, 600, 700, 500] {
            let mut m = Mapper::new(Profile::default_combined());
            assert_eq!(click(&mut m, &w, b, x, 200), selection(x, false));
            assert!(m.selection().is_empty());
            assert_eq!(
                click(&mut m, &w, b, 200, 200),
                selection(200, false),
                "{b:?}, previous x={x}"
            );
            assert_eq!(m.selection(), &units(&[1]));
        }
    }
}

#[test]
fn double_tap_revalidates_live_owned_full_generational_target() {
    for b in [Button::A, Button::MouseLeft] {
        for change in 0..4 {
            let mut w = world();
            let mut m = Mapper::new(Profile::default_combined());
            click(&mut m, &w, b, 200, 200);
            match change {
                0 => {
                    w.spec.units.remove(0);
                }
                1 => w.spec.units[0].generation += 1,
                2 => w.spec.units[0].owner = 1,
                _ => w.spec.units[0].selectable = false,
            }
            assert_eq!(
                click(&mut m, &w, b, 200, 200),
                selection(200, false),
                "{b:?}, change={change}"
            );
            if change == 1 {
                assert_eq!(m.selection(), &UnitSet::from(vec![w.spec.units[0].id()]));
            } else {
                assert!(m.selection().is_empty());
            }
        }
    }
}

fn steer(m: &mut Mapper, w: &TestWorld, action: Action) {
    let wheel = m.wheel().expect("wheel open");
    let slot = wheel
        .slots
        .iter()
        .position(|&a| a == action)
        .expect("slot exists");
    let (s, c) = slot_direction(slot, wheel.slots.len()).sin_cos();
    let scale = |v: Fx| ((i64::from(v.raw()) * 32767) >> 16) as i16;
    // Both axes belong to one poll, then return to centre so the camera does
    // not move after a confirming tap closes the wheel.
    let t = m.now() + 10;
    for (axis, value) in [(Axis::LeftX, scale(c)), (Axis::LeftY, scale(s))] {
        m.handle(
            &InputEvent::new(t, EventKind::Axis { axis, value }),
            w,
            Tick(1),
        );
    }
    assert_eq!(m.wheel().unwrap().highlighted, Some(slot));
    let t = m.now() + 1;
    for axis in [Axis::LeftX, Axis::LeftY] {
        m.handle(
            &InputEvent::new(t, EventKind::Axis { axis, value: 0 }),
            w,
            Tick(1),
        );
    }
}

#[test]
fn rapid_nested_wheel_confirmations_do_not_turn_into_world_selection() {
    let w = world();
    let mut m = selected(sticky_profile(), &w);
    tap(&mut m, &w, Button::Lt);
    steer(&mut m, &w, Action::OpenRing { ring: Ring::Unit });
    assert!(tap(&mut m, &w, Button::A).is_empty());
    assert_eq!(m.wheel().unwrap().kind, WheelKind::Unit);
    steer(&mut m, &w, Action::OpenRing { ring: Ring::Squads });
    assert!(tap(&mut m, &w, Button::A).is_empty());
    assert_eq!(m.wheel().unwrap().kind, WheelKind::Squads);
    let group = ControlGroup::new(5).unwrap();
    steer(&mut m, &w, Action::RecallGroup { group });
    assert_eq!(
        tap(&mut m, &w, Button::A),
        vec![Output::Client {
            command: ClientCommand::RecallGroup { group }
        }]
    );
    assert!(m.wheel().is_none());
    assert_eq!(
        click(&mut m, &w, Button::A, 200, 200),
        selection(200, false)
    );
}

#[test]
fn wheel_confirmed_build_heal_attack_accept_next_a_target_and_then_fresh_selection() {
    let w = world();
    for action in [
        Action::BuildOption { index: 0 },
        Action::HealMode,
        Action::AttackMode,
    ] {
        let mut m = selected(sticky_profile(), &w);
        if matches!(action, Action::BuildOption { .. }) {
            button(&mut m, &w, Button::Lb, true);
            let t = m.now() + m.tuning().hold_ms;
            m.handle(&InputEvent::new(t, EventKind::Frame), &w, Tick(1));
            button(&mut m, &w, Button::Lb, false);
        } else {
            tap(&mut m, &w, Button::Lt);
            if action == Action::HealMode {
                steer(&mut m, &w, Action::OpenRing { ring: Ring::Unit });
                tap(&mut m, &w, Button::A);
            }
        }
        steer(&mut m, &w, action);
        assert!(tap(&mut m, &w, Button::A).is_empty(), "{action:?}");
        assert!(m.wheel().is_none());
        let (x, mode, command) = match action {
            Action::BuildOption { .. } => (
                800,
                Mode::Build(UnitDefId(20)),
                SimCommand::Build {
                    units: units(&[1]),
                    def: UnitDefId(20),
                    at: Some(pos(800, 200)),
                    queue: QueueMode::Replace,
                },
            ),
            Action::HealMode => (
                300,
                Mode::Heal,
                SimCommand::Heal {
                    units: units(&[1]),
                    target: uid(2),
                    queue: QueueMode::Replace,
                },
            ),
            _ => (
                600,
                Mode::Attack,
                SimCommand::Attack {
                    units: units(&[1]),
                    target: Target::Unit(uid(4)),
                    queue: QueueMode::Replace,
                },
            ),
        };
        assert_eq!(m.mode(), mode);
        assert_eq!(
            click(&mut m, &w, Button::A, x, 200),
            vec![Output::Sim {
                tick: Tick(1),
                command
            }]
        );
        assert_eq!(m.mode(), Mode::None);
        assert_eq!(
            click(&mut m, &w, Button::A, 200, 200),
            selection(200, false)
        );
    }
}

#[test]
fn cancelled_mouse_click_and_controller_selection_do_not_seed_a_later_click() {
    let w = world();
    for p in profiles() {
        let mut m = selected(p, &w);
        point(&mut m, &w, 200, 200);
        button(&mut m, &w, Button::MouseLeft, true);
        m.perform(Action::Cancel, &w, Tick(1));
        assert!(button(&mut m, &w, Button::MouseLeft, false).is_empty());
        assert_eq!(
            click(&mut m, &w, Button::MouseLeft, 200, 200),
            selection(200, false)
        );
    }
    let mut m = Mapper::new(Profile::default_combined());
    click(&mut m, &w, Button::A, 200, 200);
    m.perform(Action::Cancel, &w, Tick(1));
    assert_eq!(
        click(&mut m, &w, Button::A, 200, 200),
        selection(200, false)
    );
}

#[test]
fn rb_add_selection_and_unrelated_second_tap_bindings_keep_their_meaning() {
    let w = world();
    let mut m = Mapper::new(Profile::default_combined());
    click(&mut m, &w, Button::A, 200, 200);
    button(&mut m, &w, Button::Rb, true);
    assert_eq!(click(&mut m, &w, Button::A, 300, 200), selection(300, true));
    assert_eq!(click(&mut m, &w, Button::A, 400, 200), selection(400, true));
    assert!(button(&mut m, &w, Button::Rb, false).is_empty());
    assert_eq!(m.selection(), &units(&[1, 2, 3]));
    tap(&mut m, &w, Button::Rb);
    assert_eq!(
        tap(&mut m, &w, Button::Rb),
        vec![Output::SelectAll {
            scope: Scope::Map,
            units: units(&[1, 2, 3, 6])
        }]
    );
    let group = ControlGroup::new(1).unwrap();
    m.perform(Action::AssignGroup { group }, &w, Tick(1));
    assert_eq!(
        tap(&mut m, &w, Button::DpadUp),
        vec![Output::Client {
            command: ClientCommand::RecallGroup { group }
        }]
    );
    assert!(tap(&mut m, &w, Button::DpadUp).is_empty());
    assert_eq!(m.selection(), &units(&[1, 2, 3, 6]));
}

#[test]
fn custom_press_and_release_point_selection_keep_same_unit_second_tap() {
    let w = world();
    for gesture in ["press", "release"] {
        let profile = Profile::from_json(&format!(r#"{{"name":"point selection on {gesture}","bindings":[
            {{"input":{{"button":"a"}},"gesture":"{gesture}","action":{{"type":"select"}}}},
            {{"input":{{"button":"a"}},"gesture":"second_tap","action":{{"type":"select_same_type","scope":"on_screen"}}}}
        ]}}"#)).unwrap();
        let mut m = Mapper::new(profile);
        assert_eq!(
            click(&mut m, &w, Button::A, 200, 200),
            selection(200, false)
        );
        let out = button(&mut m, &w, Button::A, true);
        assert_eq!(
            out,
            vec![Output::Client {
                command: ClientCommand::SelectSameType {
                    scope: Scope::OnScreen
                }
            }]
        );
        // A custom explicit Release binding still executes on release. Its
        // existing behavior is independent of second-tap recognition.
        button(&mut m, &w, Button::A, false);
    }
}

#[test]
fn legacy_delayed_double_tap_remains_independent_of_point_selection_provenance() {
    let w = world();
    let profile = Profile::from_json(r#"{"name":"delayed point selection","bindings":[
        {"input":{"button":"a"},"gesture":"tap","action":{"type":"select"}},
        {"input":{"button":"a"},"gesture":"double_tap","action":{"type":"select_same_type","scope":"on_screen"}}
    ]}"#).unwrap();
    let mut m = selected(profile, &w);
    assert!(click(&mut m, &w, Button::A, 800, 200).is_empty());
    assert_eq!(
        tap(&mut m, &w, Button::A),
        vec![Output::Client {
            command: ClientCommand::SelectSameType {
                scope: Scope::OnScreen
            }
        }]
    );
    assert_eq!(m.selection(), &units(&[1, 2]));
}

#[test]
fn direct_modal_cancel_between_taps_requires_a_fresh_point_selection() {
    let w = world();
    for b in [Button::A, Button::MouseLeft] {
        let mut m = Mapper::new(Profile::default_combined());
        click(&mut m, &w, b, 200, 200);
        m.perform(Action::HealMode, &w, Tick(1));
        m.perform(Action::Cancel, &w, Tick(1));
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(click(&mut m, &w, b, 200, 200), selection(200, false));
    }
}

#[test]
fn lost_pointer_surface_breaks_history_but_neutral_pad_pointer_leave_does_not() {
    let w = world();
    let mut mouse = Mapper::new(Profile::default_combined());
    click(&mut mouse, &w, Button::MouseLeft, 200, 200);
    event(&mut mouse, &w, EventKind::PointerLeaveTerrain);
    assert_eq!(
        click(&mut mouse, &w, Button::MouseLeft, 200, 200),
        selection(200, false)
    );

    let mut pad = Mapper::new(Profile::wp_s_controller());
    pad.warp_cursor(pos(200, 200));
    assert_eq!(tap(&mut pad, &w, Button::A), selection(200, false));
    event(&mut pad, &w, EventKind::PointerLeaveTerrain);
    assert_eq!(
        tap(&mut pad, &w, Button::A),
        vec![Output::Client {
            command: ClientCommand::SelectSameType {
                scope: Scope::OnScreen
            }
        }]
    );
}

#[test]
fn interrupted_held_mouse_selection_does_not_rearm_on_release_after_return() {
    let w = world();
    for return_before_release in [false, true] {
        let mut m = Mapper::new(Profile::default_combined());
        point(&mut m, &w, 200, 200);
        button(&mut m, &w, Button::MouseLeft, true);
        event(&mut m, &w, EventKind::PointerLeaveTerrain);
        if return_before_release {
            point(&mut m, &w, 200, 200);
        }
        button(&mut m, &w, Button::MouseLeft, false);
        assert_eq!(
            click(&mut m, &w, Button::MouseLeft, 200, 200),
            selection(200, false)
        );
    }
}

#[test]
fn recorded_selection_interruption_is_narrow_and_round_trips() {
    let w = world();
    let interruption = InputEvent::new(50, EventKind::SelectionTapInterrupted);
    let json = serde_json::to_string(&interruption).unwrap();
    assert_eq!(json, r#"{"t":50,"type":"selection_tap_interrupted"}"#);
    assert_eq!(
        serde_json::from_str::<InputEvent>(&json).unwrap(),
        interruption
    );

    let mut m = Mapper::new(Profile::default_combined());
    click(&mut m, &w, Button::A, 200, 200);
    button(&mut m, &w, Button::Rb, true);
    assert_eq!(click(&mut m, &w, Button::A, 300, 200), selection(300, true));
    event(&mut m, &w, EventKind::SelectionTapInterrupted);
    assert_eq!(m.selection(), &units(&[1, 2]));
    assert_eq!(m.production_batch_count(), 5, "queue modifier remains held");
    assert_eq!(click(&mut m, &w, Button::A, 300, 200), selection(300, true));
    button(&mut m, &w, Button::Rb, false);
    tap(&mut m, &w, Button::Rb);
    event(&mut m, &w, EventKind::SelectionTapInterrupted);
    assert_eq!(
        tap(&mut m, &w, Button::Rb),
        vec![Output::SelectAll {
            scope: Scope::Map,
            units: units(&[1, 2, 3, 6])
        }]
    );

    m.perform(Action::AttackMode, &w, Tick(1));
    event(&mut m, &w, EventKind::SelectionTapInterrupted);
    assert_eq!(m.mode(), Mode::Attack);
    assert_eq!(m.selection(), &units(&[1, 2, 3, 6]));
}

#[test]
fn valid_map_origin_same_target_selection_keeps_its_second_tap() {
    let w = world();
    let mut m = Mapper::new(Profile::default_combined());
    event(
        &mut m,
        &w,
        EventKind::MapPointer {
            x: 100,
            y: 100,
            world: pos(200, 200),
        },
    );
    assert_eq!(tap(&mut m, &w, Button::MouseLeft), selection(200, false));
    event(
        &mut m,
        &w,
        EventKind::MapPointer {
            x: 100,
            y: 100,
            world: pos(200, 200),
        },
    );
    assert_eq!(
        tap(&mut m, &w, Button::MouseLeft),
        vec![Output::Client {
            command: ClientCommand::SelectSameType {
                scope: Scope::OnScreen
            }
        }]
    );
}
