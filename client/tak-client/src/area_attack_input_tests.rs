//! Asset-free area-attack regressions through the real Bevy input schedule.
use super::*;
use tak_commands::{QueueMode, Scope, SimCommand, Target, UnitDefId, UnitSet};

const START: (i32, i32) = (250, 110);
const END: (i32, i32) = (390, 210);
const HUD: Vec2 = Vec2::new(700.0, 300.0);

fn profiles() -> [Profile; 2] {
    [
        Profile::default_combined(),
        Profile::named("original").unwrap(),
    ]
}

fn fixture(profile: Profile) -> (App, Entity) {
    let mut app = super::overlay_input_tests::app();
    app.world_mut().resource_mut::<Feel>().overlay_open = false;
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .set_profile(profile);
    {
        let mut state = app.world_mut().non_send_mut::<SimRes>();
        let world = state.session.sim.world_mut();
        let ids: Vec<_> = world.units().map(|(id, _)| id).collect();
        for id in ids {
            if world.unit(id).unwrap().owner == PlayerId(0) {
                world.unit_mut(id).unwrap().pos = tak_math::Vec2::from_ints(200, 160);
            } else {
                world.despawn(id).unwrap();
            }
        }
    }
    let mut camera = app.world_mut().resource_mut::<CamState>();
    camera.geom.target = [320.0, 160.0];
    camera.geom.zoom = 0.5;
    let mut window = Window {
        resolution: WindowResolution::new(800, 600),
        ..default()
    };
    window.set_cursor_position(Some(Vec2::new(400.0, 300.0)));
    let window = app.world_mut().spawn((window, PrimaryWindow)).id();
    app.world_mut().spawn((
        HudPanel,
        ComputedNode {
            size: Vec2::new(100.0, 140.0),
            ..default()
        },
        UiGlobalTransform::from(bevy::math::Affine2::from_translation(HUD)),
        InheritedVisibility::VISIBLE,
    ));
    select_all(&mut app);
    assert_eq!(selection(&app).len(), 1);
    (app, window)
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

fn select_all(app: &mut App) {
    perform(app, Action::SelectAll { scope: Scope::Map });
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

fn enemy(app: &mut App, pos: (i32, i32)) -> UnitId {
    spawn(app, PlayerId(1), UnitDefId(4), pos)
}

fn frame(app: &mut App) {
    app.update();
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

fn world_pointer(app: &mut App, window: Entity, pos: (i32, i32)) {
    let p = app
        .world()
        .resource::<CamState>()
        .geom
        .to_screen([pos.0 as f32, 0.0, pos.1 as f32]);
    pointer(app, window, Vec2::new(p[0], p[1]));
}

fn key(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
    frame(app);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(key);
    frame(app);
}

fn arm(app: &mut App) {
    key(app, KeyCode::KeyA);
    assert_eq!(
        app.world().resource::<InputRes>().mapper.mode(),
        Mode::Attack
    );
}

fn down(app: &mut App, window: Entity, pos: (i32, i32)) {
    world_pointer(app, window, pos);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    frame(app);
}

fn up(app: &mut App, window: Entity, pos: (i32, i32)) {
    world_pointer(app, window, pos);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    frame(app);
}

fn drag(app: &mut App, window: Entity) {
    down(app, window, START);
    up(app, window, END);
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

fn attacks(units: &UnitSet, targets: &[UnitId], append: bool) -> Vec<SimCommand> {
    targets
        .iter()
        .enumerate()
        .map(|(index, &target)| SimCommand::Attack {
            units: units.clone(),
            target: Target::Unit(target),
            queue: if append || index != 0 {
                QueueMode::Append
            } else {
                QueueMode::Replace
            },
        })
        .collect()
}

fn record(app: &mut App) {
    app.world_mut()
        .resource_mut::<InputRes>()
        .mapper
        .take_feedback();
    let recorder = tak_input::Recorder::new(
        serde_json::json!({"synthetic": "area attack adapter"}),
        &app.world().resource::<InputRes>().mapper,
    );
    let mut feel = app.world_mut().resource_mut::<Feel>();
    feel.feedback.clear();
    feel.recorder = Some(recorder);
}

#[test]
fn explicit_attack_rectangle_queues_sorted_targets_only_on_release_and_preserves_army() {
    for profile in profiles() {
        for shifted in [false, true] {
            let (mut app, window) = fixture(profile.clone());
            let selected = selection(&app);
            // Slot order deliberately opposes spatial order.
            let first = enemy(&mut app, (360, 180));
            let second = enemy(&mut app, (280, 140));
            enemy(&mut app, (410, 160));
            // This is inside the old selection circle, but outside the rectangle.
            enemy(&mut app, (320, 100));
            arm(&mut app);
            if shifted {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::ShiftLeft);
            }
            down(&mut app, window, START);
            assert!(
                app.world()
                    .resource::<InputRes>()
                    .mapper
                    .attack_area_bounds()
                    .is_none()
            );
            world_pointer(&mut app, window, END);
            frame(&mut app);
            assert_eq!(
                app.world()
                    .resource::<InputRes>()
                    .mapper
                    .attack_area_bounds(),
                Some((
                    tak_math::Vec2::from_ints(START.0, START.1),
                    tak_math::Vec2::from_ints(END.0, END.1)
                ))
            );
            assert!(
                commands(&app).is_empty(),
                "preview must not submit an attack"
            );
            assert_eq!(selection(&app), selected);
            // Simultaneous modifier release still applies to this mouse release.
            if shifted {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .release(KeyCode::ShiftLeft);
            }
            up(&mut app, window, END);
            assert!(
                app.world()
                    .resource::<InputRes>()
                    .mapper
                    .attack_area_bounds()
                    .is_none()
            );
            assert_eq!(
                commands(&app),
                attacks(&selected, &[first, second], shifted)
            );
            assert_eq!(selection(&app), selected);
            assert_eq!(
                app.world().resource::<InputRes>().mapper.mode(),
                if shifted { Mode::Attack } else { Mode::None }
            );
            assert!(!app.world().resource::<MinimapState>().mouse_drag);
            frame(&mut app);
            assert_eq!(commands(&app).len(), 2, "idle Frame repeated the queue");
        }
    }
}

#[test]
fn reverse_rectangle_and_exact_boundary_targets_use_the_same_inclusive_bounds() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let selected = selection(&app);
        let targets = [enemy(&mut app, START), enemy(&mut app, END)];
        enemy(&mut app, (START.0 - 1, START.1));
        enemy(&mut app, (END.0 + 1, END.1));
        arm(&mut app);
        down(&mut app, window, END);
        up(&mut app, window, START);
        assert_eq!(commands(&app), attacks(&selected, &targets, false));
        assert_eq!(selection(&app), selected);
    }
}

#[test]
fn empty_or_only_friendly_rectangle_never_attacks_ground_or_replaces_selection() {
    for profile in profiles() {
        for friendly in [false, true] {
            let (mut app, window) = fixture(profile.clone());
            if friendly {
                spawn(&mut app, PlayerId(0), UnitDefId(3), (320, 160));
            }
            let selected = selection(&app);
            arm(&mut app);
            drag(&mut app, window);
            assert!(commands(&app).is_empty());
            assert_eq!(selection(&app), selected);
            assert_eq!(
                app.world().resource::<InputRes>().mapper.mode(),
                Mode::Attack
            );
        }
    }
}

#[test]
fn release_snapshot_excludes_fog_hidden_targets_even_if_previously_visible() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let selected = selection(&app);
        let visible = enemy(&mut app, (280, 136));
        enemy(&mut app, (360, 184));
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            sim.fog_on = true;
            sim.fog.update(&[(320.0, 160.0, 150.0)]);
        }
        arm(&mut app);
        down(&mut app, window, START);
        app.world_mut()
            .non_send_mut::<SimRes>()
            .fog
            .update(&[(280.0, 136.0, 16.0)]);
        up(&mut app, window, END);
        assert_eq!(commands(&app), attacks(&selected, &[visible], false));
        assert_eq!(selection(&app), selected);
    }
}

