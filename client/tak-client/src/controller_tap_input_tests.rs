//! Asset-free tap-provenance regressions through the real Bevy input schedule.
use super::*;
use bevy::input::gamepad::GamepadInput;
use tak_commands::{QueueMode, SimCommand, Target, UnitDefId, UnitSet};
use tak_input::{Ring, WheelKind};

struct Fixture {
    app: App,
    window: Entity,
    pad: Entity,
    builder: UnitId,
    twin: UnitId,
    troop: UnitId,
}

fn fixture(profile: Profile) -> Fixture {
    let mut app = super::overlay_input_tests::app();
    app.world_mut().resource_mut::<Feel>().overlay_open = false;
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(profile);
    {
        let mut sim = app.world_mut().non_send_mut::<SimRes>();
        let world = sim.session.sim.world_mut();
        let units: Vec<_> = world.units().map(|(id, _)| id).collect();
        for id in units {
            world.despawn(id).unwrap();
        }
    }
    let builder = spawn(&mut app, PlayerId(0), UnitDefId(0), (320, 160));
    let twin = spawn(&mut app, PlayerId(0), UnitDefId(0), (360, 160));
    let troop = spawn(&mut app, PlayerId(0), UnitDefId(3), (280, 160));
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
    Fixture {
        app,
        window,
        pad,
        builder,
        twin,
        troop,
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

fn button(app: &mut App, pad: Entity, button: GamepadButton, down: bool) {
    let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
    if down {
        gamepad.digital_mut().press(button);
    } else {
        gamepad.digital_mut().release(button);
    }
    frame(app, pad);
}

fn tap(app: &mut App, pad: Entity, input: GamepadButton) {
    button(app, pad, input, true);
    button(app, pad, input, false);
}

fn aim(app: &mut App, target: (i32, i32)) {
    app.world_mut().resource_mut::<CamState>().geom.target = [target.0 as f32, target.1 as f32];
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
        .map(|entry| entry.command.clone())
        .collect()
}

fn record(app: &mut App) -> Mapper {
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .take_feedback();
    let initial = app.world().resource::<InputRes>().mapper.clone();
    let mut feel = app.world_mut().resource_mut::<Feel>();
    feel.feedback.clear();
    feel.recorder = Some(tak_input::Recorder::new(
        serde_json::json!({"synthetic": "selection tap provenance adapter"}),
        &initial,
    ));
    initial
}

fn assert_interrupted_replay(app: &App, initial: &Mapper, without_interruption: Option<UnitSet>) {
    let recorded = app
        .world()
        .resource::<Feel>()
        .recorder
        .as_ref()
        .unwrap()
        .recording();
    // The new adapter boundary must survive the actual JSON recording format.
    let json = serde_json::to_string(recorded).unwrap();
    let recorded: tak_input::Recording = serde_json::from_str(&json).unwrap();
    assert_eq!(
        recorded
            .frames
            .iter()
            .flat_map(|frame| &frame.events)
            .filter(|event| matches!(event, EventKind::SelectionTapInterrupted))
            .count(),
        1
    );
    let view = app.world().non_send::<SimRes>().view();
    for omit_interruption in [false, true] {
        if omit_interruption && without_interruption.is_none() {
            continue;
        }
        let mut replay = initial.clone();
        for frame in &recorded.frames {
            replay.set_camera(frame.camera);
            let mut outputs = Vec::new();
            for event in frame
                .events
                .iter()
                .chain(std::iter::once(&EventKind::Frame))
            {
                if omit_interruption && matches!(event, EventKind::SelectionTapInterrupted) {
                    continue;
                }
                outputs.extend(replay.handle(
                    &InputEvent::new(frame.t, event.clone()),
                    &view,
                    tak_commands::Tick(frame.tick),
                ));
            }
            if !omit_interruption {
                assert_eq!(
                    outputs, frame.outputs,
                    "raw adapter replay diverged at {}",
                    frame.t
                );
                assert_eq!(replay.take_feedback(), frame.feedback);
                if let Some(selection) = &frame.selection {
                    assert_eq!(replay.selection(), selection);
                }
            }
            replay.take_camera_pan();
            replay.take_camera_zoom();
        }
        assert_eq!(
            replay.selection(),
            &if omit_interruption {
                without_interruption.clone().unwrap()
            } else {
                selection(app)
            }
        );
    }
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

fn unit_ring(app: &mut App, pad: Entity) {
    button(app, pad, GamepadButton::LeftTrigger2, true);
    choose(app, pad, Action::OpenRing { ring: Ring::Unit });
    button(app, pad, GamepadButton::LeftTrigger2, false);
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .wheel()
            .unwrap()
            .kind,
        WheelKind::Unit
    );
}

fn mouse_tap(app: &mut App, window: Entity, pad: Entity, pos: (i32, i32)) {
    let point =
        app.world()
            .resource::<CamState>()
            .geom
            .to_screen([pos.0 as f32, 0.0, pos.1 as f32]);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(point[0], point[1])));
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    frame(app, pad);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    frame(app, pad);
}

