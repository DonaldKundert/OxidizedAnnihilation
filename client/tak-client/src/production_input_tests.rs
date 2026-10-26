//! Asset-free regressions for production controls through the real Bevy adapter.
use super::*;
use tak_commands::{QueueMode, SimCommand, UnitDefId, UnitSet};

const HUD_POS: Vec2 = Vec2::new(650.0, 300.0);
const WORLD_POS: Vec2 = Vec2::new(400.0, 300.0);
const MOUSE_BUTTONS: [MouseButton; 3] =
    [MouseButton::Left, MouseButton::Right, MouseButton::Middle];

fn replace_sim(app: &mut App, scenario: &tak_sim::Scenario) {
    let mut sim = tak_sim::Sim::new(scenario).unwrap();
    sim.step(&[]);
    let map = app.world().non_send::<SimRes>().session.map.clone();
    let session =
        Session::from_sim(&Vfs::new(), "synthetic", map, sim, Default::default(), None).unwrap();
    let snap = session.snapshot();
    let mut state = app.world_mut().non_send_mut::<SimRes>();
    state.session = session;
    state.prev = snap.clone();
    state.curr = snap;
}

fn fixture(profile: Profile, factory: bool) -> (App, Entity, Entity) {
    let mut app = super::overlay_input_tests::app();
    if factory {
        // Reuse only the hand-authored training definitions, never game assets.
        let mut scenario = tak_headless::bot::demo::scenario(false);
        scenario.players[0].monarch = UnitDefId(2);
        scenario.unit_defs[2].economy.mogriumstorage = 1000;
        // A real ready spell makes accidental Y tap/hold fallthrough visible.
        scenario.unit_defs[2].economy.maxmana = 100;
        scenario.economy.rules.monarch_start_mana_percent = 100;
        scenario.unit_defs[2]
            .combat
            .weapons
            .push(tak_sim::combat::WeaponStats {
                name: "synthetic_factory_spell".into(),
                kind: tak_sim::combat::WeaponKind::LineOfSight,
                range: 80,
                reload_ticks: 8,
                mana_per_shot: 1,
                ..default()
            });
        let mut alternate = scenario.unit_defs[3].clone();
        alternate.name = "synthetic_alternate_product".into();
        scenario.unit_defs.push(alternate);
        scenario.unit_defs[2].build.canbuild.push(UnitDefId(5));
        replace_sim(&mut app, &scenario);
    }
    app.world_mut().resource_mut::<Feel>().overlay_open = false;
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(profile);
    let mut camera = app.world_mut().resource_mut::<CamState>();
    camera.geom.target = [320.0, 160.0];
    camera.geom.zoom = 0.25;
    let mut window = Window {
        resolution: WindowResolution::new(800, 600),
        ..default()
    };
    window.set_cursor_position(Some(HUD_POS));
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    let panel = app
        .world_mut()
        .spawn((
            HudPanel,
            ComputedNode {
                size: Vec2::new(180.0, 140.0),
                ..default()
            },
            UiGlobalTransform::from(bevy::math::Affine2::from_translation(HUD_POS)),
            InheritedVisibility::VISIBLE,
        ))
        .id();
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
        });
    assert_eq!(
        app.world().resource::<InputRes>().mapper.selection().len(),
        1
    );
    (app, window, panel)
}

fn button(app: &mut App, kind: ButtonKind) -> Entity {
    app.world_mut()
        .spawn((
            kind,
            ComputedNode {
                size: Vec2::splat(40.0),
                ..default()
            },
            UiGlobalTransform::from(bevy::math::Affine2::from_translation(HUD_POS)),
            InheritedVisibility::VISIBLE,
        ))
        .id()
}

fn frame(app: &mut App) {
    app.update();
    // Match Bevy's per-frame input cleanup while preserving held buttons.
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
}

fn pointer(app: &mut App, window: Entity, pos: Vec2) {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(pos));
}

fn click(app: &mut App, mouse: MouseButton) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(mouse);
    frame(app);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(mouse);
    frame(app);
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

fn order_on_every_mouse_edge() -> Profile {
    let mut profile = Profile::classic_tak();
    let inputs = [
        InButton::MouseLeft,
        InButton::MouseRight,
        InButton::MouseMiddle,
    ];
    profile.bindings.retain(|binding| {
        !inputs
            .iter()
            .any(|button| binding.input == tak_input::Input::Button(*button))
    });
    for button in inputs {
        for gesture in [tak_input::Gesture::Press, tak_input::Gesture::Release] {
            profile.bindings.push(tak_input::Binding {
                input: tak_input::Input::Button(button),
                with: None,
                with_all: Vec::new(),
                gesture,
                action: Action::Context { append: false },
            });
        }
    }
    profile
}

#[test]
fn raw_hud_right_click_never_reaches_world_or_ordinary_button_action() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for kind in [None, Some(ButtonKind::Panel(0))] {
            let (mut app, window, _) = fixture(profile.clone(), false);
            if let Some(kind) = kind {
                button(&mut app, kind);
            }
            let selected = selection(&app);
            let camera = app.world().resource::<CamState>().geom.target;
            click(&mut app, MouseButton::Right);
            assert!(commands(&app).is_empty());
            assert_eq!(selection(&app), selected);
            assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
            assert_eq!(app.world().resource::<CamState>().geom.target, camera);
            assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);

            // The same raw button must still work once it starts on terrain.
            pointer(&mut app, window, WORLD_POS);
            click(&mut app, MouseButton::Right);
            assert!(matches!(
                commands(&app).as_slice(),
                [SimCommand::Move { .. }]
            ));
        }
    }
}

#[test]
fn hud_origin_captures_all_mouse_buttons_until_release_outside_panel() {
    for (index, mouse) in MOUSE_BUTTONS.into_iter().enumerate() {
        let (mut app, window, _) = fixture(order_on_every_mouse_edge(), false);
        let selected = selection(&app);
        let camera = app.world().resource::<CamState>().geom.target;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(mouse);
        frame(&mut app);
        assert!(app.world().resource::<MinimapState>().ui_held[index]);
        pointer(&mut app, window, WORLD_POS);
        frame(&mut app);
        assert!(app.world().resource::<MinimapState>().ui_held[index]);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(mouse);
        frame(&mut app);
        assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
        assert!(commands(&app).is_empty(), "{mouse:?} leaked a world order");
        assert_eq!(selection(&app), selected);
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);

        click(&mut app, mouse);
        assert!(!commands(&app).is_empty(), "{mouse:?} remained captured");
    }
}

#[test]
fn hidden_hud_panel_does_not_swallow_any_world_mouse_button() {
    for mouse in MOUSE_BUTTONS {
        let (mut app, _, panel) = fixture(order_on_every_mouse_edge(), false);
        *app.world_mut()
            .get_mut::<InheritedVisibility>(panel)
            .unwrap() = InheritedVisibility::HIDDEN;
        click(&mut app, mouse);
        let sent = commands(&app);
        assert!(!sent.is_empty(), "hidden panel swallowed {mouse:?}");
        assert!(
            sent.iter()
                .all(|command| matches!(command, SimCommand::Move { .. }))
        );
        assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
    }
}

#[test]
fn simultaneous_hud_holds_release_independently_without_leaking() {
    let (mut app, window, _) = fixture(order_on_every_mouse_edge(), false);
    for mouse in MOUSE_BUTTONS {
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(mouse);
    }
    frame(&mut app);
    assert_eq!(app.world().resource::<MinimapState>().ui_held, [true; 3]);
    pointer(&mut app, window, WORLD_POS);
    for (mouse, expected) in [
        (MouseButton::Right, [true, false, true]),
        (MouseButton::Left, [false, false, true]),
        (MouseButton::Middle, [false; 3]),
    ] {
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(mouse);
        frame(&mut app);
        assert_eq!(app.world().resource::<MinimapState>().ui_held, expected);
        assert!(commands(&app).is_empty());
    }
    click(&mut app, MouseButton::Right);
    assert!(!commands(&app).is_empty());
}

#[test]
fn captured_left_button_does_not_swallow_unrelated_world_right_button() {
    let (mut app, window, _) = fixture(Profile::classic_tak(), false);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    frame(&mut app);
    pointer(&mut app, window, WORLD_POS);
    click(&mut app, MouseButton::Right);
    assert!(matches!(
        commands(&app).as_slice(),
        [SimCommand::Move { .. }]
    ));
    assert_eq!(
        app.world().resource::<MinimapState>().ui_held,
        [true, false, false]
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    frame(&mut app);
    assert_eq!(commands(&app).len(), 1);
    assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
}

#[test]
fn opening_save_or_skirmish_modal_resets_all_hud_captures() {
    for opening_key in [KeyCode::F8, KeyCode::F7] {
        let (mut app, window, _) = fixture(order_on_every_mouse_edge(), false);
        if opening_key == KeyCode::F7 {
            app.insert_resource(skirmish_ui::SkirmishState::new(Default::default(), false));
            app.init_resource::<skirmish_ui::PendingRequest>();
        }
        // Keep the save-browser branch strictly synthetic and filesystem-free.
        app.world_mut().resource_mut::<Feel>().quick_save_dir =
            Err("Synthetic adapter test has no checkpoint directory".into());
        for mouse in MOUSE_BUTTONS {
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(mouse);
        }
        frame(&mut app);
        assert_eq!(app.world().resource::<MinimapState>().ui_held, [true; 3]);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(opening_key);
        frame(&mut app);
        assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
        assert!(app.world().resource::<Feel>().overlay_captured_input);
        pointer(&mut app, window, WORLD_POS);
        for mouse in MOUSE_BUTTONS {
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(mouse);
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(opening_key);
        frame(&mut app);
        assert!(commands(&app).is_empty());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        frame(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Escape);
        frame(&mut app);
        frame(&mut app);
        click(&mut app, MouseButton::Right);
        assert!(
            !commands(&app).is_empty(),
            "{opening_key:?} left stale capture"
        );
    }
}

#[test]
fn production_buttons_append_one_or_five_and_cancel_one_or_five_without_world_input() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for shifted in [false, true] {
            let (mut app, _, _) = fixture(profile.clone(), true);
            button(&mut app, ButtonKind::Build(1));
            let selected = selection(&app);
            let camera = app.world().resource::<CamState>().geom.target;
            let zoom = app.world().resource::<CamState>().geom.zoom;
            if shifted {
                // Deliberately arrive on the same frame as the portrait click.
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::ShiftLeft);
            }
            click(&mut app, MouseButton::Left);
            let count = if shifted { 5 } else { 1 };
            let sent = commands(&app);
            assert_eq!(sent.len(), count);
            assert!(sent.iter().all(|command| matches!(command,
                SimCommand::Build { units, def: UnitDefId(5), at: None, queue: QueueMode::Append }
                if *units == selected)));
            app.world_mut().non_send_mut::<SimRes>().step().unwrap();
            {
                let sim = app.world().non_send::<SimRes>();
                assert_eq!(
                    app.world()
                        .resource::<InputRes>()
                        .mapper
                        .factory_production_count(1, &sim.view()),
                    Some(count as u32),
                );
            }
            click(&mut app, MouseButton::Right);
            let sent = commands(&app);
            assert_eq!(sent.len(), count + 1);
            assert!(matches!(sent.last().unwrap(),
                SimCommand::CancelProduction { units, def: UnitDefId(5), count: cancelled }
                if *units == selected && *cancelled == count as u32));
            app.world_mut().non_send_mut::<SimRes>().step().unwrap();
            {
                let sim = app.world().non_send::<SimRes>();
                assert_eq!(
                    app.world()
                        .resource::<InputRes>()
                        .mapper
                        .factory_production_count(1, &sim.view()),
                    Some(0),
                );
            }
            assert_eq!(selection(&app), selected);
            assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
            assert_eq!(app.world().resource::<CamState>().geom.target, camera);
            assert_eq!(app.world().resource::<CamState>().geom.zoom, zoom);
            assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
        }
    }
}

fn pad_frame(app: &mut App, pad: Entity) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(20));
    frame(app);
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .digital_mut()
        .clear();
}

fn pad_button(app: &mut App, pad: Entity, button: GamepadButton, down: bool) {
    {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        if down {
            gamepad.digital_mut().press(button);
        } else {
            gamepad.digital_mut().release(button);
        }
    }
    pad_frame(app, pad);
}

fn open_pad_build_wheel(app: &mut App, pad: Entity) {
    use bevy::input::gamepad::GamepadInput;
    pad_button(app, pad, GamepadButton::LeftTrigger, true);
    let hold_ms = app.world().resource::<InputRes>().mapper.tuning().hold_ms;
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_millis(u64::from(hold_ms) + 1));
    pad_frame(app, pad);
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .wheel()
            .unwrap()
            .kind,
        tak_input::WheelKind::Build
    );
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .analog_mut()
        .set(GamepadInput::Axis(GamepadAxis::LeftStickY), 1.0);
    pad_frame(app, pad);
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .analog_mut()
        .set(GamepadInput::Axis(GamepadAxis::LeftStickY), 0.0);
    pad_frame(app, pad);
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .wheel()
            .unwrap()
            .highlighted,
        Some(0)
    );
}

fn pad_fixture(batch: bool) -> (App, Entity) {
    let (mut app, window, _) = fixture(Profile::default_combined(), true);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(None);
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    pad_frame(&mut app, pad);
    if batch {
        // RB must already be held when LB opens the build wheel.
        pad_button(&mut app, pad, GamepadButton::RightTrigger, true);
    }
    (app, pad)
}

#[test]
fn gamepad_factory_wheel_adds_and_removes_one_or_five_without_release_readding() {
    for batch in [false, true] {
        let (mut app, pad) = pad_fixture(batch);
        let selected = selection(&app);
        let camera = app.world().resource::<CamState>().geom.target;
        let zoom = app.world().resource::<CamState>().geom.zoom;
        let count = if batch { 5 } else { 1 };
        open_pad_build_wheel(&mut app, pad);
        pad_button(&mut app, pad, GamepadButton::South, true);
        pad_button(&mut app, pad, GamepadButton::South, false);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
        let added = commands(&app);
        assert_eq!(added.len(), count);
        assert!(added.iter().all(|command| matches!(command,
            SimCommand::Build { units, def: UnitDefId(3), at: None, queue: QueueMode::Append }
            if *units == selected)));
        pad_button(&mut app, pad, GamepadButton::LeftTrigger, false);
        assert_eq!(commands(&app), added, "releasing LB added the item again");

        open_pad_build_wheel(&mut app, pad);
        pad_button(&mut app, pad, GamepadButton::West, true);
        pad_button(&mut app, pad, GamepadButton::West, false);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
        let removed = commands(&app);
        assert_eq!(removed.len(), count + 1);
        assert!(matches!(removed.last().unwrap(),
            SimCommand::CancelProduction { units, def: UnitDefId(3), count: cancelled }
            if *units == selected && *cancelled == count as u32));
        pad_button(&mut app, pad, GamepadButton::LeftTrigger, false);
        if batch {
            pad_button(&mut app, pad, GamepadButton::RightTrigger, false);
        }
        pad_frame(&mut app, pad);
        assert_eq!(
            commands(&app),
            removed,
            "releasing the opener/modifier re-added production"
        );
        assert_eq!(selection(&app), selected);
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
        assert_eq!(
            app.world()
                .resource::<InputRes>()
                .mapper
                .production_batch_count(),
            1
        );
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);
        assert_eq!(app.world().resource::<CamState>().geom.zoom, zoom);
    }
}

#[test]
fn gamepad_factory_wheel_cancel_then_opener_release_does_not_build() {
    for batch in [false, true] {
        let (mut app, pad) = pad_fixture(batch);
        let selected = selection(&app);
        let camera = app.world().resource::<CamState>().geom.target;
        open_pad_build_wheel(&mut app, pad);
        pad_button(&mut app, pad, GamepadButton::East, true);
        pad_button(&mut app, pad, GamepadButton::East, false);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
        pad_button(&mut app, pad, GamepadButton::LeftTrigger, false);
        if batch {
            pad_button(&mut app, pad, GamepadButton::RightTrigger, false);
        }
        pad_frame(&mut app, pad);
        assert!(commands(&app).is_empty());
        assert_eq!(selection(&app), selected);
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);
    }
}

