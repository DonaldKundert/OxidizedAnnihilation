//! Chosen defensive UI rule: Stop disarms unfinished orders, not selection.
//! The original manual establishes Stop, not these gesture-reset semantics.

mod common;

use common::{TestWorld, WorldSpec, pos, uid};
use tak_commands::{QueueMode, Scope, SimCommand, SpellId, Tick, UnitDefId, UnitSet, WorldPos};
use tak_input::{
    Action, Axis, Binding, Button, EventKind, Gesture, Input, InputEvent, Key, Mapper, Mode,
    Output, PlacementCandidate, Profile, WorldView,
};

fn world() -> TestWorld {
    TestWorld::new(serde_json::from_str::<WorldSpec>(r#"{
        "local_player":0,"enemies":[1],"units":[
            {"index":1,"owner":0,"def":10,"x":100,"y":100,"builder":"mobile","build_options":[20],"spells":[90]},
            {"index":2,"owner":0,"def":11,"x":200,"y":100,"builder":"factory","build_options":[20]},
            {"index":3,"owner":1,"def":12,"x":400,"y":400},
            {"index":4,"owner":0,"def":13,"x":600,"y":100}
        ]}"#).unwrap())
}
fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&id| uid(id)).collect()
}
fn sims(out: Vec<Output>) -> Vec<SimCommand> {
    out.into_iter()
        .filter_map(|o| match o {
            Output::Sim { tick, command } => {
                assert_eq!(tick, Tick(7));
                Some(command)
            }
            _ => None,
        })
        .collect()
}
fn event(m: &mut Mapper, w: &impl WorldView, kind: EventKind) -> Vec<Output> {
    m.handle(&InputEvent::new(m.now() + 10, kind), w, Tick(7))
}
fn key(m: &mut Mapper, w: &impl WorldView, name: &str, down: bool) -> Vec<Output> {
    event(
        m,
        w,
        if down {
            EventKind::KeyDown {
                key: Key::new(name),
            }
        } else {
            EventKind::KeyUp {
                key: Key::new(name),
            }
        },
    )
}
fn button(m: &mut Mapper, w: &impl WorldView, b: Button, down: bool) -> Vec<Output> {
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
fn hold(m: &mut Mapper, w: &impl WorldView) -> Vec<Output> {
    m.handle(
        &InputEvent::new(m.now() + m.tuning().hold_ms + 1, EventKind::Frame),
        w,
        Tick(7),
    )
}
fn point(m: &mut Mapper, w: &impl WorldView, x: i32, y: i32) {
    assert!(event(m, w, EventKind::PointerMove { x, y }).is_empty());
}
fn profile(sticky: bool) -> Profile {
    let mut p = Profile::default_combined();
    p.tuning.reticle = false;
    p.tuning.wheel_confirm_on_press = sticky;
    p
}
fn mapper(p: Profile, w: &impl WorldView) -> Mapper {
    let mut m = Mapper::new(p);
    point(&mut m, w, 100, 100);
    m.perform(Action::Select, w, Tick(7));
    assert_eq!(m.selection(), &units(&[1]));
    point(&mut m, w, 800, 600);
    m
}
fn steer_up(m: &mut Mapper, w: &impl WorldView) {
    event(
        m,
        w,
        EventKind::Axis {
            axis: Axis::LeftY,
            value: 32767,
        },
    );
}
fn stop_key(m: &mut Mapper, w: &impl WorldView) -> Vec<SimCommand> {
    let out = sims(key(m, w, "s", true));
    assert!(key(m, w, "s", false).is_empty());
    out
}
fn bind(p: &mut Profile, input: Input, gesture: Gesture, action: Action) {
    p.bindings
        .retain(|b| !(b.input == input && b.gesture == gesture && b.modifiers().is_empty()));
    p.bindings.push(Binding {
        input,
        with: None,
        with_all: vec![],
        gesture,
        action,
    });
}
fn modes() -> [(Action, Mode); 8] {
    [
        (Action::AttackMode, Mode::Attack),
        (Action::AttackMoveMode, Mode::AttackMove),
        (Action::MoveMode, Mode::Move),
        (Action::PatrolMode, Mode::Patrol),
        (Action::GuardMode, Mode::Guard),
        (Action::HealMode, Mode::Heal),
        (Action::BuildOption { index: 0 }, Mode::Build(UnitDefId(20))),
        (Action::CastSpellMode, Mode::CastSpell(SpellId(90))),
    ]
}

#[test]
fn s_held_b_and_wheel_stop_disarm_every_mode_with_or_without_queue() {
    let w = world();
    for (action, mode) in modes() {
        for queue in [false, true] {
            for stop in 0..3 {
                let mut m = mapper(profile(false), &w);
                if queue {
                    key(&mut m, &w, "shift", true);
                }
                m.perform(action, &w, Tick(7));
                assert_eq!(m.mode(), mode);
                let out = match stop {
                    0 => stop_key(&mut m, &w),
                    1 => {
                        button(&mut m, &w, Button::B, true);
                        let out = sims(hold(&mut m, &w));
                        assert!(button(&mut m, &w, Button::B, false).is_empty());
                        out
                    }
                    _ => {
                        button(&mut m, &w, Button::Lt, true);
                        steer_up(&mut m, &w);
                        sims(button(&mut m, &w, Button::Lt, false))
                    }
                };
                assert_eq!(out, vec![SimCommand::Stop { units: units(&[1]) }]);
                assert_eq!(
                    m.mode(),
                    Mode::None,
                    "mode={mode:?}, queue={queue}, stop={stop}"
                );
                assert!(m.wheel().is_none());
                assert_eq!(m.selection(), &units(&[1]));
                assert_eq!(m.production_batch_count(), if queue { 5 } else { 1 });
            }
        }
    }
}

#[test]
fn stop_consumes_open_and_not_yet_open_order_wheel_gestures() {
    let w = world();
    for sticky in [false, true] {
        for b in [Button::Lt, Button::Y, Button::Lb] {
            for opened in [false, true] {
                let mut m = mapper(profile(sticky), &w);
                button(&mut m, &w, b, true);
                if opened {
                    hold(&mut m, &w);
                }
                steer_up(&mut m, &w);
                assert_eq!(
                    stop_key(&mut m, &w),
                    vec![SimCommand::Stop { units: units(&[1]) }]
                );
                assert!(
                    m.wheel().is_none(),
                    "{b:?}, opened={opened}, sticky={sticky}"
                );
                assert_eq!(m.mode(), Mode::None);
                assert!(hold(&mut m, &w).is_empty());
                assert!(button(&mut m, &w, b, false).is_empty());
                assert!(m.wheel().is_none());
                assert_eq!(m.mode(), Mode::None);
                assert_eq!(m.selection(), &units(&[1]));
                // The next deliberate gesture is not stuck or suppressed.
                button(&mut m, &w, b, true);
                hold(&mut m, &w);
                assert!(m.wheel().is_some(), "fresh {b:?}");
            }
        }
    }
}

#[test]
fn stopped_target_clicks_do_not_fall_through_to_selection_or_context() {
    let w = world();
    for original in [false, true] {
        for (action, mode) in modes() {
            for queue in [false, true] {
                let mut p = profile(false);
                p.left_click_context = original;
                let mut m = mapper(p, &w);
                if queue {
                    key(&mut m, &w, "shift", true);
                }
                m.perform(action, &w, Tick(7));
                button(&mut m, &w, Button::MouseLeft, true);
                stop_key(&mut m, &w);
                assert!(
                    button(&mut m, &w, Button::MouseLeft, false).is_empty(),
                    "{mode:?}, original={original}, queue={queue}"
                );
                assert!(event(&mut m, &w, EventKind::Frame).is_empty());
                assert_eq!(m.selection(), &units(&[1]));
                assert_eq!(m.mode(), Mode::None);
                m.perform(Action::MoveMode, &w, Tick(7));
                button(&mut m, &w, Button::MouseLeft, true);
                assert!(matches!(
                    sims(button(&mut m, &w, Button::MouseLeft, false)).as_slice(),
                    [SimCommand::Move { .. }]
                ));
            }
        }
    }
}

#[test]
fn wheel_selected_stop_suppresses_custom_opener_release_and_tap() {
    let w = world();
    for sticky in [false, true] {
        for gesture in [Gesture::Release, Gesture::Tap] {
            let mut p = profile(sticky);
            p.command_wheel = vec![Action::Stop];
            bind(
                &mut p,
                Input::Button(Button::Lt),
                gesture,
                Action::AttackMode,
            );
            let mut m = mapper(p, &w);
            button(&mut m, &w, Button::Lt, true);
            steer_up(&mut m, &w);
            let out = if sticky {
                button(&mut m, &w, Button::A, true);
                let out = sims(button(&mut m, &w, Button::A, false));
                assert!(button(&mut m, &w, Button::Lt, false).is_empty());
                out
            } else {
                sims(button(&mut m, &w, Button::Lt, false))
            };
            assert_eq!(out, vec![SimCommand::Stop { units: units(&[1]) }]);
            assert_eq!(m.mode(), Mode::None, "sticky={sticky}, gesture={gesture:?}");
            assert!(m.wheel().is_none());
            assert_eq!(m.selection(), &units(&[1]));
        }
    }
}

#[test]
fn stop_bound_to_release_or_double_tap_owns_the_rest_of_its_gesture() {
    let w = world();
    for gesture in [Gesture::Release, Gesture::DoubleTap, Gesture::SecondTap] {
        let mut p = profile(false);
        let input = Input::Key(Key::new("f12"));
        bind(&mut p, input.clone(), Gesture::Tap, Action::AttackMode);
        bind(&mut p, input.clone(), Gesture::Release, Action::MoveMode);
        bind(&mut p, input, gesture, Action::Stop);
        let mut m = mapper(p, &w);
        key(&mut m, &w, "f12", true);
        if gesture == Gesture::Release {
            assert_eq!(
                sims(key(&mut m, &w, "f12", false)),
                vec![SimCommand::Stop { units: units(&[1]) }]
            );
        } else {
            key(&mut m, &w, "f12", false);
            assert_eq!(
                sims(key(&mut m, &w, "f12", true)),
                vec![SimCommand::Stop { units: units(&[1]) }]
            );
            assert!(key(&mut m, &w, "f12", false).is_empty());
        }
        assert_eq!(m.mode(), Mode::None, "{gesture:?}");
        assert!(hold(&mut m, &w).is_empty());
        assert_eq!(m.mode(), Mode::None);
    }
}

#[test]
fn stop_preserves_unrelated_selection_drag_and_selection_only_wheel() {
    let w = world();
    let mut m = mapper(profile(false), &w);
    point(&mut m, &w, 600, 100);
    button(&mut m, &w, Button::MouseLeft, true);
    stop_key(&mut m, &w);
    button(&mut m, &w, Button::MouseLeft, false);
    assert_eq!(m.selection(), &units(&[4]));

    let mut p = profile(false);
    p.command_wheel = vec![Action::SelectAll { scope: Scope::Map }];
    let mut m = mapper(p, &w);
    button(&mut m, &w, Button::Lt, true);
    steer_up(&mut m, &w);
    stop_key(&mut m, &w);
    assert!(m.wheel().is_some());
    button(&mut m, &w, Button::Lt, false);
    assert_eq!(m.selection(), &units(&[1, 2, 4]));
}

struct PlacementWorld(TestWorld);
impl WorldView for PlacementWorld {
    fn local_player(&self) -> tak_commands::PlayerId {
        self.0.local_player()
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&tak_input::UnitInfo)) {
        self.0.for_each_unit(f);
    }
    fn is_enemy(&self, player: tak_commands::PlayerId) -> bool {
        self.0.is_enemy(player)
    }
    fn build_options(&self, id: tak_commands::UnitId) -> Vec<UnitDefId> {
        self.0.build_options(id)
    }
    fn placement_candidate(&self, _: UnitDefId, point: WorldPos) -> Option<PlacementCandidate> {
        Some(PlacementCandidate {
            center: point,
            min: point - pos(5, 5),
            max: point + pos(5, 5),
        })
    }
}

