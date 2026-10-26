//! Asset-free minimap selection regressions through the actual Bevy schedule.
//! A complete click may arrive between two polls, retaining both edge flags.
use super::*;
use tak_commands::{Scope, SimCommand, Target, UnitDefId, UnitSet};

fn fixture(big: bool, profile: Profile) -> (App, Entity, tak_math::Vec2) {
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
    perform(&mut app, Action::SelectAll { scope: Scope::Map });
    assert!(!selection(&app).is_empty());
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .take_feedback();
    let recorder = tak_input::Recorder::new(
        serde_json::json!({"synthetic": "minimap complete click"}),
        &app.world().resource::<InputRes>().mapper,
    );
    app.world_mut().resource_mut::<Feel>().recorder = Some(recorder);
    (app, window, expected)
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

fn selection(app: &App) -> UnitSet {
    app.world()
        .resource::<InputRes>()
        .mapper
        .selection()
        .clone()
}

fn assert_raw_replay(app: &App, initial: &Mapper) {
    let recording = app
        .world()
        .resource::<Feel>()
        .recorder
        .as_ref()
        .unwrap()
        .recording();
    let json = serde_json::to_string(recording).unwrap();
    let recording: tak_input::Recording = serde_json::from_str(&json).unwrap();
    let view = app.world().non_send::<SimRes>().view();
    let mut replay = initial.clone();
    for frame in &recording.frames {
        replay.set_camera(frame.camera);
        let mut outputs = Vec::new();
        for event in frame
            .events
            .iter()
            .chain(std::iter::once(&EventKind::Frame))
        {
            outputs.extend(replay.handle(
                &InputEvent::new(frame.t, event.clone()),
                &view,
                tak_commands::Tick(frame.tick),
            ));
        }
        assert_eq!(
            outputs, frame.outputs,
            "raw outputs diverged at {}",
            frame.t
        );
        assert_eq!(replay.take_feedback(), frame.feedback);
        if let Some(selected) = &frame.selection {
            assert_eq!(replay.selection(), selected);
        }
        replay.take_camera_pan();
        replay.take_camera_zoom();
    }
    assert_eq!(replay.selection(), &selection(app));
}

fn frame(app: &mut App) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(16));
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
}

fn click(app: &mut App, fast: bool) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    if !fast {
        frame(app);
    }
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    if fast {
        let mouse = app.world().resource::<ButtonInput<MouseButton>>();
        assert!(mouse.just_pressed(MouseButton::Left));
        assert!(mouse.just_released(MouseButton::Left));
        assert!(!mouse.pressed(MouseButton::Left));
    }
    frame(app);
}

fn imported_profile() -> Profile {
    // Hand-written keyboard remaps, not copied from an installed game.
    let imported = tak_input::parse_keys_tdf(
        b"[CUSTOMKEYS] { LOWER_Q = UnitCommand Attack; CTRL_J = SelectUnits Monarch, TrackUnit; }",
    )
    .unwrap();
    Profile::default_combined().with_keys_tdf(&imported)
}

#[test]
fn minimap_complete_click_preserves_selection_like_split_click() {
    let mut failures = Vec::new();
    for big in [false, true] {
        for profile in [
            Profile::classic_tak(),
            Profile::default_combined(),
            imported_profile(),
        ] {
            for fast in [false, true] {
                let label = format!("profile={}, big={big}, fast={fast}", profile.name);
                let (mut app, _, expected) = fixture(big, profile.clone());
                let selected = selection(&app);
                click(&mut app, fast);
                assert_eq!(
                    app.world().resource::<CamState>().geom.target,
                    [f(expected.x), f(expected.y)],
                    "{label}"
                );
                let sim = app.world().non_send::<SimRes>();
                assert!(sim.session.recorded.is_empty(), "{label}");
                assert!(
                    selected
                        .iter()
                        .all(|id| sim.session.sim.world().unit(*id).is_some()),
                    "selected units must remain alive: {label}"
                );
                if selection(&app) != selected {
                    let recording = app
                        .world()
                        .resource::<Feel>()
                        .recorder
                        .as_ref()
                        .unwrap()
                        .recording();
                    eprintln!(
                        "{label}: selected={selected:?}, after={:?}, frames={:?}",
                        selection(&app),
                        recording.frames
                    );
                    failures.push(label);
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "ordinary minimap clicks changed selection: {failures:?}"
    );
}

#[test]
fn minimap_complete_click_is_not_dependent_on_previous_capture_state() {
    let mut failures = Vec::new();
    for prior_release in [false, true] {
        for prior_ui in [false, true] {
            let (mut app, _, _) = fixture(false, Profile::default_combined());
            let selected = selection(&app);
            {
                let mut mm = app.world_mut().resource_mut::<MinimapState>();
                mm.swallow_release = prior_release;
                mm.ui_held[0] = prior_ui;
            }
            click(&mut app, true);
            if selection(&app) != selected {
                failures.push((prior_release, prior_ui));
            }
            assert!(!app.world().resource::<MinimapState>().mouse_drag);
            assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
        }
    }
    assert!(
        failures.is_empty(),
        "selection changed for (prior release, prior UI) states: {failures:?}"
    );
}

#[test]
fn minimap_hover_and_terrain_transition_do_not_clear_selected_units() {
    for fast in [false, true] {
        let (mut app, window, _) = fixture(false, Profile::default_combined());
        let selected = selection(&app);
        frame(&mut app);
        assert_eq!(
            selection(&app),
            selected,
            "MapPointer alone changed selection"
        );
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
        frame(&mut app);
        assert_eq!(
            selection(&app),
            selected,
            "PointerMove alone changed selection"
        );
        let mg = app
            .world()
            .resource::<MinimapState>()
            .geom([800.0, 600.0])
            .unwrap();
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(
                mg.rect[0] + mg.rect[2] * 0.8,
                mg.rect[1] + mg.rect[3] * 0.8,
            )));
        click(&mut app, fast);
        assert_eq!(
            selection(&app),
            selected,
            "minimap reentry click changed selection, fast={fast}"
        );
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
        frame(&mut app);
        assert_eq!(
            selection(&app),
            selected,
            "post-pan terrain transition changed selection, fast={fast}"
        );
    }
}

#[test]
fn minimap_complete_click_keeps_explicit_move_attack_patrol_guard_orders() {
    for fast in [false, true] {
        for action in [
            Action::MoveMode,
            Action::AttackMode,
            Action::PatrolMode,
            Action::GuardMode,
        ] {
            let (mut app, _, expected) = fixture(false, Profile::default_combined());
            let selected = selection(&app);
            let ally = (action == Action::GuardMode).then(|| {
                app.world_mut()
                    .non_send_mut::<SimRes>()
                    .session
                    .sim
                    .world_mut()
                    .spawn(UnitDefId(0), PlayerId(0), expected, tak_math::Angle::ZERO)
                    .unwrap()
            });
            perform(&mut app, action);
            let camera = app.world().resource::<CamState>().geom.target;
            click(&mut app, fast);
            assert_eq!(selection(&app), selected, "{action:?}, fast={fast}");
            assert_eq!(
                app.world().resource::<CamState>().geom.target,
                camera,
                "{action:?}, fast={fast}"
            );
            let sim = app.world().non_send::<SimRes>();
            let commands = sim.session.recorded.entries();
            assert_eq!(commands.len(), 1, "{action:?}, fast={fast}: {commands:?}");
            assert!(
                match (&commands[0].command, action) {
                    (SimCommand::Move { to, .. }, Action::MoveMode)
                    | (SimCommand::Patrol { to, .. }, Action::PatrolMode) => *to == expected,
                    (
                        SimCommand::Attack {
                            target: Target::Ground(to),
                            ..
                        },
                        Action::AttackMode,
                    ) => *to == expected,
                    (SimCommand::Guard { target, .. }, Action::GuardMode) => Some(*target) == ally,
                    _ => false,
                },
                "{action:?}, fast={fast}: {commands:?}"
            );
        }
    }
}

#[test]
fn minimap_complete_click_keeps_custom_context_press_binding() {
    for fast in [false, true] {
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
        let (mut app, _, expected) = fixture(false, profile);
        let selected = selection(&app);
        let camera = app.world().resource::<CamState>().geom.target;
        click(&mut app, fast);
        assert_eq!(selection(&app), selected);
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);
        let sim = app.world().non_send::<SimRes>();
        let commands = sim.session.recorded.entries();
        assert_eq!(commands.len(), 1);
        assert!(matches!(&commands[0].command, SimCommand::Move { to, .. } if *to == expected));
    }
}

