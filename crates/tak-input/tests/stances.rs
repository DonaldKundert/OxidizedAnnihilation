//! Kingdoms stance vocabulary does not silently redefine saved legacy profiles.
mod common;

use common::{TestWorld, WorldSpec, uid};
use tak_commands::{FireState, Scope, SimCommand, Tick, UnitSet};
use tak_input::{
    Action, Axis, Button, EventKind, InputEvent, Key, Mapper, Output, Profile, WheelKind,
    parse_keys_tdf,
};

fn world() -> TestWorld {
    TestWorld::new(
        serde_json::from_str::<WorldSpec>(
            r#"{
        "local_player": 0, "enemies": [1], "units": [
        {"index": 1, "owner": 0, "def": 0, "x": 100, "y": 100}] }"#,
        )
        .unwrap(),
    )
}

#[test]
fn kingdoms_stance_ring_uses_true_defensive_and_mapper_submits_it() {
    let w = world();
    let mut m = Mapper::new(Profile::default_combined());
    m.perform(Action::SelectAll { scope: Scope::Map }, &w, Tick(1));
    for (time, kind) in [
        (10, EventKind::ButtonDown { button: Button::Lt }),
        (
            20,
            EventKind::Axis {
                axis: Axis::LeftX,
                value: 30000,
            },
        ),
        (
            30,
            EventKind::Axis {
                axis: Axis::LeftY,
                value: -30000,
            },
        ),
        (40, EventKind::ButtonUp { button: Button::Lt }),
    ] {
        m.handle(&InputEvent::new(time, kind), &w, Tick(1));
    }
    assert_eq!(m.wheel().unwrap().kind, WheelKind::Stance);
    let slots = m.wheel().unwrap().slots.clone();
    assert_eq!(
        slots,
        vec![
            Action::SetFireState {
                state: FireState::FireAtWill
            },
            Action::SetFireState {
                state: FireState::Defensive
            },
            Action::SetFireState {
                state: FireState::HoldFire
            },
        ]
    );
    m.handle(
        &InputEvent::new(
            50,
            EventKind::Axis {
                axis: Axis::LeftY,
                value: -15000,
            },
        ),
        &w,
        Tick(1),
    );
    assert_eq!(m.wheel().unwrap().highlighted, Some(1));
    let before_release = m.handle(
        &InputEvent::new(60, EventKind::ButtonDown { button: Button::A }),
        &w,
        Tick(1),
    );
    assert!(before_release.is_empty());
    let out = m.handle(
        &InputEvent::new(70, EventKind::ButtonUp { button: Button::A }),
        &w,
        Tick(1),
    );
    assert_eq!(
        out,
        vec![Output::Sim {
            tick: Tick(1),
            command: SimCommand::SetFireState {
                units: UnitSet::single(uid(1)),
                state: FireState::Defensive
            }
        }]
    );
}

#[test]
fn imported_defensive_action_submits_true_defensive() {
    let imp = parse_keys_tdf(b"[CUSTOMKEYS] {\n LOWER_D = UnitCommand Defensive;\n LOWER_O = UnitCommand Offensive;\n LOWER_P = UnitCommand Passive;\n }").unwrap();
    let p = Profile::classic_tak().with_keys_tdf(&imp);
    for (key, state) in [
        ("d", FireState::Defensive),
        ("o", FireState::FireAtWill),
        ("p", FireState::HoldFire),
    ] {
        let w = world();
        let mut m = Mapper::new(p.clone());
        m.perform(Action::SelectAll { scope: Scope::Map }, &w, Tick(1));
        let out = m.handle(
            &InputEvent::new(1, EventKind::KeyDown { key: Key::new(key) }),
            &w,
            Tick(1),
        );
        assert!(out.iter().any(|o| matches!(o, Output::Sim { command: SimCommand::SetFireState { state: actual, .. }, .. } if *actual == state)), "{key}: {out:?}");
    }
}

#[test]
fn legacy_profile_actions_and_saved_return_fire_token_remain_unchanged() {
    for p in [Profile::wp_j_controller(), Profile::wp_s_controller()] {
        let states: Vec<_> = p
            .command_wheel
            .iter()
            .filter_map(|a| match a {
                Action::SetFireState { state } => Some(*state),
                _ => None,
            })
            .collect();
        assert_eq!(
            states,
            vec![
                FireState::HoldFire,
                FireState::ReturnFire,
                FireState::FireAtWill
            ]
        );
    }
    let p = Profile::wp_s_mouse_keyboard();
    assert!(p.bindings.iter().any(|b| b.action
        == Action::SetFireState {
            state: FireState::ReturnFire
        }));
    let action: Action =
        serde_json::from_str(r#"{"type":"set_fire_state","state":"return_fire"}"#).unwrap();
    assert_eq!(
        action,
        Action::SetFireState {
            state: FireState::ReturnFire
        }
    );
    assert_eq!(
        serde_json::to_string(&action).unwrap(),
        r#"{"type":"set_fire_state","state":"return_fire"}"#
    );
}