#[test]
fn controller_same_target_double_a_widens_on_second_down_only() {
    for profile in [
        Profile::default_combined(),
        Profile::named("original").unwrap(),
    ] {
        let Fixture {
            mut app,
            pad,
            builder,
            twin,
            ..
        } = fixture(profile);
        tap(&mut app, pad, GamepadButton::South);
        assert_eq!(selection(&app), UnitSet::single(builder));
        button(&mut app, pad, GamepadButton::South, true);
        assert_eq!(selection(&app), [builder, twin].into_iter().collect());
        button(&mut app, pad, GamepadButton::South, false);
        assert_eq!(selection(&app), [builder, twin].into_iter().collect());
        assert!(commands(&app).is_empty());
    }
}

#[test]
fn controller_rapid_a_on_different_target_selects_that_unit_not_first_type() {
    for same_type in [false, true] {
        let Fixture {
            mut app,
            pad,
            builder,
            twin,
            troop,
            ..
        } = fixture(Profile::default_combined());
        tap(&mut app, pad, GamepadButton::South);
        assert_eq!(selection(&app), UnitSet::single(builder));
        aim(&mut app, if same_type { (360, 160) } else { (280, 160) });
        tap(&mut app, pad, GamepadButton::South);
        assert_eq!(
            selection(&app),
            UnitSet::single(if same_type { twin } else { troop })
        );
        assert!(commands(&app).is_empty());
    }
}

#[test]
fn controller_empty_and_enemy_taps_do_not_swallow_next_friendly_selection() {
    for enemy in [false, true] {
        let Fixture {
            mut app,
            pad,
            troop,
            ..
        } = fixture(Profile::default_combined());
        if enemy {
            spawn(&mut app, PlayerId(1), UnitDefId(4), (420, 160));
        }
        aim(&mut app, (420, 160));
        tap(&mut app, pad, GamepadButton::South);
        assert!(selection(&app).is_empty());
        aim(&mut app, (280, 160));
        tap(&mut app, pad, GamepadButton::South);
        assert_eq!(selection(&app), UnitSet::single(troop));
        assert!(commands(&app).is_empty());
    }
}

#[test]
fn controller_nested_sticky_rings_accept_consecutive_a_confirmations() {
    let Fixture {
        mut app,
        pad,
        builder,
        ..
    } = fixture(Profile::default_combined());
    tap(&mut app, pad, GamepadButton::South);
    unit_ring(&mut app, pad);
    choose(&mut app, pad, Action::OpenRing { ring: Ring::Squads });
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .wheel()
            .unwrap()
            .kind,
        WheelKind::Squads
    );
    assert_eq!(selection(&app), UnitSet::single(builder));
    choose(
        &mut app,
        pad,
        Action::RecallGroup {
            group: tak_commands::ControlGroup::new(5).unwrap(),
        },
    );
    tap(&mut app, pad, GamepadButton::South);
    assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
    assert!(
        selection(&app).is_empty(),
        "the empty squad should have been recalled"
    );
    assert!(commands(&app).is_empty());
}

#[test]
fn controller_sticky_heal_confirmation_does_not_swallow_next_a_target() {
    let Fixture {
        mut app,
        pad,
        builder,
        troop,
        ..
    } = fixture(Profile::default_combined());
    tap(&mut app, pad, GamepadButton::South);
    unit_ring(&mut app, pad);
    choose(&mut app, pad, Action::HealMode);
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::Heal);
    assert!(commands(&app).is_empty());
    aim(&mut app, (280, 160));
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(
        commands(&app),
        vec![SimCommand::Heal {
            units: UnitSet::single(builder),
            target: troop,
            queue: QueueMode::Replace,
        }]
    );
    assert_eq!(selection(&app), UnitSet::single(builder));
    assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
    frame(&mut app, pad);
    assert_eq!(commands(&app).len(), 1);
}