#[test]
fn minimap_capture_ends_before_the_next_world_selection_click() {
    for fast_map in [false, true] {
        for fast_world in [false, true] {
            let (mut app, window, expected) = fixture(false, Profile::default_combined());
            let selected = selection(&app);
            let next_unit = app
                .world_mut()
                .non_send_mut::<SimRes>()
                .session
                .sim
                .world_mut()
                .spawn(UnitDefId(3), PlayerId(0), expected, tak_math::Angle::ZERO)
                .unwrap();
            let initial = app.world().resource::<InputRes>().mapper.clone();
            click(&mut app, fast_map);
            assert_eq!(selection(&app), selected);
            assert!(!app.world().resource::<MinimapState>().mouse_drag);
            assert!(!app.world().resource::<MinimapState>().swallow_release);
            let recording = app
                .world()
                .resource::<Feel>()
                .recorder
                .as_ref()
                .unwrap()
                .recording();
            assert_eq!(
                recording
                    .frames
                    .iter()
                    .flat_map(|frame| &frame.events)
                    .filter(|event| matches!(event, EventKind::SelectionTapInterrupted))
                    .count(),
                1,
                "the consumed click must remain an interruption in raw replay"
            );
            assert!(recording.frames.iter().flat_map(|frame| &frame.events).all(
                |event| !matches!(
                    event,
                    EventKind::ButtonDown {
                        button: InButton::MouseLeft
                    } | EventKind::ButtonUp {
                        button: InButton::MouseLeft
                    }
                )
            ));
            // No idle poll between the map release and this fresh terrain click.
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
            click(&mut app, fast_world);
            assert_eq!(
                selection(&app),
                UnitSet::single(next_unit),
                "fresh world click was captured: fast_map={fast_map}, fast_world={fast_world}"
            );
            assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
            assert_raw_replay(&app, &initial);
        }
    }
}

fn hud_fixture(action: Option<Action>) -> (App, Entity) {
    let (mut app, window, _) = fixture(false, Profile::default_combined());
    app.world_mut().resource_mut::<CamState>().geom.target = [320.0, 160.0];
    let hud_pos = Vec2::new(650.0, 300.0);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(hud_pos));
    app.world_mut().spawn((
        HudPanel,
        ComputedNode {
            size: Vec2::new(180.0, 140.0),
            ..default()
        },
        UiGlobalTransform::from(bevy::math::Affine2::from_translation(hud_pos)),
        InheritedVisibility::VISIBLE,
    ));
    if let Some(action) = action {
        let index = crate::hud::PANEL_ACTIONS
            .iter()
            .position(|candidate| *candidate == action)
            .unwrap();
        app.world_mut().spawn((
            ButtonKind::Panel(index),
            ComputedNode {
                size: Vec2::splat(40.0),
                ..default()
            },
            UiGlobalTransform::from(bevy::math::Affine2::from_translation(hud_pos)),
            InheritedVisibility::VISIBLE,
        ));
    }
    (app, window)
}

#[test]
fn hud_release_capture_does_not_swallow_next_complete_world_selection() {
    let mut failures = Vec::new();
    for fast_world in [false, true] {
        for idle in [false, true] {
            let (mut app, window) = hud_fixture(None);
            let selected = selection(&app);
            let target = app
                .world_mut()
                .non_send_mut::<SimRes>()
                .session
                .sim
                .world_mut()
                .spawn(
                    UnitDefId(3),
                    PlayerId(0),
                    tak_math::Vec2::from_ints(320, 160),
                    tak_math::Angle::ZERO,
                )
                .unwrap();
            let initial = app.world().resource::<InputRes>().mapper.clone();
            // Ordinary down poll then release poll, without injected state.
            click(&mut app, false);
            assert_eq!(selection(&app), selected);
            assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
            if idle {
                frame(&mut app);
            }
            click(&mut app, fast_world);
            if selection(&app) != UnitSet::single(target) {
                failures.push((fast_world, idle));
            }
            assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
            assert_raw_replay(&app, &initial);
        }
    }
    assert!(
        failures.is_empty(),
        "fresh selection swallowed after HUD release: (fast world, idle)={failures:?}"
    );
}

