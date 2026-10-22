//! The Kingdoms Console profile (design system section 4): a recorded
//! (synthetic) gamepad session exercising every binding produces the
//! expected commands and state, segment by segment. The camera follows the
//! mapper's pan and zoom requests the way the client applies them, so the
//! reticle (screen centre) moves over the world.

mod common;

use common::{TestWorld, WorldSpec, pos, tick_at, uid};
use serde::Deserialize;
use tak_commands::{
    ClientCommand, ControlGroup, FireState, QueueMode, Scope, SimCommand, SpellId, Target,
    UnitDefId, UnitSet,
};
use tak_input::{
    Camera, Feedback, InputEvent, Mapper, Mode, Output, Profile, Refusal, Tuning, WheelKind,
};
use tak_math::{Fx, Vec2};

#[derive(Deserialize)]
struct Fixture {
    #[allow(dead_code)]
    description: String,
    world: WorldSpec,
    segments: Vec<Segment>,
}

#[derive(Deserialize)]
struct Segment {
    label: String,
    events: Vec<InputEvent>,
}

/// Kingdoms Console with exact, assist-free motion so positions are easy to
/// predict: linear stick, no deadzones, 1000 px/s, no smoothing.
pub fn exact_profile() -> Profile {
    let mut p = Profile::kingdoms_console();
    p.tuning = Tuning {
        cursor_curve_pct: 100,
        stick_deadzone: 0,
        stick_outer_deadzone: 0,
        cursor_zoom_pct: 0,
        cursor_speed: 1000,
        friction_pct: 100,
        magnet_radius_px: 0,
        pan_smooth_ms: 0,
        zoom_smooth_ms: 0,
        glide_ms: 0,
        reticle_smooth_ms: 0,
        ..Tuning::console()
    };
    p
}

/// A mapper and the camera the "client" keeps.
pub struct Rig {
    pub m: Mapper,
    pub cam: Camera,
}

impl Rig {
    pub fn new(p: Profile) -> Rig {
        let cam = Camera::default();
        let mut m = Mapper::new(p);
        m.set_camera(cam);
        Rig { m, cam }
    }

