//! Explicitly configured teams protect automatic targeting only. Synthetic
//! fixtures deliberately make no claim about original Kingdoms diplomacy.

mod combat_common;

#[test]
fn unteamed_skirmish_preserves_pre_team_targeting_hashes() {
    // Frozen before the team-targeting change on parent 7f0fa51. This brawl
    // includes idle acquisition/chase, retaliation and explicit friendly fire.
    let hashes = combat_common::brawl_hashes(77, 3000);
    let mut digest = tak_sim::StateHasher::new();
    for &hash in &hashes {
        digest.u64(hash);
    }
    assert_eq!(hashes.len(), 2999);
    assert_eq!(hashes[0], 0x1609_3824_640f_f37e);
    assert_eq!(hashes[2998], 0xeb0b_994c_fcc0_35cf);
    assert_eq!(digest.finish(), 0x1a4c_2c5c_f465_9b72);
}

use combat_common::{ARCHER, DUMMY, DUMMY_TARGET, SWORD, attack, health, place, scenario, sim};
use tak_commands::{
    CommandLog, FireState, LoggedCommand, PlayerId, QueueMode, SimCommand, Target, Tick, UnitId,
    UnitSet,
};
use tak_math::{Fx, Vec2};
use tak_sim::{PlayerSpec, Scenario, Sim, StartPos};

fn team_scenario(teams: Vec<Option<u8>>) -> Scenario {
    let mut sc = scenario(0x7EA5);
    sc.map
        .start_positions
        .extend([StartPos { x: 16, y: 2032 }, StartPos { x: 2032, y: 16 }]);
    sc.players.extend([
        PlayerSpec {
            start: 2,
            monarch: DUMMY,
        },
        PlayerSpec {
            start: 3,
            monarch: DUMMY,
        },
    ]);
    sc.victory.teams = teams;
    sc
}

fn attack_move(id: UnitId, to: Vec2) -> SimCommand {
    SimCommand::AttackMove {
        units: UnitSet::single(id),
        to,
        queue: QueueMode::Replace,
    }
}

fn fire_state(id: UnitId, state: FireState) -> SimCommand {
    SimCommand::SetFireState {
        units: UnitSet::single(id),
        state,
    }
}

fn mixed_hashes(teams: Vec<Option<u8>>) -> Vec<u64> {
    let mut s = sim(&team_scenario(teams));
    let a = place(&mut s, SWORD, 0, 400, 400);
    let b = place(&mut s, ARCHER, 0, 400, 500);
    let guard = place(&mut s, SWORD, 0, 500, 700);
    let c = place(&mut s, SWORD, 1, 550, 400);
    let d = place(&mut s, SWORD, 2, 600, 500);
    let e = place(&mut s, ARCHER, 3, 650, 400);
    let mut log = CommandLog::new();
    for (tick, owner, command) in [
        (1, 0, fire_state(b, FireState::ReturnFire)),
        (1, 0, attack_move(a, Vec2::from_ints(950, 400))),
        (
            1,
            0,
            SimCommand::Guard {
                units: UnitSet::single(guard),
                target: b,
                queue: QueueMode::Replace,
            },
        ),
        (1, 1, attack(&[c], Target::Unit(a))),
        (1, 2, attack_move(d, Vec2::from_ints(400, 400))),
        (
            1,
            3,
            attack(&[e], Target::Ground(Vec2::from_ints(600, 500))),
        ),
        (
            120,
            1,
            SimCommand::Stop {
                units: UnitSet::single(c),
            },
        ),
        (150, 0, fire_state(b, FireState::FireAtWill)),
        (240, 0, fire_state(a, FireState::HoldFire)),
        (300, 0, fire_state(a, FireState::ReturnFire)),
        (360, 0, attack_move(guard, Vec2::from_ints(900, 600))),
        (480, 0, attack(&[guard], Target::Unit(b))),
    ] {
        log.record(Tick(tick), PlayerId(owner), command);
    }
    (1..600)
        .map(|_| {
            let tick = s.tick();
            s.step(log.for_tick(tick))
        })
        .collect()
}

