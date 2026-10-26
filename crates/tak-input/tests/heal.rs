//! Heal/repair orders share a keyboard, HUD and wheel path without granting
//! factories, unfinished builders or another player's units work capability.

mod common;

use common::{TestWorld, WorldSpec, pos, uid};
use tak_commands::{PlayerId, QueueMode, SimCommand, Tick, UnitId, UnitSet};
use tak_input::{
    Action, Axis, BLANK, Binding, Button, CursorKind, EventKind, Feedback, Gesture, Input,
    InputEvent, Key, LineStatus, Mapper, Mode, Output, Profile, Refusal, Ring, UnitInfo, WheelKind,
    WorldView, healers_for, parse_keys_tdf, resolve_context, ring_slots, slot_direction,
};
use tak_math::Fx;

fn world() -> TestWorld {
    TestWorld::new(
        serde_json::from_str::<WorldSpec>(
            r#"{
        "local_player": 0, "enemies": [1], "units": [
            {"index": 1, "owner": 0, "def": 0, "x": 640, "y": 480,
             "builder": "mobile"},
            {"index": 2, "owner": 0, "def": 0, "x": 780, "y": 480},
            {"index": 3, "owner": 0, "def": 0, "x": 850, "y": 480,
             "under_construction": true},
            {"index": 4, "owner": 0, "def": 0, "x": 500, "y": 480,
             "builder": "factory"},
            {"index": 5, "owner": 0, "def": 0, "x": 500, "y": 550,
             "builder": "mobile", "under_construction": true},
            {"index": 6, "owner": 1, "def": 0, "x": 900, "y": 480,
             "builder": "mobile"},
            {"index": 7, "owner": 2, "def": 0, "x": 950, "y": 480,
             "builder": "mobile"},
            {"index": 8, "owner": 0, "def": 0, "x": 600, "y": 550,
             "builder": "mobile", "selectable": false},
            {"index": 9, "owner": 0, "def": 0, "x": 640, "y": 550,
             "builder": "mobile"}
        ]
    }"#,
        )
        .unwrap(),
    )
}

fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&id| uid(id)).collect()
}

fn commands(out: Vec<Output>) -> Vec<SimCommand> {
    out.into_iter()
        .filter_map(|o| match o {
            Output::Sim { command, .. } => Some(command),
            _ => None,
        })
        .collect()
}

fn event(m: &mut Mapper, w: &impl WorldView, t: u32, kind: EventKind) -> Vec<Output> {
    m.handle(&InputEvent::new(t, kind), w, Tick(1))
}

fn point(m: &mut Mapper, w: &impl WorldView, t: u32, x: i32, y: i32) {
    event(m, w, t, EventKind::PointerMove { x, y });
}

fn key(m: &mut Mapper, w: &impl WorldView, t: u32, name: &str, down: bool) -> Vec<Output> {
    event(
        m,
        w,
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

fn selected(p: Profile, w: &impl WorldView, x: i32, y: i32) -> Mapper {
    let mut m = Mapper::new(p);
    point(&mut m, w, 0, x, y);
    m.perform(Action::Select, w, Tick(0));
    m.take_feedback();
    m
}

#[test]
fn default_capability_selects_only_complete_owned_mobile_builders() {
    let w = world();
    let selection = units(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 99]);
    assert_eq!(healers_for(&w, &selection), units(&[1, 9]));
    for id in 1..=9 {
        assert_eq!(w.can_heal(uid(id)), [1, 9].contains(&id));
    }
    assert!(!w.can_heal(uid(99)));
}

struct RestrictedWorld(TestWorld);

impl WorldView for RestrictedWorld {
    fn local_player(&self) -> PlayerId {
        self.0.local_player()
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo)) {
        self.0.for_each_unit(f);
    }
    fn is_enemy(&self, owner: PlayerId) -> bool {
        self.0.is_enemy(owner)
    }
    fn can_heal(&self, unit: UnitId) -> bool {
        // Represents the adapter rejecting a dying/zero-work-rate builder.
        // Returning true for every other id cannot bypass snapshot checks.
        unit != uid(1)
    }
}