#[test]
fn hud_release_capture_does_not_swallow_next_complete_pending_order() {
    let mut failures = Vec::new();
    for action in [
        Action::MoveMode,
        Action::AttackMode,
        Action::PatrolMode,
        Action::GuardMode,
    ] {
        for fast_world in [false, true] {
            for idle in [false, true] {
                let (mut app, window) = hud_fixture(Some(action));
                let selected = selection(&app);
                let expected = tak_math::Vec2::from_ints(320, 160);
                let ally = (action == Action::GuardMode).then(|| {
                    app.world_mut()
                        .non_send_mut::<SimRes>()
                        .session
                        .sim
                        .world_mut()
                        .spawn(UnitDefId(3), PlayerId(0), expected, tak_math::Angle::ZERO)
                        .unwrap()
                });
                // The real HUD button establishes the pending order on press.
                click(&mut app, false);
                assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
                assert_ne!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
                assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
                app.world_mut()
                    .get_mut::<Window>(window)
                    .unwrap()
                    .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
                if idle {
                    frame(&mut app);
                }
                click(&mut app, fast_world);
                assert_eq!(selection(&app), selected);
                let sim = app.world().non_send::<SimRes>();
                let commands = sim.session.recorded.entries();
                let matched = commands.len() == 1
                    && match (&commands[0].command, action) {
                        (SimCommand::Move { to, .. }, Action::MoveMode)
                        | (SimCommand::Patrol { to, .. }, Action::PatrolMode) => *to == expected,
                        (
                            SimCommand::Attack {
                                target: Target::Ground(to),
                                ..
                            },
                            Action::AttackMode,
                        ) => *to == expected,
                        (SimCommand::Guard { target, .. }, Action::GuardMode) => {
                            Some(*target) == ally
                        }
                        _ => false,
                    };
                if !matched {
                    failures.push((action, fast_world, idle, commands.len()));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "HUD-established order was not confirmed: (action, fast world, idle, commands)={failures:?}"
    );
}

#[test]
fn minimap_same_poll_target_mode_press_precedes_plain_pan_capture() {
    let mut failures = Vec::new();
    for big in [false, true] {
        for (key, action) in [
            (KeyCode::KeyA, Action::AttackMode),
            (KeyCode::KeyQ, Action::AttackMode),
            (KeyCode::KeyQ, Action::AttackMoveMode),
            (KeyCode::KeyQ, Action::MoveMode),
            (KeyCode::KeyQ, Action::PatrolMode),
            (KeyCode::KeyQ, Action::GuardMode),
            (KeyCode::KeyM, Action::MoveMode),
            (KeyCode::KeyP, Action::PatrolMode),
            (KeyCode::KeyG, Action::GuardMode),
        ] {
            for earlier_key in [false, true] {
                for fast_mouse in [false, true] {
                    let mut profile = Profile::default_combined();
                    if key == KeyCode::KeyQ {
                        profile.bindings.push(tak_input::Binding {
                            with_all: Vec::new(),
                            input: tak_input::Input::Key(Key("q".into())),
                            with: Some(tak_input::Input::Key(Key("alt".into()))),
                            gesture: tak_input::Gesture::Press,
                            action,
                        });
                    }
                    let (mut app, _, expected) = fixture(big, profile);
                    let selected = selection(&app);
                    let camera = app.world().resource::<CamState>().geom.target;
                    let ally = (action == Action::GuardMode).then(|| {
                        app.world_mut()
                            .non_send_mut::<SimRes>()
                            .session
                            .sim
                            .world_mut()
                            .spawn(UnitDefId(3), PlayerId(0), expected, tak_math::Angle::ZERO)
                            .unwrap()
                    });
                    assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
                    let initial = app.world().resource::<InputRes>().mapper.clone();
                    {
                        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                        if key == KeyCode::KeyQ {
                            keys.press(KeyCode::AltLeft);
                            keys.release(KeyCode::AltLeft);
                        }
                        keys.press(key);
                        keys.release(key);
                    }
                    if earlier_key {
                        frame(&mut app);
                        assert_ne!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
                    }
                    click(&mut app, fast_mouse);
                    let sim = app.world().non_send::<SimRes>();
                    let commands = sim.session.recorded.entries();
                    let matched = commands.len() == 1
                        && match (&commands[0].command, action) {
                            (SimCommand::Move { to, .. }, Action::MoveMode)
                            | (SimCommand::Patrol { to, .. }, Action::PatrolMode)
                            | (SimCommand::AttackMove { to, .. }, Action::AttackMoveMode) => {
                                *to == expected
                            }
                            (
                                SimCommand::Attack {
                                    target: Target::Ground(to),
                                    ..
                                },
                                Action::AttackMode,
                            ) => *to == expected,
                            (SimCommand::Guard { target, .. }, Action::GuardMode) => {
                                Some(*target) == ally
                            }
                            _ => false,
                        };
                    let after_camera = app.world().resource::<CamState>().geom.target;
                    let mode = app.world().resource::<InputRes>().mapper.mode();
                    if !matched
                        || after_camera != camera
                        || mode != Mode::None
                        || selection(&app) != selected
                    {
                        let recording = app
                            .world()
                            .resource::<Feel>()
                            .recorder
                            .as_ref()
                            .unwrap()
                            .recording();
                        let events: Vec<_> = recording
                            .frames
                            .iter()
                            .flat_map(|frame| &frame.events)
                            .collect();
                        eprintln!(
                            "big={big}, key={key:?}, action={action:?}, earlier_key={earlier_key}, fast_mouse={fast_mouse}, commands={commands:?}, camera={after_camera:?}, mode={mode:?}, raw={events:?}"
                        );
                        failures.push((big, key, action, earlier_key, fast_mouse));
                    }
                    assert_raw_replay(&app, &initial);
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "same-poll target press was treated as camera pan: (big, key, action, earlier key, fast mouse)={failures:?}"
    );
}

#[test]
fn minimap_same_poll_custom_mouse_chord_precedes_plain_pan_capture() {
    let mut failures = Vec::new();
    for big in [false, true] {
        for ctrl_timing in ["absent", "same_poll", "earlier_poll"] {
            for fast_mouse in [false, true] {
                let mut profile = Profile::classic_tak();
                profile.bindings.push(tak_input::Binding {
                    with_all: Vec::new(),
                    input: tak_input::Input::Button(InButton::MouseLeft),
                    with: Some(tak_input::Input::Key(Key("ctrl".into()))),
                    gesture: tak_input::Gesture::Press,
                    action: Action::Context { append: false },
                });
                let (mut app, _, expected) = fixture(big, profile);
                let selected = selection(&app);
                let camera = app.world().resource::<CamState>().geom.target;
                let initial = app.world().resource::<InputRes>().mapper.clone();
                if ctrl_timing != "absent" {
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .press(KeyCode::ControlLeft);
                    if ctrl_timing == "earlier_poll" {
                        frame(&mut app);
                    }
                    // Its release shares the mouse poll and must remain after the order.
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .release(KeyCode::ControlLeft);
                }
                click(&mut app, fast_mouse);
                assert_eq!(selection(&app), selected);
                let sim = app.world().non_send::<SimRes>();
                let commands = sim.session.recorded.entries();
                let after_camera = app.world().resource::<CamState>().geom.target;
                let matched = if ctrl_timing == "absent" {
                    commands.is_empty() && after_camera == [f(expected.x), f(expected.y)]
                } else {
                    commands.len() == 1
                        && matches!(&commands[0].command, SimCommand::Move { to, .. } if *to == expected)
                        && after_camera == camera
                };
                if !matched {
                    let recording = app
                        .world()
                        .resource::<Feel>()
                        .recorder
                        .as_ref()
                        .unwrap()
                        .recording();
                    let events: Vec<_> = recording
                        .frames
                        .iter()
                        .flat_map(|frame| &frame.events)
                        .collect();
                    eprintln!(
                        "big={big}, ctrl={ctrl_timing}, fast_mouse={fast_mouse}, commands={commands:?}, camera={after_camera:?}, raw={events:?}"
                    );
                    failures.push((big, ctrl_timing, fast_mouse));
                }
                assert_raw_replay(&app, &initial);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "same-poll custom chord was treated as camera pan: {failures:?}"
    );
}

#[test]
fn minimap_same_poll_ctrl_a_keeps_select_all_precedence_and_plain_camera_pan() {
    let (mut app, _, expected) = fixture(false, Profile::default_combined());
    let selected = selection(&app);
    let initial = app.world().resource::<InputRes>().mapper.clone();
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        for key in [KeyCode::ControlLeft, KeyCode::KeyA] {
            keys.press(key);
            keys.release(key);
        }
    }
    click(&mut app, true);
    assert_eq!(selection(&app), selected);
    assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
    assert_eq!(
        app.world().resource::<CamState>().geom.target,
        [f(expected.x), f(expected.y)]
    );
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
    assert_raw_replay(&app, &initial);
}

#[test]
fn terrain_same_poll_target_mode_ordering_control() {
    // Separate the pre-existing generic event order from minimap capture.
    let mut failures = Vec::new();
    for (key, action) in [
        (KeyCode::KeyA, Action::AttackMode),
        (KeyCode::KeyM, Action::MoveMode),
        (KeyCode::KeyP, Action::PatrolMode),
        (KeyCode::KeyG, Action::GuardMode),
    ] {
        for fast_mouse in [false, true] {
            let (mut app, window, _) = fixture(false, Profile::default_combined());
            app.world_mut().resource_mut::<CamState>().geom.target = [320.0, 160.0];
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
            let expected = tak_math::Vec2::from_ints(320, 160);
            let ally = (action == Action::GuardMode).then(|| {
                app.world_mut()
                    .non_send_mut::<SimRes>()
                    .session
                    .sim
                    .world_mut()
                    .spawn(UnitDefId(3), PlayerId(0), expected, tak_math::Angle::ZERO)
                    .unwrap()
            });
            let selected = selection(&app);
            let initial = app.world().resource::<InputRes>().mapper.clone();
            {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                keys.press(key);
                keys.release(key);
            }
            click(&mut app, fast_mouse);
            let sim = app.world().non_send::<SimRes>();
            let commands = sim.session.recorded.entries();
            let matched = commands.len() == 1
                && match (&commands[0].command, action) {
                    (SimCommand::Move { to, .. }, Action::MoveMode)
                    | (SimCommand::Patrol { to, .. }, Action::PatrolMode) => *to == expected,
                    (
                        SimCommand::Attack {
                            target: Target::Ground(to),
                            ..
                        },
                        Action::AttackMode,
                    ) => *to == expected,
                    (SimCommand::Guard { target, .. }, Action::GuardMode) => Some(*target) == ally,
                    _ => false,
                };
            if !matched || selection(&app) != selected {
                let recording = app
                    .world()
                    .resource::<Feel>()
                    .recorder
                    .as_ref()
                    .unwrap()
                    .recording();
                let events: Vec<_> = recording
                    .frames
                    .iter()
                    .flat_map(|frame| &frame.events)
                    .collect();
                eprintln!(
                    "terrain action={action:?}, fast_mouse={fast_mouse}, commands={commands:?}, selected={:?}, raw={events:?}",
                    selection(&app)
                );
                failures.push((action, fast_mouse));
            }
            assert_raw_replay(&app, &initial);
        }
    }
    assert!(
        failures.is_empty(),
        "generic terrain event-order failures, separate from minimap capture: {failures:?}"
    );
}

#[test]
fn minimap_same_poll_attack_move_modifier_controls() {
    let mut failures = Vec::new();
    for map_surface in [false, true] {
        for earlier_alt in [false, true] {
            for fast_mouse in [false, true] {
                let (mut app, window, mut expected) = fixture(false, Profile::default_combined());
                if !map_surface {
                    app.world_mut().resource_mut::<CamState>().geom.target = [320.0, 160.0];
                    app.world_mut()
                        .get_mut::<Window>(window)
                        .unwrap()
                        .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
                    expected = tak_math::Vec2::from_ints(320, 160);
                }
                let selected = selection(&app);
                let camera = app.world().resource::<CamState>().geom.target;
                let initial = app.world().resource::<InputRes>().mapper.clone();
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::AltLeft);
                if earlier_alt {
                    frame(&mut app);
                }
                {
                    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                    keys.press(KeyCode::KeyA);
                    keys.release(KeyCode::KeyA);
                    keys.release(KeyCode::AltLeft);
                }
                click(&mut app, fast_mouse);
                let sim = app.world().non_send::<SimRes>();
                let commands = sim.session.recorded.entries();
                let matched = commands.len() == 1
                    && matches!(&commands[0].command,
                    SimCommand::AttackMove { to, .. } if *to == expected);
                if !matched
                    || selection(&app) != selected
                    || app.world().resource::<CamState>().geom.target != camera
                {
                    let recording = app
                        .world()
                        .resource::<Feel>()
                        .recorder
                        .as_ref()
                        .unwrap()
                        .recording();
                    let events: Vec<_> = recording
                        .frames
                        .iter()
                        .flat_map(|frame| &frame.events)
                        .collect();
                    eprintln!(
                        "AttackMove map={map_surface}, earlier_alt={earlier_alt}, fast_mouse={fast_mouse}, commands={commands:?}, raw={events:?}"
                    );
                    failures.push((map_surface, earlier_alt, fast_mouse));
                }
                assert_raw_replay(&app, &initial);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "AttackMove complete-tap ordering failures: (map, earlier Alt, fast mouse)={failures:?}"
    );
}

#[test]
fn minimap_same_poll_preview_does_not_predict_ineligible_or_special_modes() {
    for case in [
        "empty",
        "stale",
        "open_wheel",
        "grid",
        "alternate_gesture",
        "heal",
        "spell",
        "build",
        "wheel_press",
    ] {
        let mut profile = Profile::default_combined();
        profile.grid_build_keys = case == "grid";
        if case == "alternate_gesture" {
            profile.bindings.push(tak_input::Binding {
                with_all: Vec::new(),
                input: tak_input::Input::Key(Key("a".into())),
                with: None,
                gesture: tak_input::Gesture::Release,
                action: Action::Cancel,
            });
        }
        let special = match case {
            "heal" => Some(Action::HealMode),
            "spell" => Some(Action::CastSpell { index: u8::MAX }),
            "build" => Some(Action::BuildOption { index: u8::MAX }),
            "wheel_press" | "open_wheel" => Some(Action::BuildWheel),
            _ => None,
        };
        if let Some(action) = special {
            profile.bindings.push(tak_input::Binding {
                with_all: Vec::new(),
                input: tak_input::Input::Key(Key("q".into())),
                with: None,
                gesture: tak_input::Gesture::Press,
                action,
            });
        }
        let (mut app, _, expected) = fixture(false, profile);
        if case == "empty" {
            perform(&mut app, Action::Cancel);
        } else if case == "stale" {
            let selected = selection(&app);
            for id in selected.iter() {
                app.world_mut()
                    .non_send_mut::<SimRes>()
                    .session
                    .sim
                    .world_mut()
                    .despawn(*id)
                    .unwrap();
            }
        } else if case == "open_wheel" {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyQ);
            frame(&mut app);
            assert!(app.world().resource::<InputRes>().mapper.wheel().is_some());
        }
        let key = if special.is_some() && case != "open_wheel" {
            KeyCode::KeyQ
        } else {
            KeyCode::KeyA
        };
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(key);
            keys.release(key);
        }
        click(&mut app, true);
        assert_eq!(
            app.world().resource::<CamState>().geom.target,
            [f(expected.x), f(expected.y)],
            "preview claimed a mode for {case}"
        );
        assert!(
            app.world().non_send::<SimRes>().session.recorded.is_empty(),
            "preview emitted an order for {case}"
        );
    }
}

#[test]
fn minimap_preview_does_not_borrow_a_consumed_pad_primary_modifier() {
    let mut profile = Profile::default_combined();
    profile.tuning.reticle = false;
    profile.bindings.push(tak_input::Binding {
        with_all: Vec::new(),
        input: tak_input::Input::Button(InButton::MouseLeft),
        with: Some(tak_input::Input::Button(InButton::A)),
        gesture: tak_input::Gesture::Press,
        action: Action::Context { append: false },
    });
    let (mut app, _, _) = fixture(false, profile);
    let selected = selection(&app);
    let mut pad = Gamepad::default();
    pad.digital_mut().press(GamepadButton::South);
    pad.digital_mut().release(GamepadButton::South);
    app.world_mut().spawn(pad);
    click(&mut app, true);
    assert_eq!(
        selection(&app),
        selected,
        "map-captured pad A never reached Mapper and cannot supply a mouse chord"
    );
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
    // Two independent camera gestures may update the camera in this poll;
    // this regression owns selection/order safety, not their camera arbitration.
}

#[test]
fn minimap_preview_keeps_already_delivered_pad_primary_modifier() {
    let mut profile = Profile::default_combined();
    profile
        .bindings
        .retain(|binding| binding.input != tak_input::Input::Button(InButton::A));
    profile.bindings.extend([
        tak_input::Binding {
            with_all: Vec::new(),
            input: tak_input::Input::Button(InButton::A),
            with: None,
            gesture: tak_input::Gesture::Press,
            action: Action::Precision,
        },
        tak_input::Binding {
            with_all: Vec::new(),
            input: tak_input::Input::Button(InButton::MouseLeft),
            with: Some(tak_input::Input::Button(InButton::A)),
            gesture: tak_input::Gesture::Press,
            action: Action::Context { append: false },
        },
    ]);
    let (mut app, _, expected) = fixture(false, profile);
    let selected = selection(&app);
    let camera = app.world().resource::<CamState>().geom.target;
    let mut pad = Gamepad::default();
    pad.digital_mut().press(GamepadButton::South);
    let pad = app.world_mut().spawn(pad).id();
    frame(&mut app);
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .clear();
    click(&mut app, true);
    assert_eq!(selection(&app), selected);
    assert_eq!(app.world().resource::<CamState>().geom.target, camera);
    let sim = app.world().non_send::<SimRes>();
    let commands = sim.session.recorded.entries();
    assert_eq!(commands.len(), 1);
    assert!(matches!(&commands[0].command, SimCommand::Move { to, .. } if *to == expected));
}

fn direct_mode_parity_case(
    action: Action,
    map_surface: bool,
    fast_mouse: bool,
    earlier_mode: bool,
    trailing: Option<Action>,
    open_wheel: bool,
) -> (
    Vec<SimCommand>,
    UnitSet,
    Mode,
    Option<tak_input::WheelKind>,
    [f32; 2],
) {
    let mut profile = Profile::default_combined();
    profile.bindings.retain(|binding| {
        !matches!(&binding.input,
        tak_input::Input::Key(key) if ["q", "z", "b"].contains(&key.0.as_str()))
    });
    for (key, with, action) in [
        ("q", Some(tak_input::Input::Key(Key("alt".into()))), action),
        ("b", None, Action::BuildWheel),
        ("z", None, trailing.unwrap_or(Action::Cancel)),
    ] {
        profile.bindings.push(tak_input::Binding {
            with_all: Vec::new(),
            input: tak_input::Input::Key(Key(key.into())),
            with,
            gesture: tak_input::Gesture::Press,
            action,
        });
    }
    let (mut app, window, mut expected) = fixture(false, profile);
    if !map_surface {
        app.world_mut().resource_mut::<CamState>().geom.target = [320.0, 160.0];
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
        expected = tak_math::Vec2::from_ints(320, 160);
    }
    app.world_mut()
        .non_send_mut::<SimRes>()
        .session
        .sim
        .world_mut()
        .spawn(UnitDefId(3), PlayerId(0), expected, tak_math::Angle::ZERO)
        .unwrap();
    let initial = app.world().resource::<InputRes>().mapper.clone();
    if open_wheel {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyB);
        frame(&mut app);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_some());
    }
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        for key in [KeyCode::AltLeft, KeyCode::KeyQ] {
            keys.press(key);
            keys.release(key);
        }
    }
    if earlier_mode {
        frame(&mut app);
    }
    if trailing.is_some() {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::KeyZ);
        keys.release(KeyCode::KeyZ);
    }
    click(&mut app, fast_mouse);
    assert_raw_replay(&app, &initial);
    let input = app.world().resource::<InputRes>();
    (
        app.world()
            .non_send::<SimRes>()
            .session
            .recorded
            .entries()
            .iter()
            .map(|entry| entry.command.clone())
            .collect(),
        input.mapper.selection().clone(),
        input.mapper.mode(),
        input.mapper.wheel().map(|wheel| wheel.kind),
        app.world().resource::<CamState>().geom.target,
    )
}

#[test]
fn direct_mode_same_poll_priority_matches_existing_point_order_arbitration() {
    for action in [
        Action::AttackMode,
        Action::AttackMoveMode,
        Action::MoveMode,
        Action::PatrolMode,
        Action::GuardMode,
    ] {
        for trailing in [
            Action::Stop,
            Action::Cancel,
            Action::SelectAll { scope: Scope::Map },
        ] {
            for map_surface in [false, true] {
                for fast_mouse in [false, true] {
                    let earlier = direct_mode_parity_case(
                        action,
                        map_surface,
                        fast_mouse,
                        true,
                        Some(trailing),
                        false,
                    );
                    let same_poll = direct_mode_parity_case(
                        action,
                        map_surface,
                        fast_mouse,
                        false,
                        Some(trailing),
                        false,
                    );
                    assert_eq!(
                        same_poll, earlier,
                        "{action:?}, trailing={trailing:?}, map={map_surface}, fast={fast_mouse}"
                    );
                    if fast_mouse {
                        // A complete point order precedes the ordinary trailing
                        // action, exactly as existing Attack point clicks do.
                        assert!(!same_poll.0.is_empty());
                        assert!(!matches!(same_poll.0[0], SimCommand::Stop { .. }));
                        assert_eq!(
                            same_poll.0.len(),
                            if trailing == Action::Stop { 2 } else { 1 }
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn direct_mode_same_poll_priority_preserves_open_wheel_routes() {
    for action in [
        Action::AttackMode,
        Action::AttackMoveMode,
        Action::MoveMode,
        Action::PatrolMode,
        Action::GuardMode,
    ] {
        for fast_mouse in [false, true] {
            // Terrain follows each mode's established route, not a new
            // universal policy for different kinds of order while a wheel is open.
            let earlier = direct_mode_parity_case(action, false, fast_mouse, true, None, true);
            let same_poll = direct_mode_parity_case(action, false, fast_mouse, false, None, true);
            assert_eq!(same_poll, earlier, "{action:?}, fast={fast_mouse}");
            // The conservative preview does not claim a new map target while
            // a wheel owns the interaction; existing plain-pan capture remains.
            let map = direct_mode_parity_case(action, true, fast_mouse, false, None, true);
            assert!(map.0.is_empty(), "open-wheel map path issued {action:?}");
            assert!(map.3.is_some());
            assert!((map.4[0] - 512.0).abs() < 1.0 && (map.4[1] - 256.0).abs() < 1.0);
        }
    }
}

#[test]
fn direct_mode_same_poll_priority_keeps_ctrl_selection_chords_in_place() {
    for key in [KeyCode::KeyA, KeyCode::KeyG, KeyCode::KeyM] {
        let (mut app, _, _) = fixture(false, Profile::default_combined());
        let initial = app.world().resource::<InputRes>().mapper.clone();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            for key in [KeyCode::ControlLeft, key] {
                keys.press(key);
                keys.release(key);
            }
        }
        click(&mut app, true);
        assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
        let recording = app
            .world()
            .resource::<Feel>()
            .recorder
            .as_ref()
            .unwrap()
            .recording();
        let events: Vec<_> = recording
            .frames
            .iter()
            .flat_map(|frame| &frame.events)
            .collect();
        let interrupt = events
            .iter()
            .position(|event| matches!(event, EventKind::SelectionTapInterrupted))
            .unwrap();
        let key_down = events.iter().position(|event| matches!(event, EventKind::KeyDown { key: name } if name.0 == key_name(key).unwrap())).unwrap();
        assert!(
            interrupt < key_down,
            "Ctrl+{key:?} was incorrectly prioritized as a target mode"
        );
        assert_raw_replay(&app, &initial);
    }
}

#[test]
fn direct_mode_same_poll_priority_does_not_promote_special_actions() {
    for action in [
        Action::HealMode,
        Action::CastSpellMode,
        Action::BuildOption { index: 0 },
        Action::BuildWheel,
        Action::SpellWheel,
        Action::CommandWheel,
    ] {
        let mut profile = Profile::default_combined();
        profile.bindings.push(tak_input::Binding {
            with_all: Vec::new(),
            input: tak_input::Input::Key(Key("q".into())),
            with: None,
            gesture: tak_input::Gesture::Press,
            action,
        });
        let (mut app, window, _) = fixture(false, profile);
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::KeyQ);
            keys.release(KeyCode::KeyQ);
        }
        click(&mut app, true);
        let recording = app
            .world()
            .resource::<Feel>()
            .recorder
            .as_ref()
            .unwrap()
            .recording();
        let events: Vec<_> = recording
            .frames
            .iter()
            .flat_map(|frame| &frame.events)
            .collect();
        let release = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    EventKind::ButtonUp {
                        button: InButton::MouseLeft
                    }
                )
            })
            .unwrap();
        let key_down = events
            .iter()
            .position(|event| matches!(event, EventKind::KeyDown { key } if key.0 == "q"))
            .unwrap();
        assert!(
            release < key_down,
            "special action {action:?} changed priority"
        );
    }
}

#[test]
fn direct_mode_priority_does_not_promote_a_grid_build_override() {
    for action in [
        Action::AttackMoveMode,
        Action::MoveMode,
        Action::PatrolMode,
        Action::GuardMode,
    ] {
        let mut profile = Profile::default_combined();
        profile.grid_build_keys = true;
        profile.bindings.push(tak_input::Binding {
            with_all: Vec::new(),
            input: tak_input::Input::Key(Key("q".into())),
            with: None,
            gesture: tak_input::Gesture::Press,
            action,
        });
        let (mut app, window, _) = fixture(false, profile);
        app.world_mut().resource_mut::<CamState>().geom.target = [320.0, 160.0];
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::KeyQ);
            keys.release(KeyCode::KeyQ);
        }
        click(&mut app, true);
        let sim = app.world().non_send::<SimRes>();
        assert!(
            !sim.session
                .recorded
                .entries()
                .iter()
                .any(|entry| matches!(entry.command, SimCommand::Build { .. })),
            "grid-overridden {action:?} was promoted into an unintended Build: {:?}",
            sim.session.recorded.entries()
        );
        let recording = app
            .world()
            .resource::<Feel>()
            .recorder
            .as_ref()
            .unwrap()
            .recording();
        let events: Vec<_> = recording
            .frames
            .iter()
            .flat_map(|frame| &frame.events)
            .collect();
        let release = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    EventKind::ButtonUp {
                        button: InButton::MouseLeft
                    }
                )
            })
            .unwrap();
        let key_down = events
            .iter()
            .position(|event| matches!(event, EventKind::KeyDown { key } if key.0 == "q"))
            .unwrap();
        assert!(release < key_down);
    }
}

