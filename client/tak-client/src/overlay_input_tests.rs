//! Synthetic, asset-free regression tests for the real Bevy input schedule.
use super::*;
use clap::Parser;

pub(super) fn app() -> App {
    let args = Args::parse_from(["test", "--data", "/synthetic-not-read", "--overlay"]);
    let map = tak_content::MapDef {
        name: "synthetic-training".into(),
        ota: Default::default(),
        width: 40,
        height: 20,
        sea_level: 0,
        heights: vec![0; 800],
        kingdom: None,
        palette: None,
        features: Vec::new(),
        paved_cells: 0,
        marker_cells: 0,
        start_positions: Vec::new(),
        player_slots: 2,
        crt: None,
        passability: tak_content::PassGrid {
            width: 40,
            height: 20,
            rule: Default::default(),
            cells: vec![0; 800],
        },
        issues: Vec::new(),
        dangling: Vec::new(),
        coverage: Default::default(),
    };
    let mut sim = tak_sim::Sim::new(&tak_headless::bot::demo::scenario(false)).unwrap();
    sim.step(&[]);
    let session =
        Session::from_sim(&Vfs::new(), "synthetic", map, sim, Default::default(), None).unwrap();
    let snap = session.snapshot();
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_message::<MouseWheel>()
        .insert_resource(Feel::new(&args, None))
        .insert_resource(save_browser::BrowserArgs(args.clone()))
        .init_resource::<save_browser::BrowserState>()
        .insert_non_send(save_browser::PendingLoad::default())
        .insert_resource(InputRes {
            mapper: Mapper::new(Profile::default_combined()),
            last_axes: BTreeMap::new(),
            last_pointer: None,
        })
        .insert_resource(CamState {
            geom: ViewGeom {
                target: [0.0, 0.0],
                zoom: 1.0,
                size: [800.0, 600.0],
            },
            input_h: 0.0,
        })
        .init_resource::<MinimapState>()
        .insert_resource(Ground(Arc::new(TerrainData {
            cells: (40, 20),
            corners: vec![0.0; 41 * 21],
            sea_level: 0.0,
            chunks: vec![],
            missing: 0,
            minimap: None,
        })))
        .insert_non_send(SimRes {
            session,
            prev: snap.clone(),
            curr: snap,
            acc: 0.0,
            speed: 1.0,
            paused: false,
            fog: crate::fog::FogGrid::new(40, 20),
            fog_changed: false,
            alerts: Default::default(),
            new_alerts: vec![],
            fog_on: false,
        });
    add_interactive_input(&mut app);
    app
}

#[test]
fn closing_overlay_consumes_f11_and_simultaneous_pause() {
    let mut app = app();
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.press(KeyCode::F11);
    keys.press(KeyCode::Pause);
    app.update();
    assert!(
        !app.world().resource::<Feel>().overlay_open,
        "closing F11 must not reopen overlay"
    );
    assert!(
        !app.world().non_send::<SimRes>().paused,
        "overlay close frame must not leak Pause"
    );
}

#[test]
fn closing_escape_custom_menu_binding_is_consumed_and_capture_expires() {
    let mut app = app();
    let mut profile = Profile::default_combined();
    for binding in &mut profile.bindings {
        if binding.input == tak_input::Input::Key(Key("escape".into())) {
            binding.action = Action::ToggleOverlay;
        }
    }
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(profile);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    assert!(!app.world().resource::<Feel>().overlay_open);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.update();
    assert!(!app.world().resource::<Feel>().overlay_captured_input);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Pause);
    app.update();
    assert!(app.world().non_send::<SimRes>().paused);
}

#[test]
fn closing_pad_menu_or_b_consumes_other_pad_actions_but_keeps_quicksave() {
    for close in [GamepadButton::Start, GamepadButton::East] {
        let mut app = app();
        app.world_mut()
            .resource_scope(|world, mut input: Mut<InputRes>| {
                let sim = world.non_send::<SimRes>();
                input.mapper.perform(
                    Action::SelectAll {
                        scope: tak_commands::Scope::Map,
                    },
                    &sim.view(),
                    sim.session.tick(),
                );
                assert!(!input.mapper.selection().is_empty());
                let mut profile = input.mapper.profile().clone();
                profile
                    .bindings
                    .retain(|binding| binding.input != tak_input::Input::Button(InButton::X));
                profile.bindings.push(tak_input::Binding {
                    input: tak_input::Input::Button(InButton::X),
                    with: None,
                    gesture: tak_input::Gesture::Press,
                    action: Action::Stop,
                });
                input.mapper.set_profile(profile);
            });
        let mut pad = Gamepad::default();
        pad.digital_mut().press(close);
        pad.digital_mut().press(GamepadButton::West);
        app.world_mut().spawn(pad);
        app.update();
        let feel = app.world().resource::<Feel>();
        assert!(!feel.overlay_open, "{close:?}");
        assert!(feel.quick_save_requested);
        assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
    }
}

#[test]
fn closing_overlay_swallows_orders_and_keeps_f10() {
    let mut app = app();
    app.world_mut()
        .resource_scope(|world, mut input: Mut<InputRes>| {
            let sim = world.non_send::<SimRes>();
            input.mapper.perform(
                Action::SelectAll {
                    scope: tak_commands::Scope::Map,
                },
                &sim.view(),
                sim.session.tick(),
            );
            assert!(!input.mapper.selection().is_empty());
        });
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.press(KeyCode::F11);
    keys.press(KeyCode::F10);
    keys.press(KeyCode::KeyS);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Right);
    app.update();
    assert!(!app.world().resource::<Feel>().overlay_open);
    assert!(app.world().resource::<Feel>().quick_save_requested);
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
}