#[test]
fn adapter_capability_can_restrict_but_not_bypass_actor_eligibility() {
    let w = RestrictedWorld(world());
    assert_eq!(
        healers_for(&w, &units(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 99])),
        units(&[9])
    );
}

#[test]
fn context_heals_owned_units_and_frames_with_only_eligible_actors() {
    let w = world();
    let selection = units(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 99]);
    for target in [2, 3] {
        for queue in [QueueMode::Replace, QueueMode::Append] {
            let hovered = w.unit(uid(target)).unwrap();
            assert_eq!(
                resolve_context(
                    &w,
                    &selection,
                    hovered.pos,
                    Some(&hovered),
                    Mode::Heal,
                    queue
                ),
                Some(SimCommand::Heal {
                    units: units(&[1, 9]),
                    target: uid(target),
                    queue
                })
            );
        }
    }
    // A selected healer may be repaired by the other selected healer, but
    // must never receive a self-heal order.
    let hovered = w.unit(uid(1)).unwrap();
    assert_eq!(
        resolve_context(
            &w,
            &selection,
            hovered.pos,
            Some(&hovered),
            Mode::Heal,
            QueueMode::Replace
        ),
        Some(SimCommand::Heal {
            units: units(&[9]),
            target: uid(1),
            queue: QueueMode::Replace
        })
    );
}

#[test]
fn invalid_targets_and_empty_or_stale_actors_produce_no_order() {
    let w = world();
    let h = w.unit(uid(2)).unwrap();
    for selection in [UnitSet::new(), units(&[2, 4, 5, 6, 7, 8, 99])] {
        assert!(
            resolve_context(
                &w,
                &selection,
                h.pos,
                Some(&h),
                Mode::Heal,
                QueueMode::Replace
            )
            .is_none()
        );
    }
    for target in [1, 6, 7, 8] {
        let hovered = w.unit(uid(target)).unwrap();
        assert!(
            resolve_context(
                &w,
                &units(&[1]),
                hovered.pos,
                Some(&hovered),
                Mode::Heal,
                QueueMode::Replace
            )
            .is_none()
        );
    }
    let stale = UnitInfo { id: uid(99), ..h };
    for hovered in [None, Some(&stale)] {
        assert!(
            resolve_context(
                &w,
                &units(&[1]),
                pos(1000, 700),
                hovered,
                Mode::Heal,
                QueueMode::Replace
            )
            .is_none()
        );
    }
    // Recheck current ownership, not the caller's obsolete snapshot.
    let stale_owner = UnitInfo {
        owner: PlayerId(0),
        ..w.unit(uid(7)).unwrap()
    };
    assert!(
        resolve_context(
            &w,
            &units(&[1]),
            stale_owner.pos,
            Some(&stale_owner),
            Mode::Heal,
            QueueMode::Replace
        )
        .is_none()
    );
}

#[test]
fn h_then_shift_click_queues_and_keeps_mode_until_replace_or_cancel() {
    let w = world();
    let mut m = selected(Profile::classic_tak(), &w, 640, 480);
    key(&mut m, &w, 10, "h", true);
    assert_eq!(m.mode(), Mode::Heal);
    point(&mut m, &w, 20, 780, 480);
    assert_eq!(m.cursor_kind(&w), CursorKind::Heal);
    key(&mut m, &w, 30, "shift", true);
    event(
        &mut m,
        &w,
        40,
        EventKind::ButtonDown {
            button: Button::MouseLeft,
        },
    );
    let out = event(
        &mut m,
        &w,
        50,
        EventKind::ButtonUp {
            button: Button::MouseLeft,
        },
    );
    assert_eq!(
        commands(out),
        vec![SimCommand::Heal {
            units: units(&[1]),
            target: uid(2),
            queue: QueueMode::Append
        }]
    );
    assert_eq!(m.mode(), Mode::Heal);
    assert_eq!(m.selection(), &units(&[1]));
    key(&mut m, &w, 60, "shift", false);
    point(&mut m, &w, 70, 850, 480);
    assert_eq!(m.cursor_kind(&w), CursorKind::Heal);
    assert_eq!(
        commands(m.perform(Action::Context { append: false }, &w, Tick(2))),
        vec![SimCommand::Heal {
            units: units(&[1]),
            target: uid(3),
            queue: QueueMode::Replace
        }]
    );
    assert_eq!(m.mode(), Mode::None);
    m.perform(Action::HealMode, &w, Tick(2));
    m.perform(Action::Cancel, &w, Tick(2));
    assert_eq!(m.mode(), Mode::None);
    assert_eq!(m.selection(), &units(&[1]));
}

