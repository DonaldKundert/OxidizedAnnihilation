//! Opt-in original-style mouse commands without changing the existing
//! right-context scheme, controller, drag selection, or explicit orders.

mod common;

use common::{TestWorld, pos, run, tick_at, uid};
use tak_commands::{ClientCommand, QueueMode, Scope, SimCommand, Target, Tick, UnitSet};
use tak_input::{
    Action, Axis, Button, CursorKind, EventKind, InputEvent, Key, Mapper, Mode, Output, Profile,
};

fn world() -> TestWorld {
    TestWorld::new(
        serde_json::from_str(
            r#"{"local_player":0,"enemies":[1],"units":[
                {"index":1,"owner":0,"def":10,"x":200,"y":200,
                 "builder":"mobile","build_options":[20],"spells":[30]},
                {"index":2,"owner":0,"def":10,"x":300,"y":200},
                {"index":3,"owner":0,"def":11,"x":400,"y":200},
                {"index":4,"owner":1,"def":10,"x":600,"y":200},
                {"index":5,"owner":2,"def":10,"x":700,"y":200},
                {"index":6,"owner":0,"def":10,"x":3000,"y":200},
                {"index":7,"owner":0,"def":10,"x":500,"y":200,"selectable":false}
            ]}"#,
        )
        .unwrap(),
    )
}

fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&id| uid(id)).collect()
}

fn event(t: u32, kind: EventKind) -> InputEvent {
    InputEvent::new(t, kind)
}

fn pointer(t: u32, x: i32, y: i32) -> InputEvent {
    event(t, EventKind::PointerMove { x, y })
}

fn down(t: u32) -> InputEvent {
    event(
        t,
        EventKind::ButtonDown {
            button: Button::MouseLeft,
        },
    )
}

fn up(t: u32) -> InputEvent {
    event(
        t,
        EventKind::ButtonUp {
            button: Button::MouseLeft,
        },
    )
}

fn click(m: &mut Mapper, w: &TestWorld, t: u32, x: i32, y: i32) -> Vec<Output> {
    run(m, w, &[pointer(t, x, y), down(t + 1), up(t + 2)])
}

fn key(m: &mut Mapper, w: &TestWorld, t: u32, name: &str, pressed: bool) -> Vec<Output> {
    let kind = if pressed {
        EventKind::KeyDown {
            key: Key::new(name),
        }
    } else {
        EventKind::KeyUp {
            key: Key::new(name),
        }
    };
    run(m, w, &[event(t, kind)])
}

fn client(command: ClientCommand) -> Output {
    Output::Client { command }
}

fn order(t: u32, command: SimCommand) -> Output {
    Output::Sim {
        tick: tick_at(t),
        command,
    }
}