#[test]
fn targets_are_revalidated_for_death_hostility_position_and_full_generation_at_release() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let selected = selection(&app);
        let stale = enemy(&mut app, (280, 130));
        let dead = enemy(&mut app, (300, 150));
        let friendly = enemy(&mut app, (320, 170));
        let moved = enemy(&mut app, (340, 190));
        arm(&mut app);
        down(&mut app, window, START);
        {
            let mut sim = app.world_mut().non_send_mut::<SimRes>();
            let world = sim.session.sim.world_mut();
            world.despawn(stale).unwrap();
            world.unit_mut(dead).unwrap().combat.dying = Some(tak_sim::combat::Dying {
                since: 1,
                severity: 100,
                kind: 0,
                done: false,
            });
            world.unit_mut(friendly).unwrap().owner = PlayerId(0);
            world.unit_mut(moved).unwrap().pos = tak_math::Vec2::from_ints(430, 160);
        }
        let replacement = enemy(&mut app, (360, 180));
        assert_eq!(stale.index, replacement.index);
        assert_ne!(stale.generation, replacement.generation);
        let newcomer = enemy(&mut app, (280, 140));
        up(&mut app, window, END);
        assert_eq!(
            commands(&app),
            attacks(&selected, &[replacement, newcomer], false)
        );
        assert_eq!(selection(&app), selected);
    }
}