#[test]
fn invalid_clicks_keep_targeting_and_do_not_fall_back_to_move_or_guard() {
    let w = world();
    let mut m = selected(Profile::classic_tak(), &w, 640, 480);
    m.perform(Action::HealMode, &w, Tick(1));
    for (t, x, y) in [
        (10, 900, 480),
        (20, 950, 480),
        (30, 640, 480),
        (40, 1100, 700),
    ] {
        point(&mut m, &w, t, x, y);
        assert_eq!(m.cursor_kind(&w), CursorKind::Invalid);
        assert!(commands(m.perform(Action::Context { append: false }, &w, Tick(1))).is_empty());
        assert_eq!(m.mode(), Mode::Heal);
    }
}

#[test]
fn heal_mode_refuses_empty_and_ineligible_selections() {
    let w = world();
    let mut empty = Mapper::new(Profile::classic_tak());
    empty.perform(Action::HealMode, &w, Tick(1));
    assert_eq!(empty.mode(), Mode::None);
    assert!(empty.take_feedback().contains(&Feedback::Refused {
        reason: Refusal::NothingSelected
    }));
    for (x, y) in [(780, 480), (500, 480), (500, 550)] {
        let mut m = selected(Profile::classic_tak(), &w, x, y);
        m.perform(Action::HealMode, &w, Tick(1));
        assert_eq!(m.mode(), Mode::None);
        assert!(m.take_feedback().contains(&Feedback::Refused {
            reason: Refusal::CannotDo
        }));
    }
}

#[test]
fn actor_losing_capability_during_targeting_invalidates_cursor_and_order() {
    let w = world();
    let mut m = selected(Profile::classic_tak(), &w, 640, 480);
    m.perform(Action::HealMode, &w, Tick(1));
    let restricted = RestrictedWorld(w);
    point(&mut m, &restricted, 10, 780, 480);
    assert_eq!(m.cursor_kind(&restricted), CursorKind::Invalid);
    assert!(
        commands(m.perform(Action::Context { append: false }, &restricted, Tick(1))).is_empty()
    );
}

#[test]
fn imported_heal_and_repair_aliases_are_supported_actions() {
    let imp = parse_keys_tdf(
        b"[CUSTOMKEYS]\n{\nLOWER_Q = UnitCommand Heal;\nLOWER_R = UnitCommand Repair;\n}\n",
    )
    .unwrap();
    assert_eq!(imp.count(LineStatus::Mapped), 2);
    assert_eq!(imp.count(LineStatus::Reserved), 0);
    let p = Profile::classic_tak().with_keys_tdf(&imp);
    p.validate().unwrap();
    let encoded = p.to_json_pretty();
    assert_eq!(Profile::from_json(&encoded).unwrap(), p);
    let w = world();
    for name in ["q", "r"] {
        let mut m = selected(p.clone(), &w, 640, 480);
        key(&mut m, &w, 10, name, true);
        assert_eq!(m.mode(), Mode::Heal);
    }
}

fn wheel_profile(fixed: bool, slots: Vec<Action>) -> Profile {
    let mut p = Profile::classic_tak();
    p.fixed_slots = fixed;
    p.command_wheel = slots;
    p.bindings.push(Binding {
        with_all: Vec::new(),
        input: Input::Key(Key::new("q")),
        with: None,
        gesture: Gesture::Press,
        action: Action::CommandWheel,
    });
    p.validate().unwrap();
    p
}

