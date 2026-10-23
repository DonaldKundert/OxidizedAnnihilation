//! Explicit attack-move stays distinct from targeting a unit or force-firing.

mod common;

use common::{TestWorld, WorldSpec, pos, uid};
use tak_commands::{QueueMode, SimCommand, Target, Tick, UnitSet};
use tak_input::{
    Action, Button, CursorKind, EventKind, Feedback, InputEvent, Key, Mapper, Mode, Output,
    Profile, Refusal, WorldView, parse_keys_tdf, resolve_context,
};

fn world() -> TestWorld {
    TestWorld::new(
        serde_json::from_str::<WorldSpec>(
            r#"{
        "local_player": 0, "enemies": [1], "units": [
            {"index": 1, "owner": 0, "def": 0, "x": 640, "y": 480},
            {"index": 2, "owner": 1, "def": 0, "x": 800, "y": 480}
        ]
    }"#,
        )
        .unwrap(),
    )
}

fn commands(out: Vec<Output>) -> Vec<SimCommand> {
    out.into_iter()
        .filter_map(|o| match o {
            Output::Sim { command, .. } => Some(command),
            _ => None,
        })
        .collect()
}

fn key(t: u32, name: &str, down: bool) -> InputEvent {
    InputEvent::new(
        t,
        if down {
            EventKind::KeyDown {
                key: Key::new(name),
            }
        } else {
            EventKind::KeyUp {
                key: Key::new(name),
            }
        },
    )
}

fn selected(p: Profile, w: &TestWorld) -> Mapper {
    let mut m = Mapper::new(p);
    m.warp_cursor(pos(640, 480));
    m.perform(Action::Select, w, Tick(0));
    assert_eq!(m.selection(), &UnitSet::single(uid(1)));
    m.handle(
        &InputEvent::new(0, EventKind::PointerMove { x: 800, y: 480 }),
        w,
        Tick(0),
    );
    m
}

#[test]
fn explicit_mode_always_uses_the_ground_point_even_over_an_enemy() {
    let w = world();
    let units = UnitSet::single(uid(1));
    let hovered = w.unit(uid(2)).unwrap();
    for queue in [QueueMode::Replace, QueueMode::Append] {
        assert_eq!(
            resolve_context(
                &w,
                &units,
                pos(800, 480),
                Some(&hovered),
                Mode::AttackMove,
                queue
            ),
            Some(SimCommand::AttackMove {
                units: units.clone(),
                to: pos(800, 480),
                queue
            })
        );
        assert_eq!(
            resolve_context(
                &w,
                &units,
                pos(800, 480),
                Some(&hovered),
                Mode::Attack,
                queue
            ),
            Some(SimCommand::Attack {
                units: units.clone(),
                target: Target::Unit(uid(2)),
                queue
            })
        );
    }
    assert!(
        resolve_context(
            &w,
            &UnitSet::new(),
            pos(800, 480),
            None,
            Mode::AttackMove,
            QueueMode::Replace
        )
        .is_none()
    );
}

#[test]
fn classic_alt_a_is_attack_move_but_plain_a_keeps_force_fire() {
    let w = world();
    let mut m = selected(Profile::classic_tak(), &w);
    m.handle(&key(10, "alt", true), &w, Tick(1));
    m.handle(&key(20, "a", true), &w, Tick(1));
    assert_eq!(m.mode(), Mode::AttackMove);
    assert_eq!(m.cursor_kind(&w), CursorKind::Attack);
    let out = commands(m.perform(Action::Context { append: false }, &w, Tick(1)));
    assert_eq!(
        out,
        vec![SimCommand::AttackMove {
            units: UnitSet::single(uid(1)),
            to: pos(800, 480),
            queue: QueueMode::Replace
        }]
    );
    assert_eq!(m.mode(), Mode::None);
    m.handle(&key(30, "a", false), &w, Tick(1));
    m.handle(&key(40, "alt", false), &w, Tick(1));
    m.handle(&key(50, "a", true), &w, Tick(1));
    assert_eq!(m.mode(), Mode::Attack);
}

#[test]
fn append_keeps_attack_move_mode_and_cancel_leaves_selection_intact() {
    let w = world();
    let mut m = selected(Profile::classic_tak(), &w);
    m.perform(Action::AttackMoveMode, &w, Tick(1));
    let out = commands(m.perform(Action::Context { append: true }, &w, Tick(1)));
    assert!(matches!(
        &out[..],
        [SimCommand::AttackMove {
            queue: QueueMode::Append,
            ..
        }]
    ));
    assert_eq!(m.mode(), Mode::AttackMove);
    m.perform(Action::Cancel, &w, Tick(1));
    assert_eq!(m.mode(), Mode::None);
    assert_eq!(m.selection(), &UnitSet::single(uid(1)));
}

#[test]
fn both_attack_move_actions_refuse_an_empty_selection() {
    let w = world();
    let mut m = Mapper::new(Profile::classic_tak());
    for action in [Action::AttackMoveMode, Action::AttackMove] {
        assert!(commands(m.perform(action, &w, Tick(1))).is_empty());
        assert_eq!(m.mode(), Mode::None);
        assert!(m.take_feedback().contains(&Feedback::Refused {
            reason: Refusal::NothingSelected
        }));
    }
}

#[test]
fn controller_hold_sends_one_attack_move_and_rb_queues_it() {
    let w = world();
    let mut m = selected(Profile::kingdoms_console(), &w);
    m.warp_cursor(pos(800, 480));
    let mut out = Vec::new();
    for (t, kind) in [
        (10, EventKind::ButtonDown { button: Button::Rb }),
        (20, EventKind::ButtonDown { button: Button::X }),
        (400, EventKind::Frame),
        (420, EventKind::ButtonUp { button: Button::X }),
        (430, EventKind::ButtonUp { button: Button::Rb }),
    ] {
        out.extend(m.handle(&InputEvent::new(t, kind), &w, Tick(1)));
    }
    // Stick controls use the reticle at the screen centre, not the pointer.
    assert_eq!(
        commands(out),
        vec![SimCommand::AttackMove {
            units: UnitSet::single(uid(1)),
            to: pos(640, 480),
            queue: QueueMode::Append
        }]
    );
}

#[test]
fn custom_keys_can_bind_attack_move_without_reinterpreting_attack() {
    let imp = parse_keys_tdf(
        b"[CUSTOMKEYS]\n{\nLOWER_Q = UnitCommand AttackMove;\nLOWER_A = UnitCommand Attack;\n}\n",
    )
    .unwrap();
    let p = Profile::classic_tak().with_keys_tdf(&imp);
    let w = world();
    let mut m = selected(p, &w);
    m.handle(&key(10, "q", true), &w, Tick(1));
    assert_eq!(m.mode(), Mode::AttackMove);
    m.handle(&key(20, "a", true), &w, Tick(1));
    assert_eq!(m.mode(), Mode::Attack);
}