#[test]
fn gamepad_factory_stop_hold_consumes_old_wheel_releases_and_allows_fresh_build() {
    use GamepadButton::{East as B, LeftTrigger as LB, RightTrigger as RB};

    let release_orders = [
        [B, LB, RB],
        [B, RB, LB],
        [LB, B, RB],
        [LB, RB, B],
        [RB, B, LB],
        [RB, LB, B],
        [B, LB, RB], // All held buttons released in the same Bevy poll.
    ];
    for confirm_on_press in [false, true] {
        for batch in [false, true] {
            for (order, releases) in release_orders.iter().enumerate() {
                if !batch && ![0, 2, 6].contains(&order) {
                    continue;
                }
                let case =
                    format!("confirm_on_press={confirm_on_press}, batch={batch}, order={order}");
                let (mut app, pad) = pad_fixture(false);
                let mut profile = Profile::default_combined();
                profile.tuning.wheel_confirm_on_press = confirm_on_press;
                app.world_mut()
                    .resource_mut::<InputRes>()
                    .mapper
                    .set_profile(profile);
                let selected = selection(&app);
                let factory = *selected.iter().next().unwrap();
                let camera = app.world().resource::<CamState>().geom.target;
                let zoom = app.world().resource::<CamState>().geom.zoom;
                // Give Stop both paid finite production and a repeat recipe to
                // clear, without adding any earlier controller gesture.
                let product = {
                    let mut sim = app.world_mut().non_send_mut::<SimRes>();
                    sim.session.submit(SimCommand::Build {
                        units: selected.clone(),
                        def: UnitDefId(3),
                        at: None,
                        queue: QueueMode::Append,
                    });
                    sim.session.submit(SimCommand::SetFactoryRepeat {
                        units: selected.clone(),
                        def: Some(UnitDefId(3)),
                    });
                    for _ in 0..5 {
                        sim.step().unwrap();
                    }
                    let world = sim.session.sim.world();
                    let product = world.unit(factory).unwrap().build.factory.product.unwrap();
                    assert!(world.unit(product).unwrap().build.frame.unwrap().paid > 0);
                    product
                };
                let mut expected = commands(&app);
                expected.extend([
                    SimCommand::CancelProduction {
                        units: selected.clone(),
                        def: UnitDefId(3),
                        count: u32::MAX,
                    },
                    SimCommand::Stop {
                        units: selected.clone(),
                    },
                ]);
                if batch {
                    pad_button(&mut app, pad, RB, true);
                }
                open_pad_build_wheel(&mut app, pad);
                app.world_mut().resource_mut::<Feel>().feedback.clear();
                pad_button(&mut app, pad, B, true);
                let tuning = *app.world().resource::<InputRes>().mapper.tuning();
                app.world_mut().resource_mut::<Time>().advance_by(
                    std::time::Duration::from_millis(u64::from(tuning.hold_ms) + 1),
                );
                pad_frame(&mut app, pad);
                assert_eq!(commands(&app), expected, "Stop changed: {case}");

                // There is deliberately no A press after Stop. Releasing the
                // old B/LB/RB gesture alone must never confirm the old highlight.
                for &button in releases {
                    if button == RB && !batch {
                        continue;
                    }
                    app.world_mut()
                        .get_mut::<Gamepad>(pad)
                        .unwrap()
                        .digital_mut()
                        .release(button);
                    if order != 6 {
                        pad_frame(&mut app, pad);
                        assert_eq!(commands(&app), expected, "release {button:?}: {case}");
                    }
                }
                app.world_mut().resource_mut::<Time>().advance_by(
                    std::time::Duration::from_millis(
                        u64::from(tuning.hold_ms.max(tuning.double_tap_ms)) + 1,
                    ),
                );
                pad_frame(&mut app, pad);
                assert_eq!(
                    commands(&app),
                    expected,
                    "old gesture leaked an order: {case}"
                );
                assert_eq!(
                    selection(&app),
                    selected,
                    "B release cancelled selection: {case}"
                );
                assert_eq!(app.world().resource::<CamState>().geom.target, camera);
                assert_eq!(app.world().resource::<CamState>().geom.zoom, zoom);
                let mapper = &app.world().resource::<InputRes>().mapper;
                assert!(mapper.wheel().is_none(), "Stop left a stale wheel: {case}");
                assert_eq!(mapper.mode(), Mode::None);
                assert_eq!(mapper.production_batch_count(), 1);
                assert_eq!(
                    app.world()
                        .resource::<Feel>()
                        .feedback
                        .iter()
                        .filter(|feedback| { matches!(feedback, tak_input::Feedback::Order) })
                        .count(),
                    1,
                    "Stop must acknowledge once, with no release fallthrough: {case}"
                );
                {
                    let mut sim = app.world_mut().non_send_mut::<SimRes>();
                    sim.step().unwrap();
                    let world = sim.session.sim.world();
                    assert!(
                        world.unit(product).is_none(),
                        "Stop kept the paid frame: {case}"
                    );
                    let build = &world.unit(factory).unwrap().build;
                    assert!(build.factory.product.is_none());
                    assert!(build.factory_repeat.is_none());
                    assert_eq!(
                        tak_sim::construction::production_count(world, factory, UnitDefId(3)),
                        0
                    );
                }

                // Only a genuinely fresh LB gesture may build again. The old
                // RB is up, so even the batch case must now append just one.
                open_pad_build_wheel(&mut app, pad);
                if confirm_on_press {
                    pad_button(&mut app, pad, GamepadButton::South, true);
                    pad_button(&mut app, pad, GamepadButton::South, false);
                }
                pad_button(&mut app, pad, LB, false);
                pad_frame(&mut app, pad);
                expected.push(SimCommand::Build {
                    units: selected.clone(),
                    def: UnitDefId(3),
                    at: None,
                    queue: QueueMode::Append,
                });
                assert_eq!(commands(&app), expected, "fresh build failed: {case}");
                assert_eq!(selection(&app), selected);
                assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
            }
        }
    }
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

fn squad_fixture() -> (App, UnitSet, UnitSet) {
    let (mut app, _, _) = fixture(Profile::default_combined(), false);
    let mut scenario = tak_headless::bot::demo::scenario(false);
    scenario.initial_units = Some(
        [(0, 0, 250), (3, 0, 350), (4, 1, 528)]
            .into_iter()
            .map(|(def, owner, x)| tak_sim::campaign::InitialUnit {
                def: UnitDefId(def),
                owner: PlayerId(owner),
                pos: tak_math::Vec2::from_ints(x, 160),
                heading: tak_math::Angle::ZERO,
                health_percentage: 100,
                mana_percentage: 100,
                kills: 0,
            })
            .collect(),
    );
    replace_sim(&mut app, &scenario);
    perform(
        &mut app,
        Action::SelectCategory {
            category: tak_input::Category::Armed,
        },
    );
    let squad = selection(&app);
    assert_eq!(squad.len(), 1);
    perform(
        &mut app,
        Action::AssignGroup {
            group: tak_commands::ControlGroup::new(1).unwrap(),
        },
    );
    perform(
        &mut app,
        Action::SelectCategory {
            category: tak_input::Category::Builder,
        },
    );
    let initial = selection(&app);
    assert_eq!(initial.len(), 1);
    assert_ne!(initial, squad);
    (app, initial, squad)
}

fn assert_squad_union_then_unchanged_recall(app: &mut App, initial: &UnitSet, squad: &UnitSet) {
    let union: UnitSet = initial.iter().chain(squad.iter()).copied().collect();
    assert_eq!(
        selection(app),
        union,
        "additive recall replaced or assigned the squad"
    );
    assert!(commands(app).is_empty());
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Digit1);
    frame(app);
    assert_eq!(
        selection(app),
        *squad,
        "additive recall overwrote the stored squad"
    );
}

#[test]
fn same_frame_keyboard_chord_adds_squad_without_assigning_or_replacing_it() {
    for preheld_ctrl in [false, true] {
        for keys in [
            [KeyCode::Digit1, KeyCode::ShiftLeft, KeyCode::ControlLeft],
            [KeyCode::ControlLeft, KeyCode::Digit1, KeyCode::ShiftLeft],
            [KeyCode::ShiftLeft, KeyCode::ControlLeft, KeyCode::Digit1],
        ] {
            let (mut app, initial, squad) = squad_fixture();
            if preheld_ctrl {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::ControlLeft);
                frame(&mut app);
            }
            for key in keys {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(key);
            }
            frame(&mut app);
            for key in keys {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .release(key);
            }
            frame(&mut app);
            assert_squad_union_then_unchanged_recall(&mut app, &initial, &squad);
        }
    }
}

#[test]
fn gamepad_additive_squad_recall_keeps_modifier_until_simultaneous_release_completes() {
    let (mut app, initial, squad) = squad_fixture();
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    pad_button(&mut app, pad, GamepadButton::RightTrigger, true);
    pad_button(&mut app, pad, GamepadButton::DPadUp, true);
    {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad.digital_mut().release(GamepadButton::RightTrigger);
        gamepad.digital_mut().release(GamepadButton::DPadUp);
    }
    pad_frame(&mut app, pad);
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .production_batch_count(),
        1
    );
    assert_squad_union_then_unchanged_recall(&mut app, &initial, &squad);
}

#[test]
fn active_minimap_left_drag_does_not_exempt_other_buttons_from_hud_capture() {
    for mouse in [MouseButton::Right, MouseButton::Middle] {
        let (mut app, window, _) = fixture(order_on_every_mouse_edge(), false);
        // A minimap drag can remain active after LMB leaves the map rectangle.
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut().resource_mut::<MinimapState>().mouse_drag = true;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(mouse);
        frame(&mut app);
        let index = if mouse == MouseButton::Right { 1 } else { 2 };
        assert!(app.world().resource::<MinimapState>().mouse_drag);
        assert!(app.world().resource::<MinimapState>().ui_held[index]);
        pointer(&mut app, window, WORLD_POS);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(mouse);
        frame(&mut app);
        assert!(
            commands(&app).is_empty(),
            "{mouse:?} leaked during minimap drag"
        );
        assert!(!app.world().resource::<MinimapState>().ui_held[index]);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        frame(&mut app);
        assert!(!app.world().resource::<MinimapState>().mouse_drag);
        assert!(commands(&app).is_empty());
    }
}

#[test]
fn pointer_updates_before_custom_mouse_modifier_starts_selection_drag() {
    let (mut app, initial, squad) = squad_fixture();
    let mut profile = Profile::default_combined();
    // A mouse button can modify another binding while retaining DragSelect.
    profile.bindings.push(tak_input::Binding {
        input: tak_input::Input::Key(Key("q".into())),
        with: Some(tak_input::Input::Button(InButton::MouseLeft)),
        with_all: Vec::new(),
        gesture: tak_input::Gesture::Press,
        action: Action::Stop,
    });
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(profile);
    let window = app
        .world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap();
    let screen_pos = |app: &App, unit: UnitId| {
        let sim = app.world().non_send::<SimRes>();
        let pos = sim.session.sim.world().unit(unit).unwrap().pos;
        let screen = app
            .world()
            .resource::<CamState>()
            .geom
            .to_screen([f(pos.x), 0.0, f(pos.y)]);
        Vec2::new(screen[0], screen[1])
    };
    let old_pointer = screen_pos(&app, *initial.iter().next().unwrap());
    let target = screen_pos(&app, *squad.iter().next().unwrap());
    pointer(&mut app, window, old_pointer);
    frame(&mut app);
    // The new pointer and button-down arrive together. Starting the drag at
    // the old pointer would box-select both units instead of clicking B.
    pointer(&mut app, window, target);
    click(&mut app, MouseButton::Left);
    assert_eq!(selection(&app), squad);
    assert!(commands(&app).is_empty());
}

#[test]
fn gamepad_build_wheel_over_hud_confirms_wheel_once_not_underlying_portrait() {
    let mut profile = Profile::default_combined();
    profile.tuning.reticle = false;
    let (mut app, window, _) = fixture(profile, true);
    // The HUD portrait intentionally names a different product from slot 0.
    button(&mut app, ButtonKind::Build(1));
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(None);
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    pad_frame(&mut app, pad);
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .warp_cursor(tak_math::Vec2::from_ints(
            HUD_POS.x as i32,
            HUD_POS.y as i32,
        ));
    open_pad_build_wheel(&mut app, pad);
    pad_button(&mut app, pad, GamepadButton::South, true);
    pad_button(&mut app, pad, GamepadButton::South, false);
    assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
    assert!(!app.world().resource::<MinimapState>().pad_swallow);
    assert!(matches!(
        commands(&app).as_slice(),
        [SimCommand::Build {
            def: UnitDefId(3),
            at: None,
            queue: QueueMode::Append,
            ..
        }]
    ));
    pad_button(&mut app, pad, GamepadButton::LeftTrigger, false);
    assert_eq!(commands(&app).len(), 1, "LB release added production again");
}

#[test]
fn quick_gamepad_queue_modifier_with_mouse_hud_click_does_not_trigger_select_all() {
    let (mut app, _, _) = fixture(Profile::default_combined(), true);
    button(&mut app, ButtonKind::Build(0));
    let selected = selection(&app);
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    pad_button(&mut app, pad, GamepadButton::RightTrigger, true);
    click(&mut app, MouseButton::Left);
    let added = commands(&app);
    assert_eq!(added.len(), 5);
    assert!(added.iter().all(|command| matches!(command,
        SimCommand::Build { units, def: UnitDefId(3), at: None, queue: QueueMode::Append }
        if *units == selected)));
    // The selected factory is off screen: an unintended short RB tap would
    // SelectAll(OnScreen), clearing it rather than preserving the selection.
    pad_button(&mut app, pad, GamepadButton::RightTrigger, false);
    assert_eq!(selection(&app), selected);
    assert_eq!(commands(&app), added);
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .production_batch_count(),
        1
    );
}

#[test]
fn same_frame_stick_activity_does_not_retarget_world_or_minimap_mouse_context() {
    use bevy::input::gamepad::GamepadInput;
    for on_minimap in [false, true] {
        let mut baseline = None;
        for stick_changed in [false, true] {
            let profile = Profile::default_combined();
            assert!(profile.tuning.reticle);
            let (mut app, window, _) = fixture(profile, false);
            let (pos, expected) = if on_minimap {
                let mut minimap = app.world_mut().resource_mut::<MinimapState>();
                minimap.image = Some((200, 100));
                minimap.map = [640.0, 320.0];
                minimap.reserve = 100.0;
                let geom = minimap.geom([800.0, 600.0]).unwrap();
                let pos = Vec2::new(
                    geom.rect[0] + geom.rect[2] * 0.8,
                    geom.rect[1] + geom.rect[3] * 0.8,
                );
                let target = geom.target_at(pos.x, pos.y);
                let fx = |v: f32| tak_math::Fx::from_raw((v * 65536.0) as i32);
                (pos, tak_math::Vec2::new(fx(target[0]), fx(target[1])))
            } else {
                (Vec2::new(200.0, 220.0), tak_math::Vec2::from_ints(270, 140))
            };
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .set_cursor_position(None);
            let pad = app.world_mut().spawn(Gamepad::default()).id();
            // Establish zero axes first, then change an axis on the exact poll
            // carrying the mouse context click, not an earlier frame.
            frame(&mut app);
            pointer(&mut app, window, pos);
            if stick_changed {
                app.world_mut()
                    .get_mut::<Gamepad>(pad)
                    .unwrap()
                    .analog_mut()
                    .set(GamepadInput::Axis(GamepadAxis::LeftStickX), 1.0);
            }
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_millis(16));
            click(&mut app, MouseButton::Right);
            let sent = commands(&app);
            assert!(
                matches!(
                    sent.as_slice(),
                    [SimCommand::Move { to, queue: QueueMode::Replace, .. }] if *to == expected
                ),
                "minimap={on_minimap}, stick_changed={stick_changed}: {sent:?}"
            );
            assert_ne!(expected, tak_math::Vec2::from_ints(320, 160));
            if stick_changed {
                assert_eq!(
                    Some(sent),
                    baseline,
                    "stick activity changed the mouse target"
                );
                assert!(
                    app.world().resource::<InputRes>().mapper.stick_cursor(),
                    "the pad should still take over after the mouse order is dispatched"
                );
            } else {
                baseline = Some(sent);
            }
        }
    }
}

#[test]
fn same_poll_shift_and_hud_click_release_still_adds_and_removes_five() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for shift in [KeyCode::ShiftLeft, KeyCode::ShiftRight] {
            let (mut app, _, _) = fixture(profile.clone(), true);
            button(&mut app, ButtonKind::Build(0));
            let selected = selection(&app);
            for mouse in [MouseButton::Left, MouseButton::Right] {
                {
                    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                    keys.press(shift);
                    keys.release(shift);
                }
                {
                    let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
                    buttons.press(mouse);
                    buttons.release(mouse);
                }
                // A fast chord can deliver all four edges in a single poll.
                frame(&mut app);
                let sent = commands(&app);
                if mouse == MouseButton::Left {
                    assert_eq!(sent.len(), 5);
                    assert!(sent.iter().all(|command| matches!(command,
                        SimCommand::Build { units, def: UnitDefId(3), at: None, queue: QueueMode::Append }
                        if *units == selected)));
                } else {
                    assert_eq!(sent.len(), 6);
                    assert!(matches!(sent.last().unwrap(),
                        SimCommand::CancelProduction { units, def: UnitDefId(3), count: 5 }
                        if *units == selected));
                }
                assert_eq!(selection(&app), selected);
                assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
                assert_eq!(
                    app.world()
                        .resource::<InputRes>()
                        .mapper
                        .production_batch_count(),
                    1
                );
            }
        }
    }
}

#[test]
fn same_poll_gamepad_queue_modifier_release_and_mouse_hud_click_adds_five_without_select_all() {
    let (mut app, _, _) = fixture(Profile::default_combined(), true);
    button(&mut app, ButtonKind::Build(0));
    let selected = selection(&app);
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad.digital_mut().press(GamepadButton::RightTrigger);
        gamepad.digital_mut().release(GamepadButton::RightTrigger);
    }
    {
        let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        mouse.press(MouseButton::Left);
        mouse.release(MouseButton::Left);
    }
    pad_frame(&mut app, pad);
    let sent = commands(&app);
    assert_eq!(sent.len(), 5);
    assert!(sent.iter().all(|command| matches!(command,
        SimCommand::Build { units, def: UnitDefId(3), at: None, queue: QueueMode::Append }
        if *units == selected)));
    assert_eq!(
        selection(&app),
        selected,
        "the consumed RB tap must not select on-screen units and clear the offscreen factory"
    );
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .production_batch_count(),
        1
    );
    pad_frame(&mut app, pad);
    assert_eq!(commands(&app), sent);
    assert_eq!(selection(&app), selected);
}