#[test]
fn controller_attack_wheel_confirmation_and_following_a_each_keep_their_role() {
    for sticky_tuning in [false, true] {
        let mut profile = Profile::default_combined();
        // This is the existing optional sticky-wheel tuning, not a remapping.
        profile.tuning.wheel_confirm_on_press = sticky_tuning;
        let Fixture {
            mut app,
            pad,
            builder,
            ..
        } = fixture(profile);
        let enemy = spawn(&mut app, PlayerId(1), UnitDefId(4), (420, 160));
        tap(&mut app, pad, GamepadButton::South);
        button(&mut app, pad, GamepadButton::LeftTrigger2, true);
        choose(&mut app, pad, Action::AttackMode);
        if sticky_tuning {
            tap(&mut app, pad, GamepadButton::South);
        }
        button(&mut app, pad, GamepadButton::LeftTrigger2, false);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
        assert_eq!(
            app.world().resource::<InputRes>().mapper.mode(),
            Mode::Attack
        );
        assert!(commands(&app).is_empty());
        aim(&mut app, (420, 160));
        tap(&mut app, pad, GamepadButton::South);
        assert_eq!(
            commands(&app),
            vec![SimCommand::Attack {
                units: UnitSet::single(builder),
                target: Target::Unit(enemy),
                queue: QueueMode::Replace,
            }]
        );
        assert_eq!(selection(&app), UnitSet::single(builder));
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
    }
}

#[test]
fn controller_build_wheel_confirmation_and_following_a_place_once() {
    for sticky_tuning in [false, true] {
        let mut profile = Profile::default_combined();
        profile.tuning.wheel_confirm_on_press = sticky_tuning;
        let Fixture {
            mut app,
            pad,
            builder,
            ..
        } = fixture(profile);
        tap(&mut app, pad, GamepadButton::South);
        button(&mut app, pad, GamepadButton::LeftTrigger, true);
        let hold = app.world().resource::<InputRes>().mapper.tuning().hold_ms;
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(u64::from(hold) + 1));
        frame(&mut app, pad);
        choose(&mut app, pad, Action::BuildOption { index: 0 });
        if sticky_tuning {
            tap(&mut app, pad, GamepadButton::South);
        }
        button(&mut app, pad, GamepadButton::LeftTrigger, false);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
        assert_eq!(
            app.world().resource::<InputRes>().mapper.mode(),
            Mode::Build(UnitDefId(1))
        );
        assert!(commands(&app).is_empty());
        aim(&mut app, (440, 240));
        tap(&mut app, pad, GamepadButton::South);
        assert_eq!(
            commands(&app),
            vec![SimCommand::Build {
                units: UnitSet::single(builder),
                def: UnitDefId(1),
                at: Some(tak_math::Vec2::from_ints(440, 240)),
                queue: QueueMode::Replace,
            }]
        );
        assert_eq!(selection(&app), UnitSet::single(builder));
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
        frame(&mut app, pad);
        assert_eq!(commands(&app).len(), 1);
    }
}

#[test]
fn controller_same_poll_rb_a_add_and_simultaneous_release_do_not_widen_first_type() {
    let Fixture {
        mut app,
        pad,
        builder,
        troop,
        ..
    } = fixture(Profile::default_combined());
    tap(&mut app, pad, GamepadButton::South);
    aim(&mut app, (280, 160));
    {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad.digital_mut().press(GamepadButton::South);
        gamepad.digital_mut().press(GamepadButton::RightTrigger);
    }
    frame(&mut app, pad);
    {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad.digital_mut().release(GamepadButton::RightTrigger);
        gamepad.digital_mut().release(GamepadButton::South);
    }
    frame(&mut app, pad);
    assert_eq!(selection(&app), [builder, troop].into_iter().collect());
    assert!(commands(&app).is_empty());
}

#[test]
fn controller_cancel_clears_selection_tap_provenance() {
    let Fixture {
        mut app,
        pad,
        builder,
        ..
    } = fixture(Profile::default_combined());
    tap(&mut app, pad, GamepadButton::South);
    tap(&mut app, pad, GamepadButton::East);
    assert!(selection(&app).is_empty());
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(selection(&app), UnitSet::single(builder));
}

