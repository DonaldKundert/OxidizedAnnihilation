//! Exercise the actual Bevy adapter, without assets or a window backend.
use super::*;
use tak_commands::{QueueMode, SimCommand};

fn fixture(big: bool, profile: Profile) -> (App, Entity, Vec2, tak_math::Vec2) {
    let mut app = super::overlay_input_tests::app();
    app.world_mut().resource_mut::<Feel>().overlay_open = false;
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(profile);
    app.world_mut().resource_mut::<CamState>().geom.target = [80.0, 40.0];
    app.world_mut().resource_mut::<CamState>().geom.zoom = 0.25;
    let mut mm = app.world_mut().resource_mut::<MinimapState>();
    mm.image = Some((200, 100));
    mm.map = [640.0, 320.0];
    mm.reserve = 100.0;
    mm.big = big;
    let mg = mm.geom([800.0, 600.0]).unwrap();
    let pos = Vec2::new(mg.rect[0] + mg.rect[2] * 0.8, mg.rect[1] + mg.rect[3] * 0.8);
    let target = mg.target_at(pos.x, pos.y);
    let expected = tak_math::Vec2::new(
        tak_math::Fx::from_raw((target[0] * 65536.0) as i32),
        tak_math::Fx::from_raw((target[1] * 65536.0) as i32),
    );
    let mut win = Window {
        resolution: WindowResolution::new(800, 600),
        ..default()
    };
    win.set_cursor_position(Some(pos));
    let window = app.world_mut().spawn((win, PrimaryWindow)).id();
    perform(
        &mut app,
        Action::SelectAll {
            scope: tak_commands::Scope::Map,
        },
    );
    assert!(
        !app.world()
            .resource::<InputRes>()
            .mapper
            .selection()
            .is_empty()
    );
    (app, window, pos, expected)
}

fn perform(app: &mut App, action: Action) {
    app.world_mut()
        .resource_scope(|world, mut input: Mut<InputRes>| {
            let sim = world.non_send::<SimRes>();
            input
                .mapper
                .perform(action, &sim.view(), sim.session.tick());
        });
}

fn click(app: &mut App, button: MouseButton) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(button);
    app.update();
    let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
    mouse.clear();
    mouse.release(button);
    app.update();
}

#[test]
fn minimap_context_targets_distant_map_point_in_both_layouts_and_profiles() {
    for big in [false, true] {
        for profile in [Profile::default_combined(), Profile::classic_tak()] {
            let (mut app, _, _, expected) = fixture(big, profile);
            let camera = app.world().resource::<CamState>().geom.target;
            click(&mut app, MouseButton::Right);
            let sim = app.world().non_send::<SimRes>();
            assert_eq!(sim.session.recorded.entries().len(), 1);
            assert!(
                matches!(&sim.session.recorded.entries()[0].command,
                SimCommand::Move { to, queue: QueueMode::Replace, .. } if *to == expected),
                "{:?}",
                sim.session.recorded.entries()
            );
            assert_eq!(app.world().resource::<CamState>().geom.target, camera);
        }
    }
}

#[test]
fn minimap_targeted_left_click_orders_instead_of_panning() {
    for big in [false, true] {
        let (mut app, _, _, expected) = fixture(big, Profile::classic_tak());
        perform(&mut app, Action::PatrolMode);
        let camera = app.world().resource::<CamState>().geom.target;
        click(&mut app, MouseButton::Left);
        let sim = app.world().non_send::<SimRes>();
        assert_eq!(sim.session.recorded.entries().len(), 1);
        assert!(matches!(&sim.session.recorded.entries()[0].command,
            SimCommand::Patrol { to, .. } if *to == expected));
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
    }
}

#[test]
fn minimap_plain_left_click_preserves_selection_and_pans() {
    let (mut app, _, _, expected) = fixture(false, Profile::classic_tak());
    let selection = app
        .world()
        .resource::<InputRes>()
        .mapper
        .selection()
        .clone();
    click(&mut app, MouseButton::Left);
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
    assert_eq!(
        app.world().resource::<InputRes>().mapper.selection(),
        &selection
    );
    assert_eq!(
        app.world().resource::<CamState>().geom.target,
        [f(expected.x), f(expected.y)]
    );
}

