//! The Classic TA:K profile (design system section 5): a recorded
//! (synthetic) mouse and keyboard session exercising TA:K's default keys
//! and mouse model produces the expected commands and state, segment by
//! segment; and the optional Grid build hotkeys.

mod common;

use common::{CamRig, TestWorld, WorldSpec, pos, uid};
use serde::Deserialize;
use tak_commands::{ClientCommand, ControlGroup, QueueMode, Scope, SimCommand, SpellId, Target};
use tak_input::{Feedback, InputEvent, Mode, Output, Profile, Refusal, Tuning};
use tak_math::Fx;

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

fn units(ids: &[u32]) -> tak_commands::UnitSet {
    ids.iter().map(|&i| uid(i)).collect()
}

fn sims(out: &[Output]) -> Vec<SimCommand> {
    out.iter()
        .filter_map(|o| match o {
            Output::Sim { command, .. } => Some(command.clone()),
            _ => None,
        })
        .collect()
}

fn client(command: ClientCommand) -> Output {
    Output::Client { command }
}

fn profile() -> Profile {
    let mut p = Profile::classic_tak();
    p.tuning = Tuning {
        glide_ms: 0,
        ..Tuning::console()
    };
    p
}

#[test]
fn classic_tak_session_exercises_every_binding() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/classic_tak_session.json")).expect("fixture");
    let world = TestWorld::new(fixture.world);
    let mut rig = CamRig::new(profile());
    let g1 = ControlGroup::new(1).unwrap();
    let mid = |r: &CamRig| r.cam.screen_to_world(r.cam.center_screen());
    for seg in &fixture.segments {
        let out = rig.run(&world, &seg.events);
        let fb = rig.m.take_feedback();
        let m = &rig.m;
        let label = seg.label.as_str();
        let ctx = format!("{label}: {out:?} sel {:?} fb {fb:?}", m.selection());
        let refused = fb.contains(&Feedback::Refused {
            reason: Refusal::Unsupported,
        });
        match label {
            "lmb_click_selects" => {
                assert_eq!(
                    out,
                    vec![client(ClientCommand::SelectAt {
                        pos: pos(640, 480),
                        additive: false
                    })],
                    "{ctx}"
                );
                assert_eq!(m.selection(), &units(&[1]));
            }
            "double_click_selects_type_on_screen" => {
                assert_eq!(
                    out,
                    vec![
                        client(ClientCommand::SelectAt {
                            pos: pos(700, 480),
                            additive: false
                        }),
                        client(ClientCommand::SelectSameType {
                            scope: Scope::OnScreen
                        })
                    ],
                    "{ctx}"
                );
                assert_eq!(m.selection(), &units(&[2, 3]));
            }
            "lmb_drag_selects_box_as_circle" => {
                assert_eq!(out.len(), 1, "{ctx}");
                assert!(
                    matches!(
                        out[0],
                        Output::Client {
                            command: ClientCommand::SelectBrush { center, additive: false, radius }
                        } if center == pos(710, 500) && radius > Fx::from_int(44) && radius < Fx::from_int(45)
                    ),
                    "{ctx}"
                );
                assert_eq!(m.selection(), &units(&[2, 3, 4]));
            }
            "rmb_context_attacks_enemy" => assert_eq!(
                sims(&out),
                vec![SimCommand::Attack {
                    units: units(&[2, 3, 4]),
                    target: Target::Unit(uid(6)),
                    queue: QueueMode::Replace
                }],
                "{ctx}"
            ),
            "shift_rmb_queues_move" => assert_eq!(
                sims(&out),
                vec![SimCommand::Move {
                    units: units(&[2, 3, 4]),
                    to: pos(900, 600),
                    queue: QueueMode::Append
                }],
                "{ctx}"
            ),
            "a_then_click_attacks_ground" => assert_eq!(
                sims(&out),
                vec![SimCommand::Attack {
                    units: units(&[2, 3, 4]),
                    target: Target::Ground(pos(850, 700)),
                    queue: QueueMode::Replace
                }],
                "{ctx}"
            ),
            "m_then_click_moves_over_enemy" => assert_eq!(
                sims(&out),
                vec![SimCommand::Move {
                    units: units(&[2, 3, 4]),
                    to: pos(800, 480),
                    queue: QueueMode::Replace
                }],
                "{ctx}"
            ),
            "g_then_click_guards" => assert_eq!(
                sims(&out),
                vec![SimCommand::Guard {
                    units: units(&[2, 3, 4]),
                    target: uid(1),
                    queue: QueueMode::Replace
                }],
                "{ctx}"
            ),
            "p_then_rmb_patrols" => assert_eq!(
                sims(&out),
                vec![SimCommand::Patrol {
                    units: units(&[2, 3, 4]),
                    to: pos(500, 500),
                    queue: QueueMode::Replace
                }],
                "{ctx}"
            ),
            "s_stops" => assert_eq!(
                sims(&out),
                vec![SimCommand::Stop {
                    units: units(&[2, 3, 4])
                }],
                "{ctx}"
            ),
            "c_clear_requires_capability" => {
                assert!(out.is_empty(), "{ctx}");
                assert!(
                    fb.contains(&Feedback::Refused {
                        reason: Refusal::CannotDo
                    }),
                    "{ctx}"
                );
                assert_eq!(m.mode(), Mode::None);
            }
            "alt_1_formation_is_reserved" | "ctrl_d_dismiss_is_reserved" => {
                assert!(out.is_empty() && refused, "{ctx}");
            }
            "ctrl_1_creates_squad" => {
                assert_eq!(out, vec![client(ClientCommand::AssignGroup { group: g1 })]);
            }
            "escape_deselects" => {
                assert_eq!(out, vec![client(ClientCommand::ClearSelection)]);
                assert!(m.selection().is_empty());
            }
            "1_retrieves_squad" => {
                assert_eq!(out, vec![client(ClientCommand::RecallGroup { group: g1 })]);
                assert_eq!(m.selection(), &units(&[2, 3, 4]));
            }
            "ctrl_e_selects_melee" => assert_eq!(m.selection(), &units(&[2, 3]), "{ctx}"),
            "ctrl_shift_r_adds_ballistic" => {
                assert_eq!(m.selection(), &units(&[2, 3, 4]), "{ctx}")
            }
            "ctrl_b_selects_builders" => assert_eq!(m.selection(), &units(&[7]), "{ctx}"),
            "ctrl_m_selects_and_tracks_monarch" => {
                assert_eq!(m.selection(), &units(&[1]), "{ctx}");
                assert!(m.following());
            }
            "t_toggles_tracking" => assert!(!m.following(), "{ctx}"),
            "ctrl_z_same_type_map" => assert_eq!(m.selection(), &units(&[1]), "{ctx}"),
            "ctrl_a_selects_all" => assert_eq!(
                out,
                vec![Output::SelectAll {
                    scope: Scope::Map,
                    units: units(&[1, 2, 3, 4, 7, 8])
                }],
                "{ctx}"
            ),
            "ctrl_u_selects_on_screen" => assert_eq!(
                out,
                vec![Output::SelectAll {
                    scope: Scope::OnScreen,
                    units: units(&[1, 2, 3, 4, 7])
                }],
                "{ctx}"
            ),
            "ctrl_f1_casts_first_spell" => {
                assert_eq!(m.mode(), Mode::CastSpell(SpellId(0)), "{ctx}")
            }
            "escape_cancels_cast" => {
                assert_eq!(m.mode(), Mode::None);
                assert_eq!(m.selection(), &units(&[1]));
            }
            "ctrl_f5_stores_bookmark" => assert_eq!(m.bookmark(1), Some(pos(640, 480))),
            "n_jumps_to_unit_off_screen" => assert_eq!(mid(&rig), pos(3000, 3000), "{ctx}"),
            "f5_returns_to_bookmark" => assert_eq!(mid(&rig), pos(640, 480), "{ctx}"),
            "f3_last_report" => assert_eq!(out, vec![client(ClientCommand::JumpToAlert)]),
            "tab_crystal_ball_on" => assert!(m.crystal_ball()),
            "tab_crystal_ball_off" => assert!(!m.crystal_ball()),
            "grave_damage_bars" => assert_eq!(out, vec![Output::ToggleDamageBars]),
            "equal_faster" => assert_eq!(out, vec![Output::GameSpeed { faster: true }]),
            "minus_slower" => assert_eq!(out, vec![Output::GameSpeed { faster: false }]),
            "pause_pauses" => assert_eq!(out, vec![Output::TogglePause]),
            "f1_options" | "f11_tuning" => assert_eq!(out, vec![Output::ToggleOverlay]),
            other => panic!("segment {other} has no check"),
        }
    }
}

