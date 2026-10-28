//! Asset-free Stop cancellation regressions through the real Bevy input schedule.
//! All selections, modes, wheels and orders below originate from Bevy input.
use super::*;
use bevy::input::gamepad::GamepadInput;
use tak_commands::{QueueMode, SimCommand, SpellId, Target, UnitDefId, UnitSet};
use tak_input::{Gesture, Ring, WheelKind};

struct Fixture {
    app: App,
    window: Entity,
    pad: Entity,
    builder: UnitId,
    enemy: UnitId,
}

fn profiles() -> [Profile; 2] {
    [
        Profile::default_combined(),
        Profile::named("original").unwrap(),
    ]
}

fn fixture(profile: Profile) -> Fixture {
    let mut app = super::overlay_input_tests::app();
    // The hand-authored training builder can build, heal and cast; no retail data.
    let mut scenario = tak_headless::bot::demo::scenario(false);
    scenario.unit_defs[0].economy.maxmana = 100;
    scenario.unit_defs[0].build.canreclaim = true;
    scenario.economy.rules.monarch_start_mana_percent = 100;
    scenario.unit_defs[0]
        .combat
        .weapons
        .push(tak_sim::combat::WeaponStats {
            name: "synthetic_stop_spell".into(),
            kind: tak_sim::combat::WeaponKind::LineOfSight,
            range: 80,
            reload_ticks: 8,
            mana_per_shot: 1,
            ..default()
        });
    let mut sim = tak_sim::Sim::new(&scenario).unwrap();
    sim.step(&[]);
    let map = app.world().non_send::<SimRes>().session.map.clone();
    let session = Session::from_sim(&Vfs::new(), "synthetic", map, sim, default(), None).unwrap();
    let snap = session.snapshot();
    {
        let mut state = app.world_mut().non_send_mut::<SimRes>();
        state.session = session;
        state.prev = snap.clone();
        state.curr = snap;
        let world = state.session.sim.world_mut();
        let ids: Vec<_> = world.units().map(|(id, _)| id).collect();
        for id in ids {
            world.despawn(id).unwrap();
        }
    }
    let builder = spawn(&mut app, PlayerId(0), UnitDefId(0), (320, 160));
    tak_sim::economy::set_unit_mana_percent(
        app.world_mut()
            .non_send_mut::<SimRes>()
            .session
            .sim
            .world_mut(),
        builder,
        100,
    );
    spawn(&mut app, PlayerId(0), UnitDefId(3), (280, 160));
    let enemy = spawn(&mut app, PlayerId(1), UnitDefId(4), (420, 160));
    app.world_mut().resource_mut::<Feel>().overlay_open = false;
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(profile);
    app.world_mut().resource_mut::<CamState>().geom = ViewGeom {
        target: [320.0, 160.0],
        zoom: 0.5,
        size: [800.0, 600.0],
    };
    let window = app
        .world_mut()
        .spawn((
            Window {
                resolution: WindowResolution::new(800, 600),
                ..default()
            },
            PrimaryWindow,
        ))
        .id();
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    frame(&mut app, pad);
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .warp_cursor(tak_math::Vec2::from_ints(400, 300));
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(selection(&app), UnitSet::single(builder));
    assert!(commands(&app).is_empty());
    Fixture {
        app,
        window,
        pad,
        builder,
        enemy,
    }
}

fn spawn(app: &mut App, owner: PlayerId, def: UnitDefId, pos: (i32, i32)) -> UnitId {
    app.world_mut()
        .non_send_mut::<SimRes>()
        .session
        .sim
        .world_mut()
        .spawn(
            def,
            owner,
            tak_math::Vec2::from_ints(pos.0, pos.1),
            tak_math::Angle::ZERO,
        )
        .unwrap()
}

fn frame(app: &mut App, pad: Entity) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(20));
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .clear();
}

