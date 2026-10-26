//! Factory Guard is a standing product order, never a factory production order.

mod common;

use common::{TestWorld, pos, uid};
use tak_commands::{
    FactoryRallyOrder, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_input::{
    Action, Button, CursorKind, EventKind, Feedback, InputEvent, Key, Mapper, Mode, Output,
    Profile, Refusal, UnitInfo, WorldView,
};

fn world() -> TestWorld {
    TestWorld::new(serde_json::from_str(r#"{
        "local_player":0,"enemies":[1],"units":[
            {"index":1,"owner":0,"def":10,"x":100,"y":100,"builder":"factory","build_options":[20]},
            {"index":2,"owner":0,"def":11,"x":200,"y":100},
            {"index":3,"owner":0,"def":11,"x":300,"y":100},
            {"index":4,"owner":0,"def":10,"x":400,"y":100,"builder":"factory","build_options":[20]},
            {"index":5,"owner":0,"def":10,"x":500,"y":100,"builder":"factory","build_options":[20],"under_construction":true}
        ]}"#).unwrap())
}

fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&id| uid(id)).collect()
}

fn sim(out: Vec<Output>) -> Vec<SimCommand> {
    out.into_iter()
        .filter_map(|out| match out {
            Output::Sim { command, .. } => Some(command),
            _ => None,
        })
        .collect()
}

fn event(m: &mut Mapper, w: &impl WorldView, kind: EventKind) -> Vec<Output> {
    m.handle(&InputEvent::new(m.now() + 20, kind), w, Tick(7))
}

fn key(m: &mut Mapper, w: &impl WorldView, name: &str, down: bool) {
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
    );
}

fn selected(w: &impl WorldView, ids: &[u32], profile: Profile) -> Mapper {
    let mut m = Mapper::new(profile);
    key(&mut m, w, "shift", true);
    for &id in ids {
        let p = w.unit(uid(id)).unwrap().pos;
        point(&mut m, w, p, false);
        m.perform(Action::Select, w, Tick(0));
    }
    key(&mut m, w, "shift", false);
    assert_eq!(m.selection(), &units(ids));
    m
}

fn point(m: &mut Mapper, w: &impl WorldView, to: tak_commands::WorldPos, minimap: bool) {
    event(
        m,
        w,
        if minimap {
            EventKind::MapPointer {
                x: 1200,
                y: 50,
                world: to,
            }
        } else {
            EventKind::PointerMove {
                x: to.x.raw() / 65536,
                y: to.y.raw() / 65536,
            }
        },
    );
}

fn rally(ids: &[u32], target: UnitId, queue: QueueMode) -> SimCommand {
    SimCommand::SetFactoryRally {
        units: units(ids),
        order: Some(FactoryRallyOrder::Guard { target }),
        queue,
    }
}

#[test]
fn factory_guard_splits_mixed_selections_and_respects_all_queue_paths() {
    let w = world();
    for minimap in [false, true] {
        for explicit in [false, true] {
            for mixed in [false, true] {
                for modifier in ["replace", "append", "shift", "rb"] {
                    let ids = if mixed {
                        &[1, 2, 4, 5][..]
                    } else {
                        &[1, 4, 5][..]
                    };
                    let mut m = selected(&w, ids, Profile::default_combined());
                    // Existing finite/repeated production intents must not be
                    // accompanied by cancellation or replacement production.
                    assert!(
                        !sim(m.perform(Action::BuildOption { index: 0 }, &w, Tick(1))).is_empty()
                    );
                    assert!(
                        !sim(m.perform(Action::RepeatBuildOption { index: 0 }, &w, Tick(1)))
                            .is_empty()
                    );
                    if explicit {
                        m.perform(Action::GuardMode, &w, Tick(1));
                    }
                    match modifier {
                        "shift" => key(&mut m, &w, "shift", true),
                        "rb" => {
                            event(&mut m, &w, EventKind::ButtonDown { button: Button::Rb });
                        }
                        _ => {}
                    }
                    point(&mut m, &w, pos(300, 100), minimap);
                    assert_eq!(m.cursor_kind(&w), CursorKind::Guard);
                    let queue = if modifier == "replace" {
                        QueueMode::Replace
                    } else {
                        QueueMode::Append
                    };
                    let mut expected = Vec::new();
                    if mixed {
                        expected.push(SimCommand::Guard {
                            units: units(&[2]),
                            target: uid(3),
                            queue,
                        });
                    }
                    expected.push(rally(&[1, 4], uid(3), queue));
                    assert_eq!(
                        sim(m.perform(
                            Action::Context {
                                append: modifier == "append"
                            },
                            &w,
                            Tick(7)
                        )),
                        expected,
                        "minimap={minimap}, explicit={explicit}, mixed={mixed}, modifier={modifier}"
                    );
                    assert_eq!(
                        m.mode(),
                        if explicit && queue == QueueMode::Append {
                            Mode::Guard
                        } else {
                            Mode::None
                        }
                    );
                }
            }
        }
    }
}

