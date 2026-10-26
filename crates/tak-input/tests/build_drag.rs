//! Mouse-only queued construction uses authoritative footprint samples, not
//! pixel spacing. The final Frame commits a poll after modifier/release edges.

mod common;

use common::{TestWorld, WorldSpec, pos, uid};
use tak_commands::{
    PlayerId, QueueMode, Scope, SimCommand, Tick, UnitDefId, UnitId, UnitSet, WorldPos,
};
use tak_input::{
    Action, Binding, BuilderKind, Button, Camera, EventKind, Gesture, Input, InputEvent, Key,
    MAX_BUILD_DRAG_SITES, Mapper, Mode, Output, PlacementCandidate, Profile, UnitInfo, WorldView,
};
use tak_math::Fx;

#[derive(Clone, Copy)]
enum Geometry {
    Normal,
    Unsupported,
    Empty,
    Reversed,
    CenterOutside,
}

struct World {
    inner: TestWorld,
    geometry: Geometry,
    cursor_hint: bool,
    blocked_x: Option<i32>,
    unavailable: Vec<UnitId>,
    width: i32,
}

impl World {
    fn new() -> Self {
        Self {
            inner: TestWorld::new(serde_json::from_str::<WorldSpec>(r#"{
                "local_player":0,"enemies":[1],"units":[
                    {"index":1,"owner":0,"def":10,"x":100,"y":100,"builder":"mobile","build_options":[20,21]},
                    {"index":2,"owner":0,"def":10,"x":200,"y":100,"builder":"mobile","build_options":[20,21]},
                    {"index":3,"owner":0,"def":11,"x":300,"y":100,"builder":"factory","build_options":[20]},
                    {"index":4,"owner":0,"def":12,"x":400,"y":100,"builder":"mobile","build_options":[21]},
                    {"index":5,"owner":0,"def":10,"x":500,"y":100,"builder":"mobile","build_options":[20],"under_construction":true},
                    {"index":6,"owner":0,"def":10,"x":600,"y":100,"builder":"mobile","build_options":[20],"selectable":false},
                    {"index":7,"owner":1,"def":10,"x":700,"y":100,"builder":"mobile","build_options":[20]}
                ]}"#).unwrap()),
            geometry: Geometry::Normal,
            cursor_hint: false,
            blocked_x: None,
            unavailable: Vec::new(),
            width: 20,
        }
    }
}

impl WorldView for World {
    fn local_player(&self) -> PlayerId {
        self.inner.local_player()
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo)) {
        self.inner.for_each_unit(f);
    }
    fn is_enemy(&self, owner: PlayerId) -> bool {
        self.inner.is_enemy(owner)
    }
    fn build_options(&self, unit: UnitId) -> Vec<UnitDefId> {
        self.inner.build_options(unit)
    }
    fn can_build(&self, unit: UnitId) -> bool {
        !self.unavailable.contains(&unit) && self.inner.can_build(unit)
    }
    fn can_place(&self, _: UnitDefId, _: WorldPos) -> bool {
        self.cursor_hint
    }
    fn placement_candidate(&self, def: UnitDefId, point: WorldPos) -> Option<PlacementCandidate> {
        if ![UnitDefId(20), UnitDefId(21)].contains(&def)
            || matches!(self.geometry, Geometry::Unsupported)
        {
            return None;
        }
        let x = point.x.to_int_floor().div_euclid(10) * 10;
        let y = point.y.to_int_floor().div_euclid(10) * 10;
        if self.blocked_x == Some(x) {
            return None;
        }
        let center = pos(x, y);
        let mut site = PlacementCandidate {
            center,
            min: pos(x - self.width / 2, y - 10),
            max: pos(x + self.width / 2, y + 10),
        };
        match self.geometry {
            Geometry::Empty => site.max = site.min,
            Geometry::Reversed => core::mem::swap(&mut site.min, &mut site.max),
            Geometry::CenterOutside => site.center = site.max,
            Geometry::Normal | Geometry::Unsupported => {}
        }
        Some(site)
    }
}