fn wait_hold(app: &mut App, pad: Entity) {
    let hold = app.world().resource::<InputRes>().mapper.tuning().hold_ms;
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(u64::from(hold) + 1));
    frame(app, pad);
}

fn button(app: &mut App, pad: Entity, button: GamepadButton, down: bool) {
    let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
    if down {
        gamepad.digital_mut().press(button);
    } else {
        gamepad.digital_mut().release(button);
    }
    frame(app, pad);
}

fn tap(app: &mut App, pad: Entity, button_id: GamepadButton) {
    button(app, pad, button_id, true);
    button(app, pad, button_id, false);
}

fn key(app: &mut App, pad: Entity, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
    frame(app, pad);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(key);
    frame(app, pad);
}

fn aim(app: &mut App, pos: (i32, i32)) {
    app.world_mut().resource_mut::<CamState>().geom.target = [pos.0 as f32, pos.1 as f32];
}

fn pointer(app: &mut App, window: Entity, pos: (i32, i32)) {
    let p = app
        .world()
        .resource::<CamState>()
        .geom
        .to_screen([pos.0 as f32, 0.0, pos.1 as f32]);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(p[0], p[1])));
}

fn selection(app: &App) -> UnitSet {
    app.world()
        .resource::<InputRes>()
        .mapper
        .selection()
        .clone()
}

fn commands(app: &App) -> Vec<SimCommand> {
    app.world()
        .non_send::<SimRes>()
        .session
        .recorded
        .entries()
        .iter()
        .map(|e| e.command.clone())
        .collect()
}

fn mode(app: &App) -> Mode {
    app.world().resource::<InputRes>().mapper.mode()
}

fn choose(app: &mut App, pad: Entity, action: Action) {
    let wheel = app.world().resource::<InputRes>().mapper.wheel().unwrap();
    let index = wheel.slots.iter().position(|slot| *slot == action).unwrap();
    let theta = std::f32::consts::TAU * index as f32 / wheel.slots.len() as f32;
    for (x, y) in [(theta.sin(), theta.cos()), (0.0, 0.0)] {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad
            .analog_mut()
            .set(GamepadInput::Axis(GamepadAxis::LeftStickX), x);
        gamepad
            .analog_mut()
            .set(GamepadInput::Axis(GamepadAxis::LeftStickY), y);
        frame(app, pad);
    }
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .wheel()
            .unwrap()
            .highlighted,
        Some(index)
    );
}

fn arm(app: &mut App, pad: Entity, wanted: Mode) {
    match wanted {
        Mode::Attack => key(app, pad, KeyCode::KeyA),
        Mode::AttackMove => {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::AltLeft);
            key(app, pad, KeyCode::KeyA);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::AltLeft);
            frame(app, pad);
        }
        Mode::Move => key(app, pad, KeyCode::KeyM),
        Mode::Patrol => key(app, pad, KeyCode::KeyP),
        Mode::Guard => key(app, pad, KeyCode::KeyG),
        Mode::Heal => key(app, pad, KeyCode::KeyH),
        Mode::Clear => key(app, pad, KeyCode::KeyC),
        Mode::CastSpell(_) => tap(app, pad, GamepadButton::North),
        Mode::Build(_) => {
            button(app, pad, GamepadButton::LeftTrigger, true);
            wait_hold(app, pad);
            choose(app, pad, Action::BuildOption { index: 0 });
            if app
                .world()
                .resource::<InputRes>()
                .mapper
                .tuning()
                .wheel_confirm_on_press
            {
                tap(app, pad, GamepadButton::South);
            }
            button(app, pad, GamepadButton::LeftTrigger, false);
        }
        _ => panic!("unsupported test mode {wanted:?}"),
    }
    assert_eq!(mode(app), wanted);
}

fn modes() -> [Mode; 9] {
    [
        Mode::Attack,
        Mode::AttackMove,
        Mode::Move,
        Mode::Patrol,
        Mode::Guard,
        Mode::Heal,
        Mode::Clear,
        Mode::Build(UnitDefId(1)),
        Mode::CastSpell(SpellId(0)),
    ]
}