#[test]
fn stop_cancels_pending_build_and_attack_box_releases_before_frame() {
    let w = PlacementWorld(world());
    for attack in [false, true] {
        for stop_first in [false, true] {
            let mut m = mapper(profile(false), &w);
            key(&mut m, &w, "shift", true);
            m.perform(
                if attack {
                    Action::AttackMode
                } else {
                    Action::BuildOption { index: 0 }
                },
                &w,
                Tick(7),
            );
            point(&mut m, &w, 300, 300);
            button(&mut m, &w, Button::MouseLeft, true);
            point(&mut m, &w, 500, 500);
            let mut out = vec![];
            if stop_first {
                out.extend(stop_key(&mut m, &w));
            }
            out.extend(sims(button(&mut m, &w, Button::MouseLeft, false)));
            if !stop_first {
                out.extend(stop_key(&mut m, &w));
            }
            out.extend(sims(event(&mut m, &w, EventKind::Frame)));
            assert_eq!(out, vec![SimCommand::Stop { units: units(&[1]) }]);
            assert_eq!(m.selection(), &units(&[1]));
            assert_eq!(m.mode(), Mode::None);
        }
    }
}

#[test]
fn stop_consumes_rebound_held_context_but_keeps_queue_cleanup_and_new_orders() {
    let w = world();
    let mut p = profile(false);
    bind(
        &mut p,
        Input::Key(Key::new("f12")),
        Gesture::Release,
        Action::Context { append: false },
    );
    let mut m = mapper(p, &w);
    key(&mut m, &w, "shift", true);
    key(&mut m, &w, "f12", true);
    for _ in 0..3 {
        stop_key(&mut m, &w);
    }
    assert!(key(&mut m, &w, "f12", false).is_empty());
    assert_eq!(m.production_batch_count(), 5);
    key(&mut m, &w, "shift", false);
    assert_eq!(m.production_batch_count(), 1);
    key(&mut m, &w, "f12", true);
    assert_eq!(
        sims(key(&mut m, &w, "f12", false)),
        vec![SimCommand::Move {
            units: units(&[1]),
            to: pos(800, 600),
            queue: QueueMode::Replace,
        }]
    );
}

