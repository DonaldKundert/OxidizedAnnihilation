//! Explicit Attack rectangles are bounded mouse gestures, not selection brushes.

mod common;

use common::{TestWorld, UnitSpec, pos, uid};
use tak_commands::{
    ClientCommand, PlayerId, QueueMode, Scope, SimCommand, Target, Tick, UnitId, UnitSet,
};
use tak_input::{
    Action, Axis, Binding, Button, EventKind, Gesture, Input, InputEvent, Key,
    MAX_AREA_ATTACK_TARGETS, Mapper, Mode, Output, Profile, UnitInfo, WorldView,
};

struct World {
    inner: TestWorld,
    missing: Vec<UnitId>,
    wrong_generation: Option<UnitId>,
    duplicate_snapshots: bool,
}

impl World {
    fn new() -> Self {
        Self {
            inner: TestWorld::new(serde_json::from_str(r#"{
                "local_player":0,"enemies":[1],"units":[
                    {"index":1,"owner":0,"def":10,"x":100,"y":100},
                    {"index":2,"owner":0,"def":11,"x":200,"y":100,"builder":"factory","build_options":[20]},
                    {"index":3,"owner":0,"def":12,"x":300,"y":100,"builder":"mobile","build_options":[20]},
                    {"index":10,"generation":2,"owner":1,"def":20,"x":700,"y":220},
                    {"index":10,"generation":1,"owner":1,"def":20,"x":500,"y":200},
                    {"index":12,"owner":1,"def":20,"x":600,"y":210},
                    {"index":13,"owner":0,"def":20,"x":600,"y":210},
                    {"index":14,"owner":2,"def":20,"x":600,"y":210},
                    {"index":15,"owner":1,"def":20,"x":600,"y":210,"selectable":false},
                    {"index":16,"owner":1,"def":20,"x":600,"y":230},
                    {"index":17,"owner":1,"def":20,"x":499,"y":210},
                    {"index":18,"owner":1,"def":20,"x":701,"y":210}
                ]}"#).unwrap()),
            missing: Vec::new(),
            wrong_generation: None,
            duplicate_snapshots: false,
        }
    }

    fn spec_mut(&mut self, id: UnitId) -> &mut UnitSpec {
        self.inner
            .spec
            .units
            .iter_mut()
            .find(|u| u.id() == id)
            .unwrap()
    }
}

impl WorldView for World {
    fn local_player(&self) -> PlayerId {
        self.inner.local_player()
    }
    fn is_enemy(&self, owner: PlayerId) -> bool {
        self.inner.is_enemy(owner)
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo)) {
        self.inner.for_each_unit(f);
        if self.duplicate_snapshots {
            self.inner.for_each_unit(f);
        }
    }
    fn unit(&self, id: UnitId) -> Option<UnitInfo> {
        if self.missing.contains(&id) {
            return None;
        }
        self.inner.unit(id).map(|mut u| {
            if self.wrong_generation == Some(id) {
                u.id.generation += 1;
            }
            u
        })
    }
    fn build_options(&self, id: UnitId) -> Vec<tak_commands::UnitDefId> {
        self.inner.build_options(id)
    }
}