fn units(ids: &[u32]) -> UnitSet {
    ids.iter().map(|&id| uid(id)).collect()
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
fn build(at: WorldPos, ids: &[u32]) -> SimCommand {
    SimCommand::Build {
        units: units(ids),
        def: UnitDefId(20),
        at: Some(at),
        queue: QueueMode::Append,
    }
}
fn event(m: &mut Mapper, w: &impl WorldView, kind: EventKind) -> Vec<Output> {
    m.handle(&InputEvent::new(m.now() + 10, kind), w, Tick(7))
}
fn pointer(m: &mut Mapper, w: &impl WorldView, x: i32, y: i32) -> Vec<Output> {
    event(m, w, EventKind::PointerMove { x, y })
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
fn frame(m: &mut Mapper, w: &impl WorldView) -> Vec<Output> {
    event(m, w, EventKind::Frame)
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
    m.perform(Action::BuildOption { index: 0 }, w, Tick(7));
    assert_eq!(m.mode(), Mode::Build(UnitDefId(20)));
    m
}
fn begin(m: &mut Mapper, w: &impl WorldView) {
    shift(m, w, true);
    pointer(m, w, 800, 400);
    assert!(mouse(m, w, true).is_empty());
}

#[test]
fn both_profiles_stamp_distinct_non_overlapping_sites_in_travel_order() {
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let w = World::new();
        let mut m = mapper(&w, profile, &[1, 2, 3, 4, 5]);
        begin(&mut m, &w);
        // Same snap, overlapping snap, then an adjacent footprint. Retrace
        // repeats no sites. A large jump does not fill the gap.
        for x in [804, 809, 810, 820, 800, 900, 820] {
            assert!(pointer(&mut m, &w, x, 400).is_empty());
        }
        assert_eq!(
            sims(frame(&mut m, &w)),
            vec![
                build(pos(800, 400), &[1, 2]),
                build(pos(820, 400), &[1, 2]),
                build(pos(900, 400), &[1, 2])
            ]
        );
        assert_eq!(m.selection(), &units(&[1, 2, 3, 4, 5]));
        assert_eq!(m.mode(), Mode::Build(UnitDefId(20)));
        for _ in 0..3 {
            assert!(frame(&mut m, &w).is_empty());
        }
        pointer(&mut m, &w, 900, 400);
        assert!(mouse(&mut m, &w, false).is_empty());
        assert!(frame(&mut m, &w).is_empty());
        // No accidental double-click selection suppresses a genuinely new stroke.
        pointer(&mut m, &w, 1000, 400);
        mouse(&mut m, &w, true);
        pointer(&mut m, &w, 1020, 400);
        assert_eq!(
            sims(frame(&mut m, &w)),
            vec![
                build(pos(1000, 400), &[1, 2]),
                build(pos(1020, 400), &[1, 2])
            ]
        );
    }
}

#[test]
fn threshold_is_screen_distance_and_small_click_retains_unsnapped_legacy_build() {
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let mut w = World::new();
        w.geometry = Geometry::Unsupported;
        let mut m = mapper(&w, profile, &[1]);
        begin(&mut m, &w);
        assert!(pointer(&mut m, &w, 803, 400).is_empty());
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(
            sims(mouse(&mut m, &w, false)),
            vec![build(pos(803, 400), &[1])]
        );
        assert_eq!(m.mode(), Mode::Build(UnitDefId(20)));
        // Neither can_place=false nor missing painting geometry gates a click.
        assert!(!w.can_place(UnitDefId(20), pos(803, 400)));
    }
    let w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    m.set_camera(Camera {
        world_per_px: Fx::from_int(2),
        ..*m.camera()
    });
    begin(&mut m, &w);
    pointer(&mut m, &w, 803, 400);
    assert!(frame(&mut m, &w).is_empty());
    pointer(&mut m, &w, 804, 400);
    assert_eq!(sims(frame(&mut m, &w)), vec![build(pos(1600, 800), &[1])]);
}