#[test]
fn profile_is_explicit_backward_compatible_and_preserved_by_combining() {
    let legacy = Profile::from_json(r#"{"name":"legacy","bindings":[]}"#).unwrap();
    assert!(!legacy.left_click_context);
    assert!(!legacy.to_json_pretty().contains("left_click_context"));
    assert!(!Profile::classic_tak().left_click_context);
    assert!(!Profile::default_mouse_keyboard().left_click_context);
    assert!(!Profile::default_combined().left_click_context);

    let original = Profile::original_tak();
    original.validate().unwrap();
    assert!(original.left_click_context);
    assert_eq!(original.bindings, Profile::classic_tak().bindings);
    assert_eq!(
        Profile::from_json(&original.to_json_pretty()).unwrap(),
        original
    );
    assert!(tak_input::CONTROL_SCHEMES.contains(&"original"));
    for name in ["original", "ORIGINAL", "original-tak", "original_tak"] {
        let combined = Profile::named(name).unwrap();
        combined.validate().unwrap();
        assert!(combined.left_click_context, "{name}");
        assert_eq!(combined.tuning, Profile::kingdoms_console().tuning);
        assert_eq!(
            combined.command_wheel,
            Profile::kingdoms_console().command_wheel
        );
    }
}

#[test]
fn rapid_selection_move_and_attack_clicks_are_never_swallowed_as_double_clicks() {
    let w = world();
    let mut m = Mapper::new(Profile::original_tak());
    assert_eq!(
        click(&mut m, &w, 100, 200, 200),
        vec![client(ClientCommand::SelectAt {
            pos: pos(200, 200),
            additive: false,
        })]
    );
    for t in [120, 140] {
        assert_eq!(
            click(&mut m, &w, t, 800, 300),
            vec![order(
                t + 2,
                SimCommand::Move {
                    units: units(&[1]),
                    to: pos(800, 300),
                    queue: QueueMode::Replace,
                },
            )]
        );
    }
    for t in [160, 180] {
        assert_eq!(
            click(&mut m, &w, t, 600, 200),
            vec![order(
                t + 2,
                SimCommand::Attack {
                    units: units(&[1]),
                    target: Target::Unit(uid(4)),
                    queue: QueueMode::Replace,
                },
            )]
        );
        assert_eq!(m.selection(), &units(&[1]));
    }
}

#[test]
fn own_click_selects_not_guards_and_different_units_do_not_double_select() {
    let w = world();
    let mut m = Mapper::new(Profile::original_tak());
    click(&mut m, &w, 100, 200, 200);
    assert_eq!(
        click(&mut m, &w, 120, 400, 200),
        vec![client(ClientCommand::SelectAt {
            pos: pos(400, 200),
            additive: false,
        })]
    );
    assert_eq!(m.selection(), &units(&[3]));
    assert_eq!(
        click(&mut m, &w, 140, 200, 200),
        vec![client(ClientCommand::SelectAt {
            pos: pos(200, 200),
            additive: false,
        })]
    );
    assert_eq!(m.selection(), &units(&[1]));
}

#[test]
fn actual_double_click_still_selects_same_owned_type_on_screen() {
    let w = world();
    let mut m = Mapper::new(Profile::original_tak());
    click(&mut m, &w, 100, 200, 200);
    assert_eq!(
        click(&mut m, &w, 140, 201, 200),
        vec![client(ClientCommand::SelectSameType {
            scope: Scope::OnScreen,
        })]
    );
    assert_eq!(m.selection(), &units(&[1, 2]));
}

#[test]
fn double_click_rechecks_target_ownership_and_generation() {
    for replacement in [false, true] {
        let mut w = world();
        let mut m = Mapper::new(Profile::original_tak());
        click(&mut m, &w, 100, 200, 200);
        if replacement {
            w.spec.units[0].generation += 1;
        } else {
            w.spec.units[0].owner = 1;
        }
        let out = click(&mut m, &w, 120, 200, 200);
        assert_eq!(
            out,
            vec![client(ClientCommand::SelectAt {
                pos: pos(200, 200),
                additive: false,
            })]
        );
        if replacement {
            assert_eq!(m.selection(), &UnitSet::from(vec![w.spec.units[0].id()]));
        } else {
            assert!(m.selection().is_empty());
        }
    }
}

#[test]
fn empty_or_stale_selection_cannot_issue_orders() {
    let mut w = world();
    let mut m = Mapper::new(Profile::original_tak());
    for (t, x) in [(100, 800), (120, 600), (140, 500), (160, 700)] {
        let out = click(&mut m, &w, t, x, 200);
        assert!(out.iter().all(|o| !matches!(o, Output::Sim { .. })));
        assert!(m.selection().is_empty());
    }
    click(&mut m, &w, 200, 200, 200);
    w.spec.units.retain(|u| u.index != 1);
    let out = click(&mut m, &w, 220, 600, 200);
    assert!(out.iter().all(|o| !matches!(o, Output::Sim { .. })));
    assert!(m.selection().is_empty());
}

#[test]
fn shift_adds_friendly_selection_and_queues_repeated_orders() {
    let w = world();
    let mut m = Mapper::new(Profile::original_tak());
    click(&mut m, &w, 100, 200, 200);
    key(&mut m, &w, 110, "shift", true);
    assert_eq!(
        click(&mut m, &w, 120, 400, 200),
        vec![client(ClientCommand::SelectAt {
            pos: pos(400, 200),
            additive: true,
        })]
    );
    assert_eq!(m.selection(), &units(&[1, 3]));
    for t in [140, 160] {
        assert_eq!(
            click(&mut m, &w, t, 800, 300),
            vec![order(
                t + 2,
                SimCommand::Move {
                    units: units(&[1, 3]),
                    to: pos(800, 300),
                    queue: QueueMode::Append,
                },
            )]
        );
    }
    key(&mut m, &w, 170, "shift", false);
    assert_eq!(m.selection(), &units(&[1, 3]));
}

#[test]
fn drag_selects_without_issuing_orders_and_does_not_arm_a_double_click() {
    let w = world();
    let mut m = Mapper::new(Profile::original_tak());
    click(&mut m, &w, 100, 400, 200);
    let out = run(
        &mut m,
        &w,
        &[
            pointer(120, 180, 180),
            down(121),
            pointer(130, 320, 220),
            up(140),
        ],
    );
    assert!(matches!(
        out.as_slice(),
        [Output::Client {
            command: ClientCommand::SelectBrush {
                additive: false,
                ..
            }
        }]
    ));
    assert_eq!(m.selection(), &units(&[1, 2]));
    assert_eq!(
        click(&mut m, &w, 160, 800, 300),
        vec![order(
            162,
            SimCommand::Move {
                units: units(&[1, 2]),
                to: pos(800, 300),
                queue: QueueMode::Replace,
            },
        )]
    );
}

#[test]
fn drag_threshold_is_unchanged_and_orders_wait_for_release() {
    let w = world();
    let mut m = Mapper::new(Profile::original_tak());
    click(&mut m, &w, 100, 200, 200);
    let threshold = m.tuning().drag_min_px;
    assert!(run(&mut m, &w, &[pointer(120, 800, 300), down(121), down(122)]).is_empty());
    assert!(run(&mut m, &w, &[pointer(130, 800 + threshold - 1, 300)]).is_empty());
    assert_eq!(
        run(&mut m, &w, &[up(140)]),
        vec![order(
            140,
            SimCommand::Move {
                units: units(&[1]),
                to: pos(800 + threshold - 1, 300),
                queue: QueueMode::Replace,
            },
        )]
    );
    let out = run(
        &mut m,
        &w,
        &[
            pointer(160, 800, 300),
            down(161),
            pointer(170, 800 + threshold, 300),
            up(180),
        ],
    );
    assert!(matches!(
        out.as_slice(),
        [Output::Client {
            command: ClientCommand::SelectBrush { .. }
        }]
    ));
}

#[test]
fn explicit_modes_take_precedence_even_over_owned_units() {
    let w = world();
    let mut m = Mapper::new(Profile::original_tak());
    click(&mut m, &w, 100, 200, 200);
    key(&mut m, &w, 110, "m", true);
    key(&mut m, &w, 111, "m", false);
    assert_eq!(
        click(&mut m, &w, 120, 400, 200),
        vec![order(
            122,
            SimCommand::Move {
                units: units(&[1]),
                to: pos(400, 200),
                queue: QueueMode::Replace,
            },
        )]
    );
    assert_eq!(m.mode(), Mode::None);
    assert_eq!(m.selection(), &units(&[1]));
    key(&mut m, &w, 130, "a", true);
    key(&mut m, &w, 131, "a", false);
    key(&mut m, &w, 132, "shift", true);
    for t in [140, 160] {
        assert_eq!(
            click(&mut m, &w, t, 400, 200),
            vec![order(
                t + 2,
                SimCommand::Attack {
                    units: units(&[1]),
                    target: Target::Unit(uid(3)),
                    queue: QueueMode::Append,
                },
            )]
        );
        assert_eq!(m.mode(), Mode::Attack);
    }
}

#[test]
fn cancelling_or_reloading_mid_gesture_never_issues_a_late_order() {
    let w = world();
    for reload in [false, true] {
        let mut m = Mapper::new(Profile::original_tak());
        click(&mut m, &w, 100, 200, 200);
        run(&mut m, &w, &[pointer(120, 800, 300), down(121)]);
        if reload {
            m.set_profile(Profile::original_tak());
        } else {
            key(&mut m, &w, 130, "escape", true);
            key(&mut m, &w, 131, "escape", false);
        }
        assert!(run(&mut m, &w, &[up(140)]).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
        assert!(matches!(
            click(&mut m, &w, 160, 600, 200).as_slice(),
            [Output::Sim {
                command: SimCommand::Attack { .. },
                ..
            }]
        ));
    }
}

#[test]
fn right_context_and_allied_guards_stay_available() {
    let w = world();
    let mut m = Mapper::new(Profile::original_tak());
    click(&mut m, &w, 100, 200, 200);
    assert_eq!(
        run(
            &mut m,
            &w,
            &[
                pointer(120, 400, 200),
                event(
                    121,
                    EventKind::ButtonDown {
                        button: Button::MouseRight
                    }
                ),
                event(
                    122,
                    EventKind::ButtonUp {
                        button: Button::MouseRight
                    }
                ),
            ],
        ),
        vec![order(
            121,
            SimCommand::Guard {
                units: units(&[1]),
                target: uid(3),
                queue: QueueMode::Replace,
            },
        )]
    );
    assert_eq!(
        click(&mut m, &w, 140, 700, 200),
        vec![order(
            142,
            SimCommand::Guard {
                units: units(&[1]),
                target: uid(5),
                queue: QueueMode::Replace,
            },
        )]
    );
}

#[test]
fn classic_ground_click_still_clears_selection_instead_of_moving() {
    let w = world();
    let mut m = Mapper::new(Profile::classic_tak());
    click(&mut m, &w, 100, 200, 200);
    let out = click(&mut m, &w, 1000, 800, 300);
    assert_eq!(
        out,
        vec![client(ClientCommand::SelectAt {
            pos: pos(800, 300),
            additive: false,
        })]
    );
    assert!(m.selection().is_empty());
}

#[test]
fn cursor_matches_primary_mouse_selection_without_changing_pad_or_explicit_modes() {
    let w = world();
    let mut profile = Profile::original_tak();
    // Keep the cursor in place when switching from mouse to stick, so the
    // same owned unit stays hovered in both device modes.
    profile.tuning.reticle = false;
    let mut m = Mapper::new(profile);
    click(&mut m, &w, 100, 200, 200);
    for (t, x, expected) in [
        (120, 400, CursorKind::Select),
        (140, 600, CursorKind::Attack),
        (160, 700, CursorKind::Guard),
        (180, 800, CursorKind::Move),
    ] {
        run(&mut m, &w, &[pointer(t, x, 200)]);
        assert_eq!(m.cursor_kind(&w), expected);
    }
    run(&mut m, &w, &[pointer(200, 400, 200)]);
    run(
        &mut m,
        &w,
        &[event(
            210,
            EventKind::Axis {
                axis: Axis::LeftX,
                value: 32767,
            },
        )],
    );
    assert_eq!(m.cursor_kind(&w), CursorKind::Guard);
    run(&mut m, &w, &[pointer(220, 400, 200)]);
    m.perform(Action::AttackMode, &w, Tick(7));
    assert_eq!(m.cursor_kind(&w), CursorKind::Attack);
    m.perform(Action::MoveMode, &w, Tick(7));
    assert_eq!(m.cursor_kind(&w), CursorKind::Move);
}

#[test]
fn controller_selection_action_is_not_reinterpreted_as_a_mouse_order() {
    let w = world();
    let mut m = Mapper::new(Profile::named("original").unwrap());
    click(&mut m, &w, 100, 200, 200);
    run(&mut m, &w, &[pointer(120, 800, 300)]);
    assert_eq!(
        m.perform(Action::Select, &w, Tick(4)),
        vec![client(ClientCommand::SelectAt {
            pos: pos(800, 300),
            additive: false,
        })]
    );
    assert!(m.selection().is_empty());
}