fn event(m: &mut Mapper, w: &impl WorldView, kind: EventKind) -> Vec<Output> {
    m.handle(&InputEvent::new(m.now() + 1, kind), w, Tick(7))
}
fn pointer(m: &mut Mapper, w: &impl WorldView, x: i32, y: i32) -> Vec<Output> {
    event(m, w, EventKind::PointerMove { x, y })
}
fn mouse(m: &mut Mapper, w: &impl WorldView, down: bool) -> Vec<Output> {
    event(
        m,
        w,
        if down {
            EventKind::ButtonDown {
                button: Button::MouseLeft,
            }
        } else {
            EventKind::ButtonUp {
                button: Button::MouseLeft,
            }
        },
    )
}
fn shift(m: &mut Mapper, w: &impl WorldView, down: bool) -> Vec<Output> {
    event(
        m,
        w,
        if down {
            EventKind::KeyDown {
                key: Key::new("shift"),
            }
        } else {
            EventKind::KeyUp {
                key: Key::new("shift"),
            }
        },
    )
}
fn frame(m: &mut Mapper, w: &impl WorldView) -> Vec<Output> {
    event(m, w, EventKind::Frame)
}
fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&id| uid(id)).collect()
}
fn target_ids() -> Vec<UnitId> {
    vec![UnitId::new(10, 1), UnitId::new(10, 2), uid(12)]
}
fn sims(out: Vec<Output>) -> Vec<SimCommand> {
    out.into_iter()
        .filter_map(|o| match o {
            Output::Sim { tick, command } => {
                assert_eq!(tick, Tick(7));
                Some(command)
            }
            _ => None,
        })
        .collect()
}
fn orders(out: Vec<Output>) -> Vec<(UnitSet, UnitId, QueueMode)> {
    sims(out)
        .into_iter()
        .map(|cmd| match cmd {
            SimCommand::Attack {
                units,
                target: Target::Unit(target),
                queue,
            } => (units, target, queue),
            other => panic!("unexpected command: {other:?}"),
        })
        .collect()
}
fn mapper(w: &impl WorldView, profile: Profile, ids: &[u32]) -> Mapper {
    let mut m = Mapper::new(profile);
    shift(&mut m, w, true);
    for &id in ids {
        let p = w.unit(uid(id)).unwrap().pos;
        pointer(&mut m, w, p.x.to_int_floor(), p.y.to_int_floor());
        m.perform(Action::Select, w, Tick(7));
    }
    shift(&mut m, w, false);
    assert_eq!(m.selection(), &units(ids));
    m.perform(Action::AttackMode, w, Tick(7));
    assert_eq!(m.mode(), Mode::Attack);
    m
}
fn start(m: &mut Mapper, w: &impl WorldView) {
    assert!(pointer(m, w, 500, 200).is_empty());
    assert!(mouse(m, w, true).is_empty());
    assert_eq!(m.attack_area_bounds(), None);
    assert!(pointer(m, w, 700, 220).is_empty());
    assert_eq!(m.attack_area_bounds(), Some((pos(500, 200), pos(700, 220))));
}
fn release(m: &mut Mapper, w: &impl WorldView) {
    assert!(mouse(m, w, false).is_empty());
    assert_eq!(m.attack_area_bounds(), None);
}

#[test]
fn classic_and_original_rectangle_preserve_selection_and_commit_only_at_frame() {
    let w = World::new();
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let mut m = mapper(&w, profile, &[1]);
        start(&mut m, &w);
        assert!(
            frame(&mut m, &w).is_empty(),
            "holding never issues an attack"
        );
        release(&mut m, &w);
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), Mode::Attack);
        let got = orders(frame(&mut m, &w));
        assert_eq!(
            got,
            target_ids()
                .into_iter()
                .enumerate()
                .map(|(i, target)| {
                    (
                        units(&[1]),
                        target,
                        if i == 0 {
                            QueueMode::Replace
                        } else {
                            QueueMode::Append
                        },
                    )
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), Mode::None);
        assert!(frame(&mut m, &w).is_empty());
    }
}

#[test]
fn reverse_rectangle_uses_inclusive_world_bounds_and_shift_append() {
    let w = World::new();
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let mut m = mapper(&w, profile, &[1]);
        shift(&mut m, &w, true);
        pointer(&mut m, &w, 700, 220);
        mouse(&mut m, &w, true);
        pointer(&mut m, &w, 500, 200);
        assert_eq!(m.attack_area_bounds(), Some((pos(500, 200), pos(700, 220))));
        release(&mut m, &w);
        // Queue intent belongs to release, even if Shift goes up later in this poll.
        shift(&mut m, &w, false);
        assert_eq!(
            orders(frame(&mut m, &w)),
            target_ids()
                .into_iter()
                .map(|id| { (units(&[1]), id, QueueMode::Append) })
                .collect::<Vec<_>>()
        );
        assert_eq!(m.mode(), Mode::Attack);
    }
}

#[test]
fn current_release_queue_modifier_decides_first_order() {
    let w = World::new();
    for held_at_release in [false, true] {
        let mut m = mapper(&w, Profile::classic_tak(), &[1]);
        shift(&mut m, &w, !held_at_release);
        start(&mut m, &w);
        shift(&mut m, &w, held_at_release);
        release(&mut m, &w);
        let got = orders(frame(&mut m, &w));
        assert_eq!(
            got[0].2,
            if held_at_release {
                QueueMode::Append
            } else {
                QueueMode::Replace
            }
        );
        assert!(got[1..].iter().all(|x| x.2 == QueueMode::Append));
    }
}