#[test]
fn minimap_queued_target_mode_and_cancel_use_profile_bindings() {
    let (mut app, _, _, expected) = fixture(true, Profile::classic_tak());
    perform(&mut app, Action::PatrolMode);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ShiftLeft);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    click(&mut app, MouseButton::Left);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Patrol { to, queue: QueueMode::Append, .. } if *to == expected));
    assert_eq!(
        app.world().resource::<InputRes>().mapper.mode(),
        Mode::Patrol
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
    assert_eq!(
        app.world()
            .non_send::<SimRes>()
            .session
            .recorded
            .entries()
            .len(),
        1
    );
}

#[test]
fn minimap_custom_left_context_binding_is_not_replaced_with_pan() {
    let mut profile = Profile::classic_tak();
    profile
        .bindings
        .retain(|b| b.input != tak_input::Input::Button(InButton::MouseLeft));
    profile.bindings.push(tak_input::Binding {
        with_all: Vec::new(),
        input: tak_input::Input::Button(InButton::MouseLeft),
        with: None,
        gesture: tak_input::Gesture::Press,
        action: Action::Context { append: false },
    });
    let (mut app, _, _, expected) = fixture(false, profile);
    click(&mut app, MouseButton::Left);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Move { to, .. } if *to == expected));
}

#[test]
fn minimap_overlay_close_frame_does_not_issue_orders() {
    let (mut app, _, _, _) = fixture(false, Profile::classic_tak());
    app.world_mut().resource_mut::<Feel>().overlay_open = true;
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::F11);
    click(&mut app, MouseButton::Right);
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
}

#[test]
fn minimap_pointer_exit_restores_terrain_target() {
    let (mut app, window, _, expected) = fixture(false, Profile::classic_tak());
    click(&mut app, MouseButton::Right);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
    app.update();
    click(&mut app, MouseButton::Right);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 2);
    assert!(matches!(&sim.session.recorded.entries()[1].command,
        SimCommand::Move { to, .. } if *to != expected && *to == tak_math::Vec2::new(tak_math::Fx::from_int(80), tak_math::Fx::from_int(40))));
}

#[test]
fn minimap_target_click_does_not_activate_hud_behind_fullscreen_map() {
    let (mut app, _, pos, expected) = fixture(true, Profile::classic_tak());
    app.world_mut().spawn((
        ButtonKind::Panel(0),
        ComputedNode {
            size: Vec2::splat(40.0),
            ..default()
        },
        UiGlobalTransform::from(bevy::math::Affine2::from_translation(pos)),
        InheritedVisibility::VISIBLE,
    ));
    perform(&mut app, Action::PatrolMode);
    click(&mut app, MouseButton::Left);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Patrol { to, .. } if *to == expected));
    assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
}

#[test]
fn minimap_attack_mode_targets_ground_and_keeps_fog_filter() {
    let (mut app, _, _, expected) = fixture(false, Profile::classic_tak());
    app.world_mut().non_send_mut::<SimRes>().fog_on = true;
    perform(&mut app, Action::AttackMode);
    click(&mut app, MouseButton::Left);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Attack { target: tak_commands::Target::Ground(to), .. } if *to == expected));
}

#[test]
fn minimap_drag_release_outside_map_is_swallowed_without_deselecting() {
    let (mut app, window, _, _) = fixture(false, Profile::classic_tak());
    let selection = app
        .world()
        .resource::<InputRes>()
        .mapper
        .selection()
        .clone();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(10.0, 10.0)));
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    app.update();
    assert!(!app.world().resource::<MinimapState>().mouse_drag);
    assert_eq!(
        app.world().resource::<InputRes>().mapper.selection(),
        &selection
    );
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
}