#[test]
fn invalid_and_unsupported_geometry_never_turns_long_build_drag_into_selection() {
    for geometry in [
        Geometry::Unsupported,
        Geometry::Empty,
        Geometry::Reversed,
        Geometry::CenterOutside,
    ] {
        let mut w = World::new();
        w.geometry = geometry;
        let mut m = mapper(&w, Profile::original_tak(), &[1]);
        begin(&mut m, &w);
        pointer(&mut m, &w, 900, 400);
        assert!(frame(&mut m, &w).is_empty());
        assert!(mouse(&mut m, &w, false).is_empty());
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), Mode::Build(UnitDefId(20)));
    }
}

#[test]
fn invalid_samples_are_skipped_and_valid_later_samples_continue() {
    let mut w = World::new();
    w.blocked_x = Some(820);
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    begin(&mut m, &w);
    pointer(&mut m, &w, 820, 400);
    pointer(&mut m, &w, 840, 400);
    assert_eq!(
        sims(frame(&mut m, &w)),
        vec![build(pos(800, 400), &[1]), build(pos(840, 400), &[1])]
    );
    pointer(&mut m, &w, 860, 400);
    w.blocked_x = Some(860); // Became invalid before this poll was committed.
    assert!(frame(&mut m, &w).is_empty());
}

#[test]
fn same_poll_modifier_release_discards_pending_sites_and_consumes_old_release() {
    for release_mouse_first in [false, true] {
        let w = World::new();
        let mut m = mapper(&w, Profile::classic_tak(), &[1]);
        begin(&mut m, &w);
        pointer(&mut m, &w, 820, 400);
        assert_eq!(sims(frame(&mut m, &w)).len(), 2);
        pointer(&mut m, &w, 840, 400);
        if release_mouse_first {
            assert!(mouse(&mut m, &w, false).is_empty());
        }
        assert!(shift(&mut m, &w, false).is_empty());
        shift(&mut m, &w, true);
        pointer(&mut m, &w, 860, 400);
        if !release_mouse_first {
            assert!(mouse(&mut m, &w, false).is_empty());
        }
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), Mode::Build(UnitDefId(20)));
        mouse(&mut m, &w, true);
        pointer(&mut m, &w, 880, 400);
        assert_eq!(
            sims(frame(&mut m, &w)),
            vec![build(pos(860, 400), &[1]), build(pos(880, 400), &[1])]
        );
    }
}

#[test]
fn release_before_frame_commits_once_with_shift_still_held() {
    let w = World::new();
    let mut m = mapper(&w, Profile::original_tak(), &[1]);
    begin(&mut m, &w);
    pointer(&mut m, &w, 820, 400);
    assert!(mouse(&mut m, &w, false).is_empty());
    assert_eq!(
        sims(frame(&mut m, &w)),
        vec![build(pos(800, 400), &[1]), build(pos(820, 400), &[1])]
    );
    assert!(frame(&mut m, &w).is_empty());
}

#[test]
fn leaving_terrain_ends_the_stroke_and_reentry_requires_a_fresh_gesture() {
    for boundary in [
        EventKind::PointerLeaveTerrain,
        EventKind::MapPointer {
            x: 20,
            y: 20,
            world: pos(5000, 6000),
        },
    ] {
        let w = World::new();
        let mut m = mapper(&w, Profile::original_tak(), &[1]);
        begin(&mut m, &w);
        pointer(&mut m, &w, 820, 400);
        event(&mut m, &w, boundary);
        pointer(&mut m, &w, 1000, 400);
        assert!(frame(&mut m, &w).is_empty());
        assert!(mouse(&mut m, &w, false).is_empty());
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
        mouse(&mut m, &w, true);
        pointer(&mut m, &w, 1020, 400);
        assert_eq!(
            sims(frame(&mut m, &w)),
            vec![build(pos(1000, 400), &[1]), build(pos(1020, 400), &[1])]
        );
    }
}

