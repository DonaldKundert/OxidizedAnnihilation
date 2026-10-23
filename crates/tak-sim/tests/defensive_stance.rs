//! Kingdoms Defensive is distinct from legacy ReturnFire. All fixtures are
//! synthetic; manual prose is documentary evidence, not a runtime observation.

mod combat_common;

use combat_common::{ARCHER, DUMMY_TARGET, SWORD, attack, health, place, scenario, sim};
use tak_commands::{
    CommandLog, FireState, LoggedCommand, PlayerId, QueueMode, SimCommand, SpellId, Target, Tick,
    UnitId, UnitSet,
};
use tak_math::{Fx, Vec2};
use tak_sim::{Order, Sim, systems::ARRIVAL_TOLERANCE};

fn stance(id: UnitId, state: FireState) -> SimCommand {
    SimCommand::SetFireState {
        units: UnitSet::single(id),
        state,
    }
}

fn walk(id: UnitId, to: Vec2) -> SimCommand {
    SimCommand::Move {
        units: UnitSet::single(id),
        to,
        queue: QueueMode::Replace,
    }
}

fn attack_move(id: UnitId, to: Vec2) -> SimCommand {
    SimCommand::AttackMove {
        units: UnitSet::single(id),
        to,
        queue: QueueMode::Replace,
    }
}