#[test]
fn rectangle_is_world_space_and_not_legacy_circle_or_pick_radius() {
    let w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    m.set_camera(tak_input::Camera {
        origin: pos(400, 100),
        world_per_px: tak_math::Fx::from_int(2),
        ..tak_input::Camera::default()
    });
    pointer(&mut m, &w, 50, 50);
    mouse(&mut m, &w, true);
    pointer(&mut m, &w, 150, 60);
    assert_eq!(m.attack_area_bounds(), Some((pos(500, 200), pos(700, 220))));
    release(&mut m, &w);
    assert_eq!(
        orders(frame(&mut m, &w))
            .into_iter()
            .map(|x| x.1)
            .collect::<Vec<_>>(),
        target_ids()
    );
}

#[test]
fn bounded_targets_are_distinct_and_full_id_sorted_independent_of_iteration() {
    let mut w = World::new();
    let template = w
        .inner
        .spec
        .units
        .iter()
        .find(|u| u.id() == uid(12))
        .unwrap()
        .clone();
    w.inner.spec.units.retain(|u| u.owner == 0);
    for index in (100..200).rev() {
        let mut u = template.clone();
        u.index = index;
        w.inner.spec.units.push(u);
    }
    w.duplicate_snapshots = true;
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    start(&mut m, &w);
    release(&mut m, &w);
    let first = orders(frame(&mut m, &w));
    assert_eq!(first.len(), MAX_AREA_ATTACK_TARGETS);
    assert_eq!(
        first.iter().map(|x| x.1).collect::<Vec<_>>(),
        (100..164).map(uid).collect::<Vec<_>>()
    );
    w.inner.spec.units.reverse();
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    start(&mut m, &w);
    release(&mut m, &w);
    assert_eq!(orders(frame(&mut m, &w)), first);
}

#[test]
fn release_resolves_stale_and_wrong_generation_snapshots() {
    let mut w = World::new();
    w.missing.push(UnitId::new(10, 1));
    w.wrong_generation = Some(UnitId::new(10, 2));
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    start(&mut m, &w);
    release(&mut m, &w);
    assert_eq!(
        orders(frame(&mut m, &w)),
        vec![(units(&[1]), uid(12), QueueMode::Replace)]
    );
}

#[test]
fn frame_revalidates_visibility_hostility_generation_position_and_release_membership() {
    for change in 0..6 {
        let mut w = World::new();
        let mut m = mapper(&w, Profile::classic_tak(), &[1]);
        start(&mut m, &w);
        release(&mut m, &w);
        for id in target_ids() {
            match change {
                0 => w.spec_mut(id).selectable = false,
                1 => w.spec_mut(id).owner = 2,
                2 => w.spec_mut(id).generation += 5,
                3 => w.spec_mut(id).y = 400,
                4 => w.missing.push(id),
                5 => w.inner.spec.units.retain(|u| u.id() != id),
                _ => unreachable!(),
            }
        }
        // Newly visible enemy in the box cannot join a release snapshot.
        w.spec_mut(uid(15)).selectable = true;
        assert!(frame(&mut m, &w).is_empty(), "mutation {change}");
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), Mode::Attack);
    }
}

#[test]
fn surviving_revalidated_target_gets_replace_even_if_first_disappeared() {
    let mut w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    start(&mut m, &w);
    release(&mut m, &w);
    w.spec_mut(UnitId::new(10, 1)).selectable = false;
    let got = orders(frame(&mut m, &w));
    assert_eq!(
        got[0],
        (units(&[1]), UnitId::new(10, 2), QueueMode::Replace)
    );
    assert_eq!(got[1], (units(&[1]), uid(12), QueueMode::Append));
}

#[test]
fn empty_area_preserves_army_and_mode_without_ground_attack() {
    let w = World::new();
    let mut m = mapper(&w, Profile::original_tak(), &[1]);
    pointer(&mut m, &w, 800, 400);
    mouse(&mut m, &w, true);
    pointer(&mut m, &w, 1000, 600);
    release(&mut m, &w);
    assert!(frame(&mut m, &w).is_empty());
    assert_eq!(m.selection(), &units(&[1]));
    assert_eq!(m.mode(), Mode::Attack);
}