#[test]
fn build_grid_hover_uses_non_image_parent_for_factory_and_mobile_options() {
    for factory in [false, true] {
        for console in [false, true] {
            let (mut app, _, _) = fixture(Profile::classic_tak(), factory);
            let vfs = Vfs::new();
            let tex = TextureIndex::load(&vfs);
            app.insert_non_send(Content {
                vfs,
                tex,
                sprites: SpriteStore::default(),
                kingdom: None,
            });
            app.init_resource::<Assets<Image>>()
                .init_resource::<controls::Portraits>()
                .insert_resource(hud_view::HudMode { console })
                .init_resource::<hud_view::HudHover>()
                .add_systems(Update, hud_view::build_grid_system.after(input_system));
            let grid = app
                .world_mut()
                .spawn((hud_view::BuildGrid, Node::default(), Visibility::Hidden))
                .id();
            let cell = button(&mut app, ButtonKind::Build(0));
            app.world_mut().entity_mut(cell).insert((
                Node::default(),
                BorderColor::default(),
                Visibility::Inherited,
                ChildOf(grid),
            ));
            // Match the real hierarchy: the clickable parent has no ImageNode;
            // its image is a child. A panel query requiring ImageNode misses it.
            app.world_mut().spawn((ImageNode::default(), ChildOf(cell)));
            assert!(app.world().get::<ImageNode>(cell).is_none());
            if factory {
                let units = selection(&app);
                let mut sim = app.world_mut().non_send_mut::<SimRes>();
                for _ in 0..3 {
                    sim.session.submit(SimCommand::Build {
                        units: units.clone(),
                        def: UnitDefId(3),
                        at: None,
                        queue: QueueMode::Append,
                    });
                }
                sim.step().unwrap();
            }
            frame(&mut app);
            let hover = &app.world().resource::<hud_view::HudHover>().0;
            if console {
                assert_eq!(
                    app.world().get::<Visibility>(grid),
                    Some(&Visibility::Hidden)
                );
                assert!(
                    hover.is_none(),
                    "hidden console grid exposed hover: {hover:?}"
                );
            } else {
                assert_eq!(
                    app.world().get::<Visibility>(grid),
                    Some(&Visibility::Inherited)
                );
                let hover = hover
                    .as_deref()
                    .expect("visible build parent must expose hover help");
                if factory {
                    assert!(hover.contains("training_troop: 30 mana"), "{hover}");
                    assert!(hover.contains("queued 3"), "{hover}");
                    assert!(hover.contains("LMB +1 / RMB -1"), "{hover}");
                    assert!(hover.contains("Shift five"), "{hover}");
                    assert!(hover.contains("Ctrl+LMB repeat"), "{hover}");
                    assert!(hover.contains("RMB stops"), "{hover}");
                    assert!(hover.contains("Repeating at 0 of 1 factories"), "{hover}");
                } else {
                    assert!(hover.contains("training_income: 100 mana"), "{hover}");
                    assert!(hover.contains("LMB choose placement"), "{hover}");
                    assert!(!hover.contains("queued"), "{hover}");
                }
            }
        }
    }
}

#[test]
fn same_poll_ctrl_factory_click_repeats_once_before_ctrl_and_shift_release() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for ctrl in [KeyCode::ControlLeft, KeyCode::ControlRight] {
            for shift in [None, Some(KeyCode::ShiftLeft), Some(KeyCode::ShiftRight)] {
                let (mut app, _, _) = fixture(profile.clone(), true);
                button(&mut app, ButtonKind::Build(1));
                let selected = selection(&app);
                let camera = app.world().resource::<CamState>().geom.target;
                let zoom = app.world().resource::<CamState>().geom.zoom;
                {
                    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                    for key in std::iter::once(ctrl).chain(shift) {
                        keys.press(key);
                        keys.release(key);
                    }
                }
                {
                    let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
                    mouse.press(MouseButton::Left);
                    mouse.release(MouseButton::Left);
                }
                frame(&mut app);
                assert!(
                    matches!(commands(&app).as_slice(),
                    [SimCommand::SetFactoryRepeat { units, def: Some(UnitDefId(5)) }]
                    if *units == selected),
                    "Ctrl+Shift must repeat, not append five"
                );
                assert_eq!(selection(&app), selected);
                assert_eq!(app.world().resource::<CamState>().geom.target, camera);
                assert_eq!(app.world().resource::<CamState>().geom.zoom, zoom);
                assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
                let mapper = &app.world().resource::<InputRes>().mapper;
                assert!(!mapper.ctrl_held());
                assert_eq!(mapper.production_batch_count(), 1);
                assert_eq!(mapper.mode(), Mode::None);

                // Neither modifier may leak into the next ordinary click.
                click(&mut app, MouseButton::Left);
                let sent = commands(&app);
                assert_eq!(sent.len(), 2);
                assert!(matches!(sent.last().unwrap(),
                    SimCommand::Build { units, def: UnitDefId(5), at: None, queue: QueueMode::Append }
                    if *units == selected));
            }
        }
    }
}

#[test]
fn ctrl_factory_click_owns_held_mouse_and_outside_release_without_world_orders() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        let (mut app, window, _) = fixture(profile, true);
        button(&mut app, ButtonKind::Build(0));
        let selected = selection(&app);
        let camera = app.world().resource::<CamState>().geom.target;
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::ShiftLeft);
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        frame(&mut app);
        let repeated = commands(&app);
        assert!(matches!(repeated.as_slice(),
            [SimCommand::SetFactoryRepeat { units, def: Some(UnitDefId(3)) }]
            if *units == selected));
        assert!(app.world().resource::<MinimapState>().ui_held[0]);
        pointer(&mut app, window, WORLD_POS);
        frame(&mut app);
        frame(&mut app);
        assert_eq!(commands(&app), repeated, "holding LMB reissued production");
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::ControlLeft);
            keys.release(KeyCode::ShiftLeft);
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        frame(&mut app);
        assert_eq!(commands(&app), repeated, "release leaked into the world");
        assert_eq!(selection(&app), selected);
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);
        assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
        assert!(!app.world().resource::<InputRes>().mapper.ctrl_held());
        assert_eq!(
            app.world()
                .resource::<InputRes>()
                .mapper
                .production_batch_count(),
            1
        );
    }
}

#[test]
fn ctrl_shift_mobile_portrait_keeps_placement_instead_of_repeat_or_batch() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        let (mut app, window, _) = fixture(profile, false);
        button(&mut app, ButtonKind::Build(0));
        let selected = selection(&app);
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            for key in [KeyCode::ControlRight, KeyCode::ShiftRight] {
                keys.press(key);
                keys.release(key);
            }
        }
        {
            let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            mouse.press(MouseButton::Left);
            mouse.release(MouseButton::Left);
        }
        frame(&mut app);
        assert!(commands(&app).is_empty());
        assert_eq!(selection(&app), selected);
        assert_eq!(
            app.world().resource::<InputRes>().mapper.mode(),
            Mode::Build(UnitDefId(1))
        );
        {
            let sim = app.world().non_send::<SimRes>();
            assert_eq!(
                app.world()
                    .resource::<InputRes>()
                    .mapper
                    .factory_repeat_counts(0, &sim.view()),
                None
            );
        }
        pointer(&mut app, window, WORLD_POS);
        click(&mut app, MouseButton::Left);
        assert!(matches!(commands(&app).as_slice(),
            [SimCommand::Build { units, def: UnitDefId(1), at: Some(_), .. }]
            if *units == selected));
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
    }
}

#[test]
fn gamepad_factory_repeat_owns_y_hold_and_all_y_lb_release_orders() {
    for batch in [false, true] {
        for release_order in 0..4 {
            let (mut app, pad) = pad_fixture(batch);
            let selected = selection(&app);
            let camera = app.world().resource::<CamState>().geom.target;
            let zoom = app.world().resource::<CamState>().geom.zoom;
            {
                let sim = app.world().non_send::<SimRes>();
                let spells = crate::view::unit_spells(
                    sim.session.sim.world(),
                    *selected.iter().next().unwrap(),
                );
                assert!(
                    spells.iter().any(|spell| spell.ready),
                    "the fixture must expose a ready spell to detect Y fallthrough"
                );
            }
            open_pad_build_wheel(&mut app, pad);
            if release_order == 0 {
                // North down/up and the wheel opener up can all arrive in one poll.
                {
                    let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
                    gamepad.digital_mut().press(GamepadButton::North);
                    gamepad.digital_mut().release(GamepadButton::North);
                    gamepad.digital_mut().release(GamepadButton::LeftTrigger);
                }
                pad_frame(&mut app, pad);
            } else {
                pad_button(&mut app, pad, GamepadButton::North, true);
                assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
                let hold_ms = app.world().resource::<InputRes>().mapper.tuning().hold_ms;
                app.world_mut()
                    .resource_mut::<Time>()
                    .advance_by(std::time::Duration::from_millis(u64::from(hold_ms) + 1));
                pad_frame(&mut app, pad);
                assert!(
                    app.world().resource::<InputRes>().mapper.wheel().is_none(),
                    "held Y reopened the spell wheel"
                );
                match release_order {
                    1 => {
                        pad_button(&mut app, pad, GamepadButton::North, false);
                        pad_button(&mut app, pad, GamepadButton::LeftTrigger, false);
                    }
                    2 => {
                        pad_button(&mut app, pad, GamepadButton::LeftTrigger, false);
                        pad_frame(&mut app, pad);
                        pad_button(&mut app, pad, GamepadButton::North, false);
                    }
                    _ => {
                        {
                            let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
                            gamepad.digital_mut().release(GamepadButton::LeftTrigger);
                            gamepad.digital_mut().release(GamepadButton::North);
                        }
                        pad_frame(&mut app, pad);
                    }
                }
            }
            if batch {
                pad_button(&mut app, pad, GamepadButton::RightTrigger, false);
            }
            pad_frame(&mut app, pad);
            assert!(
                matches!(commands(&app).as_slice(),
                [SimCommand::SetFactoryRepeat { units, def: Some(UnitDefId(3)) }]
                if *units == selected),
                "Y/LB release re-added production or cast a spell"
            );
            assert_eq!(selection(&app), selected);
            assert_eq!(app.world().resource::<CamState>().geom.target, camera);
            assert_eq!(app.world().resource::<CamState>().geom.zoom, zoom);
            let mapper = &app.world().resource::<InputRes>().mapper;
            assert!(mapper.wheel().is_none());
            assert_eq!(mapper.mode(), Mode::None);
            assert_eq!(mapper.production_batch_count(), 1);
        }
    }
}

#[test]
fn gamepad_factory_repeat_over_hud_uses_wheel_not_underlying_portrait() {
    let mut profile = Profile::default_combined();
    profile.tuning.reticle = false;
    let (mut app, window, _) = fixture(profile, true);
    button(&mut app, ButtonKind::Build(1));
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(None);
    let selected = selection(&app);
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    pad_frame(&mut app, pad);
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .warp_cursor(tak_math::Vec2::from_ints(
            HUD_POS.x as i32,
            HUD_POS.y as i32,
        ));
    open_pad_build_wheel(&mut app, pad);
    pad_button(&mut app, pad, GamepadButton::North, true);
    pad_button(&mut app, pad, GamepadButton::North, false);
    pad_button(&mut app, pad, GamepadButton::LeftTrigger, false);
    assert!(matches!(commands(&app).as_slice(),
        [SimCommand::SetFactoryRepeat { units, def: Some(UnitDefId(3)) }]
        if *units == selected));
    assert!(!app.world().resource::<MinimapState>().pad_swallow);
    assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
    assert_eq!(selection(&app), selected);
}

fn record_next_input_poll(app: &mut App) {
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .take_feedback();
    let recorder = tak_input::Recorder::new(
        serde_json::json!({"synthetic": "modal production adapter"}),
        &app.world().resource::<InputRes>().mapper,
    );
    let mut feel = app.world_mut().resource_mut::<Feel>();
    feel.feedback.clear();
    feel.recorder = Some(recorder);
}

fn assert_modal_recording_has_no_gameplay(app: &App) {
    let feel = app.world().resource::<Feel>();
    assert!(
        feel.feedback.is_empty(),
        "suppressed gameplay left feedback: {:?}",
        feel.feedback
    );
    let recording = feel.recorder.as_ref().unwrap().recording();
    for frame in &recording.frames {
        assert!(
            frame.submitted.is_empty(),
            "modal recorded a submitted command"
        );
        assert!(
            frame.feedback.is_empty(),
            "modal recorded false gameplay feedback"
        );
        assert!(
            frame.selection.is_none(),
            "modal recorded a transient selection change"
        );
        assert!(
            frame
                .outputs
                .iter()
                .all(|out| matches!(out, Output::ToggleOverlay)),
            "modal recorded an unsuppressed gameplay output"
        );
    }
}

#[test]
fn opening_modal_cancels_pending_repeat_wheel_and_resets_keyboard_and_pad_modifiers() {
    for opening_key in [
        Some(KeyCode::F8),
        Some(KeyCode::F7),
        Some(KeyCode::F11),
        None,
    ] {
        let (mut app, pad) = pad_fixture(true);
        if opening_key == Some(KeyCode::F7) {
            app.insert_resource(skirmish_ui::SkirmishState::new(Default::default(), false));
            app.init_resource::<skirmish_ui::PendingRequest>();
        }
        app.world_mut().resource_mut::<Feel>().quick_save_dir =
            Err("Synthetic adapter test has no checkpoint directory".into());
        let selected = selection(&app);
        open_pad_build_wheel(&mut app, pad);
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::ShiftLeft);
        }
        pad_frame(&mut app, pad);
        record_next_input_poll(&mut app);
        if let Some(key) = opening_key {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
        } else {
            app.world_mut()
                .get_mut::<Gamepad>(pad)
                .unwrap()
                .digital_mut()
                .press(GamepadButton::Start);
        }
        app.world_mut()
            .get_mut::<Gamepad>(pad)
            .unwrap()
            .digital_mut()
            .press(GamepadButton::North);
        pad_frame(&mut app, pad);
        assert!(
            commands(&app).is_empty(),
            "{opening_key:?} opening leaked Y repeat: {:?}",
            commands(&app)
        );
        assert_modal_recording_has_no_gameplay(&app);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
        assert!(!app.world().resource::<InputRes>().mapper.ctrl_held());
        assert_eq!(
            app.world()
                .resource::<InputRes>()
                .mapper
                .production_batch_count(),
            1
        );
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            for key in opening_key
                .into_iter()
                .chain([KeyCode::ControlLeft, KeyCode::ShiftLeft])
            {
                keys.release(key);
            }
        }
        {
            let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
            for button in [
                GamepadButton::Start,
                GamepadButton::North,
                GamepadButton::LeftTrigger,
                GamepadButton::RightTrigger,
            ] {
                gamepad.digital_mut().release(button);
            }
        }
        pad_frame(&mut app, pad);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        pad_frame(&mut app, pad);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Escape);
        pad_frame(&mut app, pad);
        pad_frame(&mut app, pad);
        assert!(
            commands(&app).is_empty(),
            "closing modal replayed a held gesture"
        );
        assert_eq!(selection(&app), selected);
        open_pad_build_wheel(&mut app, pad);
        pad_button(&mut app, pad, GamepadButton::South, true);
        pad_button(&mut app, pad, GamepadButton::South, false);
        pad_button(&mut app, pad, GamepadButton::LeftTrigger, false);
        assert!(
            matches!(commands(&app).as_slice(),
            [SimCommand::Build { units, def: UnitDefId(3), at: None, queue: QueueMode::Append }]
            if *units == selected),
            "modal left stale repeat/batch input"
        );
    }
}

fn rendered_build_grid(app: &mut App) -> (Entity, Entity) {
    let vfs = Vfs::new();
    let tex = TextureIndex::load(&vfs);
    app.insert_non_send(Content {
        vfs,
        tex,
        sprites: SpriteStore::default(),
        kingdom: None,
    });
    app.init_resource::<Assets<Image>>()
        .init_resource::<controls::Portraits>()
        .init_resource::<hud_view::HudArt>()
        .insert_resource(hud_view::HudMode { console: false })
        .init_resource::<hud_view::HudHover>()
        .add_systems(Startup, hud_view::setup_hud_view)
        .add_systems(Update, hud_view::build_grid_system.after(input_system));
    frame(app);
    let cell = app
        .world_mut()
        .query::<(Entity, &ButtonKind)>()
        .iter(app.world())
        .find_map(|(entity, kind)| matches!(kind, ButtonKind::Build(0)).then_some(entity))
        .unwrap();
    app.world_mut().entity_mut(cell).insert((
        ComputedNode {
            size: Vec2::new(64.0, 48.0),
            ..default()
        },
        UiGlobalTransform::from(bevy::math::Affine2::from_translation(HUD_POS)),
        InheritedVisibility::VISIBLE,
    ));
    // Inspect the actual queue badge produced by setup_hud_view, not a
    // separate formatter or a test-created approximation of the text node.
    let badge = app
        .world_mut()
        .query_filtered::<(Entity, &ChildOf, &Node), With<hud_view::BuildCellText>>()
        .iter(app.world())
        .find_map(|(entity, parent, node)| {
            (parent.parent() == cell && node.right == Val::Px(1.0)).then_some(entity)
        })
        .unwrap();
    assert!(app.world().get::<ImageNode>(cell).is_none());
    frame(app);
    (cell, badge)
}