fn assert_stopped(app: &App, builder: UnitId) {
    assert_eq!(mode(app), Mode::None, "Stop left a target mode armed");
    assert!(
        app.world().resource::<InputRes>().mapper.wheel().is_none(),
        "Stop left an order wheel armed"
    );
    assert_eq!(
        selection(app),
        UnitSet::single(builder),
        "Stop must not deselect"
    );
    assert_eq!(
        commands(app),
        vec![SimCommand::Stop {
            units: UnitSet::single(builder)
        }]
    );
}

fn recover_attack(app: &mut App, pad: Entity, builder: UnitId, enemy: UnitId, queue: QueueMode) {
    let before = commands(app);
    aim(app, (420, 160));
    // A real mouse movement recenters the reticle after mixed pointer/pad use.
    // Camera movement alone intentionally does not erase the last mouse position.
    let mut windows = app
        .world_mut()
        .query_filtered::<&mut Window, With<PrimaryWindow>>();
    for mut window in windows.iter_mut(app.world_mut()) {
        window.set_cursor_position(Some(Vec2::new(400.0, 300.0)));
    }
    frame(app, pad);
    key(app, pad, KeyCode::KeyA);
    tap(app, pad, GamepadButton::West);
    let mut expected = before;
    expected.push(SimCommand::Attack {
        units: UnitSet::single(builder),
        target: Target::Unit(enemy),
        queue,
    });
    assert_eq!(
        commands(app),
        expected,
        "fresh gesture after Stop did not recover exactly once"
    );
    assert_eq!(selection(app), UnitSet::single(builder));
}

#[test]
fn keyboard_stop_disarms_every_pending_mode_without_losing_selection() {
    for profile in profiles() {
        for wanted in modes() {
            let Fixture {
                mut app,
                pad,
                builder,
                enemy,
                ..
            } = fixture(profile.clone());
            arm(&mut app, pad, wanted);
            key(&mut app, pad, KeyCode::KeyS);
            assert_stopped(&app, builder);
            wait_hold(&mut app, pad);
            assert_stopped(&app, builder);
            recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
        }
    }
}

#[test]
fn held_b_stop_disarms_every_mode_and_its_release_does_not_cancel_selection() {
    for wanted in modes() {
        let Fixture {
            mut app,
            pad,
            builder,
            enemy,
            ..
        } = fixture(Profile::default_combined());
        arm(&mut app, pad, wanted);
        button(&mut app, pad, GamepadButton::East, true);
        wait_hold(&mut app, pad);
        assert_stopped(&app, builder);
        wait_hold(&mut app, pad);
        button(&mut app, pad, GamepadButton::East, false);
        assert_stopped(&app, builder);
        recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
    }
}

#[test]
fn wheel_stop_disarms_prior_mode_and_consumes_held_opener_release() {
    for sticky in [false, true] {
        for release_binding in [false, true] {
            let mut profile = Profile::default_combined();
            profile.tuning.wheel_confirm_on_press = sticky;
            // A valid saved remapping can attach an explicit release action.
            if release_binding {
                profile.bindings.push(tak_input::Binding {
                    input: tak_input::Input::Button(InButton::Lt),
                    with: None,
                    with_all: vec![],
                    gesture: Gesture::Release,
                    action: Action::AttackMode,
                });
            }
            let Fixture {
                mut app,
                pad,
                builder,
                enemy,
                ..
            } = fixture(profile);
            arm(&mut app, pad, Mode::Heal);
            button(&mut app, pad, GamepadButton::LeftTrigger2, true);
            choose(&mut app, pad, Action::Stop);
            if sticky {
                tap(&mut app, pad, GamepadButton::South);
                assert_stopped(&app, builder);
                wait_hold(&mut app, pad);
            }
            button(&mut app, pad, GamepadButton::LeftTrigger2, false);
            assert_stopped(&app, builder);
            recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
        }
    }
}