#[test]
fn mixed_factory_and_mobile_selection_emits_only_mobile_attacks_and_no_factory_orders() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let mobile = spawn(&mut app, PlayerId(0), UnitDefId(3), (210, 170));
        let factory = spawn(&mut app, PlayerId(0), UnitDefId(2), (210, 140));
        select_all(&mut app);
        let selected = selection(&app);
        let attackers = selected
            .iter()
            .copied()
            .filter(|id| *id != factory)
            .collect();
        assert!(selected.contains(mobile) && selected.contains(factory));
        let target = enemy(&mut app, (320, 160));
        arm(&mut app);
        drag(&mut app, window);
        assert_eq!(commands(&app), attacks(&attackers, &[target], false));
        assert_eq!(selection(&app), selected);
    }
}

#[test]
fn stale_attacker_generation_cannot_transfer_selection_or_orders_to_replacement() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let stale = selection(&app).as_slice()[0];
        enemy(&mut app, (320, 160));
        arm(&mut app);
        down(&mut app, window, START);
        app.world_mut()
            .non_send_mut::<SimRes>()
            .session
            .sim
            .world_mut()
            .despawn(stale)
            .unwrap();
        let replacement = spawn(&mut app, PlayerId(0), UnitDefId(3), (200, 160));
        assert_eq!(stale.index, replacement.index);
        assert_ne!(stale.generation, replacement.generation);
        up(&mut app, window, END);
        assert!(commands(&app).is_empty());
        assert!(!selection(&app).contains(replacement));
    }
}

#[test]
fn factory_only_attack_rectangle_preserves_selection_without_production_or_ground_orders() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let selected = selection(&app);
        let id = selected.as_slice()[0];
        app.world_mut()
            .non_send_mut::<SimRes>()
            .session
            .sim
            .world_mut()
            .unit_mut(id)
            .unwrap()
            .def = UnitDefId(2);
        enemy(&mut app, (320, 160));
        arm(&mut app);
        drag(&mut app, window);
        assert!(commands(&app).is_empty());
        assert_eq!(selection(&app), selected);
        assert_eq!(
            app.world().resource::<InputRes>().mapper.mode(),
            Mode::Attack
        );
    }
}

#[test]
fn missing_world_ray_at_press_cannot_start_a_rectangle_or_fall_through_on_release() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let selected = selection(&app);
        let target = enemy(&mut app, (320, 160));
        arm(&mut app);
        app.world_mut().resource_mut::<CamState>().geom.target = [-1000.0, -1000.0];
        pointer(&mut app, window, Vec2::new(260.0, 200.0));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        frame(&mut app);
        app.world_mut().resource_mut::<CamState>().geom.target = [320.0, 160.0];
        up(&mut app, window, END);
        assert!(commands(&app).is_empty());
        assert_eq!(selection(&app), selected);
        assert!(
            app.world()
                .resource::<InputRes>()
                .mapper
                .attack_area_bounds()
                .is_none()
        );
        arm(&mut app);
        drag(&mut app, window);
        assert_eq!(commands(&app), attacks(&selected, &[target], false));
    }
}

#[test]
fn area_attack_hint_requires_explicit_attack_and_primary_drag_and_never_appears_for_pad() {
    for profile in profiles() {
        let (mut app, _) = fixture(profile);
        let context = crate::feel::HintCx {
            has_units: true,
            ..default()
        };
        let hint = |app: &App| {
            crate::feel::hints_classic(&app.world().resource::<InputRes>().mapper, context)
                .iter()
                .any(|hint| hint.text == "queue visible enemies")
        };
        assert!(!hint(&app));
        arm(&mut app);
        assert!(hint(&app));
        assert!(
            !crate::feel::hints(&app.world().resource::<InputRes>().mapper, context)
                .iter()
                .any(|hint| hint.text == "queue visible enemies")
        );
        let mut remapped = app.world().resource::<InputRes>().mapper.profile().clone();
        for binding in &mut remapped.bindings {
            if binding.input == tak_input::Input::Button(InButton::MouseLeft)
                && binding.gesture == tak_input::Gesture::Press
            {
                binding.action = Action::Context { append: false };
            }
        }
        app.world_mut()
            .resource_mut::<InputRes>()
            .mapper
            .set_profile(remapped);
        arm(&mut app);
        assert!(!hint(&app));
    }
}