// These regressions inspect the actual widget hierarchy and run its update
// systems. They deliberately do not assert glyph shaping or rendered pixels.
fn status_entity<T: Component>(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<T>>()
        .single(app.world())
        .unwrap()
}

fn rendered_status_card(console: bool, factory: bool) -> (App, Entity, UnitId) {
    let (mut app, window, synthetic_panel) = fixture(order_on_every_mouse_edge(), factory);
    app.world_mut().despawn(synthetic_panel);
    let selected = *selection(&app).iter().next().unwrap();
    let (cell, _) = rendered_build_grid(&mut app);
    // The build-grid helper positions its first button at HUD_POS. Keep it
    // outside these status-panel input probes, without changing the real tree.
    *app.world_mut().get_mut::<UiGlobalTransform>(cell).unwrap() =
        UiGlobalTransform::from(bevy::math::Affine2::from_translation(Vec2::splat(-1000.0)));
    app.world_mut().resource_mut::<hud_view::HudMode>().console = console;
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
        .add_plugins(bevy::camera::visibility::VisibilityPlugin)
        .add_systems(
            Update,
            (hud_view::unit_card_system, hud_view::strip_system).after(input_system),
        );
    frame(&mut app);
    (app, window, selected)
}

fn status_bar(app: &mut App, top: f32) -> (Visibility, Val, Color) {
    let (track, bar) = app
        .world_mut()
        .query_filtered::<(Entity, &Node, &Children), With<hud_view::CardBarTrack>>()
        .iter(app.world())
        .find_map(|(entity, node, children)| {
            (node.top == Val::Px(top)).then_some((entity, children[0]))
        })
        .unwrap();
    assert!(app.world().get::<hud_view::CardBar>(bar).is_some());
    (
        *app.world().get::<Visibility>(track).unwrap(),
        app.world().get::<Node>(bar).unwrap().width,
        app.world().get::<BackgroundColor>(bar).unwrap().0,
    )
}

fn assert_status_build(app: &mut App, percent: Option<f32>) {
    let before = app.world().non_send::<SimRes>().session.sim.state_hash();
    let recorded = commands(app);
    frame(app);
    assert_eq!(
        app.world().non_send::<SimRes>().session.sim.state_hash(),
        before,
        "updating the HUD must not change the world or scripts"
    );
    assert_eq!(commands(app), recorded, "updating the HUD issued commands");
    let (visibility, width, color) = status_bar(app, 41.0);
    match percent {
        Some(percent) => {
            assert_eq!(visibility, Visibility::Inherited);
            assert_eq!(width, Val::Percent(percent));
            assert_eq!(color, Color::srgb(0.95, 0.75, 0.25));
        }
        None => assert_eq!(visibility, Visibility::Hidden),
    }
}

fn start_status_product(app: &mut App, factory: UnitId, repeated: bool) -> UnitId {
    let mut sim = app.world_mut().non_send_mut::<SimRes>();
    let units = UnitSet::single(factory);
    sim.session.submit(if repeated {
        SimCommand::SetFactoryRepeat {
            units,
            def: Some(UnitDefId(3)),
        }
    } else {
        SimCommand::Build {
            units,
            def: UnitDefId(3),
            at: None,
            queue: QueueMode::Append,
        }
    });
    for _ in 0..5 {
        sim.step().unwrap();
    }
    let world = sim.session.sim.world_mut();
    let build = &world.unit(factory).unwrap().build;
    assert_eq!(build.factory.product_from_repeat, repeated);
    let product = build.factory.product.unwrap();
    assert!(world.unit(product).unwrap().build.frame.unwrap().paid > 0);
    // Pin distinct fractions after normal simulation has created paid work.
    // 37/80 rounds down to 46%, preserving the existing percent-left rule.
    world.unit_mut(product).unwrap().build.frame = Some(tak_sim::construction::Frame {
        done: 37,
        total: 80,
        paid: 11,
    });
    let selected = world.unit_mut(factory).unwrap();
    selected.combat.health = 150;
    selected.combat.max_health = 200;
    tak_sim::economy::set_unit_mana_percent(world, factory, 25);
    product
}

#[test]
fn rendered_status_card_build_tracks_finite_and_repeat_without_replacing_health_or_mana() {
    for console in [false, true] {
        for repeated in [false, true] {
            let (mut app, _, factory) = rendered_status_card(console, true);
            let product = start_status_product(&mut app, factory, repeated);
            assert_status_build(&mut app, Some(46.0));
            let health = status_bar(&mut app, 25.0);
            let mana = status_bar(&mut app, 33.0);
            let [r, g, b] = crate::hud::health_rgb(0.75);
            assert_eq!(
                health,
                (
                    Visibility::Inherited,
                    Val::Percent(75.0),
                    Color::srgb(r, g, b)
                )
            );
            assert_eq!(
                mana,
                (
                    Visibility::Inherited,
                    Val::Percent(25.0),
                    Color::srgb(0.3, 0.5, 1.0)
                )
            );
            {
                let mut sim = app.world_mut().non_send_mut::<SimRes>();
                let unit = sim.session.sim.world_mut().unit_mut(product).unwrap();
                unit.build.frame.as_mut().unwrap().done = 60;
                unit.combat.health = 1;
                unit.combat.max_health = 10;
            }
            assert_status_build(&mut app, Some(75.0));
            assert_eq!(status_bar(&mut app, 25.0), health);
            assert_eq!(status_bar(&mut app, 33.0), mana);
        }
    }
}

#[test]
fn rendered_status_card_build_keeps_actual_product_after_recipe_change_disable_and_pending_cancel()
{
    for console in [false, true] {
        let (mut app, _, factory) = rendered_status_card(console, true);
        let product = start_status_product(&mut app, factory, true);
        for recipe in [Some(UnitDefId(5)), None] {
            {
                let mut sim = app.world_mut().non_send_mut::<SimRes>();
                let unit = sim.session.sim.world_mut().unit_mut(factory).unwrap();
                unit.build.factory_repeat = recipe;
                unit.orders.clear();
                unit.orders.push_back(tak_sim::Order::Build {
                    def: UnitDefId(5),
                    at: None,
                });
            }
            assert_status_build(&mut app, Some(46.0));
        }
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            sim.session
                .sim
                .world_mut()
                .unit_mut(factory)
                .unwrap()
                .build
                .factory
                .cancel_product = true;
        }
        assert_status_build(&mut app, Some(46.0));
        app.world_mut()
            .non_send_mut::<SimRes>()
            .session
            .sim
            .world_mut()
            .despawn(product)
            .unwrap();
        assert_status_build(&mut app, None);
    }
}

#[test]
fn rendered_status_card_build_preserves_selected_frame_precedence_and_zero_paid_progress() {
    for console in [false, true] {
        let (mut app, _, factory) = rendered_status_card(console, true);
        let product = start_status_product(&mut app, factory, false);
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            sim.session
                .sim
                .world_mut()
                .unit_mut(factory)
                .unwrap()
                .build
                .frame = Some(tak_sim::construction::Frame {
                done: 75,
                total: 100,
                paid: 0,
            });
        }
        assert_status_build(&mut app, Some(75.0));
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let world = sim.session.sim.world_mut();
            world.unit_mut(factory).unwrap().build.frame = None;
            world
                .unit_mut(product)
                .unwrap()
                .build
                .frame
                .as_mut()
                .unwrap()
                .paid = 0;
        }
        assert_status_build(&mut app, Some(46.0));
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            sim.session
                .sim
                .world_mut()
                .unit_mut(product)
                .unwrap()
                .build
                .frame
                .as_mut()
                .unwrap()
                .done = 0;
        }
        assert_status_build(&mut app, Some(0.0));
    }
}

#[test]
fn rendered_status_card_build_hides_recipe_queue_only_stale_complete_enemy_and_nonfactory_products()
{
    for console in [false, true] {
        let (mut app, _, factory) = rendered_status_card(console, true);
        assert_status_build(&mut app, None);
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let unit = sim.session.sim.world_mut().unit_mut(factory).unwrap();
            unit.build.factory_repeat = Some(UnitDefId(3));
        }
        assert_status_build(&mut app, None);
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let unit = sim.session.sim.world_mut().unit_mut(factory).unwrap();
            unit.build.factory_repeat = None;
            unit.orders.push_back(tak_sim::Order::Build {
                def: UnitDefId(3),
                at: None,
            });
        }
        assert_status_build(&mut app, None);
        let product = start_status_product(&mut app, factory, false);
        assert_status_build(&mut app, Some(46.0));
        for has_frame in [true, false] {
            {
                let mut sim = app.world_mut().non_send_mut::<SimRes>();
                sim.session
                    .sim
                    .world_mut()
                    .unit_mut(product)
                    .unwrap()
                    .build
                    .frame = has_frame.then_some(tak_sim::construction::Frame {
                    done: 80,
                    total: 80,
                    paid: 11,
                });
            }
            assert_status_build(&mut app, None);
        }
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let world = sim.session.sim.world_mut();
            world.unit_mut(product).unwrap().build.frame = Some(tak_sim::construction::Frame {
                done: 37,
                total: 80,
                paid: 11,
            });
            world.unit_mut(factory).unwrap().owner = PlayerId(1);
        }
        assert_status_build(&mut app, None);
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let unit = sim.session.sim.world_mut().unit_mut(factory).unwrap();
            unit.owner = PlayerId(0);
            unit.def = UnitDefId(0); // A mobile builder cannot expose stale factory state.
        }
        assert_status_build(&mut app, None);
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let world = sim.session.sim.world_mut();
            world.unit_mut(factory).unwrap().def = UnitDefId(2);
            world.despawn(product).unwrap();
            let replacement = world
                .spawn(
                    UnitDefId(3),
                    PlayerId(0),
                    tak_math::Vec2::from_ints(200, 160),
                    tak_math::Angle::ZERO,
                )
                .unwrap();
            assert_eq!(replacement.index, product.index);
            assert_ne!(replacement.generation, product.generation);
            world.unit_mut(replacement).unwrap().build.frame =
                Some(tak_sim::construction::Frame::new(100));
        }
        assert_status_build(&mut app, None);
    }
}

#[test]
fn rendered_status_card_build_clears_and_restores_on_selection_transitions() {
    for console in [false, true] {
        let (mut app, _, factory) = rendered_status_card(console, true);
        start_status_product(&mut app, factory, true);
        let group = tak_commands::ControlGroup::new(1).unwrap();
        perform(&mut app, Action::AssignGroup { group });
        assert_status_build(&mut app, Some(46.0));
        app.world_mut()
            .non_send_mut::<SimRes>()
            .session
            .sim
            .world_mut()
            .spawn(
                UnitDefId(0),
                PlayerId(0),
                tak_math::Vec2::from_ints(300, 160),
                tak_math::Angle::ZERO,
            )
            .unwrap();
        perform(
            &mut app,
            Action::SelectCategory {
                category: tak_input::Category::Builder,
            },
        );
        assert_ne!(selection(&app), UnitSet::single(factory));
        assert_status_build(&mut app, None);
        perform(&mut app, Action::RecallGroup { group });
        assert_eq!(selection(&app), UnitSet::single(factory));
        assert_status_build(&mut app, Some(46.0));
        perform(&mut app, Action::Cancel);
        assert!(selection(&app).is_empty());
        assert_status_build(&mut app, None);
        assert_eq!(status_bar(&mut app, 25.0).0, Visibility::Hidden);
        assert_eq!(status_bar(&mut app, 33.0).0, Visibility::Hidden);
    }
}

#[test]
fn rendered_status_card_wraps_full_text_in_bounded_normal_flow_above_original_art() {
    for console in [false, true] {
        let (mut app, _, _) = rendered_status_card(console, true);
        let card = status_entity::<hud_view::UnitCard>(&mut app);
        let stack = status_entity::<hud_view::SelectionStack>(&mut app);
        let details = status_entity::<hud_view::SelectionDetails>(&mut app);
        let name = status_entity::<hud_view::CardName>(&mut app);
        let info = status_entity::<hud_view::CardText>(&mut app);
        let strip = status_entity::<hud_view::SelStrip>(&mut app);
        let world = app.world();
        let card_node = world.get::<Node>(card).unwrap();
        assert_eq!(card_node.width, Val::Px(512.0));
        assert_eq!(card_node.height, Val::Px(49.0));
        assert_eq!(card_node.bottom, Val::Px(0.0));
        let stack_node = world.get::<Node>(stack).unwrap();
        assert_eq!(stack_node.position_type, PositionType::Absolute);
        assert_eq!(stack_node.left, Val::Px(0.0));
        assert_eq!(stack_node.bottom, Val::Px(52.0));
        assert_eq!(stack_node.width, Val::Px(512.0));
        assert_eq!(stack_node.max_width, Val::Percent(100.0));
        assert_eq!(stack_node.height, Val::Auto);
        assert_eq!(stack_node.flex_direction, FlexDirection::ColumnReverse);
        assert!(
            world.get::<HudPanel>(stack).is_none(),
            "gaps must not capture input"
        );
        assert_eq!(
            world.get::<Children>(stack).unwrap().as_ref(),
            &[details, strip]
        );
        assert_eq!(
            world.get::<Children>(details).unwrap().as_ref(),
            &[name, info]
        );
        assert_eq!(
            world.get::<Node>(details).unwrap().flex_direction,
            FlexDirection::Column
        );
        for entity in [details, name, info, strip] {
            let node = world.get::<Node>(entity).unwrap();
            assert_eq!(node.position_type, PositionType::Relative);
            assert_eq!(node.width, Val::Percent(100.0));
            assert_eq!(node.min_width, Val::Px(0.0));
            assert_eq!(node.max_width, Val::Percent(100.0));
            assert_eq!(node.height, Val::Auto);
            assert_eq!(node.max_height, Val::Auto);
            assert_eq!(node.flex_shrink, 0.0);
            assert_eq!(node.overflow, Overflow::visible());
        }
        for (entity, font_size) in [(name, 14.0), (info, 11.0)] {
            assert_eq!(world.get::<ChildOf>(entity).unwrap().parent(), details);
            assert_eq!(
                world.get::<TextFont>(entity).unwrap().font_size,
                bevy::text::FontSize::Px(font_size)
            );
            assert_eq!(
                world.get::<TextLayout>(entity).unwrap().linebreak,
                bevy::text::LineBreak::WordOrCharacter,
                "even long unbroken custom names must wrap"
            );
        }
        assert!(world.get::<HudPanel>(details).is_some());
        assert!(world.get::<HudPanel>(strip).is_some());
        assert_eq!(world.get::<Node>(strip).unwrap().flex_wrap, FlexWrap::Wrap);
        // Portrait and all three bar tracks remain on the unchanged art,
        // not inside the growing text panel.
        let picture = status_entity::<hud_view::CardPic>(&mut app);
        assert_eq!(app.world().get::<ChildOf>(picture).unwrap().parent(), card);
        let picture_node = app.world().get::<Node>(picture).unwrap();
        assert_eq!(picture_node.left, Val::Px(59.0));
        assert_eq!(picture_node.top, Val::Px(7.0));
        assert_eq!(picture_node.width, Val::Px(48.0));
        assert_eq!(picture_node.height, Val::Px(36.0));
        let mut track_y = app
            .world_mut()
            .query_filtered::<(&Node, &ChildOf), With<hud_view::CardBarTrack>>()
            .iter(app.world())
            .map(|(node, parent)| {
                assert_eq!(parent.parent(), card);
                assert_eq!(node.left, Val::Px(114.0));
                assert_eq!(node.width, Val::Px(110.0));
                assert_eq!(node.height, Val::Px(5.0));
                let Val::Px(y) = node.top else {
                    panic!("bar must retain its art offset")
                };
                y as u32
            })
            .collect::<Vec<_>>();
        track_y.sort_unstable();
        assert_eq!(track_y, [25, 33, 41]);
    }
}

