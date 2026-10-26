//! Synthetic factory UI intents. Production and standing orders are separate;
//! batching, cancellation and gamepad paging must never replace production.

mod common;

use common::{TestWorld, WorldSpec, pos, uid};
use tak_commands::{
    FactoryRallyOrder, QueueMode, Scope, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_input::{
    Action, Axis, Button, CursorKind, EventKind, Feedback, InputEvent, Key, Mapper, Mode, Output,
    Profile, Refusal, WorldView,
};

fn world() -> TestWorld {
    TestWorld::new(serde_json::from_str::<WorldSpec>(r#"{
        "local_player":0,"enemies":[1],"units":[
            {"index":1,"owner":0,"def":10,"x":100,"y":100,"builder":"factory","build_options":[20,21]},
            {"index":2,"owner":0,"def":10,"x":200,"y":100,"builder":"factory","build_options":[20,21]},
            {"index":3,"owner":0,"def":11,"x":300,"y":100,"builder":"mobile","build_options":[21]},
            {"index":4,"owner":1,"def":10,"x":400,"y":100,"builder":"factory","build_options":[20,21]},
            {"index":5,"owner":0,"def":10,"x":500,"y":100,"builder":"factory","build_options":[20,21],"under_construction":true},
            {"index":6,"owner":0,"def":10,"x":600,"y":100,"builder":"factory","build_options":[20,21],"selectable":false},
            {"index":7,"owner":0,"def":12,"x":700,"y":100},
            {"index":8,"owner":0,"def":13,"x":800,"y":100,"builder":"factory","build_options":[22]}
        ]}"#).unwrap())
}
fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&id| uid(id)).collect()
}
fn sim(out: Vec<Output>) -> Vec<SimCommand> {
    out.into_iter()
        .filter_map(|o| match o {
            Output::Sim { command, .. } => Some(command),
            _ => None,
        })
        .collect()
}
fn event(m: &mut Mapper, w: &impl WorldView, kind: EventKind) -> Vec<Output> {
    m.handle(&InputEvent::new(m.now() + 20, kind), w, Tick(7))
}
fn key(m: &mut Mapper, w: &impl WorldView, name: &str, down: bool) -> Vec<Output> {
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
fn button(m: &mut Mapper, w: &impl WorldView, b: Button, down: bool) -> Vec<Output> {
    event(
        m,
        w,
        if down {
            EventKind::ButtonDown { button: b }
        } else {
            EventKind::ButtonUp { button: b }
        },
    )
}
fn select(m: &mut Mapper, w: &impl WorldView, ids: &[u32]) {
    m.perform(Action::Cancel, w, Tick(0));
    key(m, w, "shift", true);
    for &id in ids {
        let p = w.unit(uid(id)).unwrap().pos;
        event(
            m,
            w,
            EventKind::PointerMove {
                x: p.x.raw() / 65536,
                y: p.y.raw() / 65536,
            },
        );
        m.perform(Action::Select, w, Tick(0));
    }
    key(m, w, "shift", false);
}
fn mapper(w: &impl WorldView, ids: &[u32]) -> Mapper {
    let mut m = Mapper::new(Profile::default_combined());
    select(&mut m, w, ids);
    m
}

#[test]
fn unfinished_builder_is_selectable_but_cannot_choose_or_queue_construction() {
    let mut w = world();
    w.spec.units[0].builder = Some(tak_input::BuilderKind::Mobile);
    w.spec.units[0].under_construction = true;
    for id in [1, 4, 5, 6, 7, 99] {
        assert!(!w.can_build(uid(id)));
    }
    for id in [2, 3, 8] {
        assert!(w.can_build(uid(id)));
    }
    let mut m = mapper(&w, &[1]);
    assert_eq!(m.selection(), &units(&[1]));
    assert!(sim(m.perform(Action::BuildOption { index: 0 }, &w, Tick(1))).is_empty());
    assert_eq!(m.mode(), Mode::None);
    for queue in [QueueMode::Replace, QueueMode::Append] {
        assert!(
            tak_input::resolve_context(
                &w,
                &units(&[1]),
                pos(900, 600),
                None,
                Mode::Build(UnitDefId(20)),
                queue,
            )
            .is_none()
        );
    }
    // It is still a valid target for an operational builder's assistance.
    assert_eq!(
        tak_input::resolve_context(
            &w,
            &units(&[3]),
            pos(100, 100),
            w.unit(uid(1)).as_ref(),
            Mode::Heal,
            QueueMode::Append,
        ),
        Some(SimCommand::Heal {
            units: units(&[3]),
            target: uid(1),
            queue: QueueMode::Append,
        })
    );
}

#[test]
fn lower_id_unfinished_builder_cannot_determine_mixed_selection_menu() {
    let mut w = world();
    w.spec.units[0].builder = Some(tak_input::BuilderKind::Mobile);
    w.spec.units[0].under_construction = true;
    let mut m = mapper(&w, &[1, 3]);
    m.perform(Action::BuildOption { index: 0 }, &w, Tick(1));
    assert_eq!(m.mode(), Mode::Build(UnitDefId(21)));
    event(&mut m, &w, EventKind::PointerMove { x: 900, y: 600 });
    assert_eq!(
        sim(m.perform(Action::Context { append: true }, &w, Tick(1))),
        vec![SimCommand::Build {
            units: units(&[3]),
            def: UnitDefId(21),
            at: Some(pos(900, 600)),
            queue: QueueMode::Append,
        }]
    );
    assert_eq!(m.mode(), Mode::Build(UnitDefId(21)));
    // Eligibility is checked again when placing, not only when opening a menu.
    w.spec.units[2].under_construction = true;
    assert!(sim(m.perform(Action::Context { append: true }, &w, Tick(1))).is_empty());
    w.spec.units[0].under_construction = false;
    m.perform(Action::BuildOption { index: 0 }, &w, Tick(1));
    assert_eq!(m.mode(), Mode::Build(UnitDefId(20)));
}

#[test]
fn unfinished_mobile_does_not_steal_completed_factory_production_controls() {
    let mut w = world();
    w.spec.units[0].builder = Some(tak_input::BuilderKind::Mobile);
    w.spec.units[0].under_construction = true;
    let mut m = mapper(&w, &[1, 2]);
    assert_eq!(m.factory_production_count(0, &w), Some(0));
    assert_eq!(
        sim(m.perform(Action::BuildOption { index: 0 }, &w, Tick(1))),
        vec![SimCommand::Build {
            units: units(&[2]),
            def: UnitDefId(20),
            at: None,
            queue: QueueMode::Append,
        }]
    );
    assert_eq!(m.mode(), Mode::None);
    assert_eq!(
        sim(m.perform(Action::RepeatBuildOption { index: 0 }, &w, Tick(1))),
        vec![SimCommand::SetFactoryRepeat {
            units: units(&[2]),
            def: Some(UnitDefId(20)),
        }]
    );
    assert_eq!(
        sim(m.perform(Action::CancelBuildOption { index: 0 }, &w, Tick(1))),
        vec![SimCommand::CancelProduction {
            units: units(&[2]),
            def: UnitDefId(20),
            count: 1,
        }]
    );
}

#[test]
fn idle_builder_cycle_skips_unfinished_workers_even_if_adapter_calls_them_idle() {
    let mut w = world();
    w.spec.units[0].under_construction = true;
    for u in &mut w.spec.units {
        u.idle = true;
    }
    let mut m = Mapper::new(Profile::default_combined());
    for id in [2, 3, 8, 2] {
        m.perform(Action::CycleIdleBuilder, &w, Tick(1));
        assert_eq!(m.selection(), &units(&[id]));
    }
    w.spec.units[0].under_construction = false;
    let mut m = Mapper::new(Profile::default_combined());
    m.perform(Action::CycleIdleBuilder, &w, Tick(1));
    assert_eq!(m.selection(), &units(&[1]));
}

#[test]
fn factory_clicks_append_one_and_shift_or_rb_append_five() {
    let w = world();
    let mut m = mapper(&w, &[1, 2]);
    let expected = SimCommand::Build {
        units: units(&[1, 2]),
        def: UnitDefId(20),
        at: None,
        queue: QueueMode::Append,
    };
    assert_eq!(
        sim(m.perform(Action::BuildOption { index: 0 }, &w, Tick(1))),
        vec![expected.clone()]
    );
    for input in [
        EventKind::KeyDown {
            key: Key::new("shift"),
        },
        EventKind::ButtonDown { button: Button::Rb },
    ] {
        event(&mut m, &w, input.clone());
        assert_eq!(m.production_batch_count(), 5);
        assert_eq!(
            sim(m.perform(Action::BuildOption { index: 0 }, &w, Tick(1))),
            vec![expected.clone(); 5]
        );
        event(
            &mut m,
            &w,
            match input {
                EventKind::KeyDown { key } => EventKind::KeyUp { key },
                _ => EventKind::ButtonUp { button: Button::Rb },
            },
        );
        assert_eq!(m.production_batch_count(), 1);
    }
}

#[test]
fn cancellation_targets_only_eligible_matching_factories_and_batches() {
    let w = world();
    let mut m = mapper(&w, &[1, 2, 3, 5, 8]);
    for count in [1, 5] {
        if count == 5 {
            key(&mut m, &w, "shift", true);
        }
        assert_eq!(
            sim(m.perform(Action::CancelBuildOption { index: 0 }, &w, Tick(1))),
            vec![SimCommand::CancelProduction {
                units: units(&[1, 2]),
                def: UnitDefId(20),
                count
            }]
        );
    }
    assert!(sim(m.perform(Action::CancelBuildOption { index: 255 }, &w, Tick(1))).is_empty());
    assert_eq!(m.mode(), Mode::None);
}

#[test]
fn mobile_placement_keeps_priority_in_mixed_selection() {
    let w = world();
    let mut m = mapper(&w, &[1, 2, 3]);
    key(&mut m, &w, "shift", true);
    assert!(sim(m.perform(Action::BuildOption { index: 1 }, &w, Tick(1))).is_empty());
    assert_eq!(m.mode(), Mode::Build(UnitDefId(21)));
    assert_eq!(m.factory_production_count(1, &w), None);
    assert!(sim(m.perform(Action::CancelBuildOption { index: 1 }, &w, Tick(1))).is_empty());
    event(&mut m, &w, EventKind::PointerMove { x: 900, y: 600 });
    assert_eq!(
        sim(m.perform(Action::Context { append: false }, &w, Tick(1))),
        vec![SimCommand::Build {
            units: units(&[3]),
            def: UnitDefId(21),
            at: Some(pos(900, 600)),
            queue: QueueMode::Append
        }]
    );
}

#[test]
fn ground_and_explicit_move_patrol_split_factories_from_mobile_orders() {
    let w = world();
    let mut m = mapper(&w, &[1, 2, 3, 5, 7]);
    for mode in [None, Some(Action::MoveMode), Some(Action::PatrolMode)] {
        for append in [false, true] {
            if let Some(mode) = mode {
                m.perform(mode, &w, Tick(1));
            }
            event(&mut m, &w, EventKind::PointerMove { x: 900, y: 600 });
            let queue = if append {
                QueueMode::Append
            } else {
                QueueMode::Replace
            };
            let patrol = mode == Some(Action::PatrolMode);
            let ordinary = if patrol {
                SimCommand::Patrol {
                    units: units(&[3, 7]),
                    to: pos(900, 600),
                    queue,
                }
            } else {
                SimCommand::Move {
                    units: units(&[3, 7]),
                    to: pos(900, 600),
                    queue,
                }
            };
            let order = if patrol {
                FactoryRallyOrder::Patrol { to: pos(900, 600) }
            } else {
                FactoryRallyOrder::Move { to: pos(900, 600) }
            };
            assert_eq!(
                sim(m.perform(Action::Context { append }, &w, Tick(1))),
                vec![
                    ordinary,
                    SimCommand::SetFactoryRally {
                        units: units(&[1, 2]),
                        order: Some(order),
                        queue
                    }
                ]
            );
        }
    }
}

#[test]
fn unsupported_factory_orders_do_not_cancel_production_but_mobile_orders_survive() {
    let w = world();
    let mut m = mapper(&w, &[1, 3, 7]);
    m.take_feedback();
    for action in [Action::AttackMode, Action::AttackMoveMode] {
        m.perform(action, &w, Tick(1));
        event(&mut m, &w, EventKind::PointerMove { x: 400, y: 100 });
        let commands = sim(m.perform(Action::Context { append: false }, &w, Tick(1)));
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].units(), &units(&[3, 7]));
        assert!(m.take_feedback().contains(&Feedback::Refused {
            reason: Refusal::CannotDo
        }));
    }
    select(&mut m, &w, &[1]);
    event(&mut m, &w, EventKind::PointerMove { x: 400, y: 100 });
    assert!(sim(m.perform(Action::Context { append: false }, &w, Tick(1))).is_empty());
    assert_eq!(
        sim(m.perform(Action::Stop, &w, Tick(1))),
        vec![SimCommand::Stop { units: units(&[1]) }]
    );
}