#[test]
fn controller_modal_capture_consumes_old_a_release_and_drops_double_tap_history() {
    for modal in [KeyCode::F11, KeyCode::F8, KeyCode::F7] {
        for release_on_open in [false, true] {
            let Fixture {
                mut app,
                pad,
                builder,
                ..
            } = fixture(Profile::default_combined());
            if modal == KeyCode::F7 {
                app.insert_resource(skirmish_ui::SkirmishState::new(Default::default(), false));
                app.init_resource::<skirmish_ui::PendingRequest>();
            }
            app.world_mut().resource_mut::<Feel>().quick_save_dir =
                Err("Synthetic tap test has no checkpoint directory".into());
            tap(&mut app, pad, GamepadButton::South);
            if release_on_open {
                button(&mut app, pad, GamepadButton::South, true);
            }
            let before = selection(&app);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(modal);
            if release_on_open {
                app.world_mut()
                    .get_mut::<Gamepad>(pad)
                    .unwrap()
                    .digital_mut()
                    .release(GamepadButton::South);
            }
            frame(&mut app, pad);
            assert_eq!(selection(&app), before);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(modal);
            frame(&mut app, pad);
            key(&mut app, pad, KeyCode::Escape);
            frame(&mut app, pad);
            tap(&mut app, pad, GamepadButton::South);
            assert_eq!(
                selection(&app),
                UnitSet::single(builder),
                "{modal:?}, release={release_on_open}"
            );
            assert!(commands(&app).is_empty());
        }
    }
}

#[test]
fn classic_mouse_rapid_different_target_selects_second_unit() {
    let Fixture {
        mut app,
        window,
        pad,
        builder,
        troop,
        ..
    } = fixture(Profile::default_combined());
    mouse_tap(&mut app, window, pad, (320, 160));
    assert_eq!(selection(&app), UnitSet::single(builder));
    mouse_tap(&mut app, window, pad, (280, 160));
    assert_eq!(selection(&app), UnitSet::single(troop));
    assert!(commands(&app).is_empty());
}

#[test]
fn classic_mouse_focus_loss_drops_old_double_click_candidate() {
    let Fixture {
        mut app,
        window,
        pad,
        builder,
        ..
    } = fixture(Profile::default_combined());
    mouse_tap(&mut app, window, pad, (320, 160));
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    frame(&mut app, pad);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    mouse_tap(&mut app, window, pad, (320, 160));
    assert_eq!(selection(&app), UnitSet::single(builder));
    assert!(commands(&app).is_empty());
}

#[test]
fn controller_hud_consumed_a_cannot_bridge_two_terrain_selection_taps() {
    let Fixture {
        mut app,
        pad,
        builder,
        twin,
        ..
    } = fixture(Profile::default_combined());
    let initial = record(&mut app);
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(selection(&app), UnitSet::single(builder));
    let panel = app
        .world_mut()
        .spawn((
            HudPanel,
            ComputedNode {
                size: Vec2::new(120.0, 100.0),
                ..default()
            },
            UiGlobalTransform::from(bevy::math::Affine2::from_translation(Vec2::new(
                400.0, 300.0,
            ))),
            InheritedVisibility::VISIBLE,
        ))
        .id();
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(selection(&app), UnitSet::single(builder));
    app.world_mut().despawn(panel);
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(selection(&app), UnitSet::single(builder));
    assert!(commands(&app).is_empty());
    for _ in 0..3 {
        frame(&mut app, pad);
    }
    assert_interrupted_replay(&app, &initial, Some([builder, twin].into_iter().collect()));
}

#[test]
fn controller_map_pan_consumed_a_cannot_bridge_two_terrain_selection_taps() {
    let Fixture {
        mut app,
        pad,
        builder,
        twin,
        ..
    } = fixture(Profile::default_combined());
    let initial = record(&mut app);
    tap(&mut app, pad, GamepadButton::South);
    {
        let mut minimap = app.world_mut().resource_mut::<MinimapState>();
        minimap.image = Some((200, 100));
        minimap.map = [640.0, 320.0];
        minimap.big = true;
    }
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(selection(&app), UnitSet::single(builder));
    app.world_mut().resource_mut::<MinimapState>().image = None;
    aim(&mut app, (320, 160));
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(selection(&app), UnitSet::single(builder));
    assert!(commands(&app).is_empty());
    assert_interrupted_replay(&app, &initial, Some([builder, twin].into_iter().collect()));
}