#[test]
fn rendered_status_card_preserves_active_repeat_pending_count_and_guard_rally_full_text() {
    use tak_commands::{FactoryRallyOrder, FireState};
    use tak_sim::construction::{FactoryState, Frame};

    let factory_name =
        "Synthetic Extended Factory Name With AnUnbrokenSuffixThatMustWrapWithoutEllipsis";
    let product_name = "Synthetic Repeated Product With A Deliberately Long Display Name";
    let guard_name = "Synthetic Allied Guard Target With Another Deliberately Long Display Name";
    for console in [false, true] {
        let (mut app, _, factory) = rendered_status_card(console, true);
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            sim.session.defs[2].display = factory_name.into();
            sim.session.defs[3].display = product_name.into();
            sim.session.defs[0].display = guard_name.into();
            let world = sim.session.sim.world_mut();
            let product = world
                .spawn(
                    UnitDefId(3),
                    PlayerId(0),
                    tak_math::Vec2::from_ints(200, 160),
                    tak_math::Angle::ZERO,
                )
                .unwrap();
            let guard = world
                .spawn(
                    UnitDefId(0),
                    PlayerId(0),
                    tak_math::Vec2::from_ints(300, 160),
                    tak_math::Angle::ZERO,
                )
                .unwrap();
            world.unit_mut(product).unwrap().build.frame = Some(Frame {
                done: 5,
                total: 100,
                paid: 5,
            });
            let unit = world.unit_mut(factory).unwrap();
            unit.fire_state = FireState::Defensive;
            unit.combat.health = 123_456_789;
            unit.combat.max_health = i32::MAX;
            unit.build.factory = FactoryState {
                product: Some(product),
                product_from_repeat: true,
                active: true,
                ..default()
            };
            unit.build.factory_repeat = Some(UnitDefId(3));
            unit.orders = std::iter::repeat_n(
                tak_sim::Order::Build {
                    def: UnitDefId(3),
                    at: None,
                },
                1234,
            )
            .collect();
            unit.build.factory_rally = Some(vec![
                FactoryRallyOrder::Move {
                    to: tak_math::Vec2::from_ints(320, 160),
                },
                FactoryRallyOrder::Guard { target: guard },
                FactoryRallyOrder::Patrol {
                    to: tak_math::Vec2::from_ints(400, 160),
                },
            ]);
        }
        frame(&mut app);
        let name = status_entity::<hud_view::CardName>(&mut app);
        let info = status_entity::<hud_view::CardText>(&mut app);
        assert_eq!(app.world().get::<Text>(name).unwrap().0, factory_name);
        assert_eq!(
            app.world().get::<Text>(info).unwrap().0,
            format!(
                "Defensive - Building {product_name} (+1234) | Rally 3 | Guard {guard_name}\nHealth 123456789/2147483647"
            )
        );
        assert!(
            commands(&app).is_empty(),
            "reading status must not issue commands"
        );
    }
}

#[test]
fn rendered_status_card_handles_multiselect_construction_enemy_and_empty_selection() {
    for console in [false, true] {
        let (mut app, _, selected) = rendered_status_card(console, true);
        let name = status_entity::<hud_view::CardName>(&mut app);
        let info = status_entity::<hud_view::CardText>(&mut app);
        let strip = status_entity::<hud_view::SelStrip>(&mut app);
        assert_eq!(
            app.world().get::<Node>(strip).unwrap().display,
            Display::None
        );
        app.world_mut()
            .non_send_mut::<SimRes>()
            .session
            .sim
            .world_mut()
            .spawn(
                UnitDefId(3),
                PlayerId(0),
                tak_math::Vec2::from_ints(300, 160),
                tak_math::Angle::ZERO,
            )
            .unwrap();
        perform(
            &mut app,
            Action::SelectAll {
                scope: tak_commands::Scope::Map,
            },
        );
        frame(&mut app);
        assert_eq!(selection(&app).len(), 2);
        assert!(
            app.world()
                .get::<Text>(name)
                .unwrap()
                .0
                .ends_with("  (2 selected)")
        );
        assert_eq!(
            app.world().get::<Node>(strip).unwrap().display,
            Display::Flex
        );
        assert!(app.world().get::<InheritedVisibility>(strip).unwrap().get());
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let unit = sim.session.sim.world_mut().unit_mut(selected).unwrap();
            unit.fire_state = tak_commands::FireState::Defensive;
            unit.combat.health = 12345;
            unit.combat.max_health = 99999;
            unit.build.frame = Some(tak_sim::construction::Frame {
                done: 95,
                total: 100,
                paid: 0,
            });
        }
        frame(&mut app);
        assert_eq!(
            app.world().get::<Text>(info).unwrap().0,
            "Defensive - Under construction\nHealth 12345/99999"
        );
        // An inspected enemy must not disclose its production or orders. Keep
        // the same inspected entity while changing its ownership in the fixture.
        {
            let ids = selection(&app);
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            for id in &ids {
                sim.session.sim.world_mut().unit_mut(*id).unwrap().owner = PlayerId(1);
            }
        }
        frame(&mut app);
        assert_eq!(
            app.world().get::<Text>(info).unwrap().0,
            "Enemy\nHealth 12345/99999"
        );
        assert_eq!(
            app.world().get::<Node>(strip).unwrap().display,
            Display::None
        );
        assert!(!app.world().get::<InheritedVisibility>(strip).unwrap().get());
        perform(&mut app, Action::Cancel);
        frame(&mut app);
        assert!(selection(&app).is_empty());
        assert_eq!(app.world().get::<Text>(name).unwrap().0, "Nothing selected");
        assert_eq!(app.world().get::<Text>(info).unwrap().0, "");
        assert_eq!(
            app.world().get::<Node>(strip).unwrap().display,
            Display::None
        );
        assert!(
            app.world_mut()
                .query_filtered::<&Visibility, With<hud_view::CardBarTrack>>()
                .iter(app.world())
                .all(|v| *v == Visibility::Hidden)
        );
        assert!(commands(&app).is_empty());
    }
}

#[test]
fn rendered_status_panel_captures_mouse_edges_but_hidden_strip_and_stack_gap_do_not() {
    const DETAILS_POS: Vec2 = Vec2::new(260.0, 420.0);
    const GAP_POS: Vec2 = Vec2::new(260.0, 340.0);
    for console in [false, true] {
        for (index, mouse) in MOUSE_BUTTONS.into_iter().enumerate() {
            let (mut app, window, _) = rendered_status_card(console, false);
            let details = status_entity::<hud_view::SelectionDetails>(&mut app);
            let strip = status_entity::<hud_view::SelStrip>(&mut app);
            let stack = status_entity::<hud_view::SelectionStack>(&mut app);
            // Supply deterministic resolved rectangles to the real input
            // adapter. Bevy's actual visibility propagation remains enabled;
            // this does not pretend to run its glyph/layout/render pipeline.
            for (entity, center, size) in [
                (details, DETAILS_POS, Vec2::new(512.0, 100.0)),
                (strip, GAP_POS, Vec2::new(512.0, 36.0)),
                (stack, Vec2::new(260.0, 400.0), Vec2::new(512.0, 160.0)),
            ] {
                app.world_mut().entity_mut(entity).insert((
                    ComputedNode { size, ..default() },
                    UiGlobalTransform::from(bevy::math::Affine2::from_translation(center)),
                ));
            }
            frame(&mut app);
            assert!(
                app.world()
                    .get::<InheritedVisibility>(details)
                    .unwrap()
                    .get()
            );
            assert!(!app.world().get::<InheritedVisibility>(strip).unwrap().get());
            let selected = selection(&app);
            pointer(&mut app, window, DETAILS_POS);
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(mouse);
            frame(&mut app);
            assert!(app.world().resource::<MinimapState>().ui_held[index]);
            pointer(&mut app, window, WORLD_POS);
            frame(&mut app);
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(mouse);
            frame(&mut app);
            assert!(
                commands(&app).is_empty(),
                "{mouse:?} leaked through status details"
            );
            assert_eq!(selection(&app), selected);
            assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
            pointer(&mut app, window, GAP_POS);
            click(&mut app, mouse);
            assert!(
                commands(&app)
                    .iter()
                    .all(|command| matches!(command, SimCommand::Move { .. }))
            );
            assert!(
                !commands(&app).is_empty(),
                "{mouse:?} was swallowed by empty stack/hidden strip"
            );
        }
    }
}

#[test]
fn unfinished_builder_hud_and_idle_cycle_become_actionable_only_on_completion() {
    use tak_input::WorldView;

    let (mut app, _, _) = fixture(Profile::classic_tak(), false);
    let selected = selection(&app);
    let builder = *selected.iter().next().unwrap();
    app.world_mut()
        .non_send_mut::<SimRes>()
        .session
        .sim
        .world_mut()
        .unit_mut(builder)
        .unwrap()
        .build
        .frame = Some(tak_sim::construction::Frame {
        done: 95,
        total: 100,
        paid: 0,
    });
    let (cell, _) = rendered_build_grid(&mut app);
    app.add_systems(Update, hud_view::unit_card_system.after(input_system));
    frame(&mut app);
    let grid = app.world().get::<ChildOf>(cell).unwrap().parent();
    assert_eq!(
        app.world().get::<Visibility>(grid),
        Some(&Visibility::Hidden)
    );
    let info = app
        .world_mut()
        .query_filtered::<Entity, With<hud_view::CardText>>()
        .single(app.world())
        .unwrap();
    assert!(
        app.world()
            .get::<Text>(info)
            .unwrap()
            .0
            .contains("Under construction")
    );
    {
        let sim = app.world().non_send::<SimRes>();
        let view = sim.view();
        assert!(view.unit(builder).unwrap().selectable);
        assert!(view.unit(builder).unwrap().under_construction);
        assert!(!view.can_build(builder));
        assert!(!view.can_heal(builder));
        assert!(!view.is_idle(builder));
    }
    // Even a stale visible-button callback cannot enter placement mode.
    click(&mut app, MouseButton::Left);
    assert!(commands(&app).is_empty());
    assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
    assert_eq!(selection(&app), selected);
    perform(&mut app, Action::Cancel);
    perform(&mut app, Action::CycleIdleBuilder);
    assert!(selection(&app).is_empty());
    perform(
        &mut app,
        Action::SelectAll {
            scope: tak_commands::Scope::Map,
        },
    );
    assert_eq!(selection(&app), selected);
    app.world_mut()
        .non_send_mut::<SimRes>()
        .session
        .sim
        .world_mut()
        .unit_mut(builder)
        .unwrap()
        .build
        .frame = None;
    frame(&mut app);
    assert_eq!(
        app.world().get::<Visibility>(grid),
        Some(&Visibility::Inherited)
    );
    assert!(app.world().get::<Text>(info).unwrap().0.contains("Idle"));
    assert!(
        !app.world()
            .get::<Text>(info)
            .unwrap()
            .0
            .contains("Under construction")
    );
    click(&mut app, MouseButton::Left);
    assert_eq!(
        app.world().resource::<InputRes>().mapper.mode(),
        Mode::Build(UnitDefId(1))
    );
}

#[test]
fn gamepad_unfinished_builder_rejects_build_then_requires_fresh_gesture_after_completion() {
    for (confirm_on_press, confirm_first) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let mut profile = Profile::default_combined();
        profile.tuning.wheel_confirm_on_press = confirm_on_press;
        let (mut app, window, _) = fixture(profile, false);
        let selected = selection(&app);
        let builder = *selected.iter().next().unwrap();
        let position = {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let unit = sim.session.sim.world_mut().unit_mut(builder).unwrap();
            unit.build.frame = Some(tak_sim::construction::Frame {
                done: 95,
                total: 100,
                paid: 0,
            });
            unit.pos
        };
        // With no wheel available, A is ordinary selection. Keep the
        // reticle on the same frame so that behavior does not hide the test.
        app.world_mut().resource_mut::<CamState>().geom.target = [f(position.x), f(position.y)];
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(None);
        let pad = app.world_mut().spawn(Gamepad::default()).id();
        pad_frame(&mut app, pad);
        pad_button(&mut app, pad, GamepadButton::LeftTrigger, true);
        let tuning = *app.world().resource::<InputRes>().mapper.tuning();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(
                u64::from(tuning.hold_ms) + 1,
            ));
        pad_frame(&mut app, pad);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
        pad_button(&mut app, pad, GamepadButton::South, true);
        pad_button(&mut app, pad, GamepadButton::South, false);
        assert!(commands(&app).is_empty());
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());

        // Repeat confirm and complete while it and the failed opener are held.
        // Neither release order may revive the earlier build attempt.
        pad_button(&mut app, pad, GamepadButton::South, true);
        app.world_mut()
            .non_send_mut::<SimRes>()
            .session
            .sim
            .world_mut()
            .unit_mut(builder)
            .unwrap()
            .build
            .frame = None;
        pad_frame(&mut app, pad);
        let releases = if confirm_first {
            [GamepadButton::South, GamepadButton::LeftTrigger]
        } else {
            [GamepadButton::LeftTrigger, GamepadButton::South]
        };
        for button in releases {
            pad_button(&mut app, pad, button, false);
            assert!(commands(&app).is_empty());
            assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
            assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
        }
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(
                u64::from(tuning.hold_ms.max(tuning.double_tap_ms)) + 1,
            ));
        pad_frame(&mut app, pad);
        assert!(commands(&app).is_empty());
        assert_eq!(selection(&app), selected);
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());

        // A fresh gesture selects normally: A confirms a sticky wheel;
        // the default mobile wheel confirms on the opener's release.
        open_pad_build_wheel(&mut app, pad);
        pad_button(&mut app, pad, GamepadButton::South, true);
        pad_button(&mut app, pad, GamepadButton::South, false);
        assert_eq!(
            app.world().resource::<InputRes>().mapper.wheel().is_none(),
            confirm_on_press
        );
        assert_eq!(
            app.world().resource::<InputRes>().mapper.mode(),
            if confirm_on_press {
                Mode::Build(UnitDefId(1))
            } else {
                Mode::None
            }
        );
        pad_button(&mut app, pad, GamepadButton::LeftTrigger, false);
        assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
        assert!(commands(&app).is_empty());
        assert_eq!(
            app.world().resource::<InputRes>().mapper.mode(),
            Mode::Build(UnitDefId(1))
        );
        pad_button(&mut app, pad, GamepadButton::West, true);
        pad_button(&mut app, pad, GamepadButton::West, false);
        pad_frame(&mut app, pad);
        assert!(matches!(commands(&app).as_slice(),
            [SimCommand::Build { units, def: UnitDefId(1), at: Some(_), queue: QueueMode::Replace }]
            if *units == selected));
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
        assert_eq!(selection(&app), selected);
    }
}

#[test]
fn rendered_menu_uses_completed_factory_instead_of_lower_id_unfinished_mobile() {
    let (mut app, _, _) = fixture(Profile::classic_tak(), false);
    let mut scenario = tak_headless::bot::demo::scenario(false);
    scenario.initial_units = Some(
        [(0, 0, 112), (2, 0, 240), (4, 1, 528)]
            .into_iter()
            .map(|(def, owner, x)| tak_sim::campaign::InitialUnit {
                def: UnitDefId(def),
                owner: PlayerId(owner),
                pos: tak_math::Vec2::from_ints(x, 160),
                heading: tak_math::Angle::ZERO,
                health_percentage: 100,
                mana_percentage: 100,
                kills: 0,
            })
            .collect(),
    );
    replace_sim(&mut app, &scenario);
    perform(
        &mut app,
        Action::SelectAll {
            scope: tak_commands::Scope::Map,
        },
    );
    let selected = selection(&app);
    let ids: Vec<_> = selected.iter().copied().collect();
    assert_eq!(ids.len(), 2);
    app.world_mut()
        .non_send_mut::<SimRes>()
        .session
        .sim
        .world_mut()
        .unit_mut(ids[0])
        .unwrap()
        .build
        .frame = Some(tak_sim::construction::Frame {
        done: 95,
        total: 100,
        paid: 0,
    });
    rendered_build_grid(&mut app);
    let hover = app
        .world()
        .resource::<hud_view::HudHover>()
        .0
        .as_deref()
        .unwrap();
    assert!(hover.starts_with("training_troop:"), "{hover}");
    assert!(hover.contains("queued 0"), "{hover}");
    click(&mut app, MouseButton::Left);
    assert_eq!(
        commands(&app),
        vec![SimCommand::Build {
            units: UnitSet::single(ids[1]),
            def: UnitDefId(3),
            at: None,
            queue: QueueMode::Append,
        }]
    );
    assert_eq!(selection(&app), selected);
    assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
}

#[test]
fn rendered_factory_badge_preserves_finite_count_then_adds_repeat_marker_and_clears_it() {
    let (mut app, _, _) = fixture(Profile::classic_tak(), true);
    let (_, badge) = rendered_build_grid(&mut app);
    assert_eq!(app.world().get::<Text>(badge).unwrap().0, "");
    for _ in 0..3 {
        click(&mut app, MouseButton::Left);
    }
    app.world_mut().non_send_mut::<SimRes>().step().unwrap();
    frame(&mut app);
    assert_eq!(app.world().get::<Text>(badge).unwrap().0, "x3");
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ControlLeft);
    click(&mut app, MouseButton::Left);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::ControlLeft);
    app.world_mut().non_send_mut::<SimRes>().step().unwrap();
    frame(&mut app);
    assert_eq!(app.world().get::<Text>(badge).unwrap().0, "x3 +++");
    let hover = app
        .world()
        .resource::<hud_view::HudHover>()
        .0
        .as_deref()
        .unwrap();
    assert!(hover.contains("queued 3"), "{hover}");
    assert!(hover.contains("Repeating at 1 of 1 factories"), "{hover}");
    click(&mut app, MouseButton::Right);
    app.world_mut().non_send_mut::<SimRes>().step().unwrap();
    frame(&mut app);
    assert_eq!(app.world().get::<Text>(badge).unwrap().0, "x2");
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .factory_repeat_counts(0, &sim.view()),
        Some((0, 1))
    );
}