#[test]
fn unteamed_mixed_orders_preserve_pre_team_targeting_hashes() {
    let hashes = mixed_hashes(vec![]);
    let mut digest = tak_sim::StateHasher::new();
    for &hash in &hashes {
        digest.u64(hash);
    }
    // Captured on 7f0fa51 before changing production targeting code.
    assert_eq!(hashes.len(), 599);
    assert_eq!(hashes[0], 0x7e90_47b4_7680_bfd2);
    assert_eq!(hashes[598], 0x6619_7bc1_b495_f919);
    assert_eq!(digest.finish(), 0x378d_68c1_fc92_763a);
}

fn issue(s: &mut Sim, player: u8, commands: Vec<SimCommand>) {
    let commands: Vec<_> = commands
        .into_iter()
        .enumerate()
        .map(|(seq, command)| LoggedCommand {
            tick: s.tick(),
            player: PlayerId(player),
            seq: seq as u32,
            command,
        })
        .collect();
    s.step(&commands);
}

fn steps(s: &mut Sim, count: usize) {
    for _ in 0..count {
        s.step(&[]);
    }
}

fn team_cases() -> Vec<(Vec<Option<u8>>, bool)> {
    vec![
        (vec![Some(0), Some(0)], false),
        (vec![Some(255), Some(255)], false),
        (vec![Some(0), Some(255)], true),
        (vec![None, None], true),
        (vec![], true),
        (vec![None], true),
        (vec![Some(0), None], true),
        (vec![None, Some(0)], true),
    ]
}

#[test]
fn relation_is_symmetric_explicit_only_and_safe_for_every_player_id() {
    let sc = team_scenario(vec![Some(255), Some(255), None]);
    let s = sim(&sc);
    let before = s.state_hash();
    for a in 0..=u8::MAX {
        for b in 0..=u8::MAX {
            let valid = a < 4 && b < 4;
            let allied = valid && (a == b || (a <= 1 && b <= 1));
            let hostile = valid && !allied;
            let (a, b) = (PlayerId(a), PlayerId(b));
            assert_eq!(s.world().are_allied(a, b), allied, "allied {a:?} {b:?}");
            assert_eq!(s.world().are_hostile(a, b), hostile, "hostile {a:?} {b:?}");
            assert_eq!(s.world().are_allied(a, b), s.world().are_allied(b, a));
            assert_eq!(s.world().are_hostile(a, b), s.world().are_hostile(b, a));
        }
    }
    assert_eq!(
        s.state_hash(),
        before,
        "relationship queries must be read-only"
    );
    for (teams, hostile) in team_cases() {
        let s = sim(&team_scenario(teams));
        assert_eq!(s.world().are_hostile(PlayerId(0), PlayerId(1)), hostile);
        assert_eq!(s.world().are_allied(PlayerId(0), PlayerId(1)), !hostile);
    }
}

#[test]
fn automatic_weapon_acquisition_obeys_explicit_teams() {
    for (teams, hostile) in team_cases() {
        let mut s = sim(&team_scenario(teams.clone()));
        let a = place(&mut s, SWORD, 0, 1000, 1000);
        let target = place(&mut s, DUMMY_TARGET, 1, 1020, 1000);
        s.step(&[]);
        assert_eq!(
            health(&s, target),
            Some(if hostile { 460 } else { 1000 }),
            "{teams:?}"
        );
        assert_eq!(
            s.world().unit(a).unwrap().combat.weapons[0].target,
            hostile.then_some(Target::Unit(target)),
            "{teams:?}"
        );
    }
}

#[test]
fn nearer_lower_slot_ally_is_skipped_in_favor_of_an_enemy() {
    let mut s = sim(&team_scenario(vec![Some(8), Some(8), Some(9)]));
    let a = place(&mut s, ARCHER, 0, 1000, 1000);
    let ally = place(&mut s, DUMMY_TARGET, 1, 1100, 1000);
    let enemy = place(&mut s, DUMMY_TARGET, 2, 1200, 1000);
    s.step(&[]);
    assert_eq!(
        s.world().unit(a).unwrap().combat.weapons[0].target,
        Some(Target::Unit(enemy))
    );
    assert_eq!(health(&s, ally), Some(1000));
}