#[test]
fn direct_mode_priority_does_not_promote_stop_through_uncertain_pad_a() {
    for delivered_a in [false, true] {
        let mut profile = Profile::named("original").unwrap();
        profile.tuning.reticle = false;
        if delivered_a {
            profile
                .bindings
                .retain(|binding| binding.input != tak_input::Input::Button(InButton::A));
            profile.bindings.push(tak_input::Binding {
                with_all: Vec::new(),
                input: tak_input::Input::Button(InButton::A),
                with: None,
                gesture: tak_input::Gesture::Press,
                action: Action::Precision,
            });
        }
        for (with, action) in [
            (
                None,
                if delivered_a {
                    Action::MoveMode
                } else {
                    Action::Stop
                },
            ),
            (
                Some(tak_input::Input::Button(InButton::A)),
                if delivered_a {
                    Action::Stop
                } else {
                    Action::MoveMode
                },
            ),
        ] {
            profile.bindings.push(tak_input::Binding {
                with_all: Vec::new(),
                input: tak_input::Input::Key(Key("q".into())),
                with,
                gesture: tak_input::Gesture::Press,
                action,
            });
        }
        let (mut app, _, _) = fixture(false, profile);
        let initial = app.world().resource::<InputRes>().mapper.clone();
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::South);
        pad.digital_mut().release(GamepadButton::South);
        app.world_mut().spawn(pad);
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::KeyQ);
            keys.release(KeyCode::KeyQ);
        }
        click(&mut app, true);
        let sim = app.world().non_send::<SimRes>();
        let commands = sim.session.recorded.entries();
        assert_eq!(commands.len(), 2, "delivered A={delivered_a}: {commands:?}");
        assert!(
            matches!(commands[0].command, SimCommand::Move { .. })
                && matches!(commands[1].command, SimCommand::Stop { .. }),
            "actual Stop must retain normal order, delivered A={delivered_a}: {commands:?}"
        );
        assert_raw_replay(&app, &initial);
    }
}