#[test]
fn rendered_factory_repeat_count_distinguishes_mixed_selection_then_all_repeating() {
    let (mut app, _, _) = fixture(Profile::classic_tak(), true);
    let mut scenario = tak_headless::bot::demo::scenario(false);
    scenario.unit_defs[2].economy.mogriumstorage = 1000;
    scenario.initial_units = Some(
        [(2, 0, 112), (2, 0, 240), (4, 1, 528)]
            .into_iter()
            .map(|(def, owner, x)| tak_sim::campaign::InitialUnit {
                def: UnitDefId(def),
                owner: PlayerId(owner),
                pos: tak_math::Vec2::from_ints(x, 160),
                heading: tak_math::Angle::ZERO,
                health_percentage: 100,
                mana_percentage: 100,
                kills: 0,
            })
            .collect(),
    );
    replace_sim(&mut app, &scenario);
    perform(
        &mut app,
        Action::SelectAll {
            scope: tak_commands::Scope::Map,
        },
    );
    let selected = selection(&app);
    assert_eq!(selected.len(), 2);
    let one_factory = UnitSet::single(*selected.iter().next().unwrap());
    {
        let mut sim = app.world_mut().non_send_mut::<SimRes>();
        sim.session.submit(SimCommand::SetFactoryRepeat {
            units: one_factory,
            def: Some(UnitDefId(3)),
        });
        sim.step().unwrap();
    }
    let (_, badge) = rendered_build_grid(&mut app);
    assert!(app.world().get::<Text>(badge).unwrap().0.contains("+++"));
    let hover = app
        .world()
        .resource::<hud_view::HudHover>()
        .0
        .as_deref()
        .unwrap();
    assert!(hover.contains("Repeating at 1 of 2 factories"), "{hover}");
    {
        let sim = app.world().non_send::<SimRes>();
        assert_eq!(
            app.world()
                .resource::<InputRes>()
                .mapper
                .factory_repeat_counts(0, &sim.view()),
            Some((1, 2))
        );
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ControlLeft);
    click(&mut app, MouseButton::Left);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::ControlLeft);
    app.world_mut().non_send_mut::<SimRes>().step().unwrap();
    frame(&mut app);
    let hover = app
        .world()
        .resource::<hud_view::HudHover>()
        .0
        .as_deref()
        .unwrap();
    assert!(hover.contains("Repeating at 2 of 2 factories"), "{hover}");
    let sim = app.world().non_send::<SimRes>();
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .factory_repeat_counts(0, &sim.view()),
        Some((2, 2))
    );
}

#[test]
fn fast_ctrl_hud_click_precedes_unrelated_release_even_without_profile_ctrl_chords() {
    let mut profile = Profile::classic_tak();
    let ctrl = tak_input::Input::Key(Key("ctrl".into()));
    profile
        .bindings
        .retain(|binding| binding.input != ctrl && !binding.modifiers().contains(&ctrl));
    let (mut app, window, _) = fixture(profile, true);
    button(&mut app, ButtonKind::Build(0));
    let selected = selection(&app);
    pointer(&mut app, window, WORLD_POS);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Right);
    frame(&mut app);
    let before = commands(&app);
    pointer(&mut app, window, HUD_POS);
    {
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.press(KeyCode::ControlLeft);
        keys.release(KeyCode::ControlLeft);
    }
    {
        let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        mouse.press(MouseButton::Left);
        mouse.release(MouseButton::Left);
        mouse.release(MouseButton::Right);
    }
    frame(&mut app);
    let sent = commands(&app);
    assert_eq!(&sent[..before.len()], before.as_slice());
    assert!(
        matches!(&sent[before.len()..],
        [SimCommand::SetFactoryRepeat { units, def: Some(UnitDefId(3)) }]
        if *units == selected),
        "unrelated RMB release dispatched HUD action before Ctrl down"
    );
    assert_eq!(selection(&app), selected);
    assert!(!app.world().resource::<InputRes>().mapper.ctrl_held());
    assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
}

#[test]
fn cycle_idle_builder_excludes_armed_and_disabled_but_active_repeat_until_cancel_finishes() {
    let mut profile = Profile::classic_tak();
    let input = tak_input::Input::Key(Key("i".into()));
    profile.bindings.retain(|binding| binding.input != input);
    profile.bindings.push(tak_input::Binding {
        input,
        with: None,
        with_all: Vec::new(),
        gesture: tak_input::Gesture::Press,
        action: Action::CycleIdleBuilder,
    });
    let (mut app, _, _) = fixture(profile, true);
    let selected = selection(&app);
    let factory = *selected.iter().next().unwrap();
    {
        let mut sim = app.world_mut().non_send_mut::<SimRes>();
        let local = sim.session.local;
        // Observe the valid command boundary before construction begins:
        // an armed repeat recipe alone must already make the factory busy.
        tak_sim::construction::apply_factory_command(
            sim.session.sim.world_mut(),
            local,
            &SimCommand::SetFactoryRepeat {
                units: selected.clone(),
                def: Some(UnitDefId(3)),
            },
        );
        let unit = sim.session.sim.world().unit(factory).unwrap();
        assert_eq!(unit.build.factory_repeat, Some(UnitDefId(3)));
        assert!(unit.build.factory.product.is_none());
        assert!(unit.orders.is_empty());
    }
    perform(&mut app, Action::Cancel);
    assert!(selection(&app).is_empty());
    let cycle = |app: &mut App| {
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::KeyI);
            keys.release(KeyCode::KeyI);
        }
        frame(app);
    };
    cycle(&mut app);
    assert!(
        selection(&app).is_empty(),
        "armed repeat factory was reported idle"
    );
    {
        let mut sim = app.world_mut().non_send_mut::<SimRes>();
        sim.step().unwrap();
        sim.session.submit(SimCommand::SetFactoryRepeat {
            units: selected.clone(),
            def: None,
        });
        sim.step().unwrap();
        let unit = sim.session.sim.world().unit(factory).unwrap();
        assert!(unit.build.factory_repeat.is_none());
        assert!(unit.build.factory.product.is_some());
        assert!(unit.orders.is_empty());
    }
    cycle(&mut app);
    assert!(
        selection(&app).is_empty(),
        "active repeat product with its recipe disabled was reported idle"
    );
    {
        let mut sim = app.world_mut().non_send_mut::<SimRes>();
        sim.session.submit(SimCommand::CancelProduction {
            units: selected.clone(),
            def: UnitDefId(3),
            count: 1,
        });
        sim.step().unwrap();
        let unit = sim.session.sim.world().unit(factory).unwrap();
        assert!(unit.build.factory_repeat.is_none());
        assert!(unit.build.factory.product.is_none());
        assert!(unit.orders.is_empty());
    }
    cycle(&mut app);
    assert_eq!(
        selection(&app),
        selected,
        "finished cancellation must restore idle-builder eligibility"
    );
}

#[test]
fn opening_overlay_with_f11_or_pad_menu_consumes_same_poll_ctrl_hud_production() {
    for (pad_menu, factory) in [(false, false), (false, true), (true, false), (true, true)] {
        let (mut app, window, _) = fixture(Profile::default_combined(), factory);
        button(&mut app, ButtonKind::Build(0));
        let selected = selection(&app);
        let camera = app.world().resource::<CamState>().geom.target;
        let pad = app.world_mut().spawn(Gamepad::default()).id();
        pad_frame(&mut app, pad);
        record_next_input_poll(&mut app);
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::ShiftLeft);
            if !pad_menu {
                keys.press(KeyCode::F11);
            }
        }
        if pad_menu {
            app.world_mut()
                .get_mut::<Gamepad>(pad)
                .unwrap()
                .digital_mut()
                .press(GamepadButton::Start);
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        pad_frame(&mut app, pad);
        assert!(app.world().resource::<Feel>().overlay_open);
        assert_modal_recording_has_no_gameplay(&app);
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
        assert_eq!(selection(&app), selected);
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);
        assert!(
            commands(&app).is_empty(),
            "pad_menu={pad_menu} leaked a HUD production command"
        );
        assert!(!app.world().resource::<InputRes>().mapper.ctrl_held());
        assert_eq!(
            app.world()
                .resource::<InputRes>()
                .mapper
                .production_batch_count(),
            1
        );
        pointer(&mut app, window, WORLD_POS);
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            for key in [KeyCode::F11, KeyCode::ControlLeft, KeyCode::ShiftLeft] {
                keys.release(key);
            }
        }
        app.world_mut()
            .get_mut::<Gamepad>(pad)
            .unwrap()
            .digital_mut()
            .release(GamepadButton::Start);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        pad_frame(&mut app, pad);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        pad_frame(&mut app, pad);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Escape);
        pad_frame(&mut app, pad);
        pad_frame(&mut app, pad);
        assert!(!app.world().resource::<Feel>().overlay_open);
        assert!(
            commands(&app).is_empty(),
            "modal exit replayed the consumed production click"
        );
        assert_eq!(selection(&app), selected);
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);
        assert_eq!(app.world().resource::<MinimapState>().ui_held, [false; 3]);
        pointer(&mut app, window, HUD_POS);
        click(&mut app, MouseButton::Left);
        if factory {
            assert!(matches!(commands(&app).as_slice(),
                [SimCommand::Build { units, def: UnitDefId(3), at: None, queue: QueueMode::Append }]
                if *units == selected));
        } else {
            assert!(commands(&app).is_empty());
            assert_eq!(
                app.world().resource::<InputRes>().mapper.mode(),
                Mode::Build(UnitDefId(1))
            );
        }
    }
}

#[test]
fn opening_overlay_rolls_back_same_poll_selection_groups_and_camera() {
    use bevy::input::gamepad::GamepadInput;
    let group = tak_commands::ControlGroup::new(1).unwrap();
    for action in [
        Action::RecallGroup { group },
        Action::AssignGroup { group },
        Action::SelectAll {
            scope: tak_commands::Scope::Map,
        },
        Action::JumpToGroup { group },
    ] {
        for pad_menu in [false, true] {
            let (mut app, initial, squad) = squad_fixture();
            let mut profile = app.world().resource::<InputRes>().mapper.profile().clone();
            let input = tak_input::Input::Key(Key("k".into()));
            profile.bindings.retain(|binding| binding.input != input);
            profile.bindings.push(tak_input::Binding {
                input,
                with: None,
                with_all: Vec::new(),
                gesture: tak_input::Gesture::Press,
                action,
            });
            app.world_mut()
                .resource_mut::<InputRes>()
                .mapper
                .set_profile(profile);
            let pad = app.world_mut().spawn(Gamepad::default()).id();
            pad_frame(&mut app, pad);
            let camera = app.world().resource::<CamState>().geom.target;
            let zoom = app.world().resource::<CamState>().geom.zoom;
            record_next_input_poll(&mut app);
            {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                keys.press(KeyCode::KeyK);
                keys.press(KeyCode::ArrowRight);
                if !pad_menu {
                    keys.press(KeyCode::F11);
                }
            }
            {
                let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
                gamepad
                    .analog_mut()
                    .set(GamepadInput::Axis(GamepadAxis::RightStickX), 1.0);
                if pad_menu {
                    gamepad.digital_mut().press(GamepadButton::Start);
                }
            }
            pad_frame(&mut app, pad);
            assert!(app.world().resource::<Feel>().overlay_open);
            assert_eq!(
                selection(&app),
                initial,
                "{action:?} mutated selection while opening settings"
            );
            assert_eq!(app.world().resource::<CamState>().geom.target, camera);
            assert_eq!(app.world().resource::<CamState>().geom.zoom, zoom);
            assert_modal_recording_has_no_gameplay(&app);
            {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                for key in [KeyCode::KeyK, KeyCode::ArrowRight, KeyCode::F11] {
                    keys.release(key);
                }
            }
            {
                let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
                gamepad
                    .analog_mut()
                    .set(GamepadInput::Axis(GamepadAxis::RightStickX), 0.0);
                gamepad.digital_mut().release(GamepadButton::Start);
            }
            pad_frame(&mut app, pad);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Escape);
            pad_frame(&mut app, pad);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::Escape);
            pad_frame(&mut app, pad);
            pad_frame(&mut app, pad);
            assert_eq!(selection(&app), initial);
            assert_eq!(
                app.world().resource::<CamState>().geom.target,
                camera,
                "cancelled camera movement resumed after settings closed"
            );
            assert!(commands(&app).is_empty());
            // Recall the original saved group after exit. A same-poll
            // assignment must not have silently overwritten it.
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Digit1);
            pad_frame(&mut app, pad);
            assert_eq!(
                selection(&app),
                squad,
                "{action:?} corrupted the saved group"
            );
        }
    }
}

#[test]
fn stop_then_hud_build_refunds_paid_product_before_starting_new_queue_head() {
    // Real input adaptation and the ordinary Session driver must protect
    // fresh UI orders from the legacy finite Stop+Build same-tick behavior.
    // The raw legacy replay characterization lives in the core sim tests.
    for sequence in [
        "same_poll",
        "paused_polls",
        "controller_hold",
        "stale_snapshot",
    ] {
        let mut profile = Profile::default_combined();
        profile.tuning.reticle = false;
        let (mut app, window, _) = fixture(profile, true);
        let mut scenario = tak_headless::bot::demo::scenario(false);
        scenario.players[0].monarch = UnitDefId(2);
        scenario.unit_defs[2].economy.mogriumstorage = 1000;
        let mut alternate = scenario.unit_defs[3].clone();
        alternate.name = "synthetic_alternate_product".into();
        alternate.build.buildcost = 40;
        scenario.unit_defs.push(alternate);
        scenario.unit_defs[2].build.canbuild.push(UnitDefId(5));
        replace_sim(&mut app, &scenario);
        let selected = selection(&app);
        let factory = *selected.iter().next().unwrap();
        let portrait = button(&mut app, ButtonKind::Build(0));
        click(&mut app, MouseButton::Left);
        for _ in 0..5 {
            app.world_mut().non_send_mut::<SimRes>().step().unwrap();
        }
        let (product, tick) = {
            let sim = app.world().non_send::<SimRes>();
            let world = sim.session.sim.world();
            let product = world.unit(factory).unwrap().build.factory.product.unwrap();
            let frame = world.unit(product).unwrap().build.frame.unwrap();
            assert!(frame.paid > 0 && frame.done > 0);
            assert_eq!(world.unit(product).unwrap().def, UnitDefId(3));
            assert_eq!(
                tak_sim::economy::mana(world, PlayerId(0)).0,
                (100 << 16) - frame.paid
            );
            (product, sim.session.tick())
        };
        *app.world_mut().get_mut::<ButtonKind>(portrait).unwrap() = ButtonKind::Build(1);
        app.world_mut().resource_mut::<Feel>().feedback.clear();
        match sequence {
            "same_poll" => {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyS);
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Left);
                frame(&mut app);
            }
            "paused_polls" => {
                app.world_mut().non_send_mut::<SimRes>().paused = true;
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyS);
                frame(&mut app);
                click(&mut app, MouseButton::Left);
                app.world_mut().non_send_mut::<SimRes>().paused = false;
            }
            "stale_snapshot" => {
                // An extra pending original product must not exhaust a stale
                // pre-poll cancellation count before reaching the paid frame.
                // Repeating Stop before a tick must still refund only once.
                app.world_mut().non_send_mut::<SimRes>().paused = true;
                *app.world_mut().get_mut::<ButtonKind>(portrait).unwrap() = ButtonKind::Build(0);
                click(&mut app, MouseButton::Left);
                for _ in 0..2 {
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .press(KeyCode::KeyS);
                    frame(&mut app);
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .release(KeyCode::KeyS);
                    frame(&mut app);
                }
                *app.world_mut().get_mut::<ButtonKind>(portrait).unwrap() = ButtonKind::Build(1);
                click(&mut app, MouseButton::Left);
                app.world_mut().non_send_mut::<SimRes>().paused = false;
            }
            "controller_hold" => {
                app.world_mut()
                    .get_mut::<Window>(window)
                    .unwrap()
                    .set_cursor_position(None);
                let pad = app.world_mut().spawn(Gamepad::default()).id();
                pad_frame(&mut app, pad);
                app.world_mut()
                    .resource_mut::<InputRes>()
                    .mapper
                    .warp_cursor(tak_math::Vec2::from_ints(
                        HUD_POS.x as i32,
                        HUD_POS.y as i32,
                    ));
                pad_button(&mut app, pad, GamepadButton::East, true);
                let hold_ms = app.world().resource::<InputRes>().mapper.tuning().hold_ms;
                app.world_mut()
                    .resource_mut::<Time>()
                    .advance_by(std::time::Duration::from_millis(u64::from(hold_ms) + 1));
                pad_button(&mut app, pad, GamepadButton::South, true);
            }
            _ => unreachable!(),
        }
        let initial = SimCommand::Build {
            units: selected.clone(),
            def: UnitDefId(3),
            at: None,
            queue: QueueMode::Append,
        };
        let mut expected = vec![initial.clone()];
        if sequence == "stale_snapshot" {
            expected.push(initial);
        }
        for _ in 0..if sequence == "stale_snapshot" { 2 } else { 1 } {
            expected.extend([
                SimCommand::CancelProduction {
                    units: selected.clone(),
                    def: UnitDefId(3),
                    count: u32::MAX,
                },
                SimCommand::Stop {
                    units: selected.clone(),
                },
            ]);
        }
        expected.push(SimCommand::Build {
            units: selected,
            def: UnitDefId(5),
            at: None,
            queue: QueueMode::Append,
        });
        assert_eq!(commands(&app), expected, "{sequence}");
        assert_eq!(
            app.world()
                .resource::<Feel>()
                .feedback
                .iter()
                .filter(|feedback| { matches!(feedback, tak_input::Feedback::Order) })
                .count(),
            if sequence == "stale_snapshot" { 4 } else { 2 },
            "one acknowledgement per user Stop/Build, not the preparation"
        );
        let mut sim = app.world_mut().non_send_mut::<SimRes>();
        assert!(
            sim.session.recorded.entries()[1..]
                .iter()
                .all(|entry| entry.tick == tick)
        );
        sim.step().unwrap();
        let world = sim.session.sim.world();
        assert!(world.unit(product).is_none(), "{sequence}");
        assert!(world.unit(factory).unwrap().build.factory.product.is_none());
        assert_eq!(tak_sim::economy::mana(world, PlayerId(0)).0, 100 << 16);
        assert_eq!(
            tak_sim::construction::production_count(world, factory, UnitDefId(3)),
            0
        );
        assert_eq!(
            tak_sim::construction::production_count(world, factory, UnitDefId(5)),
            1
        );
        sim.step().unwrap();
        let product = sim
            .session
            .sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product
            .unwrap();
        assert_eq!(
            sim.session.sim.world().unit(product).unwrap().def,
            UnitDefId(5)
        );
        while sim
            .session
            .sim
            .world()
            .unit(product)
            .unwrap()
            .build
            .frame
            .is_some()
        {
            assert!(sim.session.tick().0 < 100);
            sim.step().unwrap();
        }
        let world = sim.session.sim.world();
        assert_eq!(world.unit(product).unwrap().def, UnitDefId(5));
        assert!(world.unit(factory).unwrap().orders.is_empty());
        assert!(world.unit(factory).unwrap().build.factory.product.is_none());
        assert!(!world.units().any(|(_, unit)| unit.def == UnitDefId(3)));
        assert_eq!(tak_sim::economy::mana(world, PlayerId(0)).0, 60 << 16);
    }
}