#[test]
fn target_cursor_matches_factory_dispatch_on_terrain_and_minimap() {
    let w = world();
    for minimap in [false, true] {
        for selected in [&[1][..], &[7][..], &[1, 7][..]] {
            for (target, kind) in [(3, CursorKind::Guard), (4, CursorKind::Attack)] {
                let mut m = mapper(&w, selected);
                let to = w.unit(uid(target)).unwrap().pos;
                let pointer = if minimap {
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
                };
                event(&mut m, &w, pointer);
                let has_mobile = selected.contains(&7);
                assert_eq!(
                    m.cursor_kind(&w),
                    if has_mobile || kind == CursorKind::Guard {
                        kind
                    } else {
                        CursorKind::Invalid
                    },
                    "minimap={minimap}, selected={selected:?}, target={target}"
                );
                let commands = sim(m.perform(Action::Context { append: false }, &w, Tick(1)));
                let mut expected = Vec::new();
                if has_mobile {
                    expected.push(if kind == CursorKind::Guard {
                        SimCommand::Guard {
                            units: units(&[7]),
                            target: uid(target),
                            queue: QueueMode::Replace,
                        }
                    } else {
                        SimCommand::Attack {
                            units: units(&[7]),
                            target: tak_commands::Target::Unit(uid(target)),
                            queue: QueueMode::Replace,
                        }
                    });
                }
                if selected.contains(&1) && kind == CursorKind::Guard {
                    expected.push(SimCommand::SetFactoryRally {
                        units: units(&[1]),
                        order: Some(FactoryRallyOrder::Guard {
                            target: uid(target),
                        }),
                        queue: QueueMode::Replace,
                    });
                }
                assert_eq!(commands, expected);
            }
        }
    }
}