#[test]
fn minimap_preview_declines_fresh_a_dependent_mode_and_mouse_predictions() {
    for delivered_a in [false, true] {
        for mouse_chord in [false, true] {
            let mut profile = Profile::default_combined();
            profile.tuning.reticle = false;
            if delivered_a {
                profile
                    .bindings
                    .retain(|binding| binding.input != tak_input::Input::Button(InButton::A));
                profile.bindings.push(tak_input::Binding {
                    with_all: Vec::new(),
                    input: tak_input::Input::Button(InButton::A),
                    with: None,
                    gesture: tak_input::Gesture::Press,
                    action: Action::Precision,
                });
            }
            if mouse_chord {
                profile.bindings.extend([
                    tak_input::Binding {
                        with_all: Vec::new(),
                        input: tak_input::Input::Button(InButton::MouseLeft),
                        with: Some(tak_input::Input::Key(Key("ctrl".into()))),
                        gesture: tak_input::Gesture::Press,
                        action: Action::Context { append: false },
                    },
                    tak_input::Binding {
                        with_all: vec![
                            tak_input::Input::Key(Key("ctrl".into())),
                            tak_input::Input::Button(InButton::A),
                        ],
                        input: tak_input::Input::Button(InButton::MouseLeft),
                        with: None,
                        gesture: tak_input::Gesture::Press,
                        action: Action::DragSelect,
                    },
                ]);
            } else {
                for (with, action) in [
                    (None, Action::MoveMode),
                    (Some(tak_input::Input::Button(InButton::A)), Action::Stop),
                ] {
                    profile.bindings.push(tak_input::Binding {
                        with_all: Vec::new(),
                        input: tak_input::Input::Key(Key("q".into())),
                        with,
                        gesture: tak_input::Gesture::Press,
                        action,
                    });
                }
            }
            let (mut app, _, _) = fixture(false, profile);
            let selected = selection(&app);
            let initial = app.world().resource::<InputRes>().mapper.clone();
            let mut pad = Gamepad::default();
            pad.digital_mut().press(GamepadButton::South);
            pad.digital_mut().release(GamepadButton::South);
            app.world_mut().spawn(pad);
            let key = if mouse_chord {
                KeyCode::ControlLeft
            } else {
                KeyCode::KeyQ
            };
            {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                keys.press(key);
                keys.release(key);
            }
            click(&mut app, true);
            assert_eq!(
                selection(&app),
                selected,
                "delivered A={delivered_a}, mouse chord={mouse_chord}"
            );
            assert!(
                app.world()
                    .non_send::<SimRes>()
                    .session
                    .recorded
                    .entries()
                    .iter()
                    .all(|entry| matches!(entry.command, SimCommand::Stop { .. }))
            );
            assert_raw_replay(&app, &initial);
        }
    }
}