#[test]
fn attack_rectangle_caps_at_64_sorted_full_ids_without_repeated_frame_submissions() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let selected = selection(&app);
        let mut targets = Vec::new();
        for i in 0..75 {
            targets.push(enemy(&mut app, (270 + i % 12 * 8, 130 + i / 12 * 8)));
        }
        // Retire an early slot so generation is part of the expected target.
        let stale = targets[2];
        app.world_mut()
            .non_send_mut::<SimRes>()
            .session
            .sim
            .world_mut()
            .despawn(stale)
            .unwrap();
        targets[2] = enemy(&mut app, (320, 160));
        targets.sort();
        arm(&mut app);
        drag(&mut app, window);
        for _ in 0..4 {
            frame(&mut app);
        }
        assert_eq!(commands(&app), attacks(&selected, &targets[..64], false));
        assert_eq!(selection(&app), selected);
    }
}

#[test]
fn ordinary_large_drag_still_selects_friendlies_without_any_attack() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let friendly = spawn(&mut app, PlayerId(0), UnitDefId(3), (320, 160));
        enemy(&mut app, (300, 160));
        drag(&mut app, window);
        assert!(commands(&app).is_empty());
        assert_eq!(selection(&app), UnitSet::single(friendly));
        assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::None);
    }
}

#[test]
fn explicit_short_click_and_same_poll_click_keep_single_target_attack_semantics() {
    for profile in profiles() {
        for same_poll in [false, true] {
            let (mut app, window) = fixture(profile.clone());
            let selected = selection(&app);
            let target = enemy(&mut app, (320, 160));
            arm(&mut app);
            if same_poll {
                world_pointer(&mut app, window, (320, 160));
                let mut mouse = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
                mouse.press(MouseButton::Left);
                mouse.release(MouseButton::Left);
                frame(&mut app);
            } else {
                down(&mut app, window, (320, 160));
                assert!(commands(&app).is_empty());
                up(&mut app, window, (321, 160));
            }
            assert_eq!(commands(&app), attacks(&selected, &[target], false));
            assert_eq!(selection(&app), selected);
        }
    }
}

#[test]
fn rapid_selection_click_then_attack_drag_is_not_consumed_by_stale_second_tap() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let selected = selection(&app);
        let target = enemy(&mut app, (320, 160));
        down(&mut app, window, (200, 160));
        up(&mut app, window, (200, 160));
        assert_eq!(selection(&app), selected);
        arm(&mut app);
        drag(&mut app, window);
        assert_eq!(commands(&app), attacks(&selected, &[target], false));
        assert_eq!(selection(&app), selected);
        // A second independently armed rectangle in the double-tap window
        // must still complete, rather than selecting every matching unit.
        arm(&mut app);
        drag(&mut app, window);
        let mut expected = attacks(&selected, &[target], false);
        expected.extend(attacks(&selected, &[target], false));
        assert_eq!(commands(&app), expected);
        assert_eq!(selection(&app), selected);
    }
}

#[test]
fn same_poll_cancel_or_stop_wins_over_released_attack_rectangle() {
    for profile in profiles() {
        for cancel in [KeyCode::Escape, KeyCode::KeyS] {
            let (mut app, window) = fixture(profile.clone());
            let selected = selection(&app);
            let target = enemy(&mut app, (320, 160));
            arm(&mut app);
            down(&mut app, window, START);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(cancel);
            up(&mut app, window, END);
            let expected = if cancel == KeyCode::KeyS {
                vec![SimCommand::Stop {
                    units: selected.clone(),
                }]
            } else {
                Vec::new()
            };
            assert_eq!(
                commands(&app),
                expected,
                "{cancel:?} lost same-poll arbitration"
            );
            assert_eq!(selection(&app), selected);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(cancel);
            frame(&mut app);
            arm(&mut app);
            drag(&mut app, window);
            let mut expected = expected;
            expected.extend(attacks(&selected, &[target], false));
            assert_eq!(commands(&app), expected, "fresh attack remained cancelled");
        }
    }
}