#[test]
fn keyboard_stop_closes_command_build_spell_and_unit_wheels_before_old_release() {
    for kind in [
        WheelKind::Command,
        WheelKind::Build,
        WheelKind::Spell,
        WheelKind::Unit,
    ] {
        for same_poll_release in [false, true] {
            let Fixture {
                mut app,
                pad,
                builder,
                enemy,
                ..
            } = fixture(Profile::default_combined());
            let opener = match kind {
                WheelKind::Build => GamepadButton::LeftTrigger,
                WheelKind::Spell => GamepadButton::North,
                _ => GamepadButton::LeftTrigger2,
            };
            button(&mut app, pad, opener, true);
            if matches!(kind, WheelKind::Build | WheelKind::Spell) {
                wait_hold(&mut app, pad);
            }
            if kind == WheelKind::Unit {
                choose(&mut app, pad, Action::OpenRing { ring: Ring::Unit });
                button(&mut app, pad, opener, false);
            }
            choose(
                &mut app,
                pad,
                match kind {
                    WheelKind::Build => Action::BuildOption { index: 0 },
                    WheelKind::Spell => Action::CastSpell { index: 0 },
                    WheelKind::Unit => Action::HealMode,
                    _ => Action::AttackMode,
                },
            );
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyS);
            if same_poll_release {
                app.world_mut()
                    .get_mut::<Gamepad>(pad)
                    .unwrap()
                    .digital_mut()
                    .release(opener);
            }
            frame(&mut app, pad);
            assert_stopped(&app, builder);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::KeyS);
            frame(&mut app, pad);
            button(&mut app, pad, opener, false);
            wait_hold(&mut app, pad);
            assert_stopped(&app, builder);
            recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
        }
    }
}

#[test]
fn keyboard_stop_retires_unfinished_pad_holds_and_taps_not_just_visible_wheels() {
    for opener in [
        GamepadButton::LeftTrigger,
        GamepadButton::North,
        GamepadButton::West,
    ] {
        let Fixture {
            mut app,
            pad,
            builder,
            enemy,
            ..
        } = fixture(Profile::default_combined());
        aim(&mut app, (420, 160));
        button(&mut app, pad, opener, true);
        key(&mut app, pad, KeyCode::KeyS);
        wait_hold(&mut app, pad);
        assert_stopped(&app, builder);
        button(&mut app, pad, opener, false);
        wait_hold(&mut app, pad);
        assert_stopped(&app, builder);
        recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
    }
}

#[test]
fn stop_consumes_mixed_queued_mouse_attack_and_pad_context_but_preserves_queue_modifier() {
    for modifier in [KeyCode::ShiftLeft, KeyCode::ShiftRight] {
        let Fixture {
            mut app,
            window,
            pad,
            builder,
            enemy,
        } = fixture(Profile::default_combined());
        arm(&mut app, pad, Mode::Attack);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(modifier);
        pointer(&mut app, window, (380, 120));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        frame(&mut app, pad);
        pointer(&mut app, window, (460, 200));
        button(&mut app, pad, GamepadButton::West, true);
        key(&mut app, pad, KeyCode::KeyS);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        button(&mut app, pad, GamepadButton::West, false);
        assert_stopped(&app, builder);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(None);
        recover_attack(&mut app, pad, builder, enemy, QueueMode::Append);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(modifier);
        frame(&mut app, pad);
        recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
    }
}