#[test]
fn direct_mode_priority_keeps_grid_uncertainty_conservative_with_alt_or_nonbuilder() {
    // Known bounded limitation: do not duplicate grid_index to prove that Alt
    // or the current army disables Grid. Such keys retain their old ordering.
    for alt in [false, true] {
        let mut profile = Profile::default_combined();
        profile.grid_build_keys = true;
        profile.bindings.push(tak_input::Binding {
            with_all: Vec::new(),
            input: tak_input::Input::Key(Key("q".into())),
            with: None,
            gesture: tak_input::Gesture::Press,
            action: Action::MoveMode,
        });
        let (mut app, window, _) = fixture(false, profile);
        if !alt {
            for id in selection(&app).iter() {
                app.world_mut()
                    .non_send_mut::<SimRes>()
                    .session
                    .sim
                    .world_mut()
                    .despawn(*id)
                    .unwrap();
            }
            app.world_mut()
                .non_send_mut::<SimRes>()
                .session
                .sim
                .world_mut()
                .spawn(
                    UnitDefId(3),
                    PlayerId(0),
                    tak_math::Vec2::from_ints(80, 40),
                    tak_math::Angle::ZERO,
                )
                .unwrap();
            perform(&mut app, Action::SelectAll { scope: Scope::Map });
        }
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(400.0, 300.0)));
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            if alt {
                keys.press(KeyCode::AltLeft);
                keys.release(KeyCode::AltLeft);
            }
            keys.press(KeyCode::KeyQ);
            keys.release(KeyCode::KeyQ);
        }
        click(&mut app, true);
        let recording = app
            .world()
            .resource::<Feel>()
            .recorder
            .as_ref()
            .unwrap()
            .recording();
        let events: Vec<_> = recording
            .frames
            .iter()
            .flat_map(|frame| &frame.events)
            .collect();
        let release = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    EventKind::ButtonUp {
                        button: InButton::MouseLeft
                    }
                )
            })
            .unwrap();
        let key_down = events
            .iter()
            .position(|event| matches!(event, EventKind::KeyDown { key } if key.0 == "q"))
            .unwrap();
        assert!(
            release < key_down,
            "grid uncertainty was silently promoted, alt={alt}"
        );
        assert!(
            !app.world()
                .non_send::<SimRes>()
                .session
                .recorded
                .entries()
                .iter()
                .any(|entry| matches!(entry.command, SimCommand::Build { .. }))
        );
    }
}