#[test]
fn explicit_target_cursors_allow_guard_not_attack_and_ground_rally_stays_available() {
    let w = world();
    for minimap in [false, true] {
        for selected in [&[1][..], &[7][..], &[1, 7][..]] {
            for (action, x, kind) in [
                (Action::GuardMode, 300, CursorKind::Guard),
                (Action::AttackMode, 400, CursorKind::Attack),
                (Action::AttackMoveMode, 900, CursorKind::Attack),
            ] {
                let mut m = mapper(&w, selected);
                m.perform(action, &w, Tick(1));
                event(
                    &mut m,
                    &w,
                    if minimap {
                        EventKind::MapPointer {
                            x: 1200,
                            y: 50,
                            world: pos(x, 100),
                        }
                    } else {
                        EventKind::PointerMove { x, y: 100 }
                    },
                );
                assert_eq!(
                    m.cursor_kind(&w),
                    if selected.contains(&7) || kind == CursorKind::Guard {
                        kind
                    } else {
                        CursorKind::Invalid
                    }
                );
            }
        }
        for mode in [None, Some(Action::MoveMode), Some(Action::PatrolMode)] {
            let mut m = mapper(&w, &[1]);
            if let Some(action) = mode {
                m.perform(action, &w, Tick(1));
            }
            let to = pos(900, 600);
            event(
                &mut m,
                &w,
                if minimap {
                    EventKind::MapPointer {
                        x: 1200,
                        y: 50,
                        world: to,
                    }
                } else {
                    EventKind::PointerMove { x: 900, y: 600 }
                },
            );
            let patrol = mode == Some(Action::PatrolMode);
            assert_eq!(
                m.cursor_kind(&w),
                if patrol {
                    CursorKind::Patrol
                } else {
                    CursorKind::Move
                }
            );
            assert_eq!(
                sim(m.perform(Action::Context { append: false }, &w, Tick(1))),
                vec![SimCommand::SetFactoryRally {
                    units: units(&[1]),
                    order: Some(if patrol {
                        FactoryRallyOrder::Patrol { to }
                    } else {
                        FactoryRallyOrder::Move { to }
                    }),
                    queue: QueueMode::Replace,
                }]
            );
        }
    }
}