#[test]
fn minimap_context_only_attacks_visible_enemy_at_map_point() {
    for fog_on in [false, true] {
        let (mut app, window, _, _) = fixture(false, Profile::classic_tak());
        let (enemy, target) = app
            .world()
            .non_send::<SimRes>()
            .session
            .sim
            .world()
            .units()
            .find(|(_, unit)| unit.owner != PlayerId(0))
            .map(|(id, unit)| (id, unit.pos))
            .unwrap();
        let mg = app
            .world()
            .resource::<MinimapState>()
            .geom([800.0, 600.0])
            .unwrap();
        let pos = Vec2::new(
            mg.rect[0] + mg.rect[2] * f(target.x) / 640.0,
            mg.rect[1] + mg.rect[3] * f(target.y) / 320.0,
        );
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(pos));
        app.world_mut().non_send_mut::<SimRes>().fog_on = fog_on;
        click(&mut app, MouseButton::Right);
        let sim = app.world().non_send::<SimRes>();
        assert_eq!(sim.session.recorded.entries().len(), 1);
        let command = &sim.session.recorded.entries()[0].command;
        if fog_on {
            assert!(
                matches!(command, SimCommand::Move { .. }),
                "hidden unit leaked: {command:?}"
            );
        } else {
            assert!(
                matches!(command, SimCommand::Attack { target: tak_commands::Target::Unit(id), .. } if *id == enemy),
                "visible unit not targeted: {command:?}"
            );
        }
    }
}

#[test]
fn minimap_stationary_mouse_does_not_steal_held_gamepad_cursor() {
    use bevy::input::gamepad::GamepadInput;
    let (mut app, _, _, expected) = fixture(false, Profile::default_combined());
    let mut pad = Gamepad::default();
    pad.analog_mut()
        .set(GamepadInput::Axis(GamepadAxis::LeftStickX), 1.0);
    app.world_mut().spawn(pad);
    for _ in 0..4 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(16));
        app.update();
        assert!(
            app.world().resource::<InputRes>().mapper.stick_cursor(),
            "stationary map mouse stole the cursor"
        );
        assert_ne!(
            app.world().resource::<InputRes>().mapper.cursor_world(),
            expected
        );
    }
    click(&mut app, MouseButton::Right);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Move { to, .. } if *to == expected));
}

#[test]
fn minimap_left_order_chord_overrides_plain_selection_binding() {
    let mut profile = Profile::classic_tak();
    profile.bindings.push(tak_input::Binding {
        with_all: Vec::new(),
        input: tak_input::Input::Button(InButton::MouseLeft),
        with: Some(tak_input::Input::Key(Key("alt".into()))),
        gesture: tak_input::Gesture::Press,
        action: Action::Context { append: false },
    });
    let (mut app, _, _, expected) = fixture(false, profile);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::AltLeft);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    let camera = app.world().resource::<CamState>().geom.target;
    click(&mut app, MouseButton::Left);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Move { to, .. } if *to == expected));
    assert_eq!(app.world().resource::<CamState>().geom.target, camera);
}

fn pad_fixture(big: bool, mut profile: Profile) -> (App, Entity, Entity, Vec2, tak_math::Vec2) {
    // Free-cursor profiles can reach both layouts. Reticle mode is exercised
    // separately with the full-screen map that contains the centre reticle.
    profile.tuning.reticle = false;
    let (mut app, window, pos, _) = fixture(big, profile);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(None);
    app.update();
    let pos = pos.round();
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .warp_cursor(tak_math::Vec2::new(
            tak_math::Fx::from_int(pos.x as i32),
            tak_math::Fx::from_int(pos.y as i32),
        ));
    let mg = app
        .world()
        .resource::<MinimapState>()
        .geom([800.0, 600.0])
        .unwrap();
    let target = mg.target_at(pos.x, pos.y);
    let fx = |v: f32| tak_math::Fx::from_raw((v * 65536.0) as i32);
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    (
        app,
        window,
        pad,
        pos,
        tak_math::Vec2::new(fx(target[0]), fx(target[1])),
    )
}

fn pad_click(app: &mut App, pad: Entity, button: GamepadButton) {
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .press(button);
    app.update();
    {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad.digital_mut().clear();
        gamepad.digital_mut().release(button);
    }
    app.update();
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .clear();
}