#[test]
fn original_context_click_is_disarmed_but_selection_click_and_drag_survive() {
    let w = world();
    for (x, y) in [(800, 600), (400, 400)] {
        let mut p = profile(false);
        p.left_click_context = true;
        let mut m = mapper(p, &w);
        point(&mut m, &w, x, y);
        button(&mut m, &w, Button::MouseLeft, true);
        stop_key(&mut m, &w);
        assert!(button(&mut m, &w, Button::MouseLeft, false).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
        button(&mut m, &w, Button::MouseLeft, true);
        assert_eq!(sims(button(&mut m, &w, Button::MouseLeft, false)).len(), 1);
    }
    for drag in [false, true] {
        let mut p = profile(false);
        p.left_click_context = true;
        let mut m = mapper(p, &w);
        point(&mut m, &w, if drag { 570 } else { 600 }, 100);
        button(&mut m, &w, Button::MouseLeft, true);
        if drag {
            point(&mut m, &w, 630, 100);
        }
        stop_key(&mut m, &w);
        assert!(sims(button(&mut m, &w, Button::MouseLeft, false)).is_empty());
        assert_eq!(m.selection(), &units(&[4]));
    }
}

#[test]
fn already_started_selection_brush_is_not_mistaken_for_unfired_target_tap() {
    let w = world();
    for targeted in [false, true] {
        let mut m = mapper(profile(false), &w);
        if targeted {
            m.perform(Action::MoveMode, &w, Tick(7));
        }
        point(&mut m, &w, 600, 100);
        button(&mut m, &w, Button::A, true);
        hold(&mut m, &w);
        stop_key(&mut m, &w);
        let out = button(&mut m, &w, Button::A, false);
        assert!(out.iter().any(|o| matches!(
            o,
            Output::Client {
                command: tak_commands::ClientCommand::SelectBrush { .. }
            }
        )));
        assert!(sims(out).is_empty());
        assert_eq!(m.mode(), Mode::None);
    }
}

#[test]
fn hud_stop_discards_delayed_order_taps_but_preserves_delayed_selection() {
    let w = world();
    for order in [false, true] {
        let mut p = profile(false);
        let input = Input::Key(Key::new("f12"));
        bind(
            &mut p,
            input.clone(),
            Gesture::Tap,
            if order {
                Action::AttackMode
            } else {
                Action::Select
            },
        );
        bind(
            &mut p,
            input,
            Gesture::DoubleTap,
            if order {
                Action::MoveMode
            } else {
                Action::SelectAll { scope: Scope::Map }
            },
        );
        let mut m = mapper(p, &w);
        point(&mut m, &w, 600, 100);
        key(&mut m, &w, "f12", true);
        key(&mut m, &w, "f12", false);
        m.perform(Action::Stop, &w, Tick(7));
        let t = m.now() + m.tuning().double_tap_ms + 1;
        m.handle(&InputEvent::new(t, EventKind::Frame), &w, Tick(7));
        assert_eq!(m.mode(), Mode::None);
        assert_eq!(m.selection(), &units(if order { &[1] } else { &[4] }));
    }
}

#[test]
fn stop_consumes_only_resolved_custom_order_chord_and_preserves_physical_modifier() {
    let w = world();
    let mut p = profile(false);
    let input = Input::Key(Key::new("f12"));
    bind(
        &mut p,
        input.clone(),
        Gesture::Release,
        Action::SelectAll { scope: Scope::Map },
    );
    p.bindings.push(Binding {
        input,
        with: Some(Input::Key(Key::new("shift"))),
        with_all: vec![],
        gesture: Gesture::Release,
        action: Action::AttackMode,
    });
    for queue in [false, true] {
        let mut m = mapper(p.clone(), &w);
        if queue {
            key(&mut m, &w, "shift", true);
        }
        key(&mut m, &w, "f12", true);
        stop_key(&mut m, &w);
        assert!(sims(key(&mut m, &w, "f12", false)).is_empty());
        assert_eq!(m.mode(), Mode::None);
        assert_eq!(m.selection(), &units(if queue { &[1] } else { &[1, 2, 4] }));
        if queue {
            assert_eq!(m.production_batch_count(), 5);
            key(&mut m, &w, "shift", false);
            assert_eq!(m.production_batch_count(), 1);
        }
    }
}

#[test]
fn triggering_stop_owns_unrelated_custom_release_even_without_a_pending_mode() {
    let w = world();
    for stop_gesture in [Gesture::Press, Gesture::Hold] {
        for later in [Gesture::Release, Gesture::Tap] {
            let mut p = profile(false);
            let input = Input::Key(Key::new("f12"));
            bind(&mut p, input.clone(), stop_gesture, Action::Stop);
            bind(&mut p, input, later, Action::Cancel);
            let mut m = mapper(p, &w);
            let mut out = sims(key(&mut m, &w, "f12", true));
            if stop_gesture == Gesture::Hold {
                out.extend(sims(hold(&mut m, &w)));
            }
            assert_eq!(out, vec![SimCommand::Stop { units: units(&[1]) }]);
            assert!(key(&mut m, &w, "f12", false).is_empty());
            assert_eq!(m.selection(), &units(&[1]));
            assert_eq!(m.mode(), Mode::None);
        }
    }
}

#[test]
fn sticky_wheel_stop_retires_its_detached_opener_even_for_selection_release_binding() {
    let w = world();
    let mut p = profile(true);
    p.command_wheel = vec![Action::Stop];
    bind(
        &mut p,
        Input::Button(Button::Lt),
        Gesture::Release,
        Action::SelectAll { scope: Scope::Map },
    );
    let mut m = mapper(p, &w);
    button(&mut m, &w, Button::Lt, true);
    steer_up(&mut m, &w);
    button(&mut m, &w, Button::A, true);
    assert_eq!(
        sims(button(&mut m, &w, Button::A, false)),
        vec![SimCommand::Stop { units: units(&[1]) }]
    );
    assert!(button(&mut m, &w, Button::Lt, false).is_empty());
    assert_eq!(m.selection(), &units(&[1]));
}

#[test]
fn stopped_selection_click_never_becomes_an_order_after_target_or_pointer_changes() {
    for change in 0..4 {
        let mut w = world();
        let mut p = profile(false);
        p.left_click_context = true;
        let mut m = mapper(p, &w);
        // Own target pick radius is 12; this begins just inside its edge.
        point(&mut m, &w, 611, 100);
        button(&mut m, &w, Button::MouseLeft, true);
        if change == 3 {
            point(&mut m, &w, 640, 100);
        }
        assert_eq!(
            stop_key(&mut m, &w),
            vec![SimCommand::Stop { units: units(&[1]) }]
        );
        match change {
            0 | 3 => point(&mut m, &w, 614, 100),
            1 => w.spec.units.retain(|unit| unit.index != 4),
            2 => {
                w.spec
                    .units
                    .iter_mut()
                    .find(|unit| unit.index == 4)
                    .unwrap()
                    .owner = 1
            }
            _ => unreachable!(),
        }
        let out = button(&mut m, &w, Button::MouseLeft, false);
        assert!(out.is_empty(), "change={change}, stale output={out:?}");
        assert_eq!(m.selection(), &units(&[1]), "change={change}");
        assert_eq!(m.mode(), Mode::None);
    }
}

#[test]
fn fresh_target_mode_cannot_borrow_a_stopped_selection_gesture() {
    for repeated_stop in [false, true] {
        let w = world();
        let mut p = profile(false);
        p.left_click_context = true;
        let mut m = mapper(p, &w);
        point(&mut m, &w, 611, 100);
        button(&mut m, &w, Button::MouseLeft, true);
        stop_key(&mut m, &w);
        point(&mut m, &w, 614, 100);
        if repeated_stop {
            stop_key(&mut m, &w);
        }
        m.perform(Action::MoveMode, &w, Tick(7));
        assert!(button(&mut m, &w, Button::MouseLeft, false).is_empty());
        assert_eq!(m.mode(), Mode::Move);
        assert_eq!(m.selection(), &units(&[1]));
        button(&mut m, &w, Button::MouseLeft, true);
        assert_eq!(
            sims(button(&mut m, &w, Button::MouseLeft, false)),
            vec![SimCommand::Move {
                units: units(&[1]),
                to: pos(614, 100),
                queue: QueueMode::Replace,
            }]
        );
        assert_eq!(m.mode(), Mode::None);
    }
}

#[test]
fn old_controller_selection_cannot_borrow_new_target_mode_or_order_wheel() {
    let w = world();
    for wheel in [false, true] {
        let mut p = profile(true);
        p.command_wheel = vec![Action::AttackMode];
        let mut m = mapper(p, &w);
        button(&mut m, &w, Button::A, true);
        stop_key(&mut m, &w);
        if wheel {
            button(&mut m, &w, Button::Lt, true);
            steer_up(&mut m, &w);
        } else {
            m.perform(Action::AttackMode, &w, Tick(7));
        }
        assert!(button(&mut m, &w, Button::A, false).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), if wheel { Mode::None } else { Mode::Attack });
        assert_eq!(m.wheel().is_some(), wheel);
        button(&mut m, &w, Button::A, true);
        let out = sims(button(&mut m, &w, Button::A, false));
        if wheel {
            assert!(out.is_empty());
            assert_eq!(m.mode(), Mode::Attack);
        } else {
            assert!(matches!(out.as_slice(), [SimCommand::Attack { .. }]));
        }
    }
}