#[test]
fn mixed_factory_and_mobile_excludes_factory_without_changing_selection() {
    let w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1, 2, 3]);
    start(&mut m, &w);
    release(&mut m, &w);
    let got = orders(frame(&mut m, &w));
    assert_eq!(got.len(), 3);
    assert!(got.iter().all(|x| x.0 == units(&[1, 3])));
    assert_eq!(m.selection(), &units(&[1, 2, 3]));
}

#[test]
fn factory_only_and_stale_or_unavailable_attackers_do_not_spend_mode() {
    for change in 0..5 {
        let mut w = World::new();
        let selected = if change == 0 { 2 } else { 1 };
        let mut m = mapper(&w, Profile::classic_tak(), &[selected]);
        start(&mut m, &w);
        release(&mut m, &w);
        match change {
            0 => {}
            1 => w.missing.push(uid(1)),
            2 => w.spec_mut(uid(1)).owner = 1,
            3 => w.spec_mut(uid(1)).selectable = false,
            4 => w.wrong_generation = Some(uid(1)),
            _ => unreachable!(),
        }
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(m.selection(), &units(&[selected]));
        assert_eq!(m.mode(), Mode::Attack);
    }
}

#[test]
fn cancellation_before_or_after_release_never_falls_through_or_resumes() {
    let w = World::new();
    for after_release in [false, true] {
        for cancel in 0..9 {
            let mut m = mapper(&w, Profile::classic_tak(), &[1]);
            start(&mut m, &w);
            if after_release {
                release(&mut m, &w);
            }
            let out = match cancel {
                0 => m.perform(Action::Cancel, &w, Tick(7)),
                1 => event(&mut m, &w, EventKind::PointerLeaveTerrain),
                2 => event(
                    &mut m,
                    &w,
                    EventKind::MapPointer {
                        x: 10,
                        y: 10,
                        world: pos(1000, 1000),
                    },
                ),
                3 => {
                    m.warp_cursor(pos(700, 220));
                    vec![]
                }
                4 => m.perform(Action::MoveMode, &w, Tick(7)),
                5 => m.perform(Action::TogglePause, &w, Tick(7)),
                6 => m.perform(Action::ToggleOverlay, &w, Tick(7)),
                7 => event(
                    &mut m,
                    &w,
                    EventKind::Axis {
                        axis: Axis::LeftX,
                        value: 32767,
                    },
                ),
                8 => {
                    m.set_profile(Profile::classic_tak());
                    vec![]
                }
                _ => unreachable!(),
            };
            assert!(sims(out).is_empty());
            assert_eq!(m.attack_area_bounds(), None);
            pointer(&mut m, &w, 650, 210);
            assert!(mouse(&mut m, &w, false).is_empty());
            assert!(
                frame(&mut m, &w).is_empty(),
                "release {after_release}, cancel {cancel}"
            );
            assert_eq!(m.selection(), &units(&[1]));
        }
    }
}

#[test]
fn cancel_is_layered_and_a_fresh_gesture_after_boundary_is_possible() {
    let w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    start(&mut m, &w);
    m.perform(Action::Cancel, &w, Tick(7));
    assert_eq!(m.mode(), Mode::Attack);
    mouse(&mut m, &w, false);
    start(&mut m, &w);
    release(&mut m, &w);
    assert_eq!(orders(frame(&mut m, &w)).len(), 3);
    m.perform(Action::AttackMode, &w, Tick(7));
    m.perform(Action::Cancel, &w, Tick(7));
    assert_eq!(m.mode(), Mode::None);
    assert_eq!(m.selection(), &units(&[1]));
}

#[test]
fn invalid_origin_cannot_turn_into_area_attack_or_ground_attack_after_return() {
    let w = World::new();
    for map_surface in [false, true] {
        let mut m = mapper(&w, Profile::classic_tak(), &[1]);
        event(
            &mut m,
            &w,
            if map_surface {
                EventKind::MapPointer {
                    x: 500,
                    y: 200,
                    world: pos(500, 200),
                }
            } else {
                EventKind::PointerLeaveTerrain
            },
        );
        mouse(&mut m, &w, true);
        pointer(&mut m, &w, 700, 220);
        assert_eq!(m.attack_area_bounds(), None);
        assert!(mouse(&mut m, &w, false).is_empty());
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
    }
}