#[test]
fn cancel_stop_and_modal_changes_discard_samples_and_do_not_leak_release_bindings() {
    for action in [
        Action::Cancel,
        Action::Stop,
        Action::MoveMode,
        Action::BuildOption { index: 1 },
        Action::ToggleOverlay,
        Action::ToggleCrystalBall,
        Action::TogglePause,
    ] {
        let w = World::new();
        let mut profile = Profile::classic_tak();
        profile.bindings.push(Binding {
            input: Input::Button(Button::MouseLeft),
            with: None,
            with_all: Vec::new(),
            gesture: Gesture::Release,
            action: Action::Context { append: false },
        });
        let mut m = mapper(&w, profile, &[1]);
        begin(&mut m, &w);
        pointer(&mut m, &w, 820, 400);
        m.perform(action, &w, Tick(7));
        assert!(mouse(&mut m, &w, false).is_empty(), "{action:?}");
        assert!(frame(&mut m, &w).is_empty(), "{action:?}");
        assert_eq!(m.selection(), &units(&[1]));
    }
}

#[test]
fn selection_profile_and_lost_builder_invalidate_pending_and_held_strokes() {
    for mutation in 0..5 {
        let mut w = World::new();
        let mut m = mapper(&w, Profile::classic_tak(), &[1, 2]);
        begin(&mut m, &w);
        pointer(&mut m, &w, 820, 400);
        match mutation {
            0 => {
                m.perform(Action::SelectAll { scope: Scope::Map }, &w, Tick(7));
            }
            1 => m.set_profile(Profile::original_tak()),
            2 => {
                w.inner.spec.units.remove(1);
            }
            3 => w.inner.spec.units[1].under_construction = true,
            4 => w.unavailable.push(uid(2)),
            _ => unreachable!(),
        }
        pointer(&mut m, &w, 840, 400);
        assert!(mouse(&mut m, &w, false).is_empty());
        assert!(frame(&mut m, &w).is_empty());
    }
}

#[test]
fn releasing_queue_below_threshold_does_not_replace_orders_on_old_mouse_release() {
    let w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1]);
    begin(&mut m, &w);
    pointer(&mut m, &w, 803, 400);
    shift(&mut m, &w, false);
    assert!(mouse(&mut m, &w, false).is_empty());
    assert!(frame(&mut m, &w).is_empty());
}

#[test]
fn no_preheld_queue_means_ordinary_selection_drag_even_if_shift_is_pressed_later() {
    for shift_late in [false, true] {
        let w = World::new();
        let mut m = mapper(&w, Profile::classic_tak(), &[1]);
        pointer(&mut m, &w, 800, 400);
        mouse(&mut m, &w, true);
        if shift_late {
            shift(&mut m, &w, true);
        }
        pointer(&mut m, &w, 900, 400);
        assert!(frame(&mut m, &w).is_empty());
        let out = mouse(&mut m, &w, false);
        assert!(sims(out.clone()).is_empty());
        assert!(out.iter().any(|o| matches!(
            o,
            Output::Client {
                command: tak_commands::ClientCommand::SelectBrush { .. }
            }
        )));
    }
}

#[test]
fn cap_bounds_total_dispatch_not_per_frame_and_jump_never_interpolates() {
    let w = World::new();
    let mut m = mapper(&w, Profile::classic_tak(), &[1, 2, 3]);
    m.set_camera(Camera {
        width_px: 30000,
        ..*m.camera()
    });
    begin(&mut m, &w);
    let mut emitted = Vec::new();
    for x in (820..3000).step_by(20) {
        pointer(&mut m, &w, x, 400);
        emitted.extend(sims(frame(&mut m, &w)));
    }
    assert_eq!(emitted.len(), MAX_BUILD_DRAG_SITES);
    assert_eq!(emitted.first(), Some(&build(pos(800, 400), &[1, 2])));
    assert_eq!(emitted.last(), Some(&build(pos(2060, 400), &[1, 2])));
    assert!(mouse(&mut m, &w, false).is_empty());
    assert!(frame(&mut m, &w).is_empty());
    pointer(&mut m, &w, 5000, 400);
    mouse(&mut m, &w, true);
    pointer(&mut m, &w, 20000, 400);
    assert_eq!(
        sims(frame(&mut m, &w)),
        vec![
            build(pos(5000, 400), &[1, 2]),
            build(pos(20000, 400), &[1, 2])
        ]
    );
}