/// Picker snapshots can lag the authoritative per-ID lookup. Both capability
/// overrides are deliberately permissive unless specifically vetoed, so the
/// mapper must independently enforce the visible validity constraints.
struct GuardView {
    world: TestWorld,
    target: Option<UnitInfo>,
    block_target: bool,
    block_factory: bool,
}

impl GuardView {
    fn new() -> Self {
        let world = world();
        let target = world.unit(uid(3));
        Self {
            world,
            target,
            block_target: false,
            block_factory: false,
        }
    }
}

impl WorldView for GuardView {
    fn local_player(&self) -> PlayerId {
        self.world.local_player()
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo)) {
        self.world.for_each_unit(f);
    }
    fn unit(&self, id: UnitId) -> Option<UnitInfo> {
        if id == uid(3) {
            self.target
        } else {
            self.world.unit(id)
        }
    }
    fn is_enemy(&self, owner: PlayerId) -> bool {
        self.world.is_enemy(owner)
    }
    fn build_options(&self, id: UnitId) -> Vec<UnitDefId> {
        self.world.build_options(id)
    }
    fn can_factory_control(&self, _id: UnitId) -> bool {
        !self.block_factory
    }
    fn can_factory_guard_target(&self, _id: UnitId) -> bool {
        !self.block_target
    }
}

#[test]
fn factory_guard_revalidates_stale_target_and_honours_adapter_veto() {
    for minimap in [false, true] {
        for explicit in [false, true] {
            for mixed in [false, true] {
                for invalid in ["missing", "generation", "enemy", "unselectable", "veto"] {
                    let mut w = GuardView::new();
                    let ids = if mixed { &[1, 2][..] } else { &[1][..] };
                    let mut m = selected(&w, ids, Profile::default_combined());
                    if explicit {
                        m.perform(Action::GuardMode, &w, Tick(1));
                    }
                    point(&mut m, &w, pos(300, 100), minimap);
                    assert_eq!(m.cursor_kind(&w), CursorKind::Guard);
                    match invalid {
                        "missing" => w.target = None,
                        "generation" => w.target.as_mut().unwrap().id = UnitId::new(3, 1),
                        "enemy" => w.target.as_mut().unwrap().owner = PlayerId(1),
                        "unselectable" => w.target.as_mut().unwrap().selectable = false,
                        "veto" => w.block_target = true,
                        _ => unreachable!(),
                    }
                    assert_eq!(
                        m.cursor_kind(&w),
                        if mixed {
                            CursorKind::Guard
                        } else {
                            CursorKind::Invalid
                        }
                    );
                    // Mobile Guard is intentionally unchanged; factory-only
                    // target capability checks do not alter its old behavior.
                    let expected = if mixed {
                        vec![SimCommand::Guard {
                            units: units(&[2]),
                            target: uid(3),
                            queue: QueueMode::Replace,
                        }]
                    } else {
                        Vec::new()
                    };
                    assert_eq!(
                        sim(m.perform(Action::Context { append: false }, &w, Tick(7))),
                        expected,
                        "minimap={minimap}, explicit={explicit}, mixed={mixed}, invalid={invalid}"
                    );
                }
            }
        }
    }
}

#[test]
fn factory_guard_revalidates_factory_eligibility_with_permissive_adapter() {
    for invalid in ["owner", "generation", "unselectable", "unfinished", "veto"] {
        let mut w = GuardView::new();
        let mut m = selected(&w, &[1], Profile::default_combined());
        m.perform(Action::GuardMode, &w, Tick(1));
        point(&mut m, &w, pos(300, 100), false);
        assert_eq!(m.cursor_kind(&w), CursorKind::Guard);
        let factory = &mut w.world.spec.units[0];
        match invalid {
            "owner" => factory.owner = 1,
            "generation" => factory.generation = 1,
            "unselectable" => factory.selectable = false,
            "unfinished" => factory.under_construction = true,
            "veto" => w.block_factory = true,
            _ => unreachable!(),
        }
        assert_eq!(m.cursor_kind(&w), CursorKind::Invalid, "{invalid}");
        assert!(
            sim(m.perform(Action::Context { append: false }, &w, Tick(7))).is_empty(),
            "{invalid}"
        );
    }
}