#[test]
fn idle_chase_and_guard_acquisition_obey_explicit_teams() {
    for guard in [false, true] {
        for (teams, hostile) in team_cases() {
            let mut s = sim(&team_scenario(teams.clone()));
            let a = place(&mut s, SWORD, 0, 1000, 1000);
            let protected = place(&mut s, DUMMY_TARGET, 0, 1000, 1020);
            let target = place(&mut s, DUMMY_TARGET, 1, 1150, 1000);
            let commands = if guard {
                vec![SimCommand::Guard {
                    units: UnitSet::single(a),
                    target: protected,
                    queue: QueueMode::Replace,
                }]
            } else {
                vec![]
            };
            issue(&mut s, 0, commands);
            let u = s.world().unit(a).unwrap();
            assert_eq!(
                u.combat.goal,
                hostile.then_some(Vec2::from_ints(1150, 1000)),
                "guard={guard} teams={teams:?}"
            );
            if !hostile {
                steps(&mut s, 120);
                let u = s.world().unit(a).unwrap();
                assert_eq!(u.pos, Vec2::from_ints(1000, 1000));
                assert!(u.combat.weapons[0].target.is_none());
                assert_eq!(health(&s, target), Some(1000));
            }
        }
    }
}

#[test]
fn automatic_weapon_retention_discards_a_now_allied_owner() {
    let mut s = sim(&team_scenario(vec![Some(3), Some(3), Some(4)]));
    let a = place(&mut s, ARCHER, 0, 1000, 1000);
    let target = place(&mut s, DUMMY_TARGET, 2, 1200, 1000);
    s.step(&[]);
    assert_eq!(
        s.world().unit(a).unwrap().combat.weapons[0].target,
        Some(Target::Unit(target))
    );
    // Ownership mutation is a test probe, not a new capture/diplomacy command.
    s.world_mut().unit_mut(target).unwrap().owner = PlayerId(1);
    s.step(&[]);
    assert!(
        s.world().unit(a).unwrap().combat.weapons[0]
            .target
            .is_none()
    );
    // Previously launched projectiles are deliberately not canceled by teams.
    steps(&mut s, 12);
    assert_eq!(
        health(&s, target),
        Some(787),
        "the pre-existing arrow still hits its now-allied target"
    );
}

#[test]
fn attack_move_acquisition_and_route_resume_obey_explicit_teams() {
    let destination = Vec2::from_ints(1600, 1000);
    for (teams, hostile) in team_cases() {
        let mut s = sim(&team_scenario(teams.clone()));
        let a = place(&mut s, SWORD, 0, 1000, 1000);
        let target = place(&mut s, DUMMY_TARGET, 1, 1150, 1050);
        issue(&mut s, 0, vec![attack_move(a, destination)]);
        let state = s
            .world()
            .unit(a)
            .unwrap()
            .combat
            .attack_move
            .as_ref()
            .unwrap();
        assert_eq!(state.target, hostile.then_some(target), "{teams:?}");
        assert_eq!(
            state.goal,
            Some(if hostile {
                Vec2::from_ints(1150, 1050)
            } else {
                destination
            })
        );
        if !hostile {
            steps(&mut s, 180);
            assert_eq!(health(&s, target), Some(1000));
            assert_eq!(s.world().unit(a).unwrap().pos.y, Fx::from_int(1000));
        }
    }
}

#[test]
fn attack_move_retention_discards_allies_and_clears_avoided_allies() {
    let destination = Vec2::from_ints(1600, 1000);
    let mut s = sim(&team_scenario(vec![Some(3), Some(3), Some(4)]));
    let a = place(&mut s, SWORD, 0, 1000, 1000);
    let target = place(&mut s, DUMMY_TARGET, 2, 1150, 1050);
    let avoided = place(&mut s, DUMMY_TARGET, 2, 1160, 1100);
    issue(&mut s, 0, vec![attack_move(a, destination)]);
    assert_eq!(
        s.world()
            .unit(a)
            .unwrap()
            .combat
            .attack_move
            .as_ref()
            .unwrap()
            .target,
        Some(target)
    );
    s.world_mut()
        .unit_mut(a)
        .unwrap()
        .combat
        .attack_move
        .as_mut()
        .unwrap()
        .avoided
        .insert(avoided, Vec2::from_ints(1160, 1100));
    for id in [target, avoided] {
        s.world_mut().unit_mut(id).unwrap().owner = PlayerId(1);
    }
    s.step(&[]);
    let state = s
        .world()
        .unit(a)
        .unwrap()
        .combat
        .attack_move
        .as_ref()
        .unwrap();
    assert!(state.target.is_none());
    assert_eq!(state.goal, Some(destination));
    assert!(state.avoided.is_empty());
}

