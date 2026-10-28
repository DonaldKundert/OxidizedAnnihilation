//! Explicit Clear is a feature command, never an attack/guard fallback.
mod common;

use common::{TestWorld, WorldSpec, pos, uid};
use tak_commands::{FeatureId, PlayerId, QueueMode, SimCommand, Tick, UnitId, UnitSet, WorldPos};
use tak_input::{
    Action, BLANK, Binding, Button, CursorKind, EventKind, Feedback, Gesture, Input, InputEvent,
    Key, LineStatus, Mapper, Mode, Output, Profile, Refusal, UnitInfo, WorldView, clearers_for,
    parse_keys_tdf, resolve_context,
};

struct ClearWorld {
    units: TestWorld,
    target: Option<FeatureId>,
    capable: UnitSet,
}

fn world() -> ClearWorld {
    ClearWorld {
        units: TestWorld::new(serde_json::from_str::<WorldSpec>(r#"{
          "local_player":0,"enemies":[1],"units":[
            {"index":1,"owner":0,"def":0,"x":640,"y":480,"builder":"mobile"},
            {"index":2,"owner":0,"def":0,"x":500,"y":480,"builder":"factory"},
            {"index":3,"owner":0,"def":0,"x":500,"y":550},
            {"index":4,"owner":0,"def":0,"x":600,"y":550,"builder":"mobile","under_construction":true},
            {"index":5,"owner":1,"def":0,"x":800,"y":480,"builder":"mobile"},
            {"index":6,"owner":0,"def":0,"x":700,"y":550,"builder":"mobile","selectable":false},
            {"index":7,"owner":0,"def":0,"x":700,"y":600,"builder":"mobile"}
          ]}"#).unwrap()),
        target: Some(FeatureId::new(4, 2)),
        // Intentionally over-permissive: snapshot filters remain mandatory.
        capable: (1..=6).map(uid).collect(),
    }
}

impl WorldView for ClearWorld {
    fn local_player(&self) -> PlayerId {
        self.units.local_player()
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo)) {
        self.units.for_each_unit(f);
    }
    fn is_enemy(&self, owner: PlayerId) -> bool {
        self.units.is_enemy(owner)
    }
    fn can_clear(&self, unit: UnitId) -> bool {
        self.capable.contains(unit)
    }
    fn clear_target_at(&self, p: WorldPos) -> Option<FeatureId> {
        (p.x >= pos(780, 0).x && p.x < pos(820, 0).x && p.y >= pos(0, 460).y && p.y < pos(0, 500).y)
            .then_some(self.target)
            .flatten()
    }
    fn clear_target_pos(&self, target: FeatureId) -> Option<WorldPos> {
        (self.target == Some(target)).then_some(pos(800, 480))
    }
}

fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&i| uid(i)).collect()
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
fn point(m: &mut Mapper, w: &impl WorldView, t: u32, x: i32, y: i32) {
    event(m, w, t, EventKind::PointerMove { x, y });
}
fn selected(p: Profile, w: &impl WorldView, x: i32, y: i32) -> Mapper {
    let mut m = Mapper::new(p);
    point(&mut m, w, 0, x, y);
    m.perform(Action::Select, w, Tick(0));
    m.take_feedback();
    m
}

#[test]
fn capability_is_explicit_and_filters_ownership_completion_not_builder_status() {
    let w = world();
    let all = units(&[1, 2, 3, 4, 5, 6, 7, 99]);
    assert_eq!(clearers_for(&w, &all), units(&[1, 2, 3]));
    // Legacy adapters do not acquire Clear merely by reporting a builder.
    assert!(clearers_for(&w.units, &all).is_empty());
    assert!(!w.units.can_clear(uid(1)));
}

#[test]
fn explicit_feature_target_wins_over_a_unit_and_preserves_queue_mode() {
    let w = world();
    let hovered = w.unit(uid(5)).unwrap();
    for queue in [QueueMode::Replace, QueueMode::Append] {
        assert_eq!(
            resolve_context(
                &w,
                &units(&[1, 2, 3, 4, 5, 6, 7, 99]),
                pos(800, 480),
                Some(&hovered),
                Mode::Clear,
                queue
            ),
            Some(SimCommand::Clear {
                units: units(&[1, 2, 3]),
                target: w.target.unwrap(),
                queue
            })
        );
    }
    // Ordinary context is unchanged, even over a feature.
    assert!(matches!(
        resolve_context(
            &w,
            &units(&[1]),
            pos(800, 480),
            Some(&hovered),
            Mode::None,
            QueueMode::Replace
        ),
        Some(SimCommand::Attack { .. })
    ));
}