#[test]
fn custom_command_wheels_filter_heal_by_exact_capability() {
    let w = world();
    for fixed in [false, true] {
        for (x, available) in [(640, true), (500, false), (780, false)] {
            let mut m = selected(
                wheel_profile(fixed, vec![Action::Stop, Action::HealMode]),
                &w,
                x,
                480,
            );
            key(&mut m, &w, 10, "q", true);
            assert_eq!(
                m.wheel().unwrap().slots,
                if available {
                    vec![Action::Stop, Action::HealMode]
                } else if fixed {
                    vec![Action::Stop, BLANK]
                } else {
                    vec![Action::Stop]
                }
            );
        }
    }
}

fn steer(m: &mut Mapper, w: &impl WorldView, t: u32, slot: usize, count: usize) {
    let (s, c) = slot_direction(slot, count).sin_cos();
    let scale = |v: Fx| ((i64::from(v.raw()) * 32767) >> 16) as i16;
    event(
        m,
        w,
        t,
        EventKind::Axis {
            axis: Axis::LeftX,
            value: scale(c),
        },
    );
    event(
        m,
        w,
        t,
        EventKind::Axis {
            axis: Axis::LeftY,
            value: scale(s),
        },
    );
}

#[test]
fn unit_ring_keeps_heal_position_and_confirm_enters_same_mode() {
    let w = world();
    assert_eq!(ring_slots(Ring::Unit)[1], Action::HealMode);
    for (x, available) in [(640, true), (500, false)] {
        let mut m = selected(
            wheel_profile(true, vec![Action::OpenRing { ring: Ring::Unit }]),
            &w,
            x,
            480,
        );
        key(&mut m, &w, 10, "q", true);
        steer(&mut m, &w, 20, 0, 1);
        key(&mut m, &w, 30, "q", false);
        let ring = m.wheel().unwrap();
        assert_eq!(ring.kind, WheelKind::Unit);
        assert_eq!(
            ring.slots[1],
            if available { Action::HealMode } else { BLANK }
        );
        assert_eq!(ring.slots.len(), 6);
        if available {
            steer(&mut m, &w, 40, 1, 6);
            event(
                &mut m,
                &w,
                50,
                EventKind::ButtonDown {
                    button: Button::MouseRight,
                },
            );
            assert!(m.wheel().is_none());
            assert_eq!(m.mode(), Mode::Heal);
        }
    }
}

#[test]
fn empty_menu_healer_does_not_hide_a_mixed_selections_real_build_menu() {
    use tak_commands::{Scope, UnitDefId};
    let mut w = world();
    w.spec
        .units
        .iter_mut()
        .find(|u| u.index == 9)
        .unwrap()
        .build_options = vec![42, 43];
    let mut p = Profile::classic_tak();
    p.bindings.clear();
    p.bindings.push(Binding {
        with_all: Vec::new(),
        input: Input::Key(Key::new("b")),
        with: None,
        gesture: Gesture::Press,
        action: Action::BuildWheel,
    });
    for fixed in [false, true] {
        p.fixed_slots = fixed;
        let mut m = Mapper::new(p.clone());
        m.perform(Action::SelectAll { scope: Scope::Map }, &w, Tick(0));
        assert_eq!(healers_for(&w, m.selection()), units(&[1, 9]));
        key(&mut m, &w, 1, "b", true);
        let wheel = m.wheel().expect("real builder's build menu");
        assert_eq!(wheel.kind, WheelKind::Build);
        assert_eq!(wheel.slots[0], Action::BuildOption { index: 0 });
        assert_eq!(wheel.slots[1], Action::BuildOption { index: 1 });
        let mut m = Mapper::new(p.clone());
        m.perform(Action::SelectAll { scope: Scope::Map }, &w, Tick(0));
        m.perform(Action::BuildOption { index: 1 }, &w, Tick(0));
        assert_eq!(m.mode(), Mode::Build(UnitDefId(43)));
        m.perform(Action::HealMode, &w, Tick(0));
        assert_eq!(m.mode(), Mode::Heal, "empty-menu healer still has Heal");
    }
}