/// A factory with paid work and a separate friendly target. These definitions
/// and explicit initial units are synthetic; owned-data acceptance is separate.
fn guard_fixture(mixed: bool, repeated: bool) -> (App, Entity, UnitId, UnitId, UnitId) {
    let mut profile = Profile::default_combined();
    profile.tuning.reticle = false;
    let (mut app, window, _) = fixture(profile, false);
    let mut scenario = tak_headless::bot::demo::scenario(false);
    scenario.initial_units = Some(
        [(2, 0, 160), (3, 0, 240), (0, 0, 320), (4, 1, 480)]
            .into_iter()
            .map(|(def, owner, x)| tak_sim::campaign::InitialUnit {
                def: UnitDefId(def),
                owner: PlayerId(owner),
                pos: tak_math::Vec2::from_ints(x, 160),
                heading: tak_math::Angle::ZERO,
                health_percentage: 100,
                mana_percentage: 100,
                kills: 0,
            })
            .collect(),
    );
    replace_sim(&mut app, &scenario);
    perform(
        &mut app,
        Action::SelectCategory {
            category: tak_input::Category::Armed,
        },
    );
    let mobile = *selection(&app).iter().next().unwrap();
    let group = tak_commands::ControlGroup::new(1).unwrap();
    perform(&mut app, Action::AssignGroup { group });
    perform(
        &mut app,
        Action::SelectCategory {
            category: tak_input::Category::Factory,
        },
    );
    let factory = *selection(&app).iter().next().unwrap();
    if mixed {
        perform(&mut app, Action::RecallGroupAdd { group });
    }
    assert_eq!(selection(&app).len(), if mixed { 2 } else { 1 });
    let target = {
        let mut sim = app.world_mut().non_send_mut::<SimRes>();
        let target = sim
            .session
            .sim
            .world()
            .units()
            .find_map(|(id, unit)| (unit.def == UnitDefId(0)).then_some(id))
            .unwrap();
        if repeated {
            sim.session.submit(SimCommand::SetFactoryRepeat {
                units: UnitSet::single(factory),
                def: Some(UnitDefId(3)),
            });
        } else {
            sim.session.submit(SimCommand::Build {
                units: UnitSet::single(factory),
                def: UnitDefId(3),
                at: None,
                queue: QueueMode::Append,
            });
        }
        for _ in 0..5 {
            sim.step().unwrap();
        }
        // Keep both a finite queue and repeat recipe present for every case.
        sim.session.submit(SimCommand::Build {
            units: UnitSet::single(factory),
            def: UnitDefId(3),
            at: None,
            queue: QueueMode::Append,
        });
        if !repeated {
            sim.session.submit(SimCommand::SetFactoryRepeat {
                units: UnitSet::single(factory),
                def: Some(UnitDefId(3)),
            });
        }
        sim.step().unwrap();
        let build = &sim.session.sim.world().unit(factory).unwrap().build;
        assert_eq!(build.factory.product_from_repeat, repeated);
        let product = build.factory.product.unwrap();
        assert!(
            sim.session
                .sim
                .world()
                .unit(product)
                .unwrap()
                .build
                .frame
                .unwrap()
                .paid
                > 0
        );
        target
    };
    pointer(&mut app, window, WORLD_POS);
    frame(&mut app);
    app.world_mut().resource_mut::<Feel>().feedback.clear();
    (app, window, factory, mobile, target)
}

fn guard_acknowledgements(app: &App) -> usize {
    app.world()
        .resource::<Feel>()
        .feedback
        .iter()
        .filter(|feedback| matches!(feedback, tak_input::Feedback::Order))
        .count()
}

fn assert_guard_dispatch_preserves_work(
    app: &mut App,
    factory: UnitId,
    mobile: UnitId,
    target: UnitId,
    start: usize,
    mixed: bool,
    queue: QueueMode,
) {
    use tak_commands::FactoryRallyOrder;
    let sent = commands(app);
    let expected = if mixed {
        vec![
            SimCommand::Guard {
                units: UnitSet::single(mobile),
                target,
                queue,
            },
            SimCommand::SetFactoryRally {
                units: UnitSet::single(factory),
                order: Some(FactoryRallyOrder::Guard { target }),
                queue,
            },
        ]
    } else {
        vec![SimCommand::SetFactoryRally {
            units: UnitSet::single(factory),
            order: Some(FactoryRallyOrder::Guard { target }),
            queue,
        }]
    };
    assert_eq!(&sent[start..], expected);
    assert_eq!(
        guard_acknowledgements(app),
        1,
        "one gesture must acknowledge once"
    );
    let mut sim = app.world_mut().non_send_mut::<SimRes>();
    let before = sim.session.sim.world().unit(factory).unwrap().clone();
    let product = before.build.factory.product.unwrap();
    let paid = sim
        .session
        .sim
        .world()
        .unit(product)
        .unwrap()
        .build
        .frame
        .unwrap();
    sim.step().unwrap();
    let world = sim.session.sim.world();
    let after = world.unit(factory).unwrap();
    assert_eq!(
        after.orders, before.orders,
        "Guard replaced finite production"
    );
    assert_eq!(
        after.build.factory.product,
        Some(product),
        "Guard replaced the paid frame"
    );
    assert_eq!(
        after.build.factory.product_from_repeat,
        before.build.factory.product_from_repeat
    );
    assert_eq!(after.build.factory_repeat, Some(UnitDefId(3)));
    assert_eq!(
        after.build.factory_rally,
        Some(vec![FactoryRallyOrder::Guard { target }])
    );
    let frame = world.unit(product).unwrap().build.frame.unwrap();
    assert!(frame.paid >= paid.paid && frame.done >= paid.done);
}

#[test]
fn mouse_factory_guard_context_and_explicit_modes_preserve_finite_and_repeat_work() {
    for mixed in [false, true] {
        for repeated in [false, true] {
            for explicit in [false, true] {
                for append in [false, true] {
                    let (mut app, _, factory, mobile, target) = guard_fixture(mixed, repeated);
                    let selected = selection(&app);
                    let start = commands(&app).len();
                    if explicit {
                        app.world_mut()
                            .resource_mut::<ButtonInput<KeyCode>>()
                            .press(KeyCode::KeyG);
                        frame(&mut app);
                        app.world_mut()
                            .resource_mut::<ButtonInput<KeyCode>>()
                            .release(KeyCode::KeyG);
                        frame(&mut app);
                        assert_eq!(
                            app.world().resource::<InputRes>().mapper.mode(),
                            Mode::Guard
                        );
                    }
                    app.world_mut().resource_mut::<Feel>().feedback.clear();
                    if append {
                        app.world_mut()
                            .resource_mut::<ButtonInput<KeyCode>>()
                            .press(KeyCode::ShiftLeft);
                    }
                    click(
                        &mut app,
                        if explicit {
                            MouseButton::Left
                        } else {
                            MouseButton::Right
                        },
                    );
                    if append {
                        app.world_mut()
                            .resource_mut::<ButtonInput<KeyCode>>()
                            .release(KeyCode::ShiftLeft);
                        frame(&mut app);
                    }
                    assert_eq!(selection(&app), selected);
                    assert_guard_dispatch_preserves_work(
                        &mut app,
                        factory,
                        mobile,
                        target,
                        start,
                        mixed,
                        if append {
                            QueueMode::Append
                        } else {
                            QueueMode::Replace
                        },
                    );
                }
            }
        }
    }
}

fn guard_pad(app: &mut App, window: Entity) -> Entity {
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(None);
    let pad = app.world_mut().spawn(Gamepad::default()).id();
    pad_frame(app, pad);
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .warp_cursor(tak_math::Vec2::from_ints(
            WORLD_POS.x as i32,
            WORLD_POS.y as i32,
        ));
    pad
}

fn pad_guard_mode(app: &mut App, pad: Entity) {
    use bevy::input::gamepad::GamepadInput;
    pad_button(app, pad, GamepadButton::LeftTrigger2, true);
    {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad
            .analog_mut()
            .set(GamepadInput::Axis(GamepadAxis::LeftStickX), -1.0);
        gamepad
            .analog_mut()
            .set(GamepadInput::Axis(GamepadAxis::LeftStickY), -1.0);
    }
    pad_frame(app, pad);
    assert_eq!(
        app.world()
            .resource::<InputRes>()
            .mapper
            .wheel()
            .unwrap()
            .highlighted,
        Some(5)
    );
    {
        let mut gamepad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
        gamepad
            .analog_mut()
            .set(GamepadInput::Axis(GamepadAxis::LeftStickX), 0.0);
        gamepad
            .analog_mut()
            .set(GamepadInput::Axis(GamepadAxis::LeftStickY), 0.0);
    }
    pad_frame(app, pad);
    // The default command wheel confirms on its opener release, unlike
    // the factory build wheel that specially owns A/X confirmations.
    pad_button(app, pad, GamepadButton::LeftTrigger2, false);
    assert!(app.world().resource::<InputRes>().mapper.wheel().is_none());
    assert_eq!(
        app.world().resource::<InputRes>().mapper.mode(),
        Mode::Guard
    );
}

#[test]
fn gamepad_factory_guard_context_and_command_wheel_split_mixed_selection_once() {
    for mixed in [false, true] {
        for repeated in [false, true] {
            for explicit in [false, true] {
                let (mut app, window, factory, mobile, target) = guard_fixture(mixed, repeated);
                let selected = selection(&app);
                let start = commands(&app).len();
                let pad = guard_pad(&mut app, window);
                if explicit {
                    pad_guard_mode(&mut app, pad);
                }
                app.world_mut().resource_mut::<Feel>().feedback.clear();
                pad_button(&mut app, pad, GamepadButton::West, true);
                assert_eq!(
                    commands(&app).len(),
                    start,
                    "X tap dispatched before release"
                );
                pad_button(&mut app, pad, GamepadButton::West, false);
                pad_frame(&mut app, pad);
                assert_eq!(selection(&app), selected);
                assert_guard_dispatch_preserves_work(
                    &mut app,
                    factory,
                    mobile,
                    target,
                    start,
                    mixed,
                    QueueMode::Replace,
                );
            }
        }
    }
}

#[test]
fn original_mouse_friendly_primary_click_still_selects_instead_of_factory_guard() {
    let (mut app, window, _, _, target) = guard_fixture(false, true);
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(Profile::original_tak());
    let start = commands(&app);
    pointer(&mut app, window, WORLD_POS);
    click(&mut app, MouseButton::Left);
    assert_eq!(selection(&app), UnitSet::single(target));
    assert_eq!(commands(&app), start);
    assert_eq!(guard_acknowledgements(&app), 0);
}

#[test]
fn factory_guard_target_invalidated_before_mouse_click_or_pad_release_is_rejected() {
    use tak_input::WorldView;
    for on_pad in [false, true] {
        for invalid in ["enemy", "zero_health", "dying", "despawned"] {
            let (mut app, window, factory, _, target) = guard_fixture(false, true);
            let start = commands(&app);
            let selected = selection(&app);
            let pad = on_pad.then(|| guard_pad(&mut app, window));
            if let Some(pad) = pad {
                pad_guard_mode(&mut app, pad);
                pad_button(&mut app, pad, GamepadButton::West, true);
            } else {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyG);
                frame(&mut app);
            }
            {
                // Explicit synthetic fixture mutation between hardware polls,
                // not an owned-data shortcut or a delayed stale UnitInfo mock.
                let mut sim = app.world_mut().non_send_mut::<SimRes>();
                let world = sim.session.sim.world_mut();
                match invalid {
                    "enemy" => world.unit_mut(target).unwrap().owner = PlayerId(1),
                    "zero_health" => world.unit_mut(target).unwrap().combat.health = 0,
                    "dying" => {
                        world.unit_mut(target).unwrap().combat.dying =
                            Some(tak_sim::combat::Dying {
                                since: 0,
                                severity: 100,
                                kind: 0,
                                done: false,
                            })
                    }
                    "despawned" => {
                        world.despawn(target).unwrap();
                    }
                    _ => unreachable!(),
                }
                assert!(!sim.view().can_factory_guard_target(target), "{invalid}");
            }
            app.world_mut().resource_mut::<Feel>().feedback.clear();
            if let Some(pad) = pad {
                pad_button(&mut app, pad, GamepadButton::West, false);
            } else {
                click(&mut app, MouseButton::Left);
            }
            assert_eq!(commands(&app), start, "on_pad={on_pad}, target={invalid}");
            assert_eq!(guard_acknowledgements(&app), 0);
            assert_eq!(selection(&app), selected);
            assert!(
                app.world()
                    .non_send::<SimRes>()
                    .session
                    .sim
                    .world()
                    .unit(factory)
                    .unwrap()
                    .build
                    .factory_rally
                    .is_none()
            );
        }
    }
}

#[test]
fn modal_suppresses_held_factory_guard_release_and_allows_explicit_cancel() {
    for modal in [KeyCode::F11, KeyCode::F8] {
        let (mut app, window, factory, mobile, target) = guard_fixture(true, true);
        app.world_mut().resource_mut::<Feel>().quick_save_dir =
            Err("Synthetic Guard test has no save directory".into());
        let pad = guard_pad(&mut app, window);
        pad_guard_mode(&mut app, pad);
        let start = commands(&app);
        let selected = selection(&app);
        pad_button(&mut app, pad, GamepadButton::West, true);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(modal);
        pad_frame(&mut app, pad);
        assert_eq!(
            app.world().resource::<InputRes>().mapper.mode(),
            Mode::Guard
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(modal);
        pad_button(&mut app, pad, GamepadButton::West, false);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        pad_frame(&mut app, pad);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Escape);
        pad_frame(&mut app, pad);
        assert_eq!(commands(&app), start, "closing modal replayed held Guard");
        assert_eq!(selection(&app), selected);
        // Modals preserve the already chosen mode, but not its held trigger.
        // A fresh B explicitly cancels it without cancelling the selection.
        pad_button(&mut app, pad, GamepadButton::East, true);
        pad_button(&mut app, pad, GamepadButton::East, false);
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
        assert_eq!(selection(&app), selected);
        assert_eq!(commands(&app), start);
        app.world_mut().resource_mut::<Feel>().feedback.clear();
        pad_button(&mut app, pad, GamepadButton::West, true);
        pad_button(&mut app, pad, GamepadButton::West, false);
        assert_guard_dispatch_preserves_work(
            &mut app,
            factory,
            mobile,
            target,
            start.len(),
            true,
            QueueMode::Replace,
        );
    }
}

// Shift-drag construction is exercised through the same Bevy poll as real
// mouse/keyboard/controller input. The scenario and geometry are synthetic.
fn placement_fixture(profile: Profile) -> (App, Entity, Entity) {
    let (mut app, window, panel) = fixture(profile, false);
    button(&mut app, ButtonKind::Build(0));
    click(&mut app, MouseButton::Left);
    assert!(commands(&app).is_empty());
    assert_eq!(
        app.world().resource::<InputRes>().mapper.mode(),
        Mode::Build(UnitDefId(1))
    );
    (app, window, panel)
}

fn placement_pointer(app: &mut App, window: Entity, x: i32, z: i32) {
    let screen = app
        .world()
        .resource::<CamState>()
        .geom
        .to_screen([x as f32, 0.0, z as f32]);
    pointer(app, window, Vec2::new(screen[0], screen[1]));
}

fn placement_sample(app: &mut App, window: Entity, x: i32, z: i32) {
    placement_pointer(app, window, x, z);
    frame(app);
}

fn placement_down(app: &mut App, window: Entity, x: i32, z: i32) {
    placement_sample(app, window, x, z);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ShiftLeft);
    frame(app);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    frame(app);
}

fn placement_up(app: &mut App) {
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::ShiftLeft);
    frame(app);
}