#[test]
fn c_shift_click_repeat_replace_and_cancel_share_one_targeting_path() {
    let w = world();
    let mut m = selected(Profile::classic_tak(), &w, 640, 480);
    key(&mut m, &w, 10, "c", true);
    key(&mut m, &w, 11, "c", false);
    key(&mut m, &w, 12, "c", true);
    assert_eq!(m.mode(), Mode::Clear);
    point(&mut m, &w, 20, 800, 480);
    assert_eq!(m.cursor_kind(&w), CursorKind::Clear);
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
        vec![SimCommand::Clear {
            units: units(&[1]),
            target: w.target.unwrap(),
            queue: QueueMode::Append
        }]
    );
    assert_eq!(m.mode(), Mode::Clear);
    key(&mut m, &w, 60, "shift", false);
    assert_eq!(
        commands(m.perform(Action::Context { append: false }, &w, Tick(2))),
        vec![SimCommand::Clear {
            units: units(&[1]),
            target: w.target.unwrap(),
            queue: QueueMode::Replace
        }]
    );
    assert_eq!(m.mode(), Mode::None);
    m.perform(Action::ClearMode, &w, Tick(2));
    m.perform(Action::Cancel, &w, Tick(2));
    assert_eq!(m.mode(), Mode::None);
    assert_eq!(m.selection(), &units(&[1]));
}

#[test]
fn invalid_or_lost_targets_never_fall_back_and_new_lifetimes_use_new_handles() {
    let mut w = world();
    let mut m = selected(Profile::classic_tak(), &w, 640, 480);
    m.perform(Action::ClearMode, &w, Tick(1));
    for (x, y) in [(640, 480), (900, 700), (820, 480)] {
        point(&mut m, &w, 1, x, y);
        assert_eq!(m.cursor_kind(&w), CursorKind::Invalid);
        assert!(commands(m.perform(Action::Context { append: false }, &w, Tick(1))).is_empty());
        assert_eq!(m.mode(), Mode::Clear);
    }
    point(&mut m, &w, 2, 800, 480);
    w.target = None;
    assert_eq!(m.cursor_kind(&w), CursorKind::Invalid);
    assert!(commands(m.perform(Action::Context { append: false }, &w, Tick(1))).is_empty());
    w.target = Some(FeatureId::new(4, 3));
    assert_eq!(
        commands(m.perform(Action::Context { append: false }, &w, Tick(1))),
        vec![SimCommand::Clear {
            units: units(&[1]),
            target: FeatureId::new(4, 3),
            queue: QueueMode::Replace
        }]
    );
}

#[test]
fn actor_capability_loss_invalidates_pending_cursor_and_dispatch() {
    let mut w = world();
    let mut m = selected(Profile::classic_tak(), &w, 640, 480);
    m.perform(Action::ClearMode, &w, Tick(1));
    point(&mut m, &w, 1, 800, 480);
    w.capable = UnitSet::new();
    assert_eq!(m.cursor_kind(&w), CursorKind::Invalid);
    assert!(commands(m.perform(Action::Context { append: false }, &w, Tick(1))).is_empty());
    assert_eq!(m.mode(), Mode::Clear);
}

#[test]
fn clear_mode_refuses_empty_and_ineligible_selections_and_stop_disarms_it() {
    let w = world();
    let mut m = Mapper::new(Profile::classic_tak());
    m.perform(Action::ClearMode, &w, Tick(1));
    assert_eq!(m.mode(), Mode::None);
    assert!(m.take_feedback().contains(&Feedback::Refused {
        reason: Refusal::NothingSelected
    }));
    for (x, y) in [(600, 550), (700, 600)] {
        let mut m = selected(Profile::classic_tak(), &w, x, y);
        m.perform(Action::ClearMode, &w, Tick(1));
        assert_eq!(m.mode(), Mode::None);
        assert!(m.take_feedback().contains(&Feedback::Refused {
            reason: Refusal::CannotDo
        }));
    }
    let mut m = selected(Profile::classic_tak(), &w, 640, 480);
    m.perform(Action::ClearMode, &w, Tick(1));
    assert!(matches!(
        commands(m.perform(Action::Stop, &w, Tick(1))).as_slice(),
        [SimCommand::Stop { .. }]
    ));
    assert_eq!(m.mode(), Mode::None);
}

