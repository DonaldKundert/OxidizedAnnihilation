//! Adapters can sort same-poll Attack presses without changing input state.

mod common;

use common::TestWorld;
use tak_commands::{Scope, Tick};
use tak_input::{Action, Button, EventKind, Input, InputEvent, Key, Mapper, Mode, Profile};

fn key(name: &str) -> Input {
    Input::Key(Key::new(name))
}
fn world() -> TestWorld {
    TestWorld::new(serde_json::from_str(r#"{"local_player":0,"enemies":[],"units":[]}"#).unwrap())
}
fn edge(m: &mut Mapper, name: &str, down: bool) {
    let kind = if down {
        EventKind::KeyDown {
            key: Key::new(name),
        }
    } else {
        EventKind::KeyUp {
            key: Key::new(name),
        }
    };
    m.handle(&InputEvent::new(m.now() + 1, kind), &world(), Tick(0));
}

#[test]
fn pending_downs_distinguish_classic_attack_from_ctrl_and_alt_chords_without_mutation() {
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let mut m = Mapper::new(profile);
        let now = m.now();
        assert_eq!(
            m.press_action_with_pending_downs(&key("a"), &[]),
            Some(Action::AttackMode)
        );
        assert_eq!(
            m.press_action_with_pending_downs(&key("a"), &[key("a"), key("ctrl")]),
            Some(Action::SelectAll { scope: Scope::Map })
        );
        assert_eq!(
            m.press_action_with_pending_downs(&key("a"), &[key("alt"), key("a")]),
            Some(Action::AttackMoveMode)
        );
        assert_eq!(m.press_action(&key("a")), Some(Action::AttackMode));
        assert_eq!(m.mode(), Mode::None);
        assert_eq!(m.now(), now);
        assert!(m.selection().is_empty());
        assert!(m.take_feedback().is_empty());
    }
}

#[test]
fn remapped_and_multi_modifier_preview_matches_real_press_chord_resolution() {
    let profile = Profile::from_json(r#"{"name":"synthetic chords","bindings":[
        {"input":{"key":"q"},"gesture":"press","action":{"type":"attack_mode"}},
        {"input":{"key":"q"},"with":{"key":"ctrl"},"gesture":"press","action":{"type":"move_mode"}},
        {"input":{"key":"q"},"with_all":[{"key":"shift"},{"key":"ctrl"}],"gesture":"press","action":{"type":"attack_mode"}}
    ]}"#).unwrap();
    let mut m = Mapper::new(profile);
    assert_eq!(
        m.press_action_with_pending_downs(&key("q"), &[key("q")]),
        Some(Action::AttackMode)
    );
    assert_eq!(
        m.press_action_with_pending_downs(&key("q"), &[key("ctrl"), key("q")]),
        Some(Action::MoveMode)
    );
    let downs = [
        key("shift"),
        key("q"),
        key("ctrl"),
        key("shift"),
        key("unbound"),
    ];
    assert_eq!(
        m.press_action_with_pending_downs(&key("q"), &downs),
        Some(Action::AttackMode)
    );
    edge(&mut m, "ctrl", true);
    edge(&mut m, "shift", true);
    assert_eq!(m.press_action(&key("q")), Some(Action::AttackMode));
    assert_eq!(
        m.press_action_with_pending_downs(&key("q"), &[]),
        m.press_action(&key("q"))
    );
}

#[test]
fn held_modifiers_remain_until_their_release_edge_is_delivered() {
    let mut m = Mapper::new(Profile::classic_tak());
    edge(&mut m, "ctrl", true);
    // A modifier release later in this poll does not affect action ordering.
    assert_eq!(
        m.press_action_with_pending_downs(&key("a"), &[key("a")]),
        Some(Action::SelectAll { scope: Scope::Map })
    );
    edge(&mut m, "ctrl", false);
    assert_eq!(
        m.press_action_with_pending_downs(&key("a"), &[key("a")]),
        Some(Action::AttackMode)
    );
    // Current held state and pending downs participate together.
    edge(&mut m, "ctrl", true);
    let downs = [key("shift"), key("m")];
    assert_eq!(
        m.press_action_with_pending_downs(&key("m"), &downs),
        Some(Action::SelectMonarch { track: false })
    );
}

#[test]
fn release_or_tap_attack_binding_is_not_previewed_as_an_attack_press() {
    let profile = Profile::from_json(
        r#"{"name":"non-press bindings","bindings":[
        {"input":{"key":"q"},"gesture":"release","action":{"type":"attack_mode"}},
        {"input":{"key":"a"},"gesture":"tap","action":{"type":"attack_mode"}}
    ]}"#,
    )
    .unwrap();
    let m = Mapper::new(profile);
    assert_eq!(
        m.press_action_with_pending_downs(&key("q"), &[key("q")]),
        None
    );
    assert_eq!(
        m.press_action_with_pending_downs(&key("a"), &[key("a")]),
        None
    );
    assert_eq!(
        m.press_action_with_pending_downs(&Input::Button(Button::MouseLeft), &[]),
        None
    );
}