fn assert_placement_sites(app: &App, selected: &UnitSet, points: &[(i32, i32)]) {
    let expected: Vec<_> = points
        .iter()
        .map(|&(x, z)| SimCommand::Build {
            units: selected.clone(),
            def: UnitDefId(1),
            at: Some(tak_math::Vec2::from_ints(x, z)),
            queue: QueueMode::Append,
        })
        .collect();
    assert_eq!(commands(app), expected);
    assert_eq!(selection(app), *selected);
}

#[test]
fn shift_placement_small_click_keeps_ordinary_unsnapped_click_semantics() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for shifted in [false, true] {
            let (mut app, window, _) = placement_fixture(profile.clone());
            let selected = selection(&app);
            placement_sample(&mut app, window, 291, 123);
            if shifted {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::ShiftLeft);
                frame(&mut app);
            }
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            frame(&mut app);
            assert!(commands(&app).is_empty(), "mouse down placed prematurely");
            // Two screen pixels stay below the ordinary selection threshold.
            let screen = app
                .world()
                .get::<Window>(window)
                .unwrap()
                .cursor_position()
                .unwrap();
            pointer(&mut app, window, screen + Vec2::new(2.0, 0.0));
            frame(&mut app);
            assert!(commands(&app).is_empty(), "a small click became a stroke");
            let target = app.world().resource::<InputRes>().mapper.cursor_world();
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(MouseButton::Left);
            frame(&mut app);
            assert_eq!(
                commands(&app),
                vec![SimCommand::Build {
                    units: selected.clone(),
                    def: UnitDefId(1),
                    at: Some(target),
                    queue: if shifted {
                        QueueMode::Append
                    } else {
                        QueueMode::Replace
                    },
                }]
            );
            assert_eq!(selection(&app), selected);
            assert_eq!(
                app.world().resource::<InputRes>().mapper.mode(),
                if shifted {
                    Mode::Build(UnitDefId(1))
                } else {
                    Mode::None
                }
            );
        }
    }
}

#[test]
fn shift_placement_drag_samples_once_without_interpolation_overlap_or_selection() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        let (mut app, window, _) = placement_fixture(profile);
        let selected = selection(&app);
        let camera = app.world().resource::<CamState>().geom.target;
        let hint_context = crate::feel::HintCx {
            has_units: true,
            has_builder: true,
            ..default()
        };
        let mapper = &app.world().resource::<InputRes>().mapper;
        assert!(
            crate::feel::hints_classic(mapper, hint_context)
                .iter()
                .any(|hint| hint.text == "held + drag: queue sites")
        );
        assert!(
            !crate::feel::hints(mapper, hint_context)
                .iter()
                .any(|hint| hint.text.contains("drag"))
        );
        placement_down(&mut app, window, 241, 113);
        assert!(commands(&app).is_empty());
        placement_sample(&mut app, window, 305, 113);
        assert_placement_sites(&app, &selected, &[(240, 112), (304, 112)]);
        // An adjacent anchor still overlaps the previous two-cell footprint.
        placement_sample(&mut app, window, 321, 113);
        placement_sample(&mut app, window, 305, 113);
        placement_sample(&mut app, window, 241, 113);
        for _ in 0..4 {
            frame(&mut app);
        }
        assert_placement_sites(&app, &selected, &[(240, 112), (304, 112)]);
        placement_sample(&mut app, window, 353, 113);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        frame(&mut app);
        frame(&mut app);
        assert_placement_sites(&app, &selected, &[(240, 112), (304, 112), (352, 112)]);
        assert_eq!(
            app.world().resource::<InputRes>().mapper.mode(),
            Mode::Build(UnitDefId(1))
        );
        assert_eq!(app.world().resource::<CamState>().geom.target, camera);
        assert!(!app.world().resource::<MinimapState>().mouse_drag);
    }
}

#[test]
fn shift_placement_release_in_same_poll_discards_unsent_sample_and_never_replaces() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for mouse_release in [false, true] {
            let (mut app, window, _) = placement_fixture(profile.clone());
            let selected = selection(&app);
            placement_down(&mut app, window, 240, 112);
            placement_sample(&mut app, window, 304, 112);
            assert_placement_sites(&app, &selected, &[(240, 112), (304, 112)]);
            placement_pointer(&mut app, window, 352, 112);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::ShiftLeft);
            if mouse_release {
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .release(MouseButton::Left);
            }
            frame(&mut app);
            placement_sample(&mut app, window, 384, 112);
            placement_up(&mut app);
            assert_placement_sites(&app, &selected, &[(240, 112), (304, 112)]);
            // The consumed gesture does not poison the next genuine press.
            perform(&mut app, Action::BuildOption { index: 0 });
            placement_down(&mut app, window, 240, 208);
            placement_sample(&mut app, window, 304, 208);
            placement_up(&mut app);
            assert_placement_sites(
                &app,
                &selected,
                &[(240, 112), (304, 112), (240, 208), (304, 208)],
            );
        }
    }
}

#[test]
fn shift_placement_cancel_and_stop_consume_old_release_then_fresh_drag_recovers() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for key in [KeyCode::Escape, KeyCode::KeyS] {
            let (mut app, window, _) = placement_fixture(profile.clone());
            let selected = selection(&app);
            placement_down(&mut app, window, 240, 112);
            placement_sample(&mut app, window, 304, 112);
            let mut expected = commands(&app);
            if key == KeyCode::KeyS {
                expected.push(SimCommand::Stop {
                    units: selected.clone(),
                });
            }
            // Cancellation shares a poll with a new, otherwise valid site.
            placement_pointer(&mut app, window, 352, 112);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
            frame(&mut app);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(key);
            placement_sample(&mut app, window, 384, 112);
            placement_up(&mut app);
            assert_eq!(commands(&app), expected, "{key:?} leaked the old stroke");
            assert_eq!(selection(&app), selected);
            perform(&mut app, Action::BuildOption { index: 0 });
            placement_down(&mut app, window, 240, 208);
            placement_sample(&mut app, window, 304, 208);
            placement_up(&mut app);
            expected.extend(
                [(240, 208), (304, 208)]
                    .into_iter()
                    .map(|(x, z)| SimCommand::Build {
                        units: selected.clone(),
                        def: UnitDefId(1),
                        at: Some(tak_math::Vec2::from_ints(x, z)),
                        queue: QueueMode::Append,
                    }),
            );
            assert_eq!(commands(&app), expected, "{key:?} prevented a fresh stroke");
            assert_eq!(selection(&app), selected);
        }
    }
}

#[test]
fn shift_placement_surface_and_focus_boundaries_terminate_until_fresh_mouse_down() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for boundary in ["hud", "minimap", "focus", "window", "terrain"] {
            let (mut app, window, _) = placement_fixture(profile.clone());
            let selected = selection(&app);
            placement_down(&mut app, window, 240, 112);
            placement_sample(&mut app, window, 304, 112);
            let expected = commands(&app);
            record_next_input_poll(&mut app);
            match boundary {
                "hud" => pointer(&mut app, window, HUD_POS),
                "minimap" => {
                    let pos = {
                        let mut minimap = app.world_mut().resource_mut::<MinimapState>();
                        minimap.image = Some((200, 100));
                        minimap.map = [640.0, 320.0];
                        let geom = minimap.geom([800.0, 600.0]).unwrap();
                        Vec2::new(
                            geom.rect[0] + geom.rect[2] * 0.5,
                            geom.rect[1] + geom.rect[3] * 0.5,
                        )
                    };
                    pointer(&mut app, window, pos);
                }
                "focus" => app.world_mut().get_mut::<Window>(window).unwrap().focused = false,
                "window" => app
                    .world_mut()
                    .get_mut::<Window>(window)
                    .unwrap()
                    .set_cursor_position(None),
                "terrain" => {
                    // Still inside the window, but the ray now misses the map.
                    app.world_mut().resource_mut::<CamState>().geom.target = [-1000.0, -1000.0];
                }
                _ => unreachable!(),
            }
            frame(&mut app);
            assert_eq!(
                commands(&app),
                expected,
                "{boundary} queued a boundary sample"
            );
            assert!(
                app.world()
                    .resource::<Feel>()
                    .recorder
                    .as_ref()
                    .unwrap()
                    .recording()
                    .frames
                    .iter()
                    .any(|frame| frame.events.iter().any(|event| matches!(
                        event,
                        EventKind::PointerLeaveTerrain
                    ) || boundary == "minimap"
                        && matches!(event, EventKind::MapPointer { .. }))),
                "{boundary} termination must remain reproducible in the input recording"
            );
            app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
            app.world_mut().resource_mut::<CamState>().geom.target = [320.0, 160.0];
            app.world_mut().resource_mut::<MinimapState>().image = None;
            placement_sample(&mut app, window, 352, 112);
            placement_up(&mut app);
            assert_eq!(
                commands(&app),
                expected,
                "{boundary} allowed old stroke to resume"
            );
            assert_eq!(selection(&app), selected);
            perform(&mut app, Action::BuildOption { index: 0 });
            placement_down(&mut app, window, 240, 208);
            placement_sample(&mut app, window, 304, 208);
            placement_up(&mut app);
            assert_placement_sites(
                &app,
                &selected,
                &[(240, 112), (304, 112), (240, 208), (304, 208)],
            );
        }
    }
}

#[test]
fn shift_placement_modal_opening_rolls_back_same_poll_samples_and_feedback() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for key in [KeyCode::F11, KeyCode::F8, KeyCode::F7] {
            let (mut app, window, _) = placement_fixture(profile.clone());
            let selected = selection(&app);
            if key == KeyCode::F7 {
                app.insert_resource(skirmish_ui::SkirmishState::new(Default::default(), false));
                app.init_resource::<skirmish_ui::PendingRequest>();
            }
            app.world_mut().resource_mut::<Feel>().quick_save_dir =
                Err("Synthetic placement test has no checkpoint directory".into());
            placement_down(&mut app, window, 240, 112);
            placement_sample(&mut app, window, 304, 112);
            let expected = commands(&app);
            record_next_input_poll(&mut app);
            placement_pointer(&mut app, window, 352, 112);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
            frame(&mut app);
            assert_eq!(commands(&app), expected, "{key:?} opening leaked a sample");
            assert_modal_recording_has_no_gameplay(&app);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(key);
            placement_up(&mut app);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Escape);
            frame(&mut app);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::Escape);
            frame(&mut app);
            frame(&mut app);
            assert_eq!(
                commands(&app),
                expected,
                "closing {key:?} replayed the stroke"
            );
            assert_eq!(selection(&app), selected);
            perform(&mut app, Action::BuildOption { index: 0 });
            placement_down(&mut app, window, 240, 208);
            placement_sample(&mut app, window, 304, 208);
            placement_up(&mut app);
            assert_placement_sites(
                &app,
                &selected,
                &[(240, 112), (304, 112), (240, 208), (304, 208)],
            );
        }
    }
}

#[test]
fn shift_placement_skips_whole_footprint_invalid_terrain_and_continues_on_valid_samples() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        let (mut app, window, _) = placement_fixture(profile);
        let selected = selection(&app);
        let mut scenario = tak_headless::bot::demo::scenario(false);
        let mut flags = vec![0; 40 * 20];
        // The centre cell at (19, 7) is clear; another covered cell is not.
        flags[6 * 40 + 18] = tak_sim::map::terrain::STEEP;
        scenario.map.terrain = Some(tak_sim::map::TerrainGrid {
            cols: 40,
            rows: 20,
            cell_size: 16,
            flags,
        });
        replace_sim(&mut app, &scenario);
        placement_down(&mut app, window, 240, 112);
        placement_sample(&mut app, window, 304, 112);
        assert_placement_sites(&app, &selected, &[(240, 112)]);
        placement_sample(&mut app, window, 352, 112);
        placement_up(&mut app);
        assert_placement_sites(&app, &selected, &[(240, 112), (352, 112)]);
    }
}

#[test]
fn shift_placement_rechecks_stale_foreign_and_unfinished_builders_at_dispatch() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        for invalid in ["stale", "foreign", "unfinished"] {
            for started in [false, true] {
                let (mut app, window, _) = placement_fixture(profile.clone());
                let selected = selection(&app);
                let builder = *selected.iter().next().unwrap();
                if started {
                    placement_down(&mut app, window, 240, 112);
                    placement_sample(&mut app, window, 304, 112);
                }
                let expected = commands(&app);
                {
                    let mut sim = app.world_mut().non_send_mut::<SimRes>();
                    let world = sim.session.sim.world_mut();
                    match invalid {
                        "stale" => {
                            world.despawn(builder).unwrap();
                        }
                        "foreign" => world.unit_mut(builder).unwrap().owner = PlayerId(1),
                        "unfinished" => {
                            world.unit_mut(builder).unwrap().build.frame =
                                Some(tak_sim::construction::Frame::new(100))
                        }
                        _ => unreachable!(),
                    }
                }
                if !started {
                    placement_down(&mut app, window, 240, 112);
                }
                placement_sample(&mut app, window, 352, 112);
                placement_up(&mut app);
                assert_eq!(
                    commands(&app),
                    expected,
                    "started={started}, builder={invalid}"
                );
            }
        }
    }
}

#[test]
fn shift_placement_rapid_queued_click_then_drag_does_not_become_same_type_selection() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        let (mut app, window, _) = placement_fixture(profile);
        let selected = selection(&app);
        placement_down(&mut app, window, 241, 113);
        let first = app.world().resource::<InputRes>().mapper.cursor_world();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        frame(&mut app);
        let mut expected = vec![SimCommand::Build {
            units: selected.clone(),
            def: UnitDefId(1),
            at: Some(first),
            queue: QueueMode::Append,
        }];
        assert_eq!(commands(&app), expected);
        // Time is intentionally unchanged, so the new press is within even
        // the shortest double-tap window. Shift remains held throughout.
        placement_down(&mut app, window, 240, 208);
        placement_sample(&mut app, window, 304, 208);
        placement_up(&mut app);
        expected.extend(
            [(240, 208), (304, 208)]
                .into_iter()
                .map(|(x, z)| SimCommand::Build {
                    units: selected.clone(),
                    def: UnitDefId(1),
                    at: Some(tak_math::Vec2::from_ints(x, z)),
                    queue: QueueMode::Append,
                }),
        );
        assert_eq!(
            commands(&app),
            expected,
            "a prior click consumed the new build drag"
        );
        assert_eq!(selection(&app), selected);
    }
}

#[test]
fn shift_placement_one_stroke_has_a_hard_sixty_four_command_budget() {
    for profile in [Profile::default_combined(), Profile::classic_tak()] {
        let (mut app, window, panel) = placement_fixture(profile);
        *app.world_mut()
            .get_mut::<InheritedVisibility>(panel)
            .unwrap() = InheritedVisibility::HIDDEN;
        // Hide the synthetic build button as well, so all sampled positions
        // remain terrain rather than accidentally crossing a UI hit target.
        let buttons: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<ButtonKind>>()
            .iter(app.world())
            .collect();
        for entity in buttons {
            *app.world_mut()
                .get_mut::<InheritedVisibility>(entity)
                .unwrap() = InheritedVisibility::HIDDEN;
        }
        app.world_mut().resource_mut::<CamState>().geom.zoom = 1.0;
        let selected = selection(&app);
        let sites: Vec<_> = [32, 64, 96, 224, 256]
            .into_iter()
            .flat_map(|z| (32..=608).step_by(32).map(move |x| (x, z)))
            .collect();
        placement_down(&mut app, window, sites[0].0, sites[0].1);
        for &(x, z) in &sites[1..] {
            placement_sample(&mut app, window, x, z);
        }
        placement_up(&mut app);
        assert_placement_sites(&app, &selected, &sites[..64]);
    }
}

#[test]
fn shift_placement_does_not_turn_gamepad_queued_build_into_a_mouse_stroke() {
    use bevy::input::gamepad::GamepadInput;
    let mut profile = Profile::default_combined();
    profile.tuning.reticle = false;
    let (mut app, window, _) = placement_fixture(profile);
    let selected = selection(&app);
    let pad = guard_pad(&mut app, window);
    pad_button(&mut app, pad, GamepadButton::RightTrigger, true);
    pad_button(&mut app, pad, GamepadButton::West, true);
    assert!(commands(&app).is_empty());
    let start = app.world().resource::<InputRes>().mapper.cursor_world();
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .analog_mut()
        .set(GamepadInput::Axis(GamepadAxis::LeftStickX), -1.0);
    for _ in 0..2 {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(40));
        pad_frame(&mut app, pad);
    }
    app.world_mut()
        .get_mut::<Gamepad>(pad)
        .unwrap()
        .analog_mut()
        .set(GamepadInput::Axis(GamepadAxis::LeftStickX), 0.0);
    pad_frame(&mut app, pad);
    assert_ne!(
        app.world().resource::<InputRes>().mapper.cursor_world(),
        start
    );
    assert!(
        commands(&app).is_empty(),
        "held controller context painted terrain"
    );
    let target = app.world().resource::<InputRes>().mapper.cursor_world();
    pad_button(&mut app, pad, GamepadButton::West, false);
    assert_eq!(
        commands(&app),
        vec![SimCommand::Build {
            units: selected.clone(),
            def: UnitDefId(1),
            at: Some(target),
            queue: QueueMode::Append,
        }]
    );
    pad_button(&mut app, pad, GamepadButton::RightTrigger, false);
    assert_eq!(commands(&app).len(), 1);
    assert_eq!(selection(&app), selected);
    assert_eq!(
        app.world().resource::<InputRes>().mapper.mode(),
        Mode::Build(UnitDefId(1))
    );
}