#[test]
fn default_view_does_not_fabricate_geometry() {
    let w = World::new();
    assert_eq!(
        w.inner.placement_candidate(UnitDefId(20), pos(800, 400)),
        None
    );
    assert_eq!(
        w.inner.unit(uid(1)).unwrap().builder,
        Some(BuilderKind::Mobile)
    );
}

#[test]
fn pad_a_and_x_keep_single_click_commands_even_with_mouse_geometry_available() {
    for button in [Button::A, Button::X] {
        let w = World::new();
        let mut m = mapper(&w, Profile::default_combined(), &[1]);
        shift(&mut m, &w, true);
        pointer(&mut m, &w, 803, 400);
        assert!(event(&mut m, &w, EventKind::ButtonDown { button }).is_empty());
        pointer(&mut m, &w, 923, 400);
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(
            sims(event(&mut m, &w, EventKind::ButtonUp { button })),
            vec![build(pos(923, 400), &[1])]
        );
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), Mode::Build(UnitDefId(20)));
    }
}

#[test]
fn shifted_selection_drags_without_build_mode_and_factory_work_remain_unchanged() {
    let w = World::new();
    let mut m = mapper(&w, Profile::original_tak(), &[1]);
    m.perform(Action::Cancel, &w, Tick(7));
    assert_eq!(m.mode(), Mode::None);
    begin(&mut m, &w);
    pointer(&mut m, &w, 900, 400);
    assert!(frame(&mut m, &w).is_empty());
    let out = mouse(&mut m, &w, false);
    assert!(sims(out.clone()).is_empty());
    assert!(out.iter().any(|o| matches!(
        o,
        Output::Client {
            command: tak_commands::ClientCommand::SelectBrush { additive: true, .. }
        }
    )));

    let mut m = Mapper::new(Profile::classic_tak());
    pointer(&mut m, &w, 300, 100);
    m.perform(Action::Select, &w, Tick(7));
    shift(&mut m, &w, true);
    let queued = sims(m.perform(Action::BuildOption { index: 0 }, &w, Tick(7)));
    assert_eq!(
        queued,
        vec![
            SimCommand::Build {
                units: units(&[3]),
                def: UnitDefId(20),
                at: None,
                queue: QueueMode::Append
            };
            5
        ]
    );
    assert_eq!(m.mode(), Mode::None);
    pointer(&mut m, &w, 800, 400);
    mouse(&mut m, &w, true);
    pointer(&mut m, &w, 900, 400);
    assert!(frame(&mut m, &w).is_empty());
    assert!(sims(mouse(&mut m, &w, false)).is_empty());
}

#[test]
fn rapid_shift_click_then_drag_is_not_swallowed_as_same_type_selection() {
    for profile in [Profile::classic_tak(), Profile::original_tak()] {
        let w = World::new();
        let mut m = mapper(&w, profile, &[1]);
        begin(&mut m, &w);
        assert_eq!(
            sims(mouse(&mut m, &w, false)),
            vec![build(pos(800, 400), &[1])]
        );
        pointer(&mut m, &w, 900, 400);
        assert!(mouse(&mut m, &w, true).is_empty());
        pointer(&mut m, &w, 920, 400);
        assert_eq!(
            sims(frame(&mut m, &w)),
            vec![build(pos(900, 400), &[1]), build(pos(920, 400), &[1])]
        );
        assert!(mouse(&mut m, &w, false).is_empty());
        assert!(frame(&mut m, &w).is_empty());
        assert_eq!(m.selection(), &units(&[1]));
        assert_eq!(m.mode(), Mode::Build(UnitDefId(20)));
    }
}