#[test]
fn factory_guard_accepts_allies_and_self_but_not_enemy_or_ground() {
    let mut w = world();
    w.spec.units[2].owner = 2;
    for target in [1, 3] {
        let mut m = selected(&w, &[1], Profile::default_combined());
        m.perform(Action::GuardMode, &w, Tick(1));
        point(&mut m, &w, pos(target * 100, 100), false);
        assert_eq!(m.cursor_kind(&w), CursorKind::Guard);
        assert_eq!(
            sim(m.perform(Action::Context { append: false }, &w, Tick(7))),
            vec![rally(&[1], uid(target as u32), QueueMode::Replace)]
        );
    }
    for minimap in [false, true] {
        for enemy in [false, true] {
            let mut m = selected(&w, &[1], Profile::default_combined());
            m.perform(Action::GuardMode, &w, Tick(1));
            point(
                &mut m,
                &w,
                if enemy { pos(300, 100) } else { pos(900, 600) },
                minimap,
            );
            // Change ownership after the action and pointer, before dispatch.
            w.spec.units[2].owner = 1;
            assert_eq!(m.cursor_kind(&w), CursorKind::Invalid);
            assert!(sim(m.perform(Action::Context { append: false }, &w, Tick(7))).is_empty());
        }
    }
}

#[test]
fn original_mouse_keeps_left_selection_but_explicit_and_right_guard_reach_factories() {
    let w = world();
    for minimap in [false, true] {
        let mut m = selected(&w, &[1], Profile::original_tak());
        point(&mut m, &w, pos(300, 100), minimap);
        assert_eq!(m.cursor_kind(&w), CursorKind::Select);
        assert!(
            sim(event(
                &mut m,
                &w,
                EventKind::ButtonDown {
                    button: Button::MouseLeft
                }
            ))
            .is_empty()
        );
        assert!(
            sim(event(
                &mut m,
                &w,
                EventKind::ButtonUp {
                    button: Button::MouseLeft
                }
            ))
            .is_empty()
        );
        assert_eq!(m.selection(), &units(&[3]));
        for explicit in [false, true] {
            let mut m = selected(&w, &[1], Profile::original_tak());
            if explicit {
                m.perform(Action::GuardMode, &w, Tick(1));
            }
            point(&mut m, &w, pos(300, 100), minimap);
            assert_eq!(
                m.cursor_kind(&w),
                if explicit {
                    CursorKind::Guard
                } else {
                    CursorKind::Select
                }
            );
            let button = if explicit {
                Button::MouseLeft
            } else {
                Button::MouseRight
            };
            let mut out = event(&mut m, &w, EventKind::ButtonDown { button });
            out.extend(event(&mut m, &w, EventKind::ButtonUp { button }));
            assert_eq!(sim(out), vec![rally(&[1], uid(3), QueueMode::Replace)]);
            assert_eq!(m.selection(), &units(&[1]));
        }
    }
}

#[test]
fn default_factory_guard_capability_rejects_invalid_targets() {
    let mut w = world();
    assert!(w.can_factory_guard_target(uid(3)));
    assert!(!w.can_factory_guard_target(uid(99)));
    assert!(!w.can_factory_guard_target(UnitId::new(3, 1)));
    w.spec.units[2].owner = 2;
    assert!(w.can_factory_guard_target(uid(3)));
    w.spec.units[2].owner = 1;
    assert!(!w.can_factory_guard_target(uid(3)));
    w.spec.units[2].owner = 0;
    w.spec.units[2].selectable = false;
    assert!(!w.can_factory_guard_target(uid(3)));
}

#[test]
fn guard_split_acknowledges_once_and_keeps_partial_refusal_feedback() {
    for (ids, block_target, command_count, refused) in [
        (&[1, 2][..], false, 2, false),
        (&[1][..], false, 1, false),
        (&[2][..], false, 1, false),
        (&[1, 4][..], false, 1, false),
        (&[5][..], false, 0, true),
        (&[1, 2, 5][..], false, 2, true),
        (&[1, 2][..], true, 1, true),
        (&[1][..], true, 0, true),
    ] {
        let mut w = GuardView::new();
        w.block_target = block_target;
        let mut m = selected(&w, ids, Profile::default_combined());
        m.perform(Action::GuardMode, &w, Tick(1));
        point(&mut m, &w, pos(300, 100), false);
        m.take_feedback();
        assert_eq!(
            sim(m.perform(Action::Context { append: false }, &w, Tick(7))).len(),
            command_count,
        );
        let mut expected = Vec::new();
        if refused {
            expected.push(Feedback::Refused {
                reason: Refusal::CannotDo,
            });
        }
        if command_count > 0 {
            expected.push(Feedback::Order);
        }
        assert_eq!(
            m.take_feedback(),
            expected,
            "ids={ids:?}, block_target={block_target}",
        );
    }
}