#[test]
fn profile_save_shortcut_remains_available_while_closing() {
    let mut app = app();
    let path = std::env::temp_dir().join(format!(
        "oa-overlay-profile-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    app.world_mut().resource_mut::<Feel>().profile_path = Some(path.clone());
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.press(KeyCode::ControlLeft);
    keys.press(KeyCode::KeyS);
    keys.press(KeyCode::F11);
    app.update();
    assert!(!app.world().resource::<Feel>().overlay_open);
    let saved = std::fs::read_to_string(&path).unwrap();
    Profile::from_json(&saved).unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn opening_overlay_drops_held_gesture_and_release_does_not_retrigger_it() {
    let mut app = app();
    app.world_mut().resource_mut::<Feel>().overlay_open = false;
    let mut profile = Profile::default_combined();
    for binding in &mut profile.bindings {
        if binding.input == tak_input::Input::Key(Key("tab".into())) {
            binding.action = Action::Precision;
        }
    }
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(profile);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Tab);
    app.update();
    assert!(app.world().resource::<InputRes>().mapper.precision());
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.clear();
    keys.press(KeyCode::F11);
    app.update();
    assert!(app.world().resource::<Feel>().overlay_open);
    assert!(!app.world().resource::<InputRes>().mapper.precision());
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.clear();
    keys.release(KeyCode::Tab);
    keys.release(KeyCode::F11);
    keys.press(KeyCode::Escape);
    app.update();
    assert!(!app.world().resource::<Feel>().overlay_open);
    assert!(!app.world().resource::<InputRes>().mapper.precision());
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
}

#[test]
fn captured_frame_neutralizes_sticks_and_does_not_accumulate_camera_motion() {
    use bevy::input::gamepad::GamepadInput;
    let mut app = app();
    app.world_mut().resource_mut::<Feel>().overlay_open = false;
    let mut pad = Gamepad::default();
    pad.analog_mut()
        .set(GamepadInput::Axis(GamepadAxis::LeftStickX), 1.0);
    let entity = app.world_mut().spawn(pad).id();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(16));
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::F11);
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(16));
    app.update();
    let target = app.world().resource::<CamState>().geom.target;
    assert_ne!(
        target,
        [0.0, 0.0],
        "fixture must actually drive the camera before capture"
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.world_mut()
        .get_mut::<Gamepad>(entity)
        .unwrap()
        .analog_mut()
        .set(GamepadInput::Axis(GamepadAxis::LeftStickX), 0.0);
    for _ in 0..3 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(1));
        app.update();
        assert_eq!(app.world().resource::<CamState>().geom.target, target);
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(16));
    app.update();
    assert_eq!(app.world().resource::<CamState>().geom.target, target);
}

#[test]
fn captured_frame_blocks_hud_click_minimap_drag_and_scroll() {
    for closing in [false, true] {
        let mut app = app();
        let mut win = Window {
            resolution: WindowResolution::new(800, 600),
            ..default()
        };
        win.set_cursor_position(Some(Vec2::new(100.0, 100.0)));
        let window = app.world_mut().spawn((win, PrimaryWindow)).id();
        app.world_mut().spawn((
            ButtonKind::Panel(0),
            ComputedNode {
                size: Vec2::splat(30.0),
                ..default()
            },
            UiGlobalTransform::from(bevy::math::Affine2::from_translation(Vec2::splat(100.0))),
            InheritedVisibility::VISIBLE,
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<Messages<MouseWheel>>()
            .write(MouseWheel {
                unit: MouseScrollUnit::Line,
                x: 0.0,
                y: 3.0,
                window,
                phase: bevy::input::touch::TouchPhase::Moved,
            });
        if closing {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::F11);
        }
        let zoom = app.world().resource::<CamState>().geom.zoom;
        app.update();
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
        assert_eq!(app.world().resource::<CamState>().geom.zoom, zoom);
        assert!(!app.world().resource::<MinimapState>().ui_held);
        assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());

        // A second captured frame targets the actual minimap rectangle.
        app.world_mut().resource_mut::<Feel>().overlay_open = true;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        let mut minimap = app.world_mut().resource_mut::<MinimapState>();
        minimap.image = Some((100, 100));
        minimap.map = [640.0, 320.0];
        let geom = minimap.geom([800.0, 600.0]).unwrap();
        let pos = (
            geom.rect[0] + geom.rect[2] / 2.0,
            geom.rect[1] + geom.rect[3] / 2.0,
        );
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(pos.0, pos.1)));
        if closing {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::F11);
            keys.press(KeyCode::F11);
        }
        let target = app.world().resource::<CamState>().geom.target;
        app.update();
        assert!(!app.world().resource::<MinimapState>().mouse_drag);
        assert_eq!(app.world().resource::<CamState>().geom.target, target);
    }
}

#[test]
fn stationary_pointer_is_resampled_after_overlay_closes() {
    let mut app = app();
    app.world_mut().resource_mut::<Feel>().overlay_open = false;
    app.world_mut().resource_mut::<CamState>().geom.target = [320.0, 160.0];
    let mut win = Window {
        resolution: WindowResolution::new(800, 600),
        ..default()
    };
    win.set_cursor_position(Some(Vec2::new(400.0, 300.0)));
    let window = app.world_mut().spawn((win, PrimaryWindow)).id();
    app.update();
    let before = app.world().resource::<InputRes>().mapper.cursor_world();
    app.world_mut().resource_mut::<Feel>().overlay_open = true;
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(450.0, 300.0)));
    app.update();
    assert!(app.world().resource::<InputRes>().last_pointer.is_none());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.update();
    assert!(app.world().resource::<InputRes>().last_pointer.is_some());
    assert_ne!(
        app.world().resource::<InputRes>().mapper.cursor_world(),
        before
    );
}
