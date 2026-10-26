//! Synthetic squad fixtures verify additive recall cannot overwrite a group.

mod common;
use common::{TestWorld, WorldSpec, uid};
use tak_commands::{ClientCommand, ControlGroup, Tick, UnitSet};
use tak_input::{
    Action, Binding, Button, EventKind, Gesture, Input, InputEvent, Key, LineStatus, Mapper,
    Output, Profile, ProfileError, parse_keys_tdf,
};

fn world() -> TestWorld {
    TestWorld::new(
        serde_json::from_str::<WorldSpec>(
            r#"{
      "local_player":0,"enemies":[1],"units":[
      {"index":1,"owner":0,"def":1,"x":100,"y":100},
      {"index":2,"owner":0,"def":1,"x":200,"y":100},
      {"index":3,"owner":0,"def":1,"x":300,"y":100},
      {"index":4,"owner":0,"def":1,"x":400,"y":100}] }"#,
        )
        .unwrap(),
    )
}
fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&id| uid(id)).collect()
}
fn group(n: u8) -> ControlGroup {
    ControlGroup::new(n).unwrap()
}
fn k(name: &str) -> Input {
    Input::Key(Key::new(name))
}
fn event(m: &mut Mapper, w: &TestWorld, kind: EventKind) -> Vec<Output> {
    m.handle(&InputEvent::new(m.now() + 20, kind), w, Tick(1))
}
fn key(m: &mut Mapper, w: &TestWorld, name: &str, down: bool) -> Vec<Output> {
    event(
        m,
        w,
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
fn select(m: &mut Mapper, w: &TestWorld, ids: &[u32]) {
    m.perform(Action::Cancel, w, Tick(1));
    key(m, w, "shift", true);
    for &id in ids {
        event(
            m,
            w,
            EventKind::PointerMove {
                x: id as i32 * 100,
                y: 100,
            },
        );
        m.perform(Action::Select, w, Tick(1));
    }
    key(m, w, "shift", false);
}
fn assign(m: &mut Mapper, w: &TestWorld, g: u8, ids: &[u32]) {
    select(m, w, ids);
    m.perform(Action::AssignGroup { group: group(g) }, w, Tick(1));
}
fn chord(m: &mut Mapper, w: &TestWorld, modifiers: &[&str], input: &str) -> Vec<Output> {
    for name in modifiers {
        key(m, w, name, true);
    }
    let out = key(m, w, input, true);
    key(m, w, input, false);
    for name in modifiers.iter().rev() {
        key(m, w, name, false);
    }
    out
}
fn recall_output(g: u8) -> Vec<Output> {
    vec![Output::Client {
        command: ClientCommand::RecallGroupAdd { group: group(g) },
    }]
}

#[test]
fn ctrl_shift_digits_union_without_overwriting_in_either_modifier_order() {
    let w = world();
    for modifiers in [
        ["ctrl", "shift"],
        ["shift", "ctrl"],
        ["alt", "shift"],
        ["shift", "alt"],
    ] {
        let mut m = Mapper::new(Profile::classic_tak());
        assign(&mut m, &w, 1, &[2, 3]);
        select(&mut m, &w, &[1, 2]);
        assert_eq!(chord(&mut m, &w, &modifiers, "1"), recall_output(1));
        assert_eq!(m.selection(), &units(&[1, 2, 3]));
        // A plain recall proves the saved group itself did not acquire A.
        m.perform(Action::RecallGroup { group: group(1) }, &w, Tick(1));
        assert_eq!(m.selection(), &units(&[2, 3]));
    }
}

#[test]
fn ctrl_only_assignment_remains_replacement_and_plain_recall_remains_replacement() {
    let w = world();
    let mut m = Mapper::new(Profile::classic_tak());
    assign(&mut m, &w, 1, &[2]);
    select(&mut m, &w, &[3]);
    let out = chord(&mut m, &w, &["ctrl"], "1");
    assert_eq!(
        out,
        vec![Output::Client {
            command: ClientCommand::AssignGroup { group: group(1) }
        }]
    );
    select(&mut m, &w, &[1]);
    chord(&mut m, &w, &[], "1");
    assert_eq!(m.selection(), &units(&[3]));
}

#[test]
fn imported_multimodifier_action_can_recall_a_different_group_than_assignment() {
    let imp = parse_keys_tdf(
        b"[CUSTOMKEYS]{\n CTRL_7 = CreateSquad 3;\n CTRLSHIFT_7 = RetrieveSquadAdd 9;\n }",
    )
    .unwrap();
    assert_eq!(imp.count(LineStatus::Mapped), 2);
    assert_eq!(imp.count(LineStatus::Skipped), 0);
    let p = Profile::classic_tak().with_keys_tdf(&imp);
    p.validate().unwrap();
    assert_eq!(
        p.find_chord(&k("7"), &[k("ctrl"), k("shift")], Gesture::Press),
        Some(Action::RecallGroupAdd { group: group(9) })
    );
    let w = world();
    let mut m = Mapper::new(p);
    assign(&mut m, &w, 3, &[3]);
    assign(&mut m, &w, 9, &[2]);
    select(&mut m, &w, &[1]);
    assert_eq!(chord(&mut m, &w, &["ctrl", "shift"], "7"), recall_output(9));
    assert_eq!(m.selection(), &units(&[1, 2]));
    m.perform(Action::RecallGroup { group: group(3) }, &w, Tick(1));
    assert_eq!(m.selection(), &units(&[3]));
    m.perform(Action::RecallGroup { group: group(9) }, &w, Tick(1));
    assert_eq!(m.selection(), &units(&[2]));
}

#[test]
fn additive_recall_prunes_dead_foreign_and_unselectable_from_both_sides() {
    let mut w = world();
    let mut m = Mapper::new(Profile::classic_tak());
    assign(&mut m, &w, 1, &[2, 3, 4]);
    select(&mut m, &w, &[1, 2, 4]);
    w.spec.units.retain(|u| u.index != 2);
    w.spec
        .units
        .iter_mut()
        .find(|u| u.index == 3)
        .unwrap()
        .owner = 1;
    w.spec
        .units
        .iter_mut()
        .find(|u| u.index == 4)
        .unwrap()
        .selectable = false;
    m.perform(Action::RecallGroupAdd { group: group(1) }, &w, Tick(1));
    assert_eq!(m.selection(), &units(&[1]));
    // Ownership/selectability changes do not rewrite the stored group.
    w.spec
        .units
        .iter_mut()
        .find(|u| u.index == 3)
        .unwrap()
        .owner = 0;
    w.spec
        .units
        .iter_mut()
        .find(|u| u.index == 4)
        .unwrap()
        .selectable = true;
    m.perform(Action::RecallGroup { group: group(1) }, &w, Tick(1));
    assert_eq!(m.selection(), &units(&[3, 4]));
}

#[test]
fn queue_plus_keyboard_or_default_gamepad_recall_is_additive() {
    let w = world();
    for gamepad in [false, true] {
        let mut m = Mapper::new(Profile::default_combined());
        assign(&mut m, &w, 1, &[2]);
        select(&mut m, &w, &[1]);
        let out = if gamepad {
            event(&mut m, &w, EventKind::ButtonDown { button: Button::Rb });
            event(
                &mut m,
                &w,
                EventKind::ButtonDown {
                    button: Button::DpadUp,
                },
            );
            let out = event(
                &mut m,
                &w,
                EventKind::ButtonUp {
                    button: Button::DpadUp,
                },
            );
            assert!(event(&mut m, &w, EventKind::ButtonUp { button: Button::Rb }).is_empty());
            out
        } else {
            chord(&mut m, &w, &["shift"], "1")
        };
        assert_eq!(out, recall_output(1));
        assert_eq!(m.selection(), &units(&[1, 2]));
        m.perform(Action::RecallGroup { group: group(1) }, &w, Tick(1));
        assert_eq!(m.selection(), &units(&[2]));
    }
}

#[test]
fn multi_modifier_profile_round_trips_and_validates_canonical_duplicates() {
    let p = Profile::classic_tak();
    let decoded = Profile::from_json(&p.to_json_pretty()).unwrap();
    assert_eq!(p, decoded);
    let mut p = p.clone();
    p.bindings.push(Binding {
        input: k("1"),
        with: Some(k("shift")),
        with_all: vec![k("ctrl")],
        gesture: Gesture::Press,
        action: Action::RecallGroupAdd { group: group(1) },
    });
    assert!(matches!(p.validate(), Err(ProfileError::Duplicate { .. })));
    let mut p = Profile::classic_tak();
    p.bindings.push(Binding {
        input: k("q"),
        with: Some(k("ctrl")),
        with_all: vec![k("ctrl")],
        gesture: Gesture::Press,
        action: Action::Stop,
    });
    assert!(matches!(
        p.validate(),
        Err(ProfileError::DuplicateModifier(_))
    ));
    p.bindings.last_mut().unwrap().with_all = vec![k("q")];
    assert!(matches!(p.validate(), Err(ProfileError::SelfChord(_))));
}

#[test]
fn imported_plain_additive_recall_works_without_queue_modifier() {
    let imp = parse_keys_tdf(b"[CUSTOMKEYS]{ LOWER_Q = RetrieveSquadAdd 10; }").unwrap();
    let w = world();
    let mut m = Mapper::new(Profile::classic_tak().with_keys_tdf(&imp));
    assign(&mut m, &w, 0, &[2]);
    select(&mut m, &w, &[1]);
    assert_eq!(chord(&mut m, &w, &[], "q"), recall_output(0));
    assert_eq!(m.selection(), &units(&[1, 2]));
}

#[test]
fn imported_additive_monarch_keeps_selection_without_enabling_tracking() {
    let mut w = world();
    w.spec.units[1].monarch = true;
    let imp=parse_keys_tdf(b"[CUSTOMKEYS]{\n CTRL_M=SelectUnits Monarch, TrackUnit;\n CTRLSHIFT_M=SelectUnitsAdd Monarch;\n }").unwrap();
    let mut m = Mapper::new(Profile::classic_tak().with_keys_tdf(&imp));
    select(&mut m, &w, &[1]);
    chord(&mut m, &w, &["ctrl", "shift"], "m");
    assert_eq!(m.selection(), &units(&[1, 2]));
    assert!(!m.following());
    chord(&mut m, &w, &["ctrl"], "m");
    assert_eq!(m.selection(), &units(&[2]));
    assert!(m.following());
}

#[test]
fn compacted_shift_import_erases_conflicting_default_before_fallback() {
    let imp = parse_keys_tdf(
        b"[CUSTOMKEYS]{\n CTRL_7=RetrieveSquadAdd 3;\n CTRLSHIFT_7=RetrieveSquadAdd 3;\n }",
    )
    .unwrap();
    assert_eq!(imp.count(LineStatus::Mapped), 2);
    assert_eq!(imp.bindings.len(), 1);
    assert_eq!(
        imp.redundant_triggers,
        vec![(k("7"), vec![k("ctrl"), k("shift")])]
    );
    let p = Profile::classic_tak().with_keys_tdf(&imp);
    p.validate().unwrap();
    assert_eq!(
        p.find_chord(&k("7"), &[k("ctrl"), k("shift")], Gesture::Press),
        None,
        "the old, more-specific group-7 default must not survive"
    );
    let modifiers = p.held_chord(&k("7"), |input| *input == k("ctrl") || *input == k("shift"));
    assert_eq!(
        p.find_chord(&k("7"), &modifiers, Gesture::Press),
        Some(Action::RecallGroupAdd { group: group(3) })
    );

    let w = world();
    let mut m = Mapper::new(p);
    assign(&mut m, &w, 3, &[2]);
    assign(&mut m, &w, 7, &[3]);
    select(&mut m, &w, &[1]);
    assert_eq!(chord(&mut m, &w, &["ctrl", "shift"], "7"), recall_output(3));
    assert_eq!(m.selection(), &units(&[1, 2]));
    m.perform(Action::RecallGroup { group: group(3) }, &w, Tick(1));
    assert_eq!(m.selection(), &units(&[2]));
    m.perform(Action::RecallGroup { group: group(7) }, &w, Tick(1));
    assert_eq!(m.selection(), &units(&[3]));
}