#[test]
fn grid_build_keys_are_optional_and_need_a_builder() {
    let spec: WorldSpec = serde_json::from_str(
        r#"{"local_player": 0, "enemies": [1], "units": [
            {"index": 1, "owner": 0, "def": 5, "x": 640, "y": 480, "builder": "mobile",
             "build_options": [20, 21, 22, 23, 24, 25]},
            {"index": 2, "owner": 0, "def": 6, "x": 700, "y": 480}]}"#,
    )
    .unwrap();
    let w = TestWorld::new(spec);
    use tak_input::{Button, EventKind, Key};
    let ev = |t, kind| InputEvent::new(t, kind);
    let key = |t: u32, k: &str| {
        vec![
            ev(t, EventKind::KeyDown { key: Key::new(k) }),
            ev(t + 10, EventKind::KeyUp { key: Key::new(k) }),
        ]
    };
    let click = |t: u32, x: i32| {
        vec![
            ev(t, EventKind::PointerMove { x, y: 480 }),
            ev(
                t + 1,
                EventKind::ButtonDown {
                    button: Button::MouseLeft,
                },
            ),
            ev(
                t + 2,
                EventKind::ButtonUp {
                    button: Button::MouseLeft,
                },
            ),
        ]
    };
    // Off by default: S is Stop even with a builder selected.
    let mut rig = CamRig::new(profile());
    rig.run(&w, &click(100, 640));
    let out = rig.run(&w, &key(1000, "s"));
    assert!(
        matches!(sims(&out)[..], [SimCommand::Stop { .. }]),
        "{out:?}"
    );
    // With the add-on: S (the sixth grid key) picks build option 6 for a
    // builder...
    let mut p = profile();
    p.grid_build_keys = true;
    let mut rig = CamRig::new(p);
    rig.run(&w, &click(100, 640));
    let out = rig.run(&w, &key(1000, "s"));
    assert!(out.is_empty(), "{out:?}");
    assert_eq!(rig.m.mode(), Mode::Build(tak_commands::UnitDefId(25)));
    // ...and stays Stop for anything else.
    rig.run(&w, &key(1500, "escape"));
    rig.run(&w, &click(2000, 700));
    let out = rig.run(&w, &key(3000, "s"));
    assert!(
        matches!(sims(&out)[..], [SimCommand::Stop { .. }]),
        "{out:?}"
    );
}