#[test]
fn direct_mode_priority_does_not_promote_an_alternate_gesture_stop() {
    let mut profile = Profile::named("original").unwrap();
    for (gesture, action) in [
        (tak_input::Gesture::Press, Action::MoveMode),
        (tak_input::Gesture::SecondTap, Action::Stop),
    ] {
        profile.bindings.push(tak_input::Binding {
            with_all: Vec::new(),
            input: tak_input::Input::Key(Key("q".into())),
            with: None,
            gesture,
            action,
        });
    }
    let (mut app, _, _) = fixture(false, profile);
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::KeyQ);
        keys.release(KeyCode::KeyQ);
    }
    frame(&mut app);
    perform(&mut app, Action::Cancel);
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::KeyQ);
        keys.release(KeyCode::KeyQ);
    }
    click(&mut app, true);
    let sim = app.world().non_send::<SimRes>();
    let commands = sim.session.recorded.entries();
    // Normal MouseDown ends Q's second-tap window. Prioritizing the apparent
    // Press binding would incorrectly fire its alternate Stop before Move.
    assert_eq!(
        commands.len(),
        1,
        "alternate Stop was promoted: {commands:?}"
    );
    assert!(matches!(commands[0].command, SimCommand::Move { .. }));
}

#[test]
fn direct_mode_priority_keeps_pad_primary_after_its_surface_switch() {
    use bevy::input::gamepad::GamepadInput;
    let (mut app, _) = hud_fixture(None);
    let selected = selection(&app);
    let mut profile = Profile::default_combined();
    profile.tuning.reticle = true;
    profile
        .bindings
        .retain(|binding| binding.input != tak_input::Input::Button(InButton::A));
    profile.bindings.push(tak_input::Binding {
        with_all: Vec::new(),
        input: tak_input::Input::Button(InButton::A),
        with: None,
        gesture: tak_input::Gesture::Press,
        action: Action::MoveMode,
    });
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(profile);
    let mut pad = Gamepad::default();
    pad.analog_mut()
        .set(GamepadInput::Axis(GamepadAxis::LeftStickX), 1.0);
    pad.digital_mut().press(GamepadButton::South);
    app.world_mut().spawn(pad);
    frame(&mut app);
    assert_eq!(
        app.world().resource::<InputRes>().mapper.mode(),
        Mode::Move,
        "A must see the stick's centered reticle, not the stale mouse HUD surface"
    );
    assert_eq!(selection(&app), selected);
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
    assert!(!app.world().resource::<MinimapState>().pad_swallow);
}

