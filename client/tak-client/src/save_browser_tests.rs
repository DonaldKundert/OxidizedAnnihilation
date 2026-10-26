//! Actual Bevy modal adapter tests, using only a synthetic asset-free match.
use super::*;
use clap::Parser;
use tak_commands::{QueueMode, Scope, SimCommand};
use tak_input::{Action, Output};

fn fixture() -> App {
    let mut app = super::super::overlay_input_tests::app();
    let args = Args::parse_from(["test", "--data", "/synthetic-not-read"]);
    app.insert_resource(BrowserArgs(args))
        .init_resource::<BrowserState>()
        .insert_non_send(PendingLoad::default());
    app.world_mut().resource_mut::<Feel>().overlay_open = false;
    app
}

fn key(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
    app.update();
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.clear();
    keys.release(key);
}

#[test]
fn keyboard_open_close_captures_conflicting_keys_and_restores_original_pause() {
    for originally_paused in [false, true] {
        let mut app = fixture();
        app.world_mut().non_send_mut::<SimRes>().paused = originally_paused;
        key(&mut app, KeyCode::F8);
        assert!(app.world().resource::<BrowserState>().open);
        assert!(app.world().non_send::<SimRes>().paused);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Pause);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        key(&mut app, KeyCode::Escape);
        assert!(!app.world().resource::<BrowserState>().open);
        assert!(app.world().resource::<BrowserState>().captures_input());
        assert_eq!(app.world().non_send::<SimRes>().paused, originally_paused);
        assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.update();
        assert!(!app.world().resource::<BrowserState>().captures_input());
    }
}