#[test]
fn same_poll_selection_or_mode_change_consumes_rectangle_without_leaking_an_order() {
    for profile in profiles() {
        for change_selection in [false, true] {
            let (mut app, window) = fixture(profile.clone());
            let original = selection(&app);
            let extra = spawn(&mut app, PlayerId(0), UnitDefId(3), (210, 170));
            let target = enemy(&mut app, (320, 160));
            arm(&mut app);
            down(&mut app, window, START);
            {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                if change_selection {
                    keys.press(KeyCode::ControlLeft);
                    keys.press(KeyCode::KeyA);
                } else {
                    keys.press(KeyCode::KeyM);
                }
            }
            up(&mut app, window, END);
            assert!(commands(&app).is_empty());
            assert!(
                app.world()
                    .resource::<InputRes>()
                    .mapper
                    .attack_area_bounds()
                    .is_none()
            );
            if change_selection {
                assert!(selection(&app).contains(extra));
                assert_eq!(selection(&app).len(), original.len() + 1);
            } else {
                assert_eq!(selection(&app), original);
                assert_eq!(app.world().resource::<InputRes>().mapper.mode(), Mode::Move);
            }
            {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                keys.release(KeyCode::ControlLeft);
                keys.release(KeyCode::KeyA);
                keys.release(KeyCode::KeyM);
            }
            frame(&mut app);
            let selected = selection(&app);
            arm(&mut app);
            drag(&mut app, window);
            assert_eq!(commands(&app), attacks(&selected, &[target], false));
        }
    }
}

#[test]
fn same_poll_attack_activation_precedes_primary_drag_for_actual_default_or_remapped_binding() {
    for profile in profiles() {
        for binding in ["default", "remapped", "chorded"] {
            let mut profile = profile.clone();
            let attack_key = if binding == "default" {
                KeyCode::KeyA
            } else {
                KeyCode::KeyQ
            };
            if binding != "default" {
                profile
                    .bindings
                    .retain(|b| b.input != tak_input::Input::Key(Key("q".into())));
                profile.bindings.push(tak_input::Binding {
                    input: tak_input::Input::Key(Key("q".into())),
                    with: (binding == "chorded").then(|| tak_input::Input::Key(Key("ctrl".into()))),
                    with_all: Vec::new(),
                    gesture: tak_input::Gesture::Press,
                    action: Action::AttackMode,
                });
            }
            let (mut app, window) = fixture(profile);
            let selected = selection(&app);
            let target = enemy(&mut app, (320, 160));
            world_pointer(&mut app, window, START);
            {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                keys.press(attack_key);
                if binding == "chorded" {
                    keys.press(KeyCode::ControlLeft);
                }
            }
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            frame(&mut app);
            assert!(commands(&app).is_empty());
            assert_eq!(
                app.world().resource::<InputRes>().mapper.mode(),
                Mode::Attack
            );
            up(&mut app, window, END);
            assert_eq!(
                commands(&app),
                attacks(&selected, &[target], false),
                "{binding}"
            );
            assert_eq!(selection(&app), selected);
        }
    }
}

#[test]
fn same_poll_attack_activation_cannot_override_cancel_stop_or_overlay_capture() {
    for profile in profiles() {
        for winning_key in [KeyCode::Escape, KeyCode::KeyS, KeyCode::F11] {
            let (mut app, window) = fixture(profile.clone());
            let selected = selection(&app);
            enemy(&mut app, (320, 160));
            world_pointer(&mut app, window, START);
            {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                keys.press(KeyCode::KeyA);
                keys.press(winning_key);
            }
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            frame(&mut app);
            up(&mut app, window, END);
            let expected = if winning_key == KeyCode::KeyS {
                vec![SimCommand::Stop {
                    units: selected.clone(),
                }]
            } else {
                Vec::new()
            };
            assert_eq!(
                commands(&app),
                expected,
                "{winning_key:?} lost to simultaneous Attack"
            );
            assert_eq!(selection(&app), selected);
            assert!(
                app.world()
                    .resource::<InputRes>()
                    .mapper
                    .attack_area_bounds()
                    .is_none()
            );
        }
    }
}