#[test]
fn old_delayed_selection_cannot_borrow_new_target_mode() {
    let w = world();
    let mut p = profile(false);
    let input = Input::Key(Key::new("f12"));
    bind(&mut p, input.clone(), Gesture::Tap, Action::Select);
    bind(
        &mut p,
        input,
        Gesture::DoubleTap,
        Action::SelectAll { scope: Scope::Map },
    );
    let mut m = mapper(p, &w);
    key(&mut m, &w, "f12", true);
    key(&mut m, &w, "f12", false);
    m.perform(Action::Stop, &w, Tick(7));
    m.perform(Action::AttackMode, &w, Tick(7));
    let t = m.now() + m.tuning().double_tap_ms + 1;
    assert!(
        m.handle(&InputEvent::new(t, EventKind::Frame), &w, Tick(7))
            .is_empty()
    );
    assert_eq!(m.mode(), Mode::Attack);
    assert_eq!(m.selection(), &units(&[1]));
    key(&mut m, &w, "f12", true);
    key(&mut m, &w, "f12", false);
    let t = m.now() + m.tuning().double_tap_ms + 1;
    assert!(matches!(
        sims(m.handle(&InputEvent::new(t, EventKind::Frame), &w, Tick(7))).as_slice(),
        [SimCommand::Attack { .. }]
    ));
}