#[test]
fn direct_mode_priority_keeps_controller_primary_and_repeat_button_order_unchanged() {
    use bevy::input::gamepad::GamepadInput;
    for (button, gamepad_button) in [
        (InButton::A, GamepadButton::South),
        (InButton::Y, GamepadButton::North),
    ] {
        let (mut app, _) = hud_fixture(None);
        let mut profile = Profile::default_combined();
        profile.tuning.reticle = true;
        profile
            .bindings
            .retain(|binding| binding.input != tak_input::Input::Button(button));
        profile.bindings.push(tak_input::Binding {
            with_all: Vec::new(),
            input: tak_input::Input::Button(button),
            with: None,
            gesture: tak_input::Gesture::Press,
            action: Action::MoveMode,
        });
        app.world_mut()
            .resource_mut::<InputRes>()
            .mapper
            .set_profile(profile);
        let mut pad = Gamepad::default();
        pad.analog_mut()
            .set(GamepadInput::Axis(GamepadAxis::LeftStickX), 1.0);
        pad.digital_mut().press(gamepad_button);
        app.world_mut().spawn(pad);
        frame(&mut app);
        let recording = app
            .world()
            .resource::<Feel>()
            .recorder
            .as_ref()
            .unwrap()
            .recording();
        let events: Vec<_> = recording
            .frames
            .iter()
            .flat_map(|frame| &frame.events)
            .collect();
        let axis = events.iter().position(|event| matches!(event, EventKind::Axis { axis: InAxis::LeftX, value } if *value != 0)).unwrap();
        let down = events.iter().position(|event| matches!(event, EventKind::ButtonDown { button: actual } if *actual == button)).unwrap();
        assert!(
            axis < down,
            "new keyboard mode priority reordered controller {button:?}"
        );
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::Move);
    }
}