    /// Feed events; after each, apply the requested pan and zoom (about the
    /// screen centre) like the client does.
    pub fn run(&mut self, world: &TestWorld, events: &[InputEvent]) -> Vec<Output> {
        let mut out = Vec::new();
        for e in events {
            out.extend(self.m.handle(e, world, tick_at(e.t)));
            let pan = self.m.take_camera_pan();
            self.cam.origin += pan;
            if let Some(z) = self.m.take_camera_zoom() {
                let c = self.cam.screen_to_world(self.cam.center_screen());
                self.cam.world_per_px = z;
                let half = self.cam.center_screen().scale(z);
                self.cam.origin = c - half;
            }
            self.m.set_camera(self.cam);
        }
        out
    }
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
fn kingdoms_console_session_exercises_every_binding() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/kingdoms_console_session.json"))
            .expect("fixture");
    let world = TestWorld::new(fixture.world);
    let mut rig = Rig::new(exact_profile());
    let g1 = ControlGroup::new(1).unwrap();
    let all_own = units(&[1, 2, 3, 4, 7]);
    let mut seen = Vec::new();
    for seg in &fixture.segments {
        let out = rig.run(&world, &seg.events);
        let fb = rig.m.take_feedback();
        let m = &rig.m;
        let label = seg.label.as_str();
        seen.push(label.to_owned());
        let ctx = format!("{label}: {out:?} sel {:?}", m.selection());
        match label {
            "a_tap_selects_under_reticle" => {
                assert!(m.reticle_active(), "{ctx}");
                assert_eq!(
                    out,
                    vec![client(ClientCommand::SelectAt {
                        pos: pos(640, 480),
                        additive: false
                    })],
                    "{ctx}"
                );
                assert_eq!(m.selection(), &units(&[1]), "{ctx}");
            }
            "dpad_up_hold_assigns_squad_1" => {
                assert_eq!(out, vec![client(ClientCommand::AssignGroup { group: g1 })]);
            }
            "left_stick_pans_camera_under_fixed_reticle" => {
                assert!(out.is_empty(), "{ctx}");
                assert_eq!(m.cursor_screen(), pos(640, 480), "reticle stays centred");
                assert_eq!(rig.cam.origin, pos(60, 0), "camera moved");
                assert_eq!(m.cursor_world(), pos(700, 480), "{ctx}");
            }
            "a_tap_then_second_tap_selects_type_on_screen" => {
                assert_eq!(
                    out,
                    vec![
                        client(ClientCommand::SelectAt {
                            pos: pos(700, 480),
                            additive: false
                        }),
                        client(ClientCommand::SelectSameType {
                            scope: Scope::OnScreen
                        }),
                    ],
                    "{ctx}"
                );
                assert_eq!(m.selection(), &units(&[2, 3]), "{ctx}");
            }
            "rb_held_a_adds_without_select_all" => {
                assert_eq!(
                    out,
                    vec![client(ClientCommand::SelectAt {
                        pos: pos(700, 520),
                        additive: true
                    })],
                    "{ctx}"
                );
                assert_eq!(m.selection(), &units(&[2, 3, 4]), "{ctx}");
            }
            "rt_tap_sub_select_first_type" => {
                assert!(out.is_empty(), "{ctx}");
                assert_eq!(m.selection(), &units(&[2, 3]), "{ctx}");
                let ss = m.sub_select().expect("cycle in progress");
                assert_eq!(ss.full, units(&[2, 3, 4]));
                assert_eq!(ss.types, vec![UnitDefId(11), UnitDefId(12)]);
                assert_eq!(ss.index, 0);
            }
            "rt_tap_sub_select_second_type" => {
                assert_eq!(m.selection(), &units(&[4]), "{ctx}");
                assert_eq!(m.sub_select().map(|s| s.index), Some(1));
            }
            "rt_tap_back_to_full_selection" => {
                assert_eq!(m.selection(), &units(&[2, 3, 4]), "{ctx}");
                assert!(m.sub_select().is_none());
            }
            "x_press_attacks_enemy_under_reticle" => {
                assert_eq!(m.cursor_world(), pos(800, 480), "{ctx}");
                assert_eq!(
                    out,
                    vec![sim(
                        3600,
                        SimCommand::Attack {
                            units: units(&[2, 3, 4]),
                            target: Target::Unit(uid(6)),
                            queue: QueueMode::Replace,
                        }
                    )],
                    "{ctx}"
                );
            }
            "rb_x_queues_order" => {
                assert_eq!(
                    out,
                    vec![sim(
                        3850,
                        SimCommand::Attack {
                            units: units(&[2, 3, 4]),
                            target: Target::Unit(uid(6)),
                            queue: QueueMode::Append,
                        }
                    )],
                    "{ctx}"
                );
            }
            "rb_tap_all_on_screen" => {
                assert_eq!(
                    out,
                    vec![Output::SelectAll {
                        scope: Scope::OnScreen,
                        units: all_own.clone()
                    }],
                    "{ctx}"
                );
            }
            "rb_second_tap_all_on_map" => {
                assert_eq!(
                    out,
                    vec![Output::SelectAll {
                        scope: Scope::Map,
                        units: all_own.clone()
                    }],
                    "{ctx}"
                );
            }
            "b_tap_clears_selection" => {
                assert_eq!(out, vec![client(ClientCommand::ClearSelection)], "{ctx}");
                assert!(m.selection().is_empty());
            }
            "dpad_up_tap_recalls_squad_1" => {
                assert_eq!(out, vec![client(ClientCommand::RecallGroup { group: g1 })]);
                assert_eq!(m.selection(), &units(&[1]));
            }
            "dpad_up_second_tap_jumps_camera_to_squad_1" => {
                assert!(out.is_empty(), "{ctx}");
                assert_eq!(m.cursor_world(), pos(640, 480), "camera on squad 1");
                assert_eq!(m.selection(), &units(&[1]));
            }
            "y_tap_casts_first_spell" => {
                assert_eq!(m.mode(), Mode::CastSpell(SpellId(0)), "{ctx}");
            }
            "b_tap_cancels_cast_mode" | "b_tap_cancels_again" => {
                assert!(out.is_empty(), "{ctx}");
                assert_eq!(m.mode(), Mode::None);
                assert_eq!(m.selection(), &units(&[1]), "cancel left the selection");
            }
            "y_hold_spell_wheel_confirms_on_release" => {
                assert!(m.wheel().is_none());
                assert_eq!(m.mode(), Mode::CastSpell(SpellId(0)), "{ctx}");
            }
            "lb_tap_next_idle_builder" => {
                assert_eq!(out, vec![client(ClientCommand::CycleIdleBuilder)]);
                assert_eq!(m.selection(), &units(&[7]));
            }
            "lb_hold_build_wheel_page_then_place" => {
                assert_eq!(
                    out,
                    vec![sim(
                        7700,
                        SimCommand::Build {
                            units: units(&[7]),
                            def: UnitDefId(22),
                            at: Some(pos(640, 480)),
                            queue: QueueMode::Replace,
                        }
                    )],
                    "{ctx}"
                );
            }
            "lt_orders_wheel_stance_sub_ring" => {
                assert_eq!(
                    out,
                    vec![sim(
                        8350,
                        SimCommand::SetFireState {
                            units: units(&[7]),
                            state: FireState::HoldFire,
                        }
                    )],
                    "{ctx}"
                );
                assert!(m.wheel().is_none());
            }
            "lt_orders_wheel_stop_at_twelve" => {
                assert_eq!(
                    out,
                    vec![sim(8750, SimCommand::Stop { units: units(&[7]) })],
                    "{ctx}"
                );
            }
            "b_hold_stops" => {
                assert_eq!(
                    out,
                    vec![sim(9300, SimCommand::Stop { units: units(&[7]) })],
                    "{ctx}"
                );
                assert_eq!(m.selection(), &units(&[7]), "hold B keeps the selection");
            }
            "view_tap_jumps_to_alert" => {
                assert_eq!(out, vec![client(ClientCommand::JumpToAlert)]);
            }
            "view_hold_crystal_ball_on" => assert!(m.crystal_ball()),
            "view_release_crystal_ball_off" => assert!(!m.crystal_ball()),
            "menu_opens_game_menu" => assert_eq!(out, vec![Output::ToggleOverlay]),
            "l3_tap_centres_on_monarch" => {
                assert_eq!(out, vec![client(ClientCommand::CenterOnMonarch)]);
            }
            "l3_second_tap_selects_monarch" => {
                assert_eq!(m.selection(), &units(&[1]), "{ctx}");
                assert!(!m.following());
            }
            "l3_hold_tracks_monarch" => {
                assert_eq!(m.selection(), &units(&[1]));
                assert!(m.following(), "{ctx}");
            }
            "r3_hold_toggles_tracking" => assert!(!m.following()),
            "right_stick_zooms_in" => {
                assert!(rig.cam.world_per_px < Fx::ONE, "{:?}", rig.cam);
                assert_eq!(m.cursor_world(), pos(640, 480), "zoom about the reticle");
            }
            "r3_tap_resets_zoom" => assert_eq!(rig.cam.world_per_px, Fx::HALF),
            "a_hold_brush_selects_circle" => {
                assert_eq!(out.len(), 1, "{ctx}");
                let Output::Client {
                    command:
                        ClientCommand::SelectBrush {
                            center,
                            radius,
                            additive,
                        },
                } = out[0]
                else {
                    panic!("{ctx}");
                };
                assert_eq!(center, pos(640, 480));
                assert!(radius > Fx::from_int(16) && !additive, "{ctx}");
                assert_eq!(m.selection(), &units(&[1]));
            }
            "x_hold_attack_move_is_reserved" => {
                assert_eq!(
                    out,
                    vec![sim(
                        13600,
                        SimCommand::Move {
                            units: units(&[1]),
                            to: pos(640, 480),
                            queue: QueueMode::Replace,
                        }
                    )],
                    "{ctx}"
                );
                assert!(
                    fb.contains(&Feedback::Refused {
                        reason: Refusal::Unsupported
                    }),
                    "{fb:?}"
                );
            }
            "rt_hold_fast_pan" => {
                // 60 ms at 1000 px/s, zoom 0.5, fast pan 200 %: 60 units.
                assert_eq!(m.cursor_world(), pos(700, 480), "{ctx}");
                assert!(out.is_empty());
            }
            other => panic!("segment {other} has no check"),
        }
    }
    assert_eq!(seen.len(), fixture.segments.len());
}