#[test]
fn selection_tap_released_after_stop_keeps_its_old_origin_when_delayed() {
    let w = world();
    let mut p = profile(false);
    let input = Input::Key(Key::new("f12"));
    bind(&mut p, input.clone(), Gesture::Tap, Action::Select);
    bind(
        &mut p,
        input,
        Gesture::DoubleTap,
        Action::SelectAll { scope: Scope::Map },
    );
    let mut m = mapper(p, &w);
    key(&mut m, &w, "f12", true);
    m.perform(Action::Stop, &w, Tick(7));
    key(&mut m, &w, "f12", false);
    m.perform(Action::AttackMode, &w, Tick(7));
    let t = m.now() + m.tuning().double_tap_ms + 1;
    assert!(
        m.handle(&InputEvent::new(t, EventKind::Frame), &w, Tick(7))
            .is_empty()
    );
    assert_eq!(m.mode(), Mode::Attack);
    assert_eq!(m.selection(), &units(&[1]));
}

#[test]
fn preserved_selection_wheel_cannot_confirm_a_target_mode_armed_after_stop() {
    let w = world();
    let mut p = profile(true);
    p.command_wheel = vec![Action::Select];
    let mut m = mapper(p, &w);
    button(&mut m, &w, Button::Lt, true);
    steer_up(&mut m, &w);
    button(&mut m, &w, Button::Lt, false);
    stop_key(&mut m, &w);
    assert!(m.wheel().is_some());
    m.perform(Action::AttackMode, &w, Tick(7));
    button(&mut m, &w, Button::A, true);
    assert!(button(&mut m, &w, Button::A, false).is_empty());
    assert_eq!(m.mode(), Mode::Attack);
    assert_eq!(m.selection(), &units(&[1]));
    assert!(m.wheel().is_none());
    button(&mut m, &w, Button::A, true);
    assert!(matches!(
        sims(button(&mut m, &w, Button::A, false)).as_slice(),
        [SimCommand::Attack { .. }]
    ));
}