#[test]
fn original_mouse_still_selects_friendlies_with_factory_only_selection() {
    let w = world();
    let mut m = Mapper::new(Profile::original_tak());
    select(&mut m, &w, &[1]);
    event(&mut m, &w, EventKind::PointerMove { x: 300, y: 100 });
    assert_eq!(m.cursor_kind(&w), CursorKind::Select);
    event(&mut m, &w, EventKind::PointerMove { x: 400, y: 100 });
    assert_eq!(m.cursor_kind(&w), CursorKind::Invalid);
}

fn open_build(m: &mut Mapper, w: &impl WorldView) {
    button(m, w, Button::Lb, true);
    m.handle(
        &InputEvent::new(m.now() + m.tuning().hold_ms + 1, EventKind::Frame),
        w,
        Tick(1),
    );
    event(
        m,
        w,
        EventKind::Axis {
            axis: Axis::LeftY,
            value: 32767,
        },
    );
    event(
        m,
        w,
        EventKind::Axis {
            axis: Axis::LeftY,
            value: 0,
        },
    );
    assert_eq!(m.wheel().unwrap().highlighted, Some(0));
}

#[test]
fn keyboard_stop_consumes_build_wheel_release_for_factories_and_mobile_builders() {
    let w = world();
    for id in [1, 3] {
        for sticky in [false, true] {
            let mut m = mapper(&w, &[id]);
            let mut profile = m.profile().clone();
            profile.tuning.wheel_confirm_on_press = sticky;
            profile.bindings.push(tak_input::Binding {
                input: tak_input::Input::Button(Button::Lb),
                with: None,
                with_all: Vec::new(),
                gesture: tak_input::Gesture::Release,
                action: Action::BuildOption { index: 0 },
            });
            profile.validate().unwrap();
            m.set_profile(profile);
            open_build(&mut m, &w);
            assert_eq!(
                sim(key(&mut m, &w, "s", true)),
                vec![SimCommand::Stop {
                    units: units(&[id])
                }]
            );
            assert!(m.wheel().is_none());
            assert!(sim(key(&mut m, &w, "s", false)).is_empty());
            assert!(sim(button(&mut m, &w, Button::Lb, false)).is_empty());
            assert_eq!(m.mode(), Mode::None);
            assert_eq!(m.selection(), &units(&[id]));
        }
    }
}