#[test]
fn selection_or_new_builder_mode_cancels_pending_area() {
    let w = World::new();
    for after_release in [false, true] {
        let mut m = mapper(&w, Profile::classic_tak(), &[1, 3]);
        start(&mut m, &w);
        if after_release {
            release(&mut m, &w);
        }
        m.perform(Action::BuildOption { index: 0 }, &w, Tick(7));
        assert_eq!(m.mode(), Mode::Build(tak_commands::UnitDefId(20)));
        assert_eq!(m.attack_area_bounds(), None);
        assert!(mouse(&mut m, &w, false).is_empty());
        assert!(frame(&mut m, &w).is_empty());
        let mut m = mapper(&w, Profile::classic_tak(), &[1]);
        start(&mut m, &w);
        if after_release {
            release(&mut m, &w);
        }
        m.perform(Action::SelectAll { scope: Scope::Map }, &w, Tick(7));
        assert_eq!(m.attack_area_bounds(), None);
        assert!(mouse(&mut m, &w, false).is_empty());
        assert!(frame(&mut m, &w).is_empty());
    }
}

#[test]
fn short_click_attacks_remain_immediate_and_do_not_seed_double_selection() {
    let w = World::new();
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let mut m = mapper(&w, profile, &[1]);
        pointer(&mut m, &w, 600, 210);
        mouse(&mut m, &w, true);
        pointer(&mut m, &w, 601, 210);
        assert_eq!(m.attack_area_bounds(), None);
        assert_eq!(
            orders(mouse(&mut m, &w, false)),
            vec![(units(&[1]), uid(12), QueueMode::Replace)]
        );
        assert!(frame(&mut m, &w).is_empty());
        m.perform(Action::AttackMode, &w, Tick(7));
        start(&mut m, &w);
        release(&mut m, &w);
        assert_eq!(orders(frame(&mut m, &w)).len(), 3);
        assert_eq!(m.selection(), &units(&[1]));
    }
}

#[test]
fn rapid_queued_attack_rectangles_are_not_swallowed_by_second_tap() {
    let w = World::new();
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let mut m = mapper(&w, profile, &[1]);
        shift(&mut m, &w, true);
        for _ in 0..3 {
            start(&mut m, &w);
            release(&mut m, &w);
            assert_eq!(orders(frame(&mut m, &w)).len(), 3);
            assert_eq!(m.selection(), &units(&[1]));
            assert_eq!(m.mode(), Mode::Attack);
        }
    }
}

#[test]
fn ordinary_drag_and_controller_drag_keep_the_existing_selection_circle() {
    let w = World::new();
    for mode in [Mode::None, Mode::Attack] {
        let mut profile = Profile::classic_tak();
        let button = if mode == Mode::None {
            Button::MouseLeft
        } else {
            Button::A
        };
        if mode == Mode::Attack {
            profile.bindings.push(Binding {
                input: Input::Button(button),
                with: None,
                with_all: Vec::new(),
                gesture: Gesture::Press,
                action: Action::DragSelect,
            });
        }
        let mut m = mapper(&w, profile, &[1]);
        if mode == Mode::None {
            m.perform(Action::Cancel, &w, Tick(7));
        }
        pointer(&mut m, &w, 500, 200);
        event(&mut m, &w, EventKind::ButtonDown { button });
        pointer(&mut m, &w, 700, 220);
        assert_eq!(m.attack_area_bounds(), None);
        let out = event(&mut m, &w, EventKind::ButtonUp { button });
        let (center, radius) = tak_input::drag_circle(pos(500, 200), pos(700, 220));
        assert_eq!(
            out,
            vec![Output::Client {
                command: ClientCommand::SelectBrush {
                    center,
                    radius,
                    additive: false,
                }
            }]
        );
        assert_eq!(m.selection(), &units(&[13]));
    }
}