#[test]
fn classic_mouse_consumed_hud_and_map_clicks_record_selection_interruption() {
    for map in [false, true] {
        let Fixture {
            mut app,
            window,
            pad,
            builder,
            ..
        } = fixture(Profile::default_combined());
        let initial = record(&mut app);
        mouse_tap(&mut app, window, pad, (320, 160));
        assert_eq!(selection(&app), UnitSet::single(builder));
        let panel = if map {
            let mut minimap = app.world_mut().resource_mut::<MinimapState>();
            minimap.image = Some((200, 100));
            minimap.map = [640.0, 320.0];
            minimap.big = true;
            None
        } else {
            Some(
                app.world_mut()
                    .spawn((
                        HudPanel,
                        ComputedNode {
                            size: Vec2::new(120.0, 100.0),
                            ..default()
                        },
                        UiGlobalTransform::from(bevy::math::Affine2::from_translation(Vec2::new(
                            400.0, 300.0,
                        ))),
                        InheritedVisibility::VISIBLE,
                    ))
                    .id(),
            )
        };
        mouse_tap(&mut app, window, pad, (320, 160));
        if let Some(panel) = panel {
            app.world_mut().despawn(panel);
        }
        app.world_mut().resource_mut::<MinimapState>().image = None;
        aim(&mut app, (320, 160));
        mouse_tap(&mut app, window, pad, (320, 160));
        assert_eq!(selection(&app), UnitSet::single(builder));
        assert!(commands(&app).is_empty());
        // PointerLeaveTerrain also cancels HUD-origin mouse history, so only
        // the pad tests use removal of this event as their negative control.
        assert_interrupted_replay(&app, &initial, None);
    }
}

#[test]
fn controller_held_a_brush_release_does_not_seed_a_same_type_refinement() {
    let Fixture {
        mut app,
        pad,
        builder,
        ..
    } = fixture(Profile::default_combined());
    button(&mut app, pad, GamepadButton::South, true);
    let hold = app.world().resource::<InputRes>().mapper.tuning().hold_ms;
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(u64::from(hold) + 1));
    frame(&mut app, pad);
    assert!(app.world().resource::<InputRes>().mapper.brush().is_some());
    button(&mut app, pad, GamepadButton::South, false);
    assert!(app.world().resource::<InputRes>().mapper.brush().is_none());
    assert!(selection(&app).contains(builder));
    tap(&mut app, pad, GamepadButton::South);
    assert_eq!(selection(&app), UnitSet::single(builder));
    assert!(commands(&app).is_empty());
}

#[test]
fn controller_rb_and_dpad_second_taps_keep_non_selection_target_semantics() {
    let Fixture {
        mut app,
        pad,
        builder,
        twin,
        troop,
        ..
    } = fixture(Profile::default_combined());
    let distant = spawn(&mut app, PlayerId(0), UnitDefId(3), (620, 160));
    tap(&mut app, pad, GamepadButton::RightTrigger);
    assert_eq!(
        selection(&app),
        [builder, twin, troop].into_iter().collect()
    );
    aim(&mut app, (420, 240));
    tap(&mut app, pad, GamepadButton::RightTrigger);
    assert_eq!(
        selection(&app),
        [builder, twin, troop, distant].into_iter().collect()
    );

    // Assign group 1 using the actual hold gesture, then recall and double-tap
    // to jump while aiming at empty terrain. It is not SelectSameType.
    aim(&mut app, (320, 160));
    tap(&mut app, pad, GamepadButton::South);
    button(&mut app, pad, GamepadButton::DPadUp, true);
    let hold = app.world().resource::<InputRes>().mapper.tuning().hold_ms;
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(u64::from(hold) + 1));
    frame(&mut app, pad);
    button(&mut app, pad, GamepadButton::DPadUp, false);
    tap(&mut app, pad, GamepadButton::East);
    aim(&mut app, (440, 240));
    tap(&mut app, pad, GamepadButton::DPadUp);
    assert_eq!(selection(&app), UnitSet::single(builder));
    tap(&mut app, pad, GamepadButton::DPadUp);
    // The existing camera glide must finish even though neither tap targets a unit.
    for _ in 0..60 {
        frame(&mut app, pad);
    }
    let target = app.world().resource::<CamState>().geom.target;
    assert!(
        (target[0] - 320.0).abs() < 1.0 && (target[1] - 160.0).abs() < 1.0,
        "{target:?}"
    );
    assert!(commands(&app).is_empty());
}