#[test]
fn gamepad_x_removes_from_factory_wheel_and_opener_release_does_not_readd() {
    let w = world();
    for batch in [false, true] {
        let mut m = mapper(&w, &[1]);
        if batch {
            button(&mut m, &w, Button::Rb, true);
        }
        open_build(&mut m, &w);
        button(&mut m, &w, Button::X, true);
        assert_eq!(
            sim(button(&mut m, &w, Button::X, false)),
            vec![SimCommand::CancelProduction {
                units: units(&[1]),
                def: UnitDefId(20),
                count: if batch { 5 } else { 1 }
            }]
        );
        assert!(m.wheel().is_none());
        assert!(sim(button(&mut m, &w, Button::Lb, false)).is_empty());
    }
}

#[test]
fn factory_wheel_a_adds_and_mobile_wheel_x_still_places() {
    let w = world();
    let mut m = mapper(&w, &[1]);
    open_build(&mut m, &w);
    button(&mut m, &w, Button::A, true);
    assert_eq!(
        sim(button(&mut m, &w, Button::A, false)),
        vec![SimCommand::Build {
            units: units(&[1]),
            def: UnitDefId(20),
            at: None,
            queue: QueueMode::Append
        }]
    );
    assert!(sim(button(&mut m, &w, Button::Lb, false)).is_empty());
    select(&mut m, &w, &[3]);
    let mut tuning = *m.tuning();
    tuning.wheel_confirm_on_press = true;
    m.set_tuning(tuning);
    open_build(&mut m, &w);
    button(&mut m, &w, Button::X, true);
    assert!(sim(button(&mut m, &w, Button::X, false)).is_empty());
    assert_eq!(m.mode(), Mode::Build(UnitDefId(21)));
}

#[test]
fn multipage_rb_pages_without_batching_but_preheld_rb_batches() {
    let mut w = world();
    w.spec.units[0].build_options = (20..30).collect();
    let mut m = mapper(&w, &[1]);
    open_build(&mut m, &w);
    button(&mut m, &w, Button::Rb, true);
    assert_eq!(m.wheel().unwrap().page, 1);
    assert_eq!(m.production_batch_count(), 1);
    button(&mut m, &w, Button::Rb, false);
    event(
        &mut m,
        &w,
        EventKind::Axis {
            axis: Axis::LeftY,
            value: 32767,
        },
    );
    button(&mut m, &w, Button::A, true);
    assert_eq!(sim(button(&mut m, &w, Button::A, false)).len(), 1);
    button(&mut m, &w, Button::Lb, false);
    button(&mut m, &w, Button::Rb, true);
    open_build(&mut m, &w);
    button(&mut m, &w, Button::A, true);
    assert_eq!(sim(button(&mut m, &w, Button::A, false)).len(), 5);
}

struct Counts(TestWorld);
impl WorldView for Counts {
    fn local_player(&self) -> tak_commands::PlayerId {
        self.0.local_player()
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&tak_input::UnitInfo)) {
        self.0.for_each_unit(f)
    }
    fn is_enemy(&self, p: tak_commands::PlayerId) -> bool {
        self.0.is_enemy(p)
    }
    fn build_options(&self, id: UnitId) -> Vec<UnitDefId> {
        self.0.build_options(id)
    }
    fn production_count(&self, id: UnitId, def: UnitDefId) -> u32 {
        if def == UnitDefId(20) {
            id.index + 2
        } else {
            0
        }
    }
}
#[test]
fn hud_count_matches_exact_factory_dispatch_set() {
    let w = Counts(world());
    let mut m = mapper(&w, &[1, 2, 3, 5, 8]);
    assert_eq!(m.factory_production_count(0, &w), Some(7));
    assert_eq!(m.factory_production_count(1, &w), None);
    m.perform(Action::SelectAll { scope: Scope::Map }, &w, Tick(1));
    assert_eq!(m.factory_production_count(0, &w), Some(7));
}

#[test]
fn shift_inside_multipage_wheel_batches_without_turning_page() {
    let mut w = world();
    w.spec.units[0].build_options = (20..30).collect();
    let mut m = mapper(&w, &[1]);
    open_build(&mut m, &w);
    key(&mut m, &w, "shift", true);
    assert_eq!(m.wheel().unwrap().page, 0);
    assert_eq!(m.production_batch_count(), 5);
    button(&mut m, &w, Button::X, true);
    assert_eq!(
        sim(button(&mut m, &w, Button::X, false)),
        vec![SimCommand::CancelProduction {
            units: units(&[1]),
            def: UnitDefId(20),
            count: 5
        }]
    );
}