#[test]
fn minimap_pad_pending_target_and_context_keep_camera_and_cursor_in_both_layouts() {
    for big in [false, true] {
        for target_mode in [false, true] {
            let (mut app, _, pad, _, expected) = pad_fixture(big, Profile::default_combined());
            let camera = app.world().resource::<CamState>().geom.target;
            let cursor = app.world().resource::<InputRes>().mapper.cursor_screen();
            if target_mode {
                perform(&mut app, Action::PatrolMode);
            }
            pad_click(
                &mut app,
                pad,
                if target_mode {
                    GamepadButton::South
                } else {
                    GamepadButton::West
                },
            );
            let sim = app.world().non_send::<SimRes>();
            assert_eq!(sim.session.recorded.entries().len(), 1);
            let command = &sim.session.recorded.entries()[0].command;
            if target_mode {
                assert!(
                    matches!(command, SimCommand::Patrol { to, .. } if *to == expected),
                    "{command:?}"
                );
            } else {
                assert!(
                    matches!(command, SimCommand::Move { to, .. } if *to == expected),
                    "{command:?}"
                );
            }
            assert_eq!(app.world().resource::<CamState>().geom.target, camera);
            let input = app.world().resource::<InputRes>();
            assert!(input.mapper.stick_cursor());
            assert_eq!(input.mapper.cursor_screen(), cursor);
            assert_ne!(input.mapper.cursor_world_unmapped(), expected);
            assert_eq!(input.mapper.mode(), Mode::None);
        }
    }
}

#[test]
fn minimap_pad_plain_selection_pans_and_swallows_release_after_cursor_exits() {
    let (mut app, _, pad, _, expected) = pad_fixture(false, Profile::default_combined());
    let selected = app
        .world()
        .resource::<InputRes>()
        .mapper
        .selection()
        .clone();
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .press(GamepadButton::South);
    app.update();
    assert_eq!(
        app.world().resource::<CamState>().geom.target,
        [f(expected.x), f(expected.y)]
    );
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .warp_cursor(tak_math::Vec2::ZERO);
    {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad.digital_mut().clear();
        gamepad.digital_mut().release(GamepadButton::South);
    }
    app.update();
    assert!(!app.world().resource::<MinimapState>().pad_swallow);
    assert_eq!(
        app.world().resource::<InputRes>().mapper.selection(),
        &selected
    );
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
}

#[test]
fn minimap_pad_queued_target_and_cancel_preserve_bindings() {
    let (mut app, _, pad, _, expected) = pad_fixture(true, Profile::default_combined());
    perform(&mut app, Action::PatrolMode);
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .press(GamepadButton::RightTrigger);
    pad_click(&mut app, pad, GamepadButton::South);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Patrol { to, queue: QueueMode::Append, .. } if *to == expected));
    assert_eq!(
        app.world().resource::<InputRes>().mapper.mode(),
        Mode::Patrol
    );
    pad_click(&mut app, pad, GamepadButton::East);
    assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
    assert_eq!(
        app.world()
            .non_send::<SimRes>()
            .session
            .recorded
            .entries()
            .len(),
        1
    );
}

#[test]
fn minimap_pad_remapped_a_and_same_poll_chord_issue_orders_not_pan() {
    for chord in [false, true] {
        let mut profile = Profile::default_combined();
        if !chord {
            profile
                .bindings
                .retain(|b| b.input != tak_input::Input::Button(InButton::A));
        }
        profile.bindings.push(tak_input::Binding {
            with_all: Vec::new(),
            input: tak_input::Input::Button(InButton::A),
            with: chord.then_some(tak_input::Input::Button(InButton::Rb)),
            gesture: tak_input::Gesture::Press,
            action: Action::Context { append: false },
        });
        let (mut app, _, pad, _, expected) = pad_fixture(false, profile);
        let camera = app.world().resource::<CamState>().geom.target;
        if chord {
            app.world_mut()
                .get_mut::<Gamepad>(pad)
                .unwrap()
                .digital_mut()
                .press(GamepadButton::RightTrigger);
        }
        pad_click(&mut app, pad, GamepadButton::South);
        let sim = app.world().non_send::<SimRes>();
        assert_eq!(sim.session.recorded.entries().len(), 1);
        assert!(matches!(&sim.session.recorded.entries()[0].command,
            SimCommand::Move { to, .. } if *to == expected));
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);
    }
}

#[test]
fn minimap_pad_keyboard_mode_same_frame_and_fullscreen_hud_occlusion() {
    let (mut app, _, pad, pos, expected) = pad_fixture(true, Profile::default_combined());
    app.world_mut().spawn((
        ButtonKind::Panel(0),
        ComputedNode {
            size: Vec2::splat(40.0),
            ..default()
        },
        UiGlobalTransform::from(bevy::math::Affine2::from_translation(pos)),
        InheritedVisibility::VISIBLE,
    ));
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyP);
    pad_click(&mut app, pad, GamepadButton::South);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Patrol { to, .. } if *to == expected));
    assert!(!app.world().resource::<MinimapState>().pad_swallow);
}