#[test]
fn stopped_wheel_stays_disarmed_across_modal_and_focus_boundaries() {
    for boundary in ["overlay", "focus"] {
        let Fixture {
            mut app,
            window,
            pad,
            builder,
            enemy,
        } = fixture(Profile::default_combined());
        button(&mut app, pad, GamepadButton::LeftTrigger2, true);
        choose(&mut app, pad, Action::AttackMode);
        key(&mut app, pad, KeyCode::KeyS);
        assert_stopped(&app, builder);
        if boundary == "overlay" {
            key(&mut app, pad, KeyCode::F11);
        } else {
            app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
            frame(&mut app, pad);
        }
        button(&mut app, pad, GamepadButton::LeftTrigger2, false);
        if boundary == "overlay" {
            key(&mut app, pad, KeyCode::Escape);
        } else {
            app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
            frame(&mut app, pad);
        }
        assert_stopped(&app, builder);
        recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
    }
}

#[test]
fn rapid_repeated_stop_has_one_order_per_press_and_allows_fresh_wheel() {
    let Fixture {
        mut app,
        pad,
        builder,
        ..
    } = fixture(Profile::default_combined());
    for count in 1..=3 {
        arm(&mut app, pad, Mode::Heal);
        key(&mut app, pad, KeyCode::KeyS);
        assert_eq!(mode(&app), Mode::None);
        assert_eq!(selection(&app), UnitSet::single(builder));
        assert_eq!(
            commands(&app),
            vec![
                SimCommand::Stop {
                    units: UnitSet::single(builder)
                };
                count
            ]
        );
    }
    button(&mut app, pad, GamepadButton::LeftTrigger2, true);
    choose(&mut app, pad, Action::AttackMode);
    button(&mut app, pad, GamepadButton::LeftTrigger2, false);
    assert_eq!(mode(&app), Mode::Attack);
    assert_eq!(commands(&app).len(), 3);
}

#[test]
fn keyboard_stop_consumes_an_older_b_hold_or_tap_without_deselection_or_second_stop() {
    for long_hold in [false, true] {
        let Fixture {
            mut app,
            pad,
            builder,
            enemy,
            ..
        } = fixture(Profile::default_combined());
        button(&mut app, pad, GamepadButton::East, true);
        key(&mut app, pad, KeyCode::KeyS);
        if long_hold {
            wait_hold(&mut app, pad);
        }
        button(&mut app, pad, GamepadButton::East, false);
        assert_stopped(&app, builder);
        recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
    }
}

#[test]
fn keyboard_stop_consumes_short_mouse_target_release_in_nonattack_modes() {
    for profile in profiles() {
        for wanted in [Mode::Move, Mode::Heal, Mode::Build(UnitDefId(1))] {
            let Fixture {
                mut app,
                window,
                pad,
                builder,
                enemy,
            } = fixture(profile.clone());
            arm(&mut app, pad, wanted);
            pointer(&mut app, window, (440, 240));
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            frame(&mut app, pad);
            key(&mut app, pad, KeyCode::KeyS);
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(MouseButton::Left);
            frame(&mut app, pad);
            assert_stopped(&app, builder);
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(None);
            recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
        }
    }
}

#[test]
fn stop_preserves_selection_only_mouse_drag_and_idle_wheel_gestures() {
    let Fixture {
        mut app,
        window,
        pad,
        builder,
        ..
    } = fixture(Profile::default_combined());
    pointer(&mut app, window, (250, 130));
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    frame(&mut app, pad);
    pointer(&mut app, window, (300, 190));
    frame(&mut app, pad);
    key(&mut app, pad, KeyCode::KeyS);
    assert_stopped(&app, builder);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    frame(&mut app, pad);
    assert_eq!(selection(&app).len(), 1);
    assert!(
        !selection(&app).contains(builder),
        "ordinary selection drag was swallowed by Stop"
    );
    assert_eq!(
        commands(&app),
        vec![SimCommand::Stop {
            units: UnitSet::single(builder)
        }]
    );

    let Fixture {
        mut app,
        pad,
        builder,
        ..
    } = fixture(Profile::default_combined());
    key(&mut app, pad, KeyCode::Escape);
    assert!(selection(&app).is_empty());
    button(&mut app, pad, GamepadButton::LeftTrigger2, true);
    choose(
        &mut app,
        pad,
        Action::SelectAll {
            scope: tak_commands::Scope::Map,
        },
    );
    let wheel = app.world().resource::<InputRes>().mapper.wheel().cloned();
    key(&mut app, pad, KeyCode::KeyS);
    assert_eq!(
        app.world().resource::<InputRes>().mapper.wheel(),
        wheel.as_ref()
    );
    button(&mut app, pad, GamepadButton::LeftTrigger2, false);
    assert_eq!(selection(&app).len(), 2);
    assert!(selection(&app).contains(builder));
    assert!(commands(&app).is_empty());
}