#[test]
fn hud_production_actions_consume_rb_tap_without_changing_selection_on_release() {
    let w = world();
    for action in [
        Action::BuildOption { index: 0 },
        Action::CancelBuildOption { index: 0 },
    ] {
        let mut m = mapper(&w, &[1]);
        button(&mut m, &w, Button::Rb, true);
        let commands = sim(m.perform(action, &w, Tick(1)));
        match action {
            Action::BuildOption { .. } => assert_eq!(commands.len(), 5),
            _ => assert_eq!(
                commands,
                vec![SimCommand::CancelProduction {
                    units: units(&[1]),
                    def: UnitDefId(20),
                    count: 5
                }]
            ),
        }
        assert_eq!(m.selection(), &units(&[1]));
        assert!(
            button(&mut m, &w, Button::Rb, false).is_empty(),
            "HUD action must consume RB's select-all tap"
        );
        assert_eq!(m.selection(), &units(&[1]));
    }
}

#[test]
fn repeat_enables_exact_factory_dispatch_set_once_despite_batch_modifiers() {
    let w = world();
    let mut m = mapper(&w, &[1, 2, 3, 5, 8]);
    key(&mut m, &w, "ctrl", true);
    key(&mut m, &w, "shift", true);
    button(&mut m, &w, Button::Rb, true);
    assert!(m.ctrl_held());
    assert_eq!(m.production_batch_count(), 5);
    assert_eq!(
        sim(m.perform(Action::RepeatBuildOption { index: 0 }, &w, Tick(1))),
        vec![SimCommand::SetFactoryRepeat {
            units: units(&[1, 2]),
            def: Some(UnitDefId(20)),
        }]
    );
    assert_eq!(m.mode(), Mode::None);
    assert!(sim(m.perform(Action::RepeatBuildOption { index: 255 }, &w, Tick(1))).is_empty());
    assert!(button(&mut m, &w, Button::Rb, false).is_empty());
    assert_eq!(m.selection(), &units(&[1, 2, 3, 5, 8]));
    key(&mut m, &w, "ctrl", false);
    assert!(!m.ctrl_held());
}

#[test]
fn repeat_never_replaces_mobile_placement_in_mixed_selection() {
    let w = world();
    let mut m = mapper(&w, &[1, 2, 3]);
    assert_eq!(m.factory_repeat_counts(1, &w), None);
    assert!(
        m.perform(Action::RepeatBuildOption { index: 1 }, &w, Tick(1))
            .is_empty()
    );
    assert_eq!(m.mode(), Mode::None);
    m.perform(Action::BuildOption { index: 1 }, &w, Tick(1));
    assert_eq!(m.mode(), Mode::Build(UnitDefId(21)));
    assert!(
        m.perform(Action::RepeatBuildOption { index: 1 }, &w, Tick(1))
            .is_empty()
    );
    assert_eq!(m.mode(), Mode::Build(UnitDefId(21)));
}

struct RepeatState {
    world: TestWorld,
    blocked: Option<UnitId>,
}
impl WorldView for RepeatState {
    fn local_player(&self) -> tak_commands::PlayerId {
        self.world.local_player()
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&tak_input::UnitInfo)) {
        self.world.for_each_unit(f)
    }
    fn is_enemy(&self, p: tak_commands::PlayerId) -> bool {
        self.world.is_enemy(p)
    }
    fn build_options(&self, id: UnitId) -> Vec<UnitDefId> {
        self.world.build_options(id)
    }
    fn can_factory_control(&self, id: UnitId) -> bool {
        // Deliberately permissive: the mapper must still enforce the
        // visible ownership, completion and factory-kind constraints.
        self.blocked != Some(id)
    }
    fn factory_repeat(&self, id: UnitId) -> Option<UnitDefId> {
        match id.index {
            1 | 4 | 5 | 6 | 8 => Some(UnitDefId(20)),
            2 => Some(UnitDefId(21)),
            _ => None,
        }
    }
}

#[test]
fn repeat_hud_counts_distinguish_mixed_recipes_and_ignore_ineligible_factories() {
    let mut w = RepeatState {
        world: world(),
        blocked: None,
    };
    let mut m = mapper(&w, &[1, 2, 3, 5, 8]);
    assert_eq!(m.factory_repeat_counts(0, &w), Some((1, 2)));
    assert_eq!(m.factory_repeat_counts(1, &w), None);
    assert_eq!(m.factory_repeat_counts(255, &w), None);
    select(&mut m, &w, &[1, 2]);
    assert_eq!(m.factory_repeat_counts(1, &w), Some((1, 2)));
    w.blocked = Some(uid(2));
    assert_eq!(m.factory_repeat_counts(0, &w), Some((1, 1)));
    assert_eq!(m.factory_repeat_counts(1, &w), Some((0, 1)));
    assert_eq!(
        sim(m.perform(Action::RepeatBuildOption { index: 0 }, &w, Tick(1))),
        vec![SimCommand::SetFactoryRepeat {
            units: units(&[1]),
            def: Some(UnitDefId(20)),
        }]
    );
    assert_eq!(m.factory_repeat_counts(0, &w.world), Some((0, 2)));
}