#[test]
fn preserved_active_brush_cannot_overwrite_a_fresh_mode() {
    let w = world();
    let mut m = mapper(profile(false), &w);
    button(&mut m, &w, Button::A, true);
    hold(&mut m, &w);
    stop_key(&mut m, &w);
    m.perform(Action::AttackMode, &w, Tick(7));
    point(&mut m, &w, 600, 100);
    assert!(event(&mut m, &w, EventKind::Frame).is_empty());
    assert!(button(&mut m, &w, Button::A, false).is_empty());
    assert_eq!(m.mode(), Mode::Attack);
    assert_eq!(m.selection(), &units(&[1]));
}

#[test]
fn old_custom_wheel_hold_cannot_turn_idle_selection_menu_into_orders() {
    let w = world();
    let mut p = profile(false);
    bind(
        &mut p,
        Input::Key(Key::new("f12")),
        Gesture::Hold,
        Action::CommandWheel,
    );
    let mut m = Mapper::new(p);
    key(&mut m, &w, "f12", true);
    m.perform(Action::Stop, &w, Tick(7));
    point(&mut m, &w, 100, 100);
    m.perform(Action::Select, &w, Tick(7));
    assert_eq!(m.selection(), &units(&[1]));
    assert!(hold(&mut m, &w).is_empty());
    assert!(m.wheel().is_none());
    key(&mut m, &w, "f12", false);
    key(&mut m, &w, "f12", true);
    hold(&mut m, &w);
    assert!(m.wheel().is_some());
}