#[test]
fn return_fire_automatic_eligibility_requires_a_hostile_attacker() {
    for attack_moving in [false, true] {
        for (teams, hostile) in team_cases() {
            let mut s = sim(&team_scenario(teams.clone()));
            let a = place(&mut s, SWORD, 0, 1000, 1000);
            let attacker = place(&mut s, SWORD, 1, 1020, 1000);
            issue(&mut s, 0, vec![fire_state(a, FireState::ReturnFire)]);
            // The other player explicitly inflicts damage, even if allied.
            issue(
                &mut s,
                1,
                vec![
                    fire_state(attacker, FireState::HoldFire),
                    attack(&[attacker], Target::Unit(a)),
                ],
            );
            // The explicit attacker must first turn west to face its target.
            for _ in 0..30 {
                if health(&s, a).unwrap() < 2500 {
                    break;
                }
                s.step(&[]);
            }
            assert!(health(&s, a).unwrap() < 2500, "{teams:?}");
            assert_eq!(
                s.world().unit(a).unwrap().combat.last_attacker,
                Some(attacker)
            );
            let commands = if attack_moving {
                vec![attack_move(a, Vec2::from_ints(1600, 1000))]
            } else {
                vec![]
            };
            issue(&mut s, 0, commands);
            assert_eq!(
                s.world().unit(a).unwrap().combat.weapons[0].target,
                hostile.then_some(Target::Unit(attacker)),
                "moving={attack_moving} teams={teams:?}"
            );
            if attack_moving {
                assert_eq!(
                    s.world()
                        .unit(a)
                        .unwrap()
                        .combat
                        .attack_move
                        .as_ref()
                        .unwrap()
                        .target,
                    hostile.then_some(attacker)
                );
            }
            if !hostile {
                assert_eq!(health(&s, attacker), Some(2500));
            }
        }
    }
}

#[test]
fn explicit_attack_still_damages_owned_and_same_team_units_under_hold_fire() {
    for owner in [0, 1] {
        let mut s = sim(&team_scenario(vec![Some(3), Some(3)]));
        let a = place(&mut s, SWORD, 0, 1000, 1000);
        let target = place(&mut s, DUMMY_TARGET, owner, 1020, 1000);
        issue(
            &mut s,
            0,
            vec![
                fire_state(a, FireState::HoldFire),
                attack(&[a], Target::Unit(target)),
            ],
        );
        assert_eq!(health(&s, target), Some(460));
        assert_eq!(
            s.world().unit(a).unwrap().combat.order,
            Some(tak_sim::combat::CombatOrder::Attack(Target::Unit(target)))
        );
    }
}

#[test]
fn explicit_ground_attack_and_splash_still_damage_same_team_units() {
    let mut sc = team_scenario(vec![Some(3), Some(3)]);
    sc.unit_defs[ARCHER.0 as usize].combat.weapons[0] = combat_common::ballistic(400, 450, 0, 100);
    let mut s = sim(&sc);
    let a = place(&mut s, ARCHER, 0, 1000, 1000);
    let direct = place(&mut s, DUMMY_TARGET, 1, 1200, 1000);
    let splash = place(&mut s, DUMMY_TARGET, 0, 1200, 1030);
    issue(
        &mut s,
        0,
        vec![
            fire_state(a, FireState::HoldFire),
            attack(&[a], Target::Ground(Vec2::from_ints(1200, 1000))),
        ],
    );
    assert_eq!(health(&s, direct), Some(600));
    assert_eq!(
        health(&s, splash),
        Some(1000 - tak_sim::combat::area_damage(400, 22, 50, Fx::HALF))
    );
}