#[test]
fn repeat_revalidates_stale_selection_even_with_permissive_adapter() {
    for change in 0..6 {
        let mut w = RepeatState {
            world: world(),
            blocked: None,
        };
        let mut m = mapper(&w, &[1]);
        let unit = &mut w.world.spec.units[0];
        match change {
            0 => unit.owner = 1,
            1 => unit.selectable = false,
            2 => unit.under_construction = true,
            3 => unit.builder = None,
            4 => unit.build_options.clear(),
            _ => w.blocked = Some(uid(1)),
        }
        assert_eq!(m.factory_repeat_counts(0, &w), None);
        assert!(
            m.perform(Action::RepeatBuildOption { index: 0 }, &w, Tick(1))
                .is_empty()
        );
    }
}

#[test]
fn gamepad_y_repeat_consumes_tap_hold_and_opener_release_for_both_wheel_styles() {
    let mut w = world();
    // A caster in the selection makes a leaked Y tap / hold observable.
    w.spec.units[0].spells = vec![90];
    for sticky in [false, true] {
        for hold in [false, true] {
            for release_opener_first in [false, true] {
                for batch in [false, true] {
                    let mut m = mapper(&w, &[1]);
                    let mut tuning = *m.tuning();
                    tuning.wheel_confirm_on_press = sticky;
                    m.set_tuning(tuning);
                    if batch {
                        button(&mut m, &w, Button::Rb, true);
                    }
                    open_build(&mut m, &w);
                    m.take_feedback();
                    assert_eq!(
                        sim(button(&mut m, &w, Button::Y, true)),
                        vec![SimCommand::SetFactoryRepeat {
                            units: units(&[1]),
                            def: Some(UnitDefId(20)),
                        }]
                    );
                    assert!(m.wheel().is_none());
                    assert_eq!(m.mode(), Mode::None);
                    // Repeated physical down edges cannot repeat the command.
                    assert!(button(&mut m, &w, Button::Y, true).is_empty());
                    if release_opener_first {
                        assert!(button(&mut m, &w, Button::Lb, false).is_empty());
                    }
                    if hold {
                        assert!(
                            m.handle(
                                &InputEvent::new(
                                    m.now() + m.tuning().hold_ms + 1,
                                    EventKind::Frame
                                ),
                                &w,
                                Tick(1),
                            )
                            .is_empty()
                        );
                        assert!(m.wheel().is_none(), "Y hold must not open the spell wheel");
                    }
                    assert!(button(&mut m, &w, Button::Y, false).is_empty());
                    if !release_opener_first {
                        assert!(button(&mut m, &w, Button::Lb, false).is_empty());
                    }
                    if batch {
                        assert!(button(&mut m, &w, Button::Rb, false).is_empty());
                    }
                    assert_eq!(m.mode(), Mode::None, "Y release must not enter spell mode");
                    assert_eq!(m.selection(), &units(&[1]));
                    assert_eq!(m.take_feedback(), vec![Feedback::Order]);
                }
            }
        }
    }
}

#[test]
fn factory_repeat_consumes_explicit_release_bindings_for_y_and_wheel_opener() {
    let w = world();
    let mut m = mapper(&w, &[1]);
    let mut profile = m.profile().clone();
    for button in [Button::Y, Button::Lb] {
        profile.bindings.push(tak_input::Binding {
            input: tak_input::Input::Button(button),
            with: None,
            with_all: Vec::new(),
            gesture: tak_input::Gesture::Release,
            action: Action::BuildOption { index: 0 },
        });
    }
    profile.validate().unwrap();
    m.set_profile(profile);
    open_build(&mut m, &w);
    assert_eq!(sim(button(&mut m, &w, Button::Y, true)).len(), 1);
    assert!(button(&mut m, &w, Button::Y, false).is_empty());
    assert!(button(&mut m, &w, Button::Lb, false).is_empty());
}

#[test]
fn cancelled_factory_wheel_does_not_capture_later_y_spell_tap() {
    let mut w = world();
    w.spec.units[0].spells = vec![90];
    let mut m = mapper(&w, &[1]);
    open_build(&mut m, &w);
    button(&mut m, &w, Button::B, true);
    assert!(button(&mut m, &w, Button::B, false).is_empty());
    assert!(m.wheel().is_none());
    assert!(button(&mut m, &w, Button::Lb, false).is_empty());
    assert!(button(&mut m, &w, Button::Y, true).is_empty());
    assert!(button(&mut m, &w, Button::Y, false).is_empty());
    assert_eq!(m.mode(), Mode::CastSpell(tak_commands::SpellId(90)));
}

#[test]
fn mobile_build_wheel_keeps_y_spell_behavior() {
    let mut w = world();
    w.spec.units[2].spells = vec![90];
    let mut m = mapper(&w, &[3]);
    open_build(&mut m, &w);
    assert!(button(&mut m, &w, Button::Y, true).is_empty());
    assert!(button(&mut m, &w, Button::Y, false).is_empty());
    assert_eq!(m.mode(), Mode::CastSpell(tak_commands::SpellId(90)));
    assert!(m.wheel().is_some());
}