#[test]
fn hud_minimap_focus_cursor_and_missing_terrain_interrupt_and_consume_old_drag() {
    for profile in profiles() {
        for boundary in ["hud", "minimap", "focus", "cursor", "terrain"] {
            for release_at_boundary in [false, true] {
                let (mut app, window) = fixture(profile.clone());
                let selected = selection(&app);
                let target = enemy(&mut app, (320, 160));
                arm(&mut app);
                down(&mut app, window, START);
                world_pointer(&mut app, window, END);
                frame(&mut app);
                record(&mut app);
                match boundary {
                    "hud" => pointer(&mut app, window, HUD),
                    "minimap" => {
                        let pos = {
                            let mut minimap = app.world_mut().resource_mut::<MinimapState>();
                            minimap.image = Some((200, 100));
                            minimap.map = [640.0, 320.0];
                            let g = minimap.geom([800.0, 600.0]).unwrap();
                            Vec2::new(g.rect[0] + g.rect[2] / 2.0, g.rect[1] + g.rect[3] / 2.0)
                        };
                        pointer(&mut app, window, pos);
                    }
                    "focus" => app.world_mut().get_mut::<Window>(window).unwrap().focused = false,
                    "cursor" => app
                        .world_mut()
                        .get_mut::<Window>(window)
                        .unwrap()
                        .set_cursor_position(None),
                    "terrain" => {
                        app.world_mut().resource_mut::<CamState>().geom.target = [-1000.0, -1000.0]
                    }
                    _ => unreachable!(),
                }
                if release_at_boundary {
                    app.world_mut()
                        .resource_mut::<ButtonInput<MouseButton>>()
                        .release(MouseButton::Left);
                }
                frame(&mut app);
                assert!(
                    commands(&app).is_empty(),
                    "{boundary} submitted a boundary attack"
                );
                assert!(
                    app.world()
                        .resource::<InputRes>()
                        .mapper
                        .attack_area_bounds()
                        .is_none(),
                    "{boundary} left a visible targeting rectangle"
                );
                let recording = app
                    .world()
                    .resource::<Feel>()
                    .recorder
                    .as_ref()
                    .unwrap()
                    .recording();
                assert!(
                    recording
                        .frames
                        .iter()
                        .flat_map(|f| &f.events)
                        .any(|e| matches!(
                            e,
                            EventKind::PointerLeaveTerrain | EventKind::MapPointer { .. }
                        )),
                    "{boundary} was not recorded reproducibly"
                );
                app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
                app.world_mut().resource_mut::<CamState>().geom.target = [320.0, 160.0];
                app.world_mut().resource_mut::<MinimapState>().image = None;
                up(&mut app, window, END);
                frame(&mut app);
                assert!(
                    commands(&app).is_empty(),
                    "{boundary} allowed old release to resume"
                );
                assert_eq!(selection(&app), selected);
                arm(&mut app);
                drag(&mut app, window);
                assert_eq!(
                    commands(&app),
                    attacks(&selected, &[target], false),
                    "{boundary} poisoned fresh drag"
                );
            }
        }
    }
}

#[test]
fn same_poll_modal_opening_rolls_back_released_attack_and_closing_cannot_replay_it() {
    for profile in profiles() {
        for modal in [KeyCode::F11, KeyCode::F8, KeyCode::F7] {
            let (mut app, window) = fixture(profile.clone());
            let selected = selection(&app);
            let target = enemy(&mut app, (320, 160));
            if modal == KeyCode::F7 {
                app.insert_resource(skirmish_ui::SkirmishState::new(Default::default(), false));
                app.init_resource::<skirmish_ui::PendingRequest>();
            }
            app.world_mut().resource_mut::<Feel>().quick_save_dir =
                Err("Synthetic area attack test has no checkpoint directory".into());
            arm(&mut app);
            down(&mut app, window, START);
            record(&mut app);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(modal);
            up(&mut app, window, END);
            assert!(
                commands(&app).is_empty(),
                "{modal:?} submitted a hidden attack"
            );
            assert!(
                app.world()
                    .resource::<InputRes>()
                    .mapper
                    .attack_area_bounds()
                    .is_none()
            );
            let feel = app.world().resource::<Feel>();
            assert!(
                feel.feedback.is_empty(),
                "{modal:?} leaked gameplay feedback"
            );
            for frame in &feel.recorder.as_ref().unwrap().recording().frames {
                assert!(frame.submitted.is_empty());
                assert!(frame.selection.is_none());
                assert!(frame.feedback.is_empty());
                assert!(
                    frame
                        .outputs
                        .iter()
                        .all(|out| matches!(out, Output::ToggleOverlay))
                );
            }
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(modal);
            frame(&mut app);
            key(&mut app, KeyCode::Escape);
            frame(&mut app);
            assert!(commands(&app).is_empty(), "{modal:?} replayed after close");
            assert_eq!(selection(&app), selected);
            arm(&mut app);
            drag(&mut app, window);
            assert_eq!(commands(&app), attacks(&selected, &[target], false));
        }
    }
}