#[test]
fn minimap_pad_motion_and_mouse_takeover_do_not_drift_or_reuse_stale_target() {
    use bevy::input::gamepad::GamepadInput;
    let (mut app, window, pad, _, _) = pad_fixture(false, Profile::default_combined());
    app.update();
    let initial = app.world().resource::<InputRes>().mapper.cursor_screen();
    for _ in 0..4 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(16));
        app.update();
        let input = app.world().resource::<InputRes>();
        assert_eq!(
            input.mapper.cursor_screen(),
            initial,
            "map target warped the stick cursor"
        );
        assert!(input.mapper.stick_cursor());
    }
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .analog_mut()
        .set(GamepadInput::Axis(GamepadAxis::LeftStickX), -1.0);
    app.update();
    for _ in 0..10 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(16));
        app.update();
    }
    let input = app.world().resource::<InputRes>();
    assert!(input.mapper.stick_cursor());
    assert!(input.mapper.cursor_screen().x < initial.x);
    assert_eq!(
        input.mapper.cursor_world(),
        input.mapper.cursor_world_unmapped(),
        "leaving map retained its target"
    );
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
    click(&mut app, MouseButton::Right);
    assert!(
        !app.world().resource::<InputRes>().mapper.stick_cursor(),
        "idle adapter stole mouse control"
    );
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Move { to, .. } if *to == tak_math::Vec2::new(tak_math::Fx::from_int(80), tak_math::Fx::from_int(40))));
}

#[test]
fn minimap_pad_overlay_blocks_target_press_and_release() {
    let (mut app, _, pad, _, _) = pad_fixture(false, Profile::default_combined());
    perform(&mut app, Action::PatrolMode);
    app.world_mut().resource_mut::<Feel>().overlay_open = true;
    pad_click(&mut app, pad, GamepadButton::South);
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
    assert!(!app.world().resource::<MinimapState>().pad_swallow);
}

#[test]
fn minimap_original_left_context_orders_with_selection_and_pans_without_it() {
    for big in [false, true] {
        let (mut app, _, _, expected) = fixture(big, Profile::original_tak());
        let camera = app.world().resource::<CamState>().geom.target;
        click(&mut app, MouseButton::Left);
        let sim = app.world().non_send::<SimRes>();
        assert_eq!(sim.session.recorded.entries().len(), 1);
        assert!(matches!(&sim.session.recorded.entries()[0].command,
            SimCommand::Move { to, .. } if *to == expected));
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);

        let (mut app, _, _, expected) = fixture(big, Profile::original_tak());
        perform(&mut app, Action::Cancel);
        assert!(
            app.world()
                .resource::<InputRes>()
                .mapper
                .selection()
                .is_empty()
        );
        click(&mut app, MouseButton::Left);
        assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
        assert_eq!(
            app.world().resource::<CamState>().geom.target,
            [f(expected.x), f(expected.y)]
        );
    }
}

#[test]
fn minimap_pad_custom_release_is_not_swallowed_by_selection_pan() {
    let mut profile = Profile::default_combined();
    profile.bindings.push(tak_input::Binding {
        with_all: Vec::new(),
        input: tak_input::Input::Button(InButton::A),
        with: None,
        gesture: tak_input::Gesture::Release,
        action: Action::Context { append: false },
    });
    let (mut app, _, pad, _, expected) = pad_fixture(false, profile);
    let camera = app.world().resource::<CamState>().geom.target;
    pad_click(&mut app, pad, GamepadButton::South);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Move { to, .. } if *to == expected));
    assert_eq!(app.world().resource::<CamState>().geom.target, camera);
}

#[test]
fn minimap_pad_reticle_targets_fullscreen_map_without_losing_reticle_source() {
    let (mut app, _, pad, _, _) = pad_fixture(true, Profile::default_combined());
    let mut tuning = *app.world().resource::<InputRes>().mapper.tuning();
    tuning.reticle = true;
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_tuning(tuning);
    app.update();
    let mg = app
        .world()
        .resource::<MinimapState>()
        .geom([800.0, 600.0])
        .unwrap();
    let target = mg.target_at(400.0, 300.0);
    let expected = tak_math::Vec2::new(
        tak_math::Fx::from_raw((target[0] * 65536.0) as i32),
        tak_math::Fx::from_raw((target[1] * 65536.0) as i32),
    );
    perform(&mut app, Action::PatrolMode);
    pad_click(&mut app, pad, GamepadButton::South);
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(sim.session.recorded.entries().len(), 1);
    assert!(matches!(&sim.session.recorded.entries()[0].command,
        SimCommand::Patrol { to, .. } if *to == expected));
    assert!(app.world().resource::<InputRes>().mapper.reticle_active());
}