#[test]
fn stop_preserves_held_controller_queue_modifier_until_its_real_release() {
    let Fixture {
        mut app,
        pad,
        builder,
        enemy,
        ..
    } = fixture(Profile::default_combined());
    button(&mut app, pad, GamepadButton::RightTrigger, true);
    wait_hold(&mut app, pad);
    arm(&mut app, pad, Mode::Heal);
    key(&mut app, pad, KeyCode::KeyS);
    assert_stopped(&app, builder);
    recover_attack(&mut app, pad, builder, enemy, QueueMode::Append);
    button(&mut app, pad, GamepadButton::RightTrigger, false);
    recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
}

#[test]
fn stop_consumes_pending_primary_or_context_pad_target_release() {
    for target_button in [GamepadButton::South, GamepadButton::West] {
        for wanted in [Mode::Attack, Mode::Heal, Mode::Build(UnitDefId(1))] {
            let Fixture {
                mut app,
                pad,
                builder,
                enemy,
                ..
            } = fixture(Profile::default_combined());
            aim(&mut app, (420, 160));
            arm(&mut app, pad, wanted);
            button(&mut app, pad, target_button, true);
            key(&mut app, pad, KeyCode::KeyS);
            button(&mut app, pad, target_button, false);
            assert_stopped(&app, builder);
            recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
        }
    }
}

#[test]
fn original_stop_consumes_pending_implicit_primary_ground_or_enemy_order() {
    for target in [(440, 240), (420, 160)] {
        let Fixture {
            mut app,
            window,
            pad,
            builder,
            enemy,
        } = fixture(Profile::named("original").unwrap());
        assert_eq!(mode(&app), Mode::None);
        pointer(&mut app, window, target);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        frame(&mut app, pad);
        assert!(commands(&app).is_empty());
        key(&mut app, pad, KeyCode::KeyS);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        frame(&mut app, pad);
        assert_stopped(&app, builder);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(None);
        recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
    }
}

#[test]
fn original_stop_preserves_friendly_point_selection_and_started_selection_drag() {
    for drag in [false, true] {
        let Fixture {
            mut app,
            window,
            pad,
            builder,
            ..
        } = fixture(Profile::named("original").unwrap());
        pointer(&mut app, window, if drag { (250, 130) } else { (280, 160) });
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        frame(&mut app, pad);
        if drag {
            pointer(&mut app, window, (300, 190));
            frame(&mut app, pad);
        }
        key(&mut app, pad, KeyCode::KeyS);
        assert_stopped(&app, builder);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        frame(&mut app, pad);
        assert_eq!(selection(&app).len(), 1);
        assert!(
            !selection(&app).contains(builder),
            "original selection-only gesture was swallowed, drag={drag}"
        );
        assert_eq!(
            commands(&app),
            vec![SimCommand::Stop {
                units: UnitSet::single(builder)
            }]
        );
    }
}