#[test]
fn explicit_spell_still_targets_same_team_units_under_hold_fire() {
    let mut sc = team_scenario(vec![Some(3), Some(3)]);
    let caster_def = &mut sc.unit_defs[ARCHER.0 as usize];
    caster_def.combat.weapons[0] = combat_common::ballistic(400, 450, 0, 0);
    caster_def.combat.weapons[0].mana_per_shot = 1;
    caster_def.economy.maxmana = 10;
    let mut s = sim(&sc);
    let caster = place(&mut s, ARCHER, 0, 1000, 1000);
    let ally = place(&mut s, DUMMY_TARGET, 1, 1200, 1000);
    tak_sim::economy::set_unit_mana_percent(s.world_mut(), caster, 100);
    issue(
        &mut s,
        0,
        vec![
            fire_state(caster, FireState::HoldFire),
            SimCommand::CastSpell {
                units: UnitSet::single(caster),
                spell: tak_commands::SpellId(0),
                target: Some(Target::Unit(ally)),
                queue: QueueMode::Replace,
            },
        ],
    );
    assert_eq!(health(&s, ally), Some(600));
    assert_eq!(
        s.world().unit(caster).unwrap().economy.mana(),
        Fx::from_int(9)
    );
}

#[test]
fn automatic_protection_and_skirmish_winners_share_team_membership() {
    let mut sc = team_scenario(vec![Some(3), Some(3), Some(4), Some(3)]);
    sc.unit_defs[DUMMY.0 as usize].combat =
        combat_common::fighter("king", 1000, "human", vec![]).combat;
    let mut s = sim(&sc);
    let a = place(&mut s, SWORD, 0, 1000, 1000);
    let ally = place(&mut s, DUMMY_TARGET, 1, 1020, 1000);
    let enemy = place(&mut s, DUMMY_TARGET, 2, 1000, 1020);
    s.step(&[]);
    assert_eq!(
        s.world().unit(a).unwrap().combat.weapons[0].target,
        Some(Target::Unit(enemy))
    );
    assert_eq!(health(&s, ally), Some(1000));
    let monarch = s.world().players()[2].monarch.unwrap();
    let now = s.tick().0;
    tak_sim::combat::apply_damage(
        s.world_mut(),
        &mut tak_sim::NoScripts,
        monarch,
        1000,
        None,
        tak_sim::combat::kind::NORMAL,
        Vec2::ZERO,
        now,
    );
    s.step(&[]);
    assert_eq!(
        s.match_state(),
        &tak_sim::MatchState::Over {
            winners: vec![PlayerId(0), PlayerId(1), PlayerId(3)]
        }
    );
    assert_eq!(health(&s, ally), Some(1000));
}

#[test]
fn invalid_owner_units_are_not_automatic_targets_or_attackers() {
    let mut s = sim(&team_scenario(vec![]));
    let a = place(&mut s, SWORD, 0, 1000, 1000);
    let target = place(&mut s, DUMMY_TARGET, 1, 1020, 1000);
    s.world_mut().unit_mut(target).unwrap().owner = PlayerId(255);
    s.step(&[]);
    assert_eq!(health(&s, target), Some(1000));
    assert!(
        s.world().unit(a).unwrap().combat.weapons[0]
            .target
            .is_none()
    );
    s.world_mut().unit_mut(a).unwrap().owner = PlayerId(255);
    s.world_mut().unit_mut(target).unwrap().owner = PlayerId(1);
    s.step(&[]);
    assert_eq!(health(&s, target), Some(1000));
    assert!(
        s.world().unit(a).unwrap().combat.weapons[0]
            .target
            .is_none()
    );
}

const CHILD_ENV: &str = "OXA_TEAM_TARGETING_CHILD_OUT";

fn determinism_hashes() -> String {
    let mut text = String::new();
    for teams in [
        vec![],
        vec![None; 4],
        vec![Some(0), Some(0), Some(255), Some(255)],
        vec![Some(255), None, Some(255)],
    ] {
        for hash in mixed_hashes(teams) {
            text.push_str(&format!("{hash:016x}\n"));
        }
    }
    text
}

#[test]
fn child_writes_team_targeting_hashes() {
    if let Ok(path) = std::env::var(CHILD_ENV) {
        std::fs::write(path, determinism_hashes()).unwrap();
    }
}

#[test]
fn team_targeting_hashes_agree_across_processes() {
    let dir = std::env::temp_dir().join(format!("oxa-team-targeting-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hashes.txt");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "child_writes_team_targeting_hashes",
            "--exact",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, &path)
        .status()
        .unwrap();
    assert!(status.success());
    let child = std::fs::read_to_string(path).unwrap();
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(child.lines().count(), 2396);
    assert_eq!(
        child,
        determinism_hashes(),
        "team targeting diverged across processes"
    );
}