#[test]
fn repeat_action_is_rebindable_and_ctrl_grid_category_shortcuts_are_untouched() {
    let mut w = world();
    w.spec.units[1].categories = vec!["MELEE".to_owned()];
    let mut m = mapper(&w, &[1]);
    let mut profile = m.profile().clone();
    profile.grid_build_keys = true;
    profile.bindings.push(
        serde_json::from_str(
            r#"{
        "input":{"key":"f12"},"gesture":"press",
        "action":{"type":"repeat_build_option","index":0}
    }"#,
        )
        .unwrap(),
    );
    let profile = Profile::from_json(&profile.to_json_pretty()).unwrap();
    m.set_profile(profile);
    assert_eq!(
        sim(key(&mut m, &w, "f12", true)),
        vec![SimCommand::SetFactoryRepeat {
            units: units(&[1]),
            def: Some(UnitDefId(20)),
        }]
    );
    key(&mut m, &w, "f12", false);
    key(&mut m, &w, "ctrl", true);
    assert!(m.ctrl_held());
    assert!(sim(key(&mut m, &w, "e", true)).is_empty());
    assert_eq!(m.selection(), &units(&[2]));
    key(&mut m, &w, "e", false);
    key(&mut m, &w, "shift", true);
    assert!(m.ctrl_held());
    assert!(sim(key(&mut m, &w, "e", true)).is_empty());
    assert_eq!(m.selection(), &units(&[2]));
    key(&mut m, &w, "ctrl", false);
    assert!(!m.ctrl_held());
}

#[test]
fn stop_normalizes_observed_factory_work_once_before_legacy_stop() {
    let w = Counts(world());
    let mut m = mapper(&w, &[1, 2, 3, 5, 8]);
    m.take_feedback();
    let out = m.perform(Action::Stop, &w, Tick(9));
    assert!(
        out.iter()
            .all(|o| matches!(o, Output::Sim { tick: Tick(9), .. }))
    );
    assert_eq!(
        sim(out),
        vec![
            SimCommand::CancelProduction {
                units: units(&[1, 2]),
                def: UnitDefId(20),
                count: u32::MAX
            },
            SimCommand::Stop {
                units: units(&[1, 2, 3, 5, 8])
            },
        ]
    );
    assert_eq!(m.take_feedback(), vec![Feedback::Order]);
    assert_eq!(m.selection(), &units(&[1, 2, 3, 5, 8]));
}

#[test]
fn stop_without_observed_factory_work_keeps_single_legacy_command() {
    let w = world();
    let mut m = mapper(&w, &[1, 3, 5, 8]);
    m.take_feedback();
    assert_eq!(
        sim(m.perform(Action::Stop, &w, Tick(9))),
        vec![SimCommand::Stop {
            units: units(&[1, 3, 5, 8])
        },]
    );
    assert_eq!(m.take_feedback(), vec![Feedback::Order]);
}

#[test]
fn stop_groups_multiple_definitions_in_order_and_honours_factory_veto() {
    struct MultiCounts(TestWorld);
    impl WorldView for MultiCounts {
        fn local_player(&self) -> tak_commands::PlayerId {
            self.0.local_player()
        }
        fn for_each_unit(&self, f: &mut dyn FnMut(&tak_input::UnitInfo)) {
            self.0.for_each_unit(f);
        }
        fn is_enemy(&self, p: tak_commands::PlayerId) -> bool {
            self.0.is_enemy(p)
        }
        fn build_options(&self, id: UnitId) -> Vec<UnitDefId> {
            self.0.build_options(id)
        }
        fn can_factory_control(&self, id: UnitId) -> bool {
            id != uid(2) && self.0.can_factory_control(id)
        }
        fn production_count(&self, _id: UnitId, _def: UnitDefId) -> u32 {
            1
        }
    }
    let mut base = world();
    base.spec.units[0].build_options = vec![21, 20, 21];
    base.spec.units[7].build_options = vec![22, 20];
    let w = MultiCounts(base);
    let mut m = mapper(&w, &[1, 2, 3, 5, 8]);
    m.take_feedback();
    assert_eq!(
        sim(m.perform(Action::Stop, &w, Tick(9))),
        vec![
            SimCommand::CancelProduction {
                units: units(&[1, 8]),
                def: UnitDefId(20),
                count: u32::MAX
            },
            SimCommand::CancelProduction {
                units: units(&[1]),
                def: UnitDefId(21),
                count: u32::MAX
            },
            SimCommand::CancelProduction {
                units: units(&[8]),
                def: UnitDefId(22),
                count: u32::MAX
            },
            SimCommand::Stop {
                units: units(&[1, 2, 3, 5, 8])
            },
        ]
    );
    assert_eq!(m.take_feedback(), vec![Feedback::Order]);
}