#[test]
fn original_preserved_selection_click_cannot_turn_into_an_order_after_stop() {
    let mut stale_orders = Vec::new();
    for boundary in [
        "target_despawned",
        "target_became_enemy",
        "short_pointer_drift",
    ] {
        let Fixture {
            mut app,
            window,
            pad,
            builder,
            ..
        } = fixture(Profile::named("original").unwrap());
        let target = app
            .world()
            .non_send::<SimRes>()
            .session
            .sim
            .world()
            .units()
            .find(|(_, unit)| unit.def == UnitDefId(3))
            .map(|(id, _)| id)
            .unwrap();
        {
            let state = app.world().non_send::<SimRes>();
            let view = state.view();
            assert_eq!(
                tak_input::pick_unit(&view, tak_math::Vec2::from_ints(294, 160))
                    .unwrap()
                    .id,
                target
            );
            assert!(tak_input::pick_unit(&view, tak_math::Vec2::from_ints(295, 160)).is_none());
            assert!(
                app.world()
                    .resource::<InputRes>()
                    .mapper
                    .tuning()
                    .drag_min_px
                    > 2
            );
        }
        // The troop's synthetic pick radius is 14. Start on its inclusive edge,
        // without overlapping the selected builder's pick circle.
        pointer(&mut app, window, (294, 160));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        frame(&mut app, pad);
        key(&mut app, pad, KeyCode::KeyS);
        assert_stopped(&app, builder);
        match boundary {
            "target_despawned" => {
                app.world_mut()
                    .non_send_mut::<SimRes>()
                    .session
                    .sim
                    .world_mut()
                    .despawn(target)
                    .unwrap();
            }
            "target_became_enemy" => {
                app.world_mut()
                    .non_send_mut::<SimRes>()
                    .session
                    .sim
                    .world_mut()
                    .unit_mut(target)
                    .unwrap()
                    .owner = PlayerId(1);
            }
            // One world unit is two pixels: outside the picked unit,
            // but still below the ordinary four-pixel selection-drag threshold.
            "short_pointer_drift" => pointer(&mut app, window, (295, 160)),
            _ => unreachable!(),
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        frame(&mut app, pad);
        wait_hold(&mut app, pad);
        let expected = vec![SimCommand::Stop {
            units: UnitSet::single(builder),
        }];
        if commands(&app) != expected {
            stale_orders.push((boundary, commands(&app)));
        }
        assert_eq!(mode(&app), Mode::None);
        // The surviving gesture may select (or find nothing). It may never
        // reinterpret this already-stopped press as a fresh contextual order.
        frame(&mut app, pad);
    }
    assert!(
        stale_orders.is_empty(),
        "preserved selection converted to stale orders: {stale_orders:?}"
    );
}

#[test]
fn old_selection_pad_release_cannot_borrow_a_fresh_target_mode_or_sticky_wheel() {
    for wheel in [false, true] {
        let mut profile = Profile::default_combined();
        profile.tuning.wheel_confirm_on_press = true;
        let Fixture {
            mut app,
            pad,
            builder,
            enemy,
            ..
        } = fixture(profile);
        aim(&mut app, (420, 160));
        button(&mut app, pad, GamepadButton::South, true);
        key(&mut app, pad, KeyCode::KeyS);
        assert_stopped(&app, builder);
        if wheel {
            button(&mut app, pad, GamepadButton::LeftTrigger2, true);
            choose(&mut app, pad, Action::AttackMode);
        } else {
            key(&mut app, pad, KeyCode::KeyA);
        }
        let pending_wheel = app.world().resource::<InputRes>().mapper.wheel().cloned();
        button(&mut app, pad, GamepadButton::South, false);
        assert_eq!(
            commands(&app),
            vec![SimCommand::Stop {
                units: UnitSet::single(builder)
            }]
        );
        assert_eq!(selection(&app), UnitSet::single(builder));
        assert_eq!(mode(&app), if wheel { Mode::None } else { Mode::Attack });
        assert_eq!(
            app.world().resource::<InputRes>().mapper.wheel(),
            pending_wheel.as_ref()
        );
        if wheel {
            tap(&mut app, pad, GamepadButton::South);
            button(&mut app, pad, GamepadButton::LeftTrigger2, false);
            assert_eq!(mode(&app), Mode::Attack);
        }
        recover_attack(&mut app, pad, builder, enemy, QueueMode::Replace);
    }
}