#[test]
fn minimap_attack_short_click_keeps_exact_point_target_without_area_gesture() {
    let w = World::new();
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let mut m = mapper(&w, profile, &[1]);
        event(
            &mut m,
            &w,
            EventKind::MapPointer {
                x: 10,
                y: 10,
                world: pos(12000, 9000),
            },
        );
        mouse(&mut m, &w, true);
        // Surface coordinates may update every poll even when almost stationary.
        event(
            &mut m,
            &w,
            EventKind::MapPointer {
                x: 11,
                y: 10,
                world: pos(12100, 9000),
            },
        );
        assert_eq!(m.attack_area_bounds(), None);
        assert_eq!(
            sims(mouse(&mut m, &w, false)),
            vec![SimCommand::Attack {
                units: units(&[1]),
                target: Target::Ground(pos(12100, 9000)),
                queue: QueueMode::Replace,
            }]
        );
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
    }
}

#[test]
fn minimap_origin_drag_is_consumed_and_cannot_fall_through_to_point_attack() {
    let w = World::new();
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let mut m = mapper(&w, profile, &[1]);
        event(
            &mut m,
            &w,
            EventKind::MapPointer {
                x: 10,
                y: 10,
                world: pos(500, 200),
            },
        );
        mouse(&mut m, &w, true);
        event(
            &mut m,
            &w,
            EventKind::MapPointer {
                x: 50,
                y: 50,
                world: pos(700, 220),
            },
        );
        assert_eq!(m.attack_area_bounds(), None);
        assert!(mouse(&mut m, &w, false).is_empty());
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), Mode::Attack);
    }
}

#[test]
fn explicit_map_pointer_leave_cancels_a_short_map_attack_click() {
    let w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    event(
        &mut m,
        &w,
        EventKind::MapPointer {
            x: 10,
            y: 10,
            world: pos(500, 200),
        },
    );
    mouse(&mut m, &w, true);
    event(&mut m, &w, EventKind::PointerLeaveTerrain);
    assert!(mouse(&mut m, &w, false).is_empty());
    assert!(frame(&mut m, &w).is_empty());
    assert_eq!(m.selection(), &units(&[1]));
    assert_eq!(m.mode(), Mode::Attack);
}

#[test]
fn stale_low_id_snapshots_cannot_crowd_live_targets_out_of_cap() {
    let mut w = World::new();
    let template = w
        .inner
        .spec
        .units
        .iter()
        .find(|u| u.id() == uid(12))
        .unwrap()
        .clone();
    w.inner.spec.units.retain(|u| u.owner == 0);
    for index in 100..228 {
        let mut u = template.clone();
        u.index = index;
        w.inner.spec.units.push(u);
        if index < 164 {
            w.missing.push(uid(index));
        }
    }
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    start(&mut m, &w);
    release(&mut m, &w);
    let got = orders(frame(&mut m, &w));
    assert_eq!(got.len(), MAX_AREA_ATTACK_TARGETS);
    assert_eq!(
        got.iter().map(|x| x.1).collect::<Vec<_>>(),
        (164..228).map(uid).collect::<Vec<_>>()
    );
}

#[test]
fn duplicate_release_does_not_resample_targets_bounds_or_queue_intent() {
    let mut w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    start(&mut m, &w);
    release(&mut m, &w);
    w.spec_mut(uid(15)).selectable = true;
    shift(&mut m, &w, true);
    pointer(&mut m, &w, 600, 210);
    assert!(mouse(&mut m, &w, false).is_empty());
    let got = orders(frame(&mut m, &w));
    assert_eq!(got.iter().map(|x| x.1).collect::<Vec<_>>(), target_ids());
    assert_eq!(got[0].2, QueueMode::Replace);
    assert_eq!(m.mode(), Mode::None);
}

#[test]
fn stale_map_coordinate_after_surface_loss_cannot_arm_a_fresh_attack_click() {
    let w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    event(
        &mut m,
        &w,
        EventKind::MapPointer {
            x: 10,
            y: 10,
            world: pos(500, 200),
        },
    );
    event(&mut m, &w, EventKind::PointerLeaveTerrain);
    assert_eq!(m.map_target(), Some(pos(500, 200)));
    assert!(mouse(&mut m, &w, true).is_empty());
    assert!(mouse(&mut m, &w, false).is_empty());
    assert!(frame(&mut m, &w).is_empty());
    assert_eq!(m.selection(), &units(&[1]));
    assert_eq!(m.mode(), Mode::Attack);
}