fn issue(s: &mut Sim, commands: Vec<SimCommand>) {
    let commands: Vec<_> = commands
        .into_iter()
        .enumerate()
        .map(|(seq, command)| LoggedCommand {
            tick: s.tick(),
            player: PlayerId(0),
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

#[test]
fn defensive_fires_unprovoked_in_reach_without_position_drift() {
    for (def, distance) in [(ARCHER, 200), (SWORD, 24)] {
        let mut s = sim(&scenario(501));
        let a = place(&mut s, def, 0, 500, 500);
        let target = place(&mut s, DUMMY_TARGET, 1, 500 + distance, 500);
        let start = s.world().unit(a).unwrap().pos;
        issue(&mut s, vec![stance(a, FireState::Defensive)]);
        for _ in 0..120 {
            let u = s.world().unit(a).unwrap();
            assert_eq!(u.pos, start);
            assert_eq!(u.speed, Fx::ZERO);
            assert!(u.orders.is_empty());
            assert!(u.combat.goal.is_none());
            assert!(u.combat.leash_origin.is_none());
            s.step(&[]);
        }
        assert!(health(&s, target).is_none_or(|hp| hp < 1000));
    }
}

#[test]
fn defensive_never_chases_out_of_reach_or_unreachable_enemies() {
    for (def, distance) in [(ARCHER, 500), (SWORD, 150)] {
        let mut sc = scenario(502);
        // An impassable synthetic map makes the target unreachable as well.
        let mut grid = tak_sim::map::TerrainGrid {
            cols: 128,
            rows: 128,
            cell_size: 16,
            flags: vec![0; 128 * 128],
        };
        for row in 0..128 {
            grid.flags[row * 128 + 35] = tak_sim::map::terrain::FEATURE;
        }
        sc.map.terrain = Some(grid);
        let mut s = sim(&sc);
        let a = place(&mut s, def, 0, 500, 500);
        let target = place(&mut s, DUMMY_TARGET, 1, 500 + distance, 500);
        let start = s.world().unit(a).unwrap().pos;
        issue(&mut s, vec![stance(a, FireState::Defensive)]);
        steps(&mut s, 180);
        let u = s.world().unit(a).unwrap();
        assert_eq!(u.pos, start);
        assert!(u.orders.is_empty());
        assert!(u.combat.goal.is_none());
        assert!(u.combat.weapons.iter().all(|w| w.target.is_none()));
        assert_eq!(health(&s, target), Some(1000));
    }
}

#[test]
fn defensive_obeys_minimum_range_without_repositioning() {
    let mut sc = scenario(503);
    sc.unit_defs[ARCHER.0 as usize].combat.weapons[0].min_range = 100;
    let mut s = sim(&sc);
    let a = place(&mut s, ARCHER, 0, 500, 500);
    let target = place(&mut s, DUMMY_TARGET, 1, 550, 500);
    issue(&mut s, vec![stance(a, FireState::Defensive)]);
    steps(&mut s, 120);
    assert_eq!(s.world().unit(a).unwrap().pos, Vec2::from_ints(500, 500));
    assert_eq!(health(&s, target), Some(1000));
}

#[test]
fn defensive_move_and_patrol_follow_the_same_route_as_hold_fire() {
    for patrol in [false, true] {
        let run = |state| {
            let mut s = sim(&scenario(504));
            let a = place(&mut s, ARCHER, 0, 300, 400);
            place(&mut s, DUMMY_TARGET, 1, 400, 500);
            let to = Vec2::from_ints(900, 400);
            let command = if patrol {
                SimCommand::Patrol {
                    units: UnitSet::single(a),
                    to,
                    queue: QueueMode::Replace,
                }
            } else {
                walk(a, to)
            };
            issue(&mut s, vec![stance(a, state), command]);
            (0..300)
                .map(|_| {
                    s.step(&[]);
                    let u = s.world().unit(a).unwrap();
                    (u.pos, u.orders.clone())
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(FireState::Defensive), run(FireState::HoldFire));
    }
}

#[test]
fn explicit_attack_and_cast_override_defensive_and_team_protection() {
    for spell in [false, true] {
        let mut sc = scenario(505);
        sc.victory.teams = vec![Some(7), Some(7)];
        if spell {
            let d = &mut sc.unit_defs[ARCHER.0 as usize];
            d.combat.weapons[0].mana_per_shot = 1;
            d.economy.maxmana = 10;
        }
        let mut s = sim(&sc);
        let a = place(&mut s, ARCHER, 0, 300, 400);
        let ally = place(&mut s, DUMMY_TARGET, 1, 1000, 400);
        tak_sim::economy::set_unit_mana_percent(s.world_mut(), a, 100);
        let command = if spell {
            SimCommand::CastSpell {
                units: UnitSet::single(a),
                spell: SpellId(0),
                target: Some(Target::Unit(ally)),
                queue: QueueMode::Replace,
            }
        } else {
            attack(&[a], Target::Unit(ally))
        };
        issue(&mut s, vec![stance(a, FireState::Defensive), command]);
        steps(&mut s, 500);
        assert!(s.world().unit(a).unwrap().pos.x > Fx::from_int(300));
        assert!(health(&s, ally).is_none_or(|hp| hp < 1000));
        if spell {
            assert!(s.world().unit(a).unwrap().economy.mana() < Fx::from_int(10));
        }
    }
}

#[test]
fn defensive_filters_teams_and_discards_a_newly_friendly_target() {
    for allied in [false, true] {
        let mut sc = scenario(506);
        sc.victory.teams = if allied {
            vec![Some(7), Some(7)]
        } else {
            vec![]
        };
        let mut s = sim(&sc);
        let a = place(&mut s, ARCHER, 0, 500, 500);
        let target = place(&mut s, DUMMY_TARGET, 1, 600, 500);
        issue(&mut s, vec![stance(a, FireState::Defensive)]);
        assert_eq!(
            s.world().unit(a).unwrap().combat.weapons[0]
                .target
                .is_some(),
            !allied
        );
        s.world_mut().unit_mut(target).unwrap().owner = PlayerId(0);
        s.step(&[]);
        assert!(
            s.world().unit(a).unwrap().combat.weapons[0]
                .target
                .is_none()
        );
    }
}

#[test]
fn defensive_guard_follows_its_charge_but_does_not_chase_enemies() {
    let mut s = sim(&scenario(507));
    let a = place(&mut s, SWORD, 0, 500, 500);
    let charge = place(&mut s, DUMMY_TARGET, 0, 550, 500);
    place(&mut s, DUMMY_TARGET, 1, 650, 500);
    issue(
        &mut s,
        vec![
            stance(a, FireState::Defensive),
            SimCommand::Guard {
                units: UnitSet::single(a),
                target: charge,
                queue: QueueMode::Replace,
            },
        ],
    );
    steps(&mut s, 90);
    assert_eq!(s.world().unit(a).unwrap().pos, Vec2::from_ints(500, 500));
    s.world_mut().unit_mut(charge).unwrap().pos = Vec2::from_ints(800, 500);
    steps(&mut s, 300);
    assert!(s.world().unit(a).unwrap().pos.x > Fx::from_int(500));
}

#[test]
fn switching_to_defensive_drops_automatic_chase_and_return_origin() {
    let mut s = sim(&scenario(508));
    let a = place(&mut s, SWORD, 0, 500, 500);
    place(&mut s, DUMMY_TARGET, 1, 650, 500);
    steps(&mut s, 30);
    let u = s.world().unit(a).unwrap();
    assert!(u.combat.goal.is_some());
    assert!(u.combat.leash_origin.is_some());
    let at_switch = u.pos;
    issue(&mut s, vec![stance(a, FireState::Defensive)]);
    steps(&mut s, 120);
    let u = s.world().unit(a).unwrap();
    assert_eq!(u.pos, at_switch); // this fixture brakes fully in one tick
    assert!(u.orders.is_empty());
    assert!(u.combat.goal.is_none());
    assert!(u.combat.leash_origin.is_none());
}

#[test]
fn defensive_attack_move_stops_for_in_range_unprovoked_enemy_then_resumes() {
    let mut s = sim(&scenario(509));
    let a = place(&mut s, ARCHER, 0, 300, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 400, 450);
    let to = Vec2::from_ints(900, 400);
    issue(
        &mut s,
        vec![stance(a, FireState::Defensive), attack_move(a, to)],
    );
    let u = s.world().unit(a).unwrap();
    assert_eq!(u.orders.front(), Some(&Order::AttackMove { to }));
    assert_eq!(u.combat.attack_move.as_ref().unwrap().target, Some(enemy));
    assert!(u.combat.attack_move.as_ref().unwrap().goal.is_none());
    steps(&mut s, 1500);
    assert!(s.world().unit(enemy).is_none());
    let u = s.world().unit(a).unwrap();
    assert!(u.orders.is_empty());
    assert!(u.pos.distance(to) <= ARRIVAL_TOLERANCE);
}

#[test]
fn defensive_attack_move_never_diverts_and_releases_escaped_target() {
    let mut s = sim(&scenario(510));
    let a = place(&mut s, SWORD, 0, 300, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 324, 400);
    let to = Vec2::from_ints(900, 400);
    issue(
        &mut s,
        vec![stance(a, FireState::Defensive), attack_move(a, to)],
    );
    assert_eq!(
        s.world()
            .unit(a)
            .unwrap()
            .combat
            .attack_move
            .as_ref()
            .unwrap()
            .target,
        Some(enemy)
    );
    s.world_mut().unit_mut(enemy).unwrap().pos = Vec2::from_ints(500, 550);
    for _ in 0..800 {
        s.step(&[]);
        let u = s.world().unit(a).unwrap();
        assert_eq!(u.pos.y, Fx::from_int(400));
        if let Some(state) = &u.combat.attack_move {
            assert!(state.target.is_none());
            assert_eq!(state.goal, Some(to));
        }
    }
    assert!(s.world().unit(a).unwrap().orders.is_empty());
    assert_eq!(health(&s, enemy), Some(460));
}

#[test]
fn legacy_return_fire_stays_retaliation_and_still_chases_its_attacker_at_idle() {
    let mut s = sim(&scenario(511));
    let a = place(&mut s, SWORD, 0, 500, 500);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 650, 500);
    issue(&mut s, vec![stance(a, FireState::ReturnFire)]);
    steps(&mut s, 60);
    assert_eq!(s.world().unit(a).unwrap().pos, Vec2::from_ints(500, 500));
    assert_eq!(health(&s, enemy), Some(1000));
    s.world_mut().unit_mut(a).unwrap().combat.last_attacker = Some(enemy);
    steps(&mut s, 90);
    assert!(s.world().unit(a).unwrap().pos.x > Fx::from_int(500));
}

#[test]
fn defensive_attack_move_reselection_is_idempotent_and_preserves_the_tail() {
    let mut s = sim(&scenario(513));
    let a = place(&mut s, ARCHER, 0, 300, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 400, 450);
    let to = Vec2::from_ints(900, 400);
    let tail = Vec2::from_ints(900, 600);
    issue(
        &mut s,
        vec![
            stance(a, FireState::Defensive),
            attack_move(a, to),
            SimCommand::Move {
                units: UnitSet::single(a),
                to: tail,
                queue: QueueMode::Append,
            },
        ],
    );
    let at_rest = s.world().unit(a).unwrap().pos;
    issue(&mut s, vec![stance(a, FireState::Defensive)]);
    let u = s.world().unit(a).unwrap();
    assert_eq!(u.pos, at_rest);
    assert_eq!(
        u.orders.iter().copied().collect::<Vec<_>>(),
        vec![Order::AttackMove { to }, Order::Move { to: tail }]
    );
    assert_eq!(u.combat.attack_move.as_ref().unwrap().target, Some(enemy));
    assert!(u.combat.attack_move.as_ref().unwrap().goal.is_none());
}

#[test]
fn switching_a_pursuing_attack_move_to_defensive_keeps_only_destination_steering() {
    let mut s = sim(&scenario(514));
    let a = place(&mut s, SWORD, 0, 300, 400);
    place(&mut s, DUMMY_TARGET, 1, 400, 550);
    let to = Vec2::from_ints(900, 400);
    let tail = Vec2::from_ints(900, 600);
    issue(
        &mut s,
        vec![
            attack_move(a, to),
            SimCommand::Move {
                units: UnitSet::single(a),
                to: tail,
                queue: QueueMode::Append,
            },
        ],
    );
    steps(&mut s, 20);
    assert!(
        s.world()
            .unit(a)
            .unwrap()
            .combat
            .attack_move
            .as_ref()
            .unwrap()
            .target
            .is_some()
    );
    issue(&mut s, vec![stance(a, FireState::Defensive)]);
    let u = s.world().unit(a).unwrap();
    assert_eq!(
        u.orders.iter().copied().collect::<Vec<_>>(),
        vec![Order::AttackMove { to }, Order::Move { to: tail }]
    );
    assert!(u.combat.attack_move.as_ref().unwrap().target.is_none());
    assert_eq!(u.combat.attack_move.as_ref().unwrap().goal, Some(to));
    steps(&mut s, 1300);
    let u = s.world().unit(a).unwrap();
    assert!(u.orders.is_empty());
    assert!(u.pos.distance(tail) <= ARRIVAL_TOLERANCE);
}

#[test]
fn defensive_does_not_inherit_failed_offensive_pursuit_avoidance() {
    let mut s = sim(&scenario(515));
    let a = place(&mut s, ARCHER, 0, 300, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 400, 450);
    let to = Vec2::from_ints(900, 400);
    issue(&mut s, vec![attack_move(a, to)]);
    // A prior Offensive engagement can have failed to find an enemy path.
    let enemy_pos = s.world().unit(enemy).unwrap().pos;
    let state = s
        .world_mut()
        .unit_mut(a)
        .unwrap()
        .combat
        .attack_move
        .as_mut()
        .unwrap();
    state.target = None;
    state.goal = Some(to);
    state.avoided.insert(enemy, enemy_pos);
    issue(&mut s, vec![stance(a, FireState::Defensive)]);
    let state = s
        .world()
        .unit(a)
        .unwrap()
        .combat
        .attack_move
        .as_ref()
        .unwrap();
    assert_eq!(state.target, Some(enemy));
    assert!(state.goal.is_none());
    assert!(state.avoided.is_empty());
}

#[test]
fn switching_to_defensive_preserves_a_traveling_guard_and_queued_move() {
    let mut s = sim(&scenario(516));
    let a = place(&mut s, SWORD, 0, 300, 400);
    let charge = place(&mut s, DUMMY_TARGET, 0, 900, 400);
    let tail = Vec2::from_ints(900, 600);
    issue(
        &mut s,
        vec![
            SimCommand::Guard {
                units: UnitSet::single(a),
                target: charge,
                queue: QueueMode::Replace,
            },
            SimCommand::Move {
                units: UnitSet::single(a),
                to: tail,
                queue: QueueMode::Append,
            },
        ],
    );
    steps(&mut s, 30);
    let before = s.world().unit(a).unwrap().pos;
    issue(&mut s, vec![stance(a, FireState::Defensive)]);
    steps(&mut s, 300);
    let u = s.world().unit(a).unwrap();
    assert!(u.pos.x > before.x);
    assert_eq!(
        u.combat.order,
        Some(tak_sim::combat::CombatOrder::Guard(charge))
    );
    assert_eq!(u.orders.back(), Some(&Order::Move { to: tail }));
}

#[test]
fn installed_archer_script_fires_under_defensive_without_travel() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("defensive_archer_script") else {
        return;
    };
    let (mut s, defs, map) = combat_common::real_match(&dir, &["ARAARCH", "ZONTROLL"], 517);
    let origin = combat_common::open_area(&s, &map, 256, 256, 700);
    let a = s
        .spawn_unit(defs[0], PlayerId(0), origin, tak_math::Angle::ZERO)
        .unwrap();
    let enemy = s
        .spawn_unit(
            defs[1],
            PlayerId(1),
            origin + Vec2::from_ints(150, 0),
            tak_math::Angle::ZERO,
        )
        .unwrap();
    let tick = s.tick();
    s.step(&[
        LoggedCommand {
            tick,
            player: PlayerId(0),
            seq: 0,
            command: stance(a, FireState::Defensive),
        },
        LoggedCommand {
            tick,
            player: PlayerId(1),
            seq: 0,
            command: stance(enemy, FireState::HoldFire),
        },
    ]);
    let initial_hp = health(&s, enemy).unwrap();
    for _ in 0..300 {
        assert_eq!(s.world().unit(a).unwrap().pos, origin);
        s.step(&[]);
    }
    assert!(health(&s, enemy).is_none_or(|hp| hp < initial_hp));
    let stats = s.script_stats();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
}

#[test]
fn defensive_in_reach_engagement_needs_neither_sight_leash_nor_enemy_path() {
    let mut sc = scenario(518);
    sc.unit_defs[SWORD.0 as usize].combat.sight = 4;
    sc.unit_defs[SWORD.0 as usize].combat.leash = 4;
    let mut s = sim(&sc);
    let a = place(&mut s, SWORD, 0, 300, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 324, 400);
    let to = Vec2::from_ints(900, 400);
    issue(
        &mut s,
        vec![stance(a, FireState::Defensive), attack_move(a, to)],
    );
    assert_eq!(
        s.world()
            .unit(a)
            .unwrap()
            .combat
            .attack_move
            .as_ref()
            .unwrap()
            .target,
        Some(enemy)
    );
    let enemy_pos = s.world().unit(enemy).unwrap().pos;
    let state = s
        .world_mut()
        .unit_mut(a)
        .unwrap()
        .combat
        .attack_move
        .as_mut()
        .unwrap();
    state.origin = Vec2::ZERO;
    state.avoided.insert(enemy, enemy_pos);
    s.step(&[]);
    let state = s
        .world()
        .unit(a)
        .unwrap()
        .combat
        .attack_move
        .as_ref()
        .unwrap();
    assert_eq!(state.target, Some(enemy));
    assert!(state.goal.is_none());
}

#[test]
fn braking_defensive_attack_move_retains_a_moving_enemy_still_in_reach() {
    let mut sc = scenario(519);
    let d = &mut sc.unit_defs[ARCHER.0 as usize];
    d.maxvelocity = Fx::from_int(4);
    d.acceleration = Fx::from_int(4);
    d.brakerate = Fx::from_ratio(1, 4).unwrap();
    let mut s = sim(&sc);
    let a = place(&mut s, ARCHER, 0, 300, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 754, 400);
    issue(
        &mut s,
        vec![
            stance(a, FireState::Defensive),
            attack_move(a, Vec2::from_ints(1500, 400)),
        ],
    );
    let acquired_at = s.world().unit(a).unwrap().pos;
    assert_eq!(
        s.world()
            .unit(a)
            .unwrap()
            .combat
            .attack_move
            .as_ref()
            .unwrap()
            .target,
        Some(enemy)
    );
    s.world_mut().unit_mut(enemy).unwrap().pos.x += Fx::ONE;
    s.step(&[]);
    let u = s.world().unit(a).unwrap();
    assert!(u.pos.x > acquired_at.x, "normal braking still applies");
    let state = u.combat.attack_move.as_ref().unwrap();
    assert_eq!(state.target, Some(enemy));
    assert!(state.goal.is_none());
    assert!(state.avoided.is_empty());
}

fn replay_hashes() -> Vec<u64> {
    let mut sc = scenario(512);
    sc.victory.teams = vec![Some(0), Some(1)];
    let mut s = sim(&sc);
    let a = place(&mut s, ARCHER, 0, 300, 400);
    let b = place(&mut s, SWORD, 0, 300, 600);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 450, 450);
    let mut log = CommandLog::new();
    for (tick, command) in [
        (1, stance(a, FireState::Defensive)),
        (1, stance(b, FireState::Defensive)),
        (2, attack_move(a, Vec2::from_ints(950, 400))),
        (2, walk(b, Vec2::from_ints(600, 600))),
        (180, stance(b, FireState::ReturnFire)),
        (200, attack(&[b], Target::Unit(enemy))),
        (240, stance(b, FireState::Defensive)),
        (300, stance(a, FireState::HoldFire)),
        (360, stance(a, FireState::FireAtWill)),
    ] {
        log.record(Tick(tick), PlayerId(0), command);
    }
    // Exercise the on-disk command format rather than just direct commands.
    let log = CommandLog::from_json(&log.to_json()).unwrap();
    (1..900)
        .map(|_| {
            let tick = s.tick();
            s.step(log.for_tick(tick))
        })
        .collect()
}

const CHILD_ENV: &str = "OXA_DEFENSIVE_STANCE_CHILD_OUT";

#[test]
fn child_writes_defensive_replay_hashes() {
    if let Ok(path) = std::env::var(CHILD_ENV) {
        let text: String = replay_hashes()
            .into_iter()
            .map(|h| format!("{h:016x}\n"))
            .collect();
        std::fs::write(path, text).unwrap();
    }
}

#[test]
fn defensive_replay_matches_across_processes() {
    let dir = std::env::temp_dir().join(format!("oxa-defensive-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hashes.txt");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "child_writes_defensive_replay_hashes",
            "--exact",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, &path)
        .status()
        .unwrap();
    assert!(status.success());
    let actual = std::fs::read_to_string(&path).unwrap();
    let expected: String = replay_hashes()
        .into_iter()
        .map(|h| format!("{h:016x}\n"))
        .collect();
    assert_eq!(actual.lines().count(), 899);
    assert_eq!(actual, expected);
    std::fs::remove_dir_all(dir).unwrap();
}
