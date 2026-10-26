//! Real Bevy adapter schedule, synthetic match, no retail assets or renderer.
use super::*;
use crate::skirmish::{Faction, Opponent};

fn fixture() -> App {
    let mut app = super::super::overlay_input_tests::app();
    app.insert_resource(SkirmishState::new(SetupConfig::default(), false))
        .init_resource::<PendingRequest>();
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
fn pad(app: &mut App, id: Entity, button: GamepadButton) {
    {
        let mut pad = app.world_mut().get_mut::<Gamepad>(id).unwrap();
        pad.digital_mut().clear();
        pad.digital_mut().press(button);
    }
    app.update();
    {
        let mut pad = app.world_mut().get_mut::<Gamepad>(id).unwrap();
        pad.digital_mut().clear();
        pad.digital_mut().release(button);
    }
}
#[test]
fn keyboard_setup_changes_options_and_replacement_defaults_to_cancel() {
    let mut app = fixture();
    key(&mut app, KeyCode::F7);
    assert!(app.world().resource::<SkirmishState>().open);
    assert!(app.world().non_send::<SimRes>().paused);
    key(&mut app, KeyCode::ArrowDown);
    key(&mut app, KeyCode::ArrowRight);
    assert_eq!(
        app.world().resource::<SkirmishState>().config.faction,
        Faction::Zhon
    );
    key(&mut app, KeyCode::ArrowDown);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        app.world().resource::<SkirmishState>().config.opponent,
        Opponent::PassivePractice
    );
    key(&mut app, KeyCode::ArrowDown);
    key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.world().resource::<SkirmishState>().confirmation,
        Some((_, false))
    ));
    key(&mut app, KeyCode::Enter);
    assert!(app.world().resource::<PendingRequest>().request.is_none());
    assert!(
        app.world()
            .resource::<SkirmishState>()
            .confirmation
            .is_none()
    );
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::ArrowRight);
    key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.world().resource::<PendingRequest>().request,
        Some(Request::Launch(SetupConfig {
            faction: Faction::Zhon,
            opponent: Opponent::PassivePractice,
            ..
        }))
    ));
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
}
#[test]
fn close_preserves_pause_and_captures_held_buttons_until_release() {
    for paused in [false, true] {
        let mut app = fixture();
        app.world_mut().non_send_mut::<SimRes>().paused = paused;
        key(&mut app, KeyCode::F7);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Pause);
        key(&mut app, KeyCode::Escape);
        assert!(!app.world().resource::<SkirmishState>().open);
        assert_eq!(app.world().non_send::<SimRes>().paused, paused);
        app.update();
        assert!(app.world().resource::<SkirmishState>().captures_input());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.update();
        app.update();
        assert!(!app.world().resource::<SkirmishState>().captures_input());
        assert_eq!(app.world().non_send::<SimRes>().paused, paused);
    }
}
#[test]
fn controller_settings_route_is_exclusive_and_cancel_cannot_launch() {
    let mut app = fixture();
    app.world_mut().resource_mut::<Feel>().overlay_open = true;
    let id = app.world_mut().spawn(Gamepad::default()).id();
    pad(&mut app, id, GamepadButton::RightThumb);
    assert!(app.world().resource::<SkirmishState>().open);
    assert!(!app.world().resource::<Feel>().overlay_open);
    for _ in 0..3 {
        pad(&mut app, id, GamepadButton::DPadDown);
    }
    pad(&mut app, id, GamepadButton::South);
    pad(&mut app, id, GamepadButton::South);
    assert!(app.world().resource::<PendingRequest>().request.is_none());
    key(&mut app, KeyCode::F8);
    assert!(
        !app.world()
            .resource::<save_browser::BrowserState>()
            .captures_input()
    );
    pad(&mut app, id, GamepadButton::East);
    assert!(!app.world().resource::<SkirmishState>().open);
}
#[test]
fn checkpoint_modal_cannot_be_stolen_by_skirmish_shortcut() {
    let mut app = fixture();
    key(&mut app, KeyCode::F8);
    key(&mut app, KeyCode::F7);
    assert!(
        app.world()
            .resource::<save_browser::BrowserState>()
            .captures_input()
    );
    assert!(!app.world().resource::<SkirmishState>().open);
}
#[test]
fn mouse_launcher_captures_world_click_before_gameplay() {
    let mut app = fixture();
    let p = Vec2::new(740.0, 60.0);
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
    assert!(app.world().resource::<SkirmishState>().open);
    assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
    assert_eq!(
        crate::saves::browser::pending_orders(&app.world().non_send::<SimRes>().session),
        0
    );
}
#[test]
fn startup_requires_neutral_input_and_launches_without_discard_confirmation() {
    let mut app = fixture();
    app.insert_resource(SkirmishState::new(SetupConfig::default(), true));
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Enter);
    app.update();
    app.update();
    assert!(app.world().resource::<PendingRequest>().request.is_none());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.update();
    for _ in 0..3 {
        key(&mut app, KeyCode::ArrowDown);
    }
    key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.world().resource::<PendingRequest>().request,
        Some(Request::Launch(_))
    ));
    assert!(
        app.world()
            .resource::<SkirmishState>()
            .confirmation
            .is_none()
    );
}
#[test]
fn result_setup_never_replaces_match_and_rematch_requires_confirmation() {
    let mut app = fixture();
    {
        let mut state = app.world_mut().resource_mut::<SkirmishState>();
        state.open(Page::Result, false);
        state.result = "Victory".into();
        state.outcome_seen = true;
    }
    let before = app.world().non_send::<SimRes>().session.tick();
    key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.world().resource::<SkirmishState>().confirmation,
        Some((Request::Rematch(_), false))
    ));
    key(&mut app, KeyCode::Escape);
    key(&mut app, KeyCode::ArrowDown);
    key(&mut app, KeyCode::Enter);
    assert!(app.world().resource::<SkirmishState>().page == Page::Setup);
    assert!(app.world().resource::<PendingRequest>().request.is_none());
    assert_eq!(app.world().non_send::<SimRes>().session.tick(), before);
}
#[test]
fn preparation_failure_keeps_modal_match_and_config_available() {
    let mut app = fixture();
    key(&mut app, KeyCode::F7);
    {
        let mut state = app.world_mut().resource_mut::<SkirmishState>();
        state.waiting = true;
        state.report_error("synthetic asset failure".into());
    }
    let state = app.world().resource::<SkirmishState>();
    assert!(state.open);
    assert!(!state.waiting);
    assert!(state.message.contains("unchanged"));
    assert!(app.world().non_send::<SimRes>().paused);
}
#[test]
fn actual_victory_and_defeat_open_once_without_replacing_session() {
    for (loser, title) in [(1, "Victory"), (0, "Defeat")] {
        let mut app = fixture();
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let game = &mut sim.session.sim;
            let id = game.world().players()[loser].monarch.unwrap();
            let pos = game.world().unit(id).unwrap().pos;
            let now = game.tick().0;
            tak_sim::combat::apply_damage(
                game.world_mut(),
                &mut tak_sim::NoScripts,
                id,
                1_000_000,
                None,
                tak_sim::combat::kind::NORMAL,
                pos,
                now,
            );
            for _ in 0..=super::super::BANNER_DELAY_TICKS {
                game.step(&[]);
            }
        }
        app.update();
        let state = app.world().resource::<SkirmishState>();
        assert!(state.open);
        assert!(state.page == Page::Result);
        assert!(state.result.starts_with(title));
        assert!(app.world().resource::<PendingRequest>().request.is_none());
        app.update(); // release automatic modal arming guard
        key(&mut app, KeyCode::Escape);
        app.update();
        app.update();
        assert!(
            !app.world().resource::<SkirmishState>().open,
            "dismissed result must not automatically reopen"
        );
        key(&mut app, KeyCode::F7);
        assert!(app.world().resource::<SkirmishState>().page == Page::Result);
    }
}
#[test]
fn opening_clears_mapper_gestures_and_minimap_drag_without_clearing_selection() {
    let mut app = fixture();
    app.world_mut()
        .resource_scope(|world, mut input: Mut<InputRes>| {
            let sim = world.non_send::<SimRes>();
            input.mapper.perform(
                tak_input::Action::SelectAll {
                    scope: tak_commands::Scope::Map,
                },
                &sim.view(),
                sim.session.tick(),
            );
            let mut profile = input.mapper.profile().clone();
            profile.bindings.push(tak_input::Binding {
                with_all: Vec::new(),
                input: tak_input::Input::Key(tak_input::Key("test-wheel".into())),
                with: None,
                gesture: tak_input::Gesture::Press,
                action: tak_input::Action::CommandWheel,
            });
            input.mapper.set_profile(profile);
            input.mapper.handle(
                &tak_input::InputEvent {
                    t: 0,
                    kind: tak_input::EventKind::KeyDown {
                        key: tak_input::Key("test-wheel".into()),
                    },
                },
                &sim.view(),
                sim.session.tick(),
            );
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
        mm.ui_held = [true; 3];
        mm.pad_swallow = true;
    }
    key(&mut app, KeyCode::F7);
    let input = app.world().resource::<InputRes>();
    assert!(input.mapper.wheel().is_none());
    assert_eq!(input.mapper.selection(), &selection);
    assert!(input.last_axes.is_empty());
    let mm = app.world().resource::<MinimapState>();
    assert!(!mm.mouse_drag && mm.ui_held == [false; 3] && !mm.pad_swallow);
}
#[test]
fn verification_modes_cannot_open_or_launch_skirmish() {
    let mut app = fixture();
    app.world_mut()
        .resource_mut::<save_browser::BrowserArgs>()
        .0
        .record = Some("unused.jsonl".into());
    key(&mut app, KeyCode::F7);
    assert!(!app.world().resource::<SkirmishState>().open);
    assert!(app.world().resource::<PendingRequest>().request.is_none());
}
#[test]
fn held_stick_must_center_before_gameplay_resumes() {
    use bevy::input::gamepad::GamepadAxis;
    let mut app = fixture();
    let id = app.world_mut().spawn(Gamepad::default()).id();
    key(&mut app, KeyCode::F7);
    app.world_mut()
        .get_mut::<Gamepad>(id)
        .unwrap()
        .analog_mut()
        .set(GamepadAxis::LeftStickX, 0.8);
    key(&mut app, KeyCode::Escape);
    app.update();
    assert!(app.world().resource::<SkirmishState>().captures_input());
    assert!(app.world().resource::<InputRes>().last_axes.is_empty());
    app.world_mut()
        .get_mut::<Gamepad>(id)
        .unwrap()
        .analog_mut()
        .set(GamepadAxis::LeftStickX, 0.0);
    app.update();
    app.update();
    assert!(!app.world().resource::<SkirmishState>().captures_input());
}
#[test]
fn startup_cancel_keeps_unselected_preview_paused() {
    let mut app = fixture();
    app.insert_resource(SkirmishState::new(SetupConfig::default(), true));
    app.update();
    app.update();
    key(&mut app, KeyCode::Escape);
    let state = app.world().resource::<SkirmishState>();
    assert!(state.open && state.startup);
    assert!(state.message.contains("No match is running"));
    assert!(app.world().non_send::<SimRes>().paused);
    key(&mut app, KeyCode::ArrowUp);
    assert_eq!(
        app.world().resource::<SkirmishState>().selected,
        3,
        "startup has no Back to current match row"
    );
    assert!(app.world().resource::<PendingRequest>().request.is_none());
}
#[test]
fn launcher_is_hidden_for_verification_modes() {
    let mut app = fixture();
    app.world_mut()
        .resource_mut::<save_browser::BrowserArgs>()
        .0
        .hashes = Some("unused.txt".into());
    let id = app
        .world_mut()
        .spawn((Control::Open, Node::default(), BackgroundColor::default()))
        .id();
    app.add_systems(Update, draw);
    app.update();
    assert_eq!(app.world().get::<Node>(id).unwrap().display, Display::None);
}
#[test]
fn startup_cancel_clears_deferred_launch_without_leaving_setup() {
    let mut app = fixture();
    app.insert_resource(SkirmishState::new(SetupConfig::default(), true));
    app.update();
    app.update();
    for _ in 0..3 {
        key(&mut app, KeyCode::ArrowDown);
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.world().resource::<PendingRequest>().request.is_some());
    key(&mut app, KeyCode::Escape);
    assert!(app.world().resource::<PendingRequest>().request.is_none());
    let state = app.world().resource::<SkirmishState>();
    assert!(state.open && state.startup && !state.waiting);
    assert!(app.world().non_send::<SimRes>().paused);
}
#[test]
fn unsupported_result_never_displays_stale_setup_as_live_configuration() {
    let mut app = fixture();
    {
        let mut state = app.world_mut().resource_mut::<SkirmishState>();
        state.checkpoint_loaded(None);
        state.open(Page::Result, false);
        state.outcome_seen = true;
    }
    let id = app.world_mut().spawn((Label::Status, Text::new(""))).id();
    app.add_systems(Update, draw);
    app.update();
    let status = &app.world().get::<Text>(id).unwrap().0;
    assert!(status.contains("Rematch is unavailable"));
    assert!(!status.contains("Enemy faction"));
    assert!(!status.contains("omniscient"));
}

/// Continuous controller adapter acceptance flow. Replacement callbacks stand
/// in for the separately tested transactional renderer/session replacement;
/// this deliberately makes no physical-controller or real-asset claim.
#[test]
fn continuous_pad_startup_setup_cancel_result_rematch_and_checkpoint_callbacks() {
    for (loser, title) in [(1, "Victory"), (0, "Defeat")] {
        let mut app = fixture();
        app.insert_resource(SkirmishState::new(SetupConfig::default(), true));
        let id = app.world_mut().spawn(Gamepad::default()).id();
        app.update();
        app.update();
        pad(&mut app, id, GamepadButton::DPadDown);
        pad(&mut app, id, GamepadButton::DPadRight);
        pad(&mut app, id, GamepadButton::DPadDown);
        pad(&mut app, id, GamepadButton::DPadRight);
        pad(&mut app, id, GamepadButton::DPadDown);
        pad(&mut app, id, GamepadButton::South);
        let Some(Request::Launch(config)) = app
            .world_mut()
            .resource_mut::<PendingRequest>()
            .request
            .take()
        else {
            panic!("startup did not request launch");
        };
        assert_eq!(config.faction, Faction::Zhon);
        assert_eq!(config.opponent, Opponent::PassivePractice);
        app.world_mut()
            .resource_mut::<SkirmishState>()
            .launched(config);
        app.world_mut().non_send_mut::<SimRes>().paused = false;
        app.update();
        app.update();
        // Exercise the actual Menu -> settings -> R3 route, not direct state.
        pad(&mut app, id, GamepadButton::Start);
        assert!(app.world().resource::<Feel>().overlay_open);
        pad(&mut app, id, GamepadButton::RightThumb);
        for _ in 0..3 {
            pad(&mut app, id, GamepadButton::DPadDown);
        }
        pad(&mut app, id, GamepadButton::South);
        assert!(matches!(
            app.world().resource::<SkirmishState>().confirmation,
            Some((_, false))
        ));
        pad(&mut app, id, GamepadButton::South); // default Cancel
        assert!(app.world().resource::<PendingRequest>().request.is_none());
        pad(&mut app, id, GamepadButton::East);
        app.update();
        app.update();
        assert!(!app.world().resource::<SkirmishState>().captures_input());
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let game = &mut sim.session.sim;
            let monarch = game.world().players()[loser].monarch.unwrap();
            let pos = game.world().unit(monarch).unwrap().pos;
            let now = game.tick().0;
            tak_sim::combat::apply_damage(
                game.world_mut(),
                &mut tak_sim::NoScripts,
                monarch,
                1_000_000,
                None,
                tak_sim::combat::kind::NORMAL,
                pos,
                now,
            );
            for _ in 0..=super::super::BANNER_DELAY_TICKS {
                game.step(&[]);
            }
        }
        app.update();
        app.update();
        assert!(
            app.world()
                .resource::<SkirmishState>()
                .result
                .starts_with(title)
        );
        pad(&mut app, id, GamepadButton::South);
        pad(&mut app, id, GamepadButton::DPadRight);
        pad(&mut app, id, GamepadButton::South);
        let Some(Request::Rematch(rematch)) = app
            .world_mut()
            .resource_mut::<PendingRequest>()
            .request
            .take()
        else {
            panic!("result did not request rematch");
        };
        assert_eq!(rematch, config);
        app.world_mut()
            .resource_mut::<SkirmishState>()
            .launched(rematch);
        assert!(!app.world().resource::<SkirmishState>().outcome_seen);
        // The checkpoint transition receives inference from the real restored
        // session. Supported configuration preserves exact seed; unsupported
        // synthetic sessions must invalidate the preceding match's rematch.
        app.world_mut()
            .resource_mut::<SkirmishState>()
            .checkpoint_loaded(Some(config));
        assert_eq!(
            app.world().resource::<SkirmishState>().rematch,
            Some(config)
        );
        let inferred = crate::skirmish::from_session(&app.world().non_send::<SimRes>().session);
        assert!(inferred.is_none());
        app.world_mut()
            .resource_mut::<SkirmishState>()
            .checkpoint_loaded(inferred);
        assert!(app.world().resource::<SkirmishState>().rematch.is_none());
        assert!(!app.world().resource::<SkirmishState>().captures_input());
        assert!(app.world().non_send::<SimRes>().session.recorded.is_empty());
    }
}
#[test]
fn committed_launch_clears_startup_and_requires_release_before_reopening() {
    let mut app = fixture();
    app.insert_resource(SkirmishState::new(SetupConfig::default(), true));
    app.update();
    app.update();
    for _ in 0..3 {
        key(&mut app, KeyCode::ArrowDown);
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Enter);
    app.update();
    let Some(Request::Launch(config)) = app
        .world_mut()
        .resource_mut::<PendingRequest>()
        .request
        .take()
    else {
        panic!("expected initial launch");
    };
    app.world_mut()
        .resource_mut::<SkirmishState>()
        .launched(config);
    app.world_mut().non_send_mut::<SimRes>().paused = false;
    // Enter remains physically held, including stale just_pressed in this
    // asset-free harness: no second submission or modal reopening may occur.
    app.update();
    app.update();
    let state = app.world().resource::<SkirmishState>();
    assert!(!state.startup && !state.open && state.captures_input());
    assert!(app.world().resource::<PendingRequest>().request.is_none());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.update();
    app.update();
    key(&mut app, KeyCode::F7);
    for _ in 0..3 {
        key(&mut app, KeyCode::ArrowDown);
    }
    key(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.world().resource::<SkirmishState>().confirmation,
        Some((_, false))
    ));
    assert!(app.world().resource::<PendingRequest>().request.is_none());
}
