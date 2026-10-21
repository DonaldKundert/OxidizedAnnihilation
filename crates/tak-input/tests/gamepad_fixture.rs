//! A recorded (synthetic) gamepad session converts to the expected command
//! stream: select, brush select, context move/attack (plus a chorded queued
//! move), control groups, both wheels, idle-builder cycling, precision mode,
//! magnetism and cancel.

mod common;

use common::{TestWorld, WorldSpec, pos, run, tick_at, uid};
use serde::Deserialize;
use tak_commands::{
    ClientCommand, ControlGroup, QueueMode, Scope, SimCommand, Target, UnitDefId, UnitSet,
};
use tak_input::{InputEvent, Mapper, Output, Profile};
use tak_math::{Fx, Vec2};

#[derive(Deserialize)]
struct Fixture {
    #[allow(dead_code)]
    description: String,
    world: WorldSpec,
    events: Vec<InputEvent>,
}

fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&i| uid(i)).collect()
}

fn client(command: ClientCommand) -> Output {
    Output::Client { command }
}

fn sim(t: u32, command: SimCommand) -> Output {
    Output::Sim {
        tick: tick_at(t),
        command,
    }
}

#[test]
fn gamepad_session_produces_expected_commands() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/gamepad_session.json")).expect("fixture");
    let world = TestWorld::new(fixture.world);
    let mut mapper = Mapper::new(Profile::default_controller());
    let out = run(&mut mapper, &world, &fixture.events);

    let g1 = ControlGroup::new(1).unwrap();
    let soldiers = units(&[2, 3]);
    let expected = vec![
        // Tap A on the builder.
        client(ClientCommand::SelectAt {
            pos: pos(640, 480),
            additive: false,
        }),
        // Hold A for 250 ms past the hold threshold: radius 16 + 160 * 0.25.
        client(ClientCommand::SelectBrush {
            center: pos(730, 480),
            radius: Fx::from_int(56),
            additive: false,
        }),
        // X on open ground; RB + X queues.
        sim(
            1250,
            SimCommand::Move {
                units: soldiers.clone(),
                to: pos(730, 300),
                queue: QueueMode::Replace,
            },
        ),
        sim(
            1320,
            SimCommand::Move {
                units: soldiers.clone(),
                to: pos(730, 300),
                queue: QueueMode::Append,
            },
        ),
        // X on the enemy.
        sim(
            1550,
            SimCommand::Attack {
                units: soldiers.clone(),
                target: Target::Unit(uid(9)),
                queue: QueueMode::Replace,
            },
        ),
        client(ClientCommand::AssignGroup { group: g1 }),
        Output::Deselect,
        client(ClientCommand::RecallGroup { group: g1 }),
        // Command wheel, up: Stop.
        sim(
            2100,
            SimCommand::Stop {
                units: soldiers.clone(),
            },
        ),
        // Command wheel, up-right: Patrol mode, completed by X.
        sim(
            2400,
            SimCommand::Patrol {
                units: soldiers.clone(),
                to: pos(640, 390),
                queue: QueueMode::Replace,
            },
        ),
        client(ClientCommand::SelectAt {
            pos: pos(640, 480),
            additive: false,
        }),
        // Build wheel slot 1 (def 21), placed by X.
        sim(
            3000,
            SimCommand::Build {
                units: units(&[1]),
                def: UnitDefId(21),
                at: Some(pos(640, 390)),
                queue: QueueMode::Replace,
            },
        ),
        client(ClientCommand::SelectSameType {
            scope: Scope::OnScreen,
        }),
        client(ClientCommand::SelectSameType { scope: Scope::Map }),
        client(ClientCommand::JumpToAlert),
        client(ClientCommand::CycleIdleBuilder),
        client(ClientCommand::CenterOnMonarch),
        // Precision: 300 ms at 225 px/s = 67.5 px, and no snap onto u1.
        sim(
            4850,
            SimCommand::Move {
                units: units(&[1]),
                to: Vec2::new(Fx::from_int(640), Fx::from_ratio(915, 2).unwrap()),
                queue: QueueMode::Replace,
            },
        ),
        // Precision released: the next frame snaps onto u1 (own, selected).
        sim(
            5000,
            SimCommand::Move {
                units: units(&[1]),
                to: pos(640, 480),
                queue: QueueMode::Replace,
            },
        ),
        // First B closed the build wheel; second B deselects.
        Output::Deselect,
    ];
    if out != expected {
        let show = |v: &[Output]| {
            v.iter()
                .map(|o| serde_json::to_string(o).unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        };
        panic!(
            "command stream mismatch\n--- got\n{}\n--- expected\n{}",
            show(&out),
            show(&expected)
        );
    }
    assert!(mapper.selection().is_empty());
    assert!(mapper.wheel().is_none());
}

#[test]
fn fixture_round_trips_through_json() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/gamepad_session.json")).expect("fixture");
    let text = serde_json::to_string(&fixture.events).unwrap();
    let back: Vec<InputEvent> = serde_json::from_str(&text).unwrap();
    assert_eq!(back, fixture.events);
}