#[test]
fn controller_context_still_targets_one_unit_in_explicit_attack_mode() {
    for profile in profiles() {
        let (mut app, window) = fixture(profile);
        let selected = selection(&app);
        let target = enemy(&mut app, (320, 160));
        enemy(&mut app, (350, 180));
        world_pointer(&mut app, window, (320, 160));
        frame(&mut app);
        arm(&mut app);
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::West);
        let pad = app.world_mut().spawn(pad).id();
        frame(&mut app);
        assert!(commands(&app).is_empty());
        {
            let mut pad = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
            pad.digital_mut().clear();
            pad.digital_mut().release(GamepadButton::West);
        }
        frame(&mut app);
        assert_eq!(commands(&app), attacks(&selected, &[target], false));
        assert_eq!(selection(&app), selected);
    }
}

#[test]
fn explicit_attack_minimap_short_click_keeps_ground_target_queue_and_fog_semantics() {
    for profile in profiles() {
        for big in [false, true] {
            for shifted in [false, true] {
                let (mut app, window) = fixture(profile.clone());
                let selected = selection(&app);
                enemy(&mut app, (512, 256));
                app.world_mut().non_send_mut::<SimRes>().fog_on = true;
                let (pos, target) = {
                    let mut minimap = app.world_mut().resource_mut::<MinimapState>();
                    minimap.image = Some((200, 100));
                    minimap.map = [640.0, 320.0];
                    minimap.big = big;
                    let g = minimap.geom([800.0, 600.0]).unwrap();
                    let pos = Vec2::new(g.rect[0] + g.rect[2] * 0.8, g.rect[1] + g.rect[3] * 0.8);
                    let p = g.target_at(pos.x, pos.y);
                    let fx = |v: f32| tak_math::Fx::from_raw((v * 65536.0) as i32);
                    (pos, tak_math::Vec2::new(fx(p[0]), fx(p[1])))
                };
                arm(&mut app);
                if shifted {
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .press(KeyCode::ShiftLeft);
                }
                pointer(&mut app, window, pos);
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Left);
                frame(&mut app);
                assert!(commands(&app).is_empty());
                assert!(
                    app.world()
                        .resource::<InputRes>()
                        .mapper
                        .attack_area_bounds()
                        .is_none()
                );
                app.world_mut()
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .release(MouseButton::Left);
                if shifted {
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .release(KeyCode::ShiftLeft);
                }
                let camera = app.world().resource::<CamState>().geom.target;
                frame(&mut app);
                assert_eq!(
                    commands(&app),
                    vec![SimCommand::Attack {
                        units: selected.clone(),
                        target: Target::Ground(target),
                        queue: if shifted {
                            QueueMode::Append
                        } else {
                            QueueMode::Replace
                        },
                    }]
                );
                assert_eq!(selection(&app), selected);
                assert_eq!(app.world().resource::<CamState>().geom.target, camera);
                assert!(!app.world().resource::<MinimapState>().mouse_drag);
            }
        }
    }
}