#[test]
fn mouse_launcher_owns_opening_click_instead_of_issuing_world_orders() {
    for profile in [
        tak_input::Profile::default_combined(),
        tak_input::Profile::classic_tak(),
        tak_input::Profile::original_tak(),
    ] {
        let mut app = fixture();
        app.world_mut()
            .resource_scope(|world, mut input: Mut<InputRes>| {
                let sim = world.non_send::<SimRes>();
                input.mapper.set_profile(profile);
                input.mapper.perform(
                    Action::SelectAll { scope: Scope::Map },
                    &sim.view(),
                    sim.session.tick(),
                );
                assert!(!input.mapper.selection().is_empty());
            });
        let selection = app
            .world()
            .resource::<InputRes>()
            .mapper
            .selection()
            .clone();
        // A top-right launcher click with a selected unit used to become a
        // ground order when the production tuple's input systems were unordered.
        let p = Vec2::new(740.0, 20.0);
        let mut window = Window {
            resolution: bevy::window::WindowResolution::new(800, 600),
            ..default()
        };
        window.set_cursor_position(Some(p));
        app.world_mut().spawn((window, PrimaryWindow));
        app.world_mut().spawn((
            Control::Open,
            ComputedNode {
                size: Vec2::new(100.0, 30.0),
                ..default()
            },
            UiGlobalTransform::from(bevy::math::Affine2::from_translation(p)),
            InheritedVisibility::VISIBLE,
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(app.world().resource::<BrowserState>().open);
        assert!(app.world().non_send::<SimRes>().paused);
        assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
        assert_eq!(
            browser::pending_orders(&app.world().non_send::<SimRes>().session),
            0
        );
        assert_eq!(
            app.world().resource::<InputRes>().mapper.selection(),
            &selection
        );
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            mouse.clear();
            mouse.release(MouseButton::Left);
        }
        app.update();
        key(&mut app, KeyCode::Escape);
        app.update();
        assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
        assert_eq!(
            browser::pending_orders(&app.world().non_send::<SimRes>().session),
            0
        );
    }
}

#[test]
fn controller_can_open_from_settings_cancel_confirmation_then_close() {
    let mut app = fixture();
    app.world_mut().resource_mut::<Feel>().overlay_open = true;
    let mut pad = Gamepad::default();
    pad.digital_mut().press(GamepadButton::Select);
    let id = app.world_mut().spawn(pad).id();
    app.update();
    assert!(app.world().resource::<BrowserState>().open);
    assert!(!app.world().resource::<Feel>().overlay_open);
    app.world_mut()
        .resource_mut::<BrowserState>()
        .model
        .confirmation = Some(browser::Confirmation {
        operation: Operation::SaveNew,
        accept: false,
    });
    {
        let mut pad = app.world_mut().get_mut::<Gamepad>(id).unwrap();
        pad.digital_mut().clear();
        pad.digital_mut().release(GamepadButton::Select);
        pad.digital_mut().press(GamepadButton::South);
    }
    app.update();
    let state = app.world().resource::<BrowserState>();
    assert!(state.open);
    assert!(
        state.model.confirmation.is_none(),
        "A activates the default Cancel, not Confirm"
    );
    assert!(state.deferred.is_none());
    {
        let mut pad = app.world_mut().get_mut::<Gamepad>(id).unwrap();
        pad.digital_mut().clear();
        pad.digital_mut().release(GamepadButton::South);
        pad.digital_mut().press(GamepadButton::East);
    }
    app.update();
    assert!(!app.world().resource::<BrowserState>().open);
    assert!(!app.world().non_send::<SimRes>().paused);
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
}

#[test]
fn loaded_checkpoint_stays_paused_until_mouse_keyboard_or_controller_can_close() {
    let mut app = fixture();
    app.world_mut().resource_mut::<BrowserState>().loaded();
    app.world_mut().non_send_mut::<SimRes>().paused = true;
    assert!(app.world().resource::<BrowserState>().open);
    key(&mut app, KeyCode::Escape);
    assert!(!app.world().non_send::<SimRes>().paused);
}

#[test]
fn opening_drops_queue_brush_wheel_and_minimap_gestures_without_losing_selection() {
    let mut app = fixture();
    app.world_mut()
        .resource_scope(|world, mut input: Mut<InputRes>| {
            let sim = world.non_send::<SimRes>();
            input.mapper.perform(
                Action::SelectAll { scope: Scope::Map },
                &sim.view(),
                sim.session.tick(),
            );
            let mut profile = input.mapper.profile().clone();
            for (name, action) in [
                ("test-queue", Action::QueueModifier),
                ("test-brush", Action::Brush),
                ("test-wheel", Action::CommandWheel),
            ] {
                profile.bindings.push(tak_input::Binding {
                    with_all: Vec::new(),
                    input: tak_input::Input::Key(tak_input::Key(name.into())),
                    with: None,
                    gesture: tak_input::Gesture::Press,
                    action,
                });
            }
            input.mapper.set_profile(profile);
            for name in ["test-queue", "test-brush", "test-wheel"] {
                input.mapper.handle(
                    &tak_input::InputEvent {
                        t: 0,
                        kind: tak_input::EventKind::KeyDown {
                            key: tak_input::Key(name.into()),
                        },
                    },
                    &sim.view(),
                    sim.session.tick(),
                );
            }
            assert!(input.mapper.wheel().is_some());
        });
    let selection = app
        .world()
        .resource::<InputRes>()
        .mapper
        .selection()
        .clone();
    {
        let mut mm = app.world_mut().resource_mut::<MinimapState>();
        mm.mouse_drag = true;
        mm.pad_swallow = true;
    }
    key(&mut app, KeyCode::F8);
    let mapper = &app.world().resource::<InputRes>().mapper;
    assert!(mapper.wheel().is_none());
    assert!(mapper.brush().is_none());
    assert_eq!(mapper.selection(), &selection);
    assert!(!app.world().resource::<MinimapState>().pad_swallow);
    key(&mut app, KeyCode::Escape);
    app.update();
    let outputs = app
        .world_mut()
        .resource_scope(|world, mut input: Mut<InputRes>| {
            let sim = world.non_send::<SimRes>();
            input
                .mapper
                .perform(Action::MoveMode, &sim.view(), sim.session.tick());
            input.mapper.perform(
                Action::Context { append: false },
                &sim.view(),
                sim.session.tick(),
            )
        });
    assert!(
        outputs.iter().any(|output| matches!(
            output,
            Output::Sim {
                command: SimCommand::Move {
                    queue: QueueMode::Replace,
                    ..
                },
                ..
            }
        )),
        "queue modifier leaked through browser: {outputs:?}"
    );
}

#[test]
fn cancel_wins_over_a_deferred_operation_and_repeated_save_key() {
    let mut app = fixture();
    key(&mut app, KeyCode::F8);
    key(&mut app, KeyCode::KeyS);
    assert!(app.world().resource::<BrowserState>().deferred.is_some());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyS);
    key(&mut app, KeyCode::Escape);
    let state = app.world().resource::<BrowserState>();
    assert!(state.open);
    assert!(state.deferred.is_none());
    assert!(state.model.message.starts_with("Cancelled before"));
    assert!(app.world().non_send::<PendingLoad>().prepared.is_none());
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
}