#[test]
fn build_wheel_pages_hold_eight_and_rb_turns_the_page() {
    let spec: WorldSpec = serde_json::from_str(
        r#"{"local_player": 0, "enemies": [1], "units": [
            {"index": 1, "owner": 0, "def": 5, "x": 640, "y": 480, "builder": "mobile",
             "build_options": [20, 21, 22, 23, 24, 25, 26, 27, 28, 29]}]}"#,
    )
    .unwrap();
    let w = TestWorld::new(spec);
    let mut rig = Rig::new(exact_profile());
    let ev = |t, kind| InputEvent::new(t, kind);
    use tak_input::{Button, EventKind};
    let down = |b| EventKind::ButtonDown { button: b };
    let up = |b| EventKind::ButtonUp { button: b };
    rig.run(&w, &[ev(100, down(Button::A)), ev(150, up(Button::A))]);
    rig.run(
        &w,
        &[ev(1000, down(Button::Lb)), ev(1300, EventKind::Frame)],
    );
    let wh = rig.m.wheel().expect("build wheel").clone();
    assert_eq!(wh.kind, WheelKind::Build);
    assert_eq!((wh.slots.len(), wh.page, wh.pages), (8, 0, 2));
    // RB inside the wheel: next page, and no "select all" on release.
    let out = rig.run(&w, &[ev(1400, down(Button::Rb)), ev(1450, up(Button::Rb))]);
    assert!(out.is_empty(), "{out:?}");
    let wh = rig.m.wheel().unwrap();
    assert_eq!((wh.page, wh.pages), (1, 2));
    assert_eq!(wh.slots[0], tak_input::Action::BuildOption { index: 8 });
    assert!(tak_input::is_blank(&wh.slots[2]));
    // Slot 1 of page 2 is option 10.
    rig.run(
        &w,
        &[
            ev(
                1500,
                EventKind::Axis {
                    axis: tak_input::Axis::LeftX,
                    value: 13000,
                },
            ),
            ev(
                1500,
                EventKind::Axis {
                    axis: tak_input::Axis::LeftY,
                    value: 13000,
                },
            ),
            ev(
                1550,
                EventKind::Axis {
                    axis: tak_input::Axis::LeftX,
                    value: 0,
                },
            ),
            ev(
                1550,
                EventKind::Axis {
                    axis: tak_input::Axis::LeftY,
                    value: 0,
                },
            ),
            ev(1600, up(Button::Lb)),
        ],
    );
    assert_eq!(rig.m.mode(), Mode::Build(UnitDefId(29)));
    let _ = Vec2::ZERO;
}

#[test]
fn old_profiles_stay_selectable() {
    for name in tak_input::CONTROL_SCHEMES {
        let p = Profile::named(name).unwrap_or_else(|| panic!("{name}"));
        p.validate().unwrap();
    }
    assert!(!Profile::wp_s_controller().tuning.reticle);
    assert!(Profile::kingdoms_console().tuning.reticle);
    assert!(Profile::kingdoms_console().fixed_slots);
}