#[test]
fn minimap_attack_click_is_cancelled_by_focus_cursor_surface_loss_or_large_drag() {
    for profile in profiles() {
        for interruption in ["focus", "cursor", "terrain", "large_drag"] {
            let (mut app, window) = fixture(profile.clone());
            let selected = selection(&app);
            let pos = {
                let mut minimap = app.world_mut().resource_mut::<MinimapState>();
                minimap.image = Some((200, 100));
                minimap.map = [640.0, 320.0];
                let g = minimap.geom([800.0, 600.0]).unwrap();
                Vec2::new(g.rect[0] + g.rect[2] / 2.0, g.rect[1] + g.rect[3] / 2.0)
            };
            arm(&mut app);
            pointer(&mut app, window, pos);
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            frame(&mut app);
            match interruption {
                "focus" => app.world_mut().get_mut::<Window>(window).unwrap().focused = false,
                "cursor" => app
                    .world_mut()
                    .get_mut::<Window>(window)
                    .unwrap()
                    .set_cursor_position(None),
                "terrain" => world_pointer(&mut app, window, END),
                "large_drag" => pointer(&mut app, window, pos + Vec2::new(20.0, 0.0)),
                _ => unreachable!(),
            }
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(MouseButton::Left);
            frame(&mut app);
            assert!(
                commands(&app).is_empty(),
                "{interruption} leaked minimap attack"
            );
            assert_eq!(selection(&app), selected);
            assert!(
                app.world()
                    .resource::<InputRes>()
                    .mapper
                    .attack_area_bounds()
                    .is_none()
            );
            app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
            pointer(&mut app, window, pos);
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            frame(&mut app);
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(MouseButton::Left);
            frame(&mut app);
            assert!(
                matches!(
                    commands(&app).as_slice(),
                    [SimCommand::Attack {
                        target: Target::Ground(_),
                        ..
                    }]
                ),
                "{interruption} poisoned fresh map click"
            );
            assert_eq!(selection(&app), selected);
        }
    }
}

#[test]
fn recorded_minimap_focus_or_cursor_loss_replays_cancellation_once_without_idle_noise() {
    for profile in profiles() {
        for loss in ["focus", "cursor"] {
            let (mut app, window) = fixture(profile.clone());
            let selected = selection(&app);
            let pos = {
                let mut minimap = app.world_mut().resource_mut::<MinimapState>();
                minimap.image = Some((200, 100));
                minimap.map = [640.0, 320.0];
                let g = minimap.geom([800.0, 600.0]).unwrap();
                Vec2::new(g.rect[0] + g.rect[2] / 2.0, g.rect[1] + g.rect[3] / 2.0)
            };
            arm(&mut app);
            pointer(&mut app, window, pos);
            frame(&mut app);
            record(&mut app);
            let initial = app.world().resource::<InputRes>().mapper.clone();
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            frame(&mut app);
            if loss == "focus" {
                app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
            } else {
                app.world_mut()
                    .get_mut::<Window>(window)
                    .unwrap()
                    .set_cursor_position(None);
            }
            frame(&mut app);
            let frames_at_loss = app
                .world()
                .resource::<Feel>()
                .recorder
                .as_ref()
                .unwrap()
                .recording()
                .frames
                .len();
            for _ in 0..4 {
                frame(&mut app);
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
                frames_at_loss,
                "{loss} recorded redundant idle leave events"
            );
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(MouseButton::Left);
            frame(&mut app);
            assert!(commands(&app).is_empty());
            let recording = app
                .world()
                .resource::<Feel>()
                .recorder
                .as_ref()
                .unwrap()
                .recording()
                .clone();
            assert_eq!(
                recording
                    .frames
                    .iter()
                    .flat_map(|f| &f.events)
                    .filter(|event| matches!(event, EventKind::PointerLeaveTerrain))
                    .count(),
                1,
                "{loss} must record the real map-surface loss exactly once"
            );
            let view = app.world().non_send::<SimRes>().view();
            for omit_loss in [false, true] {
                let mut replay = initial.clone();
                let mut replay_orders = Vec::new();
                for frame in &recording.frames {
                    replay.set_camera(frame.camera);
                    let mut outputs = Vec::new();
                    for event in frame
                        .events
                        .iter()
                        .chain(std::iter::once(&EventKind::Frame))
                    {
                        if omit_loss && matches!(event, EventKind::PointerLeaveTerrain) {
                            continue;
                        }
                        outputs.extend(replay.handle(
                            &InputEvent::new(frame.t, event.clone()),
                            &view,
                            tak_commands::Tick(frame.tick),
                        ));
                    }
                    if !omit_loss {
                        assert_eq!(outputs, frame.outputs, "{loss} replay diverged");
                    }
                    replay_orders.extend(outputs.into_iter().filter_map(|out| match out {
                        Output::Sim { command, .. } => Some(command),
                        _ => None,
                    }));
                }
                if omit_loss {
                    // Negative control: this is a real cancellation, not a
                    // vacuous replay of a gesture that could never fire.
                    assert!(matches!(
                        replay_orders.as_slice(),
                        [SimCommand::Attack { .. }]
                    ));
                } else {
                    assert!(replay_orders.is_empty());
                    assert_eq!(replay.selection(), &selected);
                }
            }
        }
    }
}