#[test]
fn minimap_pad_context_only_attacks_visible_enemy_at_map_point() {
    for fog_on in [false, true] {
        let (mut app, _, pad, _, _) = pad_fixture(false, Profile::default_combined());
        let (enemy, target) = app
            .world()
            .non_send::<SimRes>()
            .session
            .sim
            .world()
            .units()
            .find(|(_, unit)| unit.owner != PlayerId(0))
            .map(|(id, unit)| (id, unit.pos))
            .unwrap();
        let mg = app
            .world()
            .resource::<MinimapState>()
            .geom([800.0, 600.0])
            .unwrap();
        let pos = tak_math::Vec2::new(
            tak_math::Fx::from_raw(
                ((mg.rect[0] + mg.rect[2] * f(target.x) / 640.0) * 65536.0) as i32,
            ),
            tak_math::Fx::from_raw(
                ((mg.rect[1] + mg.rect[3] * f(target.y) / 320.0) * 65536.0) as i32,
            ),
        );
        app.world_mut()
            .resource_mut::<InputRes>()
            .mapper
            .warp_cursor(pos);
        app.world_mut().non_send_mut::<SimRes>().fog_on = fog_on;
        pad_click(&mut app, pad, GamepadButton::West);
        let sim = app.world().non_send::<SimRes>();
        assert_eq!(sim.session.recorded.entries().len(), 1);
        let command = &sim.session.recorded.entries()[0].command;
        if fog_on {
            assert!(
                matches!(command, SimCommand::Move { .. }),
                "hidden enemy leaked: {command:?}"
            );
        } else {
            assert!(
                matches!(command, SimCommand::Attack { target: tak_commands::Target::Unit(id), .. } if *id == enemy),
                "visible enemy not targeted: {command:?}"
            );
        }
    }
}

#[test]
fn minimap_pad_recording_omits_swallowed_buttons_and_idle_surface_checks() {
    let (mut app, _, pad, _, _) = pad_fixture(false, Profile::default_combined());
    let recorder = tak_input::Recorder::new(
        serde_json::json!({}),
        &app.world().resource::<InputRes>().mapper,
    );
    app.world_mut().resource_mut::<Feel>().recorder = Some(recorder);
    app.update();
    let recorded_frames = app
        .world()
        .resource::<Feel>()
        .recorder
        .as_ref()
        .unwrap()
        .recording()
        .frames
        .len();
    for _ in 0..3 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(16));
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<Feel>()
            .recorder
            .as_ref()
            .unwrap()
            .recording()
            .frames
            .len(),
        recorded_frames
    );
    pad_click(&mut app, pad, GamepadButton::South);
    let feel = app.world().resource::<Feel>();
    let recording = feel.recorder.as_ref().unwrap().recording();
    assert!(
        !recording
            .frames
            .iter()
            .flat_map(|frame| &frame.events)
            .any(|event| matches!(
                event,
                EventKind::ButtonDown {
                    button: InButton::A
                } | EventKind::ButtonUp {
                    button: InButton::A
                }
            )),
        "a swallowed map-pan gesture leaked into input playback"
    );
}

#[test]
fn minimap_pad_open_wheel_is_not_replaced_with_camera_pan() {
    let mut profile = Profile::default_combined();
    profile.tuning.wheel_confirm_on_press = true;
    let (mut app, _, pad, _, _) = pad_fixture(false, profile);
    pad_click(&mut app, pad, GamepadButton::LeftTrigger2);
    assert!(app.world().resource::<InputRes>().mapper.wheel().is_some());
    let camera = app.world().resource::<CamState>().geom.target;
    pad_click(&mut app, pad, GamepadButton::South);
    assert_eq!(app.world().resource::<CamState>().geom.target, camera);
    assert!(!app.world().resource::<MinimapState>().pad_swallow);
}