#[test]
fn immobile_capable_builder_dispatch_is_not_filtered_as_factory_production() {
    let w = world();
    let mut m = selected(Profile::classic_tak(), &w, 500, 480);
    m.perform(Action::ClearMode, &w, Tick(1));
    point(&mut m, &w, 1, 800, 480);
    assert_eq!(m.cursor_kind(&w), CursorKind::Clear);
    assert_eq!(
        commands(m.perform(Action::Context { append: false }, &w, Tick(1))),
        vec![SimCommand::Clear {
            units: units(&[2]),
            target: w.target.unwrap(),
            queue: QueueMode::Replace
        }]
    );
}

#[test]
fn clear_key_import_and_profiles_preserve_supported_action() {
    let imp = parse_keys_tdf(b"[CUSTOMKEYS] { LOWER_Q = UnitCommand Clear; }").unwrap();
    assert_eq!(imp.count(LineStatus::Mapped), 1);
    assert_eq!(imp.count(LineStatus::Reserved), 0);
    let p = Profile::classic_tak().with_keys_tdf(&imp);
    assert_eq!(Profile::from_json(&p.to_json_pretty()).unwrap(), p);
    let w = world();
    let mut m = selected(p, &w, 640, 480);
    key(&mut m, &w, 1, "q", true);
    assert_eq!(m.mode(), Mode::Clear);
    assert!(
        Profile::default_combined()
            .command_wheel
            .contains(&Action::ClearMode)
    );
}

#[test]
fn custom_wheels_hide_clear_without_shifting_fixed_positions() {
    let w = world();
    for fixed in [false, true] {
        for (x, y, available) in [
            (640, 480, true),
            (500, 480, true),
            (500, 550, true),
            (700, 600, false),
        ] {
            let mut p = Profile::classic_tak();
            p.fixed_slots = fixed;
            p.command_wheel = vec![Action::Stop, Action::ClearMode];
            p.bindings.push(Binding {
                with_all: Vec::new(),
                input: Input::Key(Key::new("q")),
                with: None,
                gesture: Gesture::Press,
                action: Action::CommandWheel,
            });
            let mut m = selected(p, &w, x, y);
            key(&mut m, &w, 1, "q", true);
            assert_eq!(
                m.wheel().unwrap().slots,
                if available {
                    vec![Action::Stop, Action::ClearMode]
                } else if fixed {
                    vec![Action::Stop, BLANK]
                } else {
                    vec![Action::Stop]
                }
            );
        }
    }
}

#[test]
fn classic_and_original_primary_click_complete_clear_without_selection_or_attack() {
    let w = world();
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let mut m = selected(profile, &w, 640, 480);
        key(&mut m, &w, 1, "c", true);
        point(&mut m, &w, 2, 800, 480);
        event(
            &mut m,
            &w,
            3,
            EventKind::ButtonDown {
                button: Button::MouseLeft,
            },
        );
        let out = event(
            &mut m,
            &w,
            4,
            EventKind::ButtonUp {
                button: Button::MouseLeft,
            },
        );
        assert_eq!(
            commands(out),
            vec![SimCommand::Clear {
                units: units(&[1]),
                target: w.target.unwrap(),
                queue: QueueMode::Replace,
            }]
        );
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), Mode::None);
    }
}

#[test]
fn controller_wheel_confirmation_arms_clear_and_context_button_targets_feature() {
    let w = world();
    let mut p = Profile::default_combined();
    p.command_wheel = vec![Action::ClearMode];
    p.tuning.reticle = false;
    p.bindings.push(Binding {
        with_all: Vec::new(),
        input: Input::Key(Key::new("q")),
        with: None,
        gesture: Gesture::Press,
        action: Action::CommandWheel,
    });
    let mut m = selected(p, &w, 640, 480);
    key(&mut m, &w, 1, "q", true);
    // The one-slot command wheel uses the same gamepad axis sampler as the
    // installed console layout; release commits its highlighted Clear.
    event(
        &mut m,
        &w,
        2,
        EventKind::Axis {
            axis: tak_input::Axis::LeftY,
            value: -32767,
        },
    );
    key(&mut m, &w, 3, "q", false);
    assert!(m.wheel().is_none());
    assert_eq!(m.mode(), Mode::Clear);
    event(
        &mut m,
        &w,
        4,
        EventKind::Axis {
            axis: tak_input::Axis::LeftY,
            value: 0,
        },
    );
    point(&mut m, &w, 5, 800, 480);
    event(&mut m, &w, 6, EventKind::ButtonDown { button: Button::X });
    let out = event(&mut m, &w, 7, EventKind::ButtonUp { button: Button::X });
    assert_eq!(
        commands(out),
        vec![SimCommand::Clear {
            units: units(&[1]),
            target: w.target.unwrap(),
            queue: QueueMode::Replace,
        }]
    );
    assert_eq!(m.mode(), Mode::None);
}
