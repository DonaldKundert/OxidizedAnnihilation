//! Attack-move uses a durable order and separate hashed engagement steering.
//! All definitions and terrain here are synthetic, with no game data.

mod combat_common;

use combat_common::{DUMMY, DUMMY_TARGET, SWORD, health, place, scenario, sim};
use tak_commands::{
    FireState, LoggedCommand, PlayerId, QueueMode, SimCommand, Target, UnitId, UnitSet,
};
use tak_math::{Fx, Vec2};
use tak_sim::combat::CombatOrder;
use tak_sim::map::{TerrainGrid, terrain};
use tak_sim::systems::ARRIVAL_TOLERANCE;
use tak_sim::{Order, Sim, WorldPos};

fn attack_move(id: UnitId, to: WorldPos, queue: QueueMode) -> SimCommand {
    SimCommand::AttackMove {
        units: UnitSet::single(id),
        to,
        queue,
    }
}

fn walk(id: UnitId, to: WorldPos, queue: QueueMode) -> SimCommand {
    SimCommand::Move {
        units: UnitSet::single(id),
        to,
        queue,
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

fn until(s: &mut Sim, limit: usize, mut done: impl FnMut(&Sim) -> bool) {
    for _ in 0..limit {
        if done(s) {
            return;
        }
        s.step(&[]);
    }
    assert!(done(s), "condition not met after {limit} ticks");
}

#[test]
fn engages_kills_and_resumes_without_losing_queued_orders() {
    let mut s = sim(&scenario(31));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 360, 440);
    let destination = Vec2::from_ints(700, 400);
    let tail = Vec2::from_ints(700, 650);
    let end = Vec2::from_ints(900, 650);
    issue(
        &mut s,
        vec![
            attack_move(a, destination, QueueMode::Replace),
            walk(a, tail, QueueMode::Append),
            attack_move(a, end, QueueMode::Append),
        ],
    );
    let u = s.world().unit(a).unwrap();
    assert_eq!(u.combat.attack_move.as_ref().unwrap().target, Some(enemy));
    assert_eq!(
        u.orders.iter().copied().collect::<Vec<_>>(),
        vec![
            Order::AttackMove { to: destination },
            Order::Move { to: tail },
            Order::AttackMove { to: end },
        ]
    );
    until(&mut s, 700, |s| {
        let u = s.world().unit(a).unwrap();
        if s.world().unit(enemy).is_some_and(|e| !e.combat.is_dying()) {
            assert_eq!(
                u.orders.front(),
                Some(&Order::AttackMove { to: destination })
            );
            assert_eq!(u.orders.len(), 3, "engagement must not replace the queue");
        }
        s.world().unit(enemy).is_none()
    });
    until(&mut s, 1600, |s| {
        s.world().unit(a).unwrap().orders.is_empty()
    });
    let u = s.world().unit(a).unwrap();
    assert!(u.pos.distance(end) <= ARRIVAL_TOLERANCE, "{:?}", u.pos);
    assert!(u.combat.attack_move.is_none());
}

#[test]
fn appended_attack_move_waits_for_the_existing_move() {
    let mut s = sim(&scenario(32));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 350, 450);
    let waypoint = Vec2::from_ints(400, 400);
    let destination = Vec2::from_ints(700, 400);
    issue(
        &mut s,
        vec![
            walk(a, waypoint, QueueMode::Replace),
            attack_move(a, destination, QueueMode::Append),
        ],
    );
    for _ in 0..100 {
        let u = s.world().unit(a).unwrap();
        assert_eq!(u.orders.front(), Some(&Order::Move { to: waypoint }));
        assert!(u.combat.attack_move.is_none());
        s.step(&[]);
    }
    until(&mut s, 1600, |s| {
        s.world().unit(a).unwrap().orders.is_empty()
    });
    assert!(s.world().unit(enemy).is_none());
    assert!(s.world().unit(a).unwrap().pos.distance(destination) <= ARRIVAL_TOLERANCE);
}

#[test]
fn combat_closing_preserves_appended_attack_move_and_move() {
    let mut s = sim(&scenario(33));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 400, 400);
    let destination = Vec2::from_ints(700, 400);
    let end = Vec2::from_ints(700, 600);
    issue(
        &mut s,
        vec![
            combat_common::attack(&[a], Target::Unit(enemy)),
            attack_move(a, destination, QueueMode::Append),
            walk(a, end, QueueMode::Append),
        ],
    );
    for _ in 0..100 {
        let u = s.world().unit(a).unwrap();
        assert_eq!(
            u.combat.order,
            Some(CombatOrder::Attack(Target::Unit(enemy)))
        );
        assert_eq!(u.orders.back(), Some(&Order::Move { to: end }));
        assert!(u.orders.contains(&Order::AttackMove { to: destination }));
        assert!(u.combat.attack_move.is_none());
        s.step(&[]);
    }
    until(&mut s, 1800, |s| {
        s.world().unit(a).unwrap().orders.is_empty()
    });
    assert!(s.world().unit(enemy).is_none());
    assert!(s.world().unit(a).unwrap().pos.distance(end) <= ARRIVAL_TOLERANCE);
}

#[test]
fn queued_attack_starts_only_after_attack_move_arrives() {
    let mut s = sim(&scenario(34));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 850, 400);
    let destination = Vec2::from_ints(500, 400);
    issue(
        &mut s,
        vec![
            attack_move(a, destination, QueueMode::Replace),
            SimCommand::Attack {
                units: UnitSet::single(a),
                target: Target::Unit(enemy),
                queue: QueueMode::Append,
            },
        ],
    );
    until(&mut s, 600, |s| {
        let u = s.world().unit(a).unwrap();
        if matches!(u.orders.front(), Some(Order::AttackMove { .. })) {
            assert!(u.combat.order.is_none());
            false
        } else {
            assert!(u.pos.distance(destination) <= ARRIVAL_TOLERANCE);
            true
        }
    });
    until(&mut s, 1000, |s| s.world().unit(enemy).is_none());
}

#[test]
fn hold_and_return_fire_do_not_acquire_unprovoked_enemies() {
    for state in [FireState::HoldFire, FireState::ReturnFire] {
        let mut s = sim(&scenario(35));
        let a = place(&mut s, SWORD, 0, 200, 400);
        let enemy = place(&mut s, DUMMY_TARGET, 1, 350, 450);
        let destination = Vec2::from_ints(700, 400);
        issue(
            &mut s,
            vec![
                SimCommand::SetFireState {
                    units: UnitSet::single(a),
                    state,
                },
                attack_move(a, destination, QueueMode::Replace),
            ],
        );
        until(&mut s, 800, |s| {
            s.world().unit(a).unwrap().orders.is_empty()
        });
        assert_eq!(health(&s, enemy), Some(1000));
        assert!(s.world().unit(a).unwrap().pos.distance(destination) <= ARRIVAL_TOLERANCE);
    }
}

#[test]
fn return_fire_engages_the_last_attacker_only_in_weapon_reach() {
    let mut s = sim(&scenario(36));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 224, 416);
    s.world_mut().unit_mut(a).unwrap().combat.last_attacker = Some(enemy);
    let destination = Vec2::from_ints(700, 400);
    issue(
        &mut s,
        vec![
            SimCommand::SetFireState {
                units: UnitSet::single(a),
                state: FireState::ReturnFire,
            },
            attack_move(a, destination, QueueMode::Replace),
        ],
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
    until(&mut s, 1400, |s| {
        s.world().unit(a).unwrap().orders.is_empty()
    });
    assert!(s.world().unit(enemy).is_none());
}

#[test]
fn stop_move_attack_and_patrol_replace_cancel_attack_move() {
    for replacement in 0..4 {
        let mut s = sim(&scenario(37));
        let a = place(&mut s, SWORD, 0, 200, 400);
        let enemy = place(&mut s, DUMMY_TARGET, 1, 350, 450);
        issue(
            &mut s,
            vec![attack_move(
                a,
                Vec2::from_ints(700, 400),
                QueueMode::Replace,
            )],
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
        let units = UnitSet::single(a);
        issue(
            &mut s,
            vec![match replacement {
                0 => SimCommand::Stop { units },
                1 => walk(a, Vec2::from_ints(100, 400), QueueMode::Replace),
                2 => combat_common::attack(&[a], Target::Ground(Vec2::from_ints(100, 400))),
                _ => SimCommand::Patrol {
                    units,
                    to: Vec2::from_ints(100, 400),
                    queue: QueueMode::Replace,
                },
            }],
        );
        let u = s.world().unit(a).unwrap();
        assert!(u.combat.attack_move.is_none());
        assert!(
            !u.orders
                .iter()
                .any(|o| matches!(o, Order::AttackMove { .. }))
        );
    }
}

#[test]
fn nearest_target_ties_are_stable_and_stale_generations_are_not_reused() {
    let mut s = sim(&scenario(38));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let first = place(&mut s, DUMMY_TARGET, 1, 300, 450);
    let second = place(&mut s, DUMMY_TARGET, 1, 300, 350);
    issue(
        &mut s,
        vec![attack_move(
            a,
            Vec2::from_ints(700, 400),
            QueueMode::Replace,
        )],
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
        Some(first)
    );
    s.world_mut().despawn(first).unwrap();
    let reused = place(&mut s, DUMMY_TARGET, 0, 300, 450);
    assert_eq!(first.index, reused.index);
    assert_ne!(first.generation, reused.generation);
    s.step(&[]);
    assert_eq!(
        s.world()
            .unit(a)
            .unwrap()
            .combat
            .attack_move
            .as_ref()
            .unwrap()
            .target,
        Some(second)
    );
}

#[test]
fn ownership_clamping_and_queue_limits_apply() {
    let mut s = sim(&scenario(39));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 350, 450);
    let units: UnitSet = [a, enemy, UnitId::new(a.index, a.generation + 1)]
        .into_iter()
        .collect();
    issue(
        &mut s,
        vec![SimCommand::AttackMove {
            units,
            to: Vec2::from_ints(9000, -20),
            queue: QueueMode::Replace,
        }],
    );
    let to = s.world().map().clamp(Vec2::from_ints(9000, -20));
    assert_eq!(
        s.world().unit(a).unwrap().orders.front(),
        Some(&Order::AttackMove { to })
    );
    assert!(s.world().unit(enemy).unwrap().orders.is_empty());
    issue(
        &mut s,
        (0..300)
            .map(|_| attack_move(a, to, QueueMode::Append))
            .collect(),
    );
    assert_eq!(
        s.world().unit(a).unwrap().orders.len(),
        tak_sim::world::MAX_ORDERS
    );
}

#[test]
fn noncombat_units_can_attack_move_as_a_plain_move() {
    let mut s = sim(&scenario(40));
    let a = place(&mut s, DUMMY, 0, 200, 400);
    let destination = Vec2::from_ints(400, 400);
    issue(
        &mut s,
        vec![attack_move(a, destination, QueueMode::Replace)],
    );
    until(&mut s, 800, |s| {
        s.world().unit(a).unwrap().orders.is_empty()
    });
    assert!(s.world().unit(a).unwrap().pos.distance(destination) <= ARRIVAL_TOLERANCE);
}

#[test]
fn unreachable_engagement_resumes_destination_and_keeps_tail() {
    let mut sc = scenario(41);
    let (cols, rows) = (128, 128);
    let mut flags = vec![0; (cols * rows) as usize];
    for row in 0..rows {
        flags[(row * cols + 30) as usize] = terrain::FEATURE;
    }
    sc.map.terrain = Some(TerrainGrid {
        cols,
        rows,
        cell_size: 16,
        flags,
    });
    let mut s = sim(&sc);
    let a = place(&mut s, SWORD, 0, 400, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 550, 400);
    let other_enemy = place(&mut s, DUMMY_TARGET, 1, 550, 460);
    let destination = Vec2::from_ints(400, 750);
    let end = Vec2::from_ints(300, 750);
    issue(
        &mut s,
        vec![
            attack_move(a, destination, QueueMode::Replace),
            walk(a, end, QueueMode::Append),
        ],
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
    until(&mut s, 2000, |s| {
        s.world().unit(a).unwrap().orders.is_empty()
    });
    assert_eq!(health(&s, enemy), Some(1000));
    assert_eq!(health(&s, other_enemy), Some(1000));
    assert!(s.world().unit(a).unwrap().pos.distance(end) <= ARRIVAL_TOLERANCE);
    assert!(s.world().unit(a).unwrap().pos.x < Fx::from_int(480));
}

#[test]
fn attack_move_order_and_each_steering_field_are_hashed() {
    let mut s = sim(&scenario(42));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 350, 450);
    let destination = Vec2::from_ints(700, 400);
    issue(
        &mut s,
        vec![attack_move(a, destination, QueueMode::Replace)],
    );
    let original = s
        .world()
        .unit(a)
        .unwrap()
        .combat
        .attack_move
        .as_ref()
        .unwrap()
        .clone();
    let hash = s.state_hash();
    for field in 0..6 {
        let mut changed = original.clone();
        match field {
            0 => changed.destination.x += Fx::ONE,
            1 => changed.target = None,
            2 => changed.origin.x += Fx::ONE,
            3 => changed.goal = None,
            4 => {
                changed.avoided.insert(enemy, Vec2::ZERO);
            }
            _ => {
                changed.avoided.insert(enemy, Vec2::from_ints(1, 0));
            }
        }
        s.world_mut().unit_mut(a).unwrap().combat.attack_move = Some(changed);
        assert_ne!(
            s.state_hash(),
            hash,
            "steering field {field} was not hashed"
        );
    }
    s.world_mut().unit_mut(a).unwrap().combat.attack_move = Some(original);
    assert_eq!(s.state_hash(), hash);
    *s.world_mut()
        .unit_mut(a)
        .unwrap()
        .orders
        .front_mut()
        .unwrap() = Order::Move { to: destination };
    assert_ne!(
        s.state_hash(),
        hash,
        "attack-move must have its own order hash tag"
    );
}

#[test]
fn return_fire_does_not_chase_its_attacker_off_the_route() {
    let mut s = sim(&scenario(43));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 350, 450);
    s.world_mut().unit_mut(a).unwrap().combat.last_attacker = Some(enemy);
    let destination = Vec2::from_ints(700, 400);
    issue(
        &mut s,
        vec![
            SimCommand::SetFireState {
                units: UnitSet::single(a),
                state: FireState::ReturnFire,
            },
            attack_move(a, destination, QueueMode::Replace),
        ],
    );
    until(&mut s, 800, |s| {
        let u = s.world().unit(a).unwrap();
        assert!(
            u.combat
                .attack_move
                .as_ref()
                .is_none_or(|a| a.target.is_none())
        );
        assert!((u.pos.y - Fx::from_int(400)).abs() < Fx::ONE);
        u.orders.is_empty()
    });
    assert_eq!(health(&s, enemy), Some(1000));
}

#[test]
fn explicit_attack_still_overrides_hold_fire() {
    let mut s = sim(&scenario(44));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 350, 450);
    issue(
        &mut s,
        vec![
            SimCommand::SetFireState {
                units: UnitSet::single(a),
                state: FireState::HoldFire,
            },
            combat_common::attack(&[a], Target::Unit(enemy)),
        ],
    );
    until(&mut s, 700, |s| s.world().unit(enemy).is_none());
}

const CHILD_OUT: &str = "OXA_ATTACK_MOVE_CHILD_OUT";

fn determinism_hashes() -> String {
    let mut out = String::new();
    for with_wall in [false, true] {
        let mut sc = scenario(0xA770);
        if with_wall {
            let (cols, rows) = (128, 128);
            let mut flags = vec![0; (cols * rows) as usize];
            for row in 0..rows {
                flags[(row * cols + 30) as usize] = terrain::FEATURE;
            }
            sc.map.terrain = Some(TerrainGrid {
                cols,
                rows,
                cell_size: 16,
                flags,
            });
        }
        let mut s = sim(&sc);
        let a = place(&mut s, SWORD, 0, 400, 400);
        let b = place(&mut s, SWORD, 0, 330, 400);
        let enemy = place(&mut s, DUMMY_TARGET, 1, 550, 400);
        place(&mut s, DUMMY_TARGET, 1, 400, 650);
        let mut log = tak_commands::CommandLog::new();
        for (seq_tick, command) in [
            (
                1,
                attack_move(a, Vec2::from_ints(400, 850), QueueMode::Replace),
            ),
            (1, walk(a, Vec2::from_ints(200, 850), QueueMode::Append)),
            (1, combat_common::attack(&[b], Target::Unit(enemy))),
            (
                1,
                attack_move(b, Vec2::from_ints(250, 850), QueueMode::Append),
            ),
            (
                600,
                SimCommand::SetFireState {
                    units: UnitSet::single(a),
                    state: FireState::HoldFire,
                },
            ),
            (
                700,
                SimCommand::SetFireState {
                    units: UnitSet::single(a),
                    state: FireState::FireAtWill,
                },
            ),
            (
                1000,
                SimCommand::Stop {
                    units: UnitSet::single(b),
                },
            ),
            (
                1100,
                attack_move(b, Vec2::from_ints(800, 900), QueueMode::Replace),
            ),
        ] {
            log.record(tak_commands::Tick(seq_tick), PlayerId(0), command);
        }
        for _ in 1..1800 {
            let tick = s.tick();
            out.push_str(&format!("{:016x}\n", s.step(log.for_tick(tick))));
        }
    }
    out
}

#[test]
fn child_writes_attack_move_hashes() {
    let Ok(path) = std::env::var(CHILD_OUT) else {
        return;
    };
    std::fs::write(path, determinism_hashes()).unwrap();
}

#[test]
fn attack_move_hashes_agree_across_processes() {
    let dir = std::env::temp_dir().join(format!("oxa-attack-move-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hashes.txt");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "child_writes_attack_move_hashes",
            "--exact",
            "--test-threads=1",
        ])
        .env(CHILD_OUT, &path)
        .status()
        .unwrap();
    assert!(status.success());
    let child = std::fs::read_to_string(path).unwrap();
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(child.lines().count(), 3598);
    assert_eq!(
        child,
        determinism_hashes(),
        "attack-move diverged across processes"
    );
}

mod construction_support;

fn builder_combat_scenario() -> tak_sim::Scenario {
    let mut sc = construction_support::scenario();
    sc.unit_defs[0].combat = combat_common::fighter(
        "builder",
        2500,
        "human",
        vec![combat_common::melee(360, 51)],
    )
    .combat;
    sc.unit_defs[construction_support::SWORD.0 as usize].combat =
        combat_common::fighter("enemy", 1000, "human", vec![]).combat;
    sc
}

#[test]
fn accepted_build_replace_cancels_old_combat_steering() {
    for attack_move_first in [false, true] {
        let mut s = sim(&builder_combat_scenario());
        let a = s.world().players()[0].monarch.unwrap();
        let enemy = place(&mut s, construction_support::SWORD, 1, 400, 250);
        let command = if attack_move_first {
            attack_move(a, Vec2::from_ints(700, 200), QueueMode::Replace)
        } else {
            combat_common::attack(&[a], Target::Unit(enemy))
        };
        issue(&mut s, vec![command]);
        issue(
            &mut s,
            vec![SimCommand::Build {
                units: UnitSet::single(a),
                def: construction_support::WALL,
                at: Some(Vec2::from_ints(700, 200)),
                queue: QueueMode::Replace,
            }],
        );
        let u = s.world().unit(a).unwrap();
        assert!(u.combat.order.is_none());
        assert!(u.combat.attack_move.is_none());
        assert!(u.orders.iter().any(|o| matches!(o, Order::Build { .. })));
    }
}

#[test]
fn queued_build_waits_until_the_active_attack_finishes() {
    let mut s = sim(&builder_combat_scenario());
    let a = s.world().players()[0].monarch.unwrap();
    let enemy = place(&mut s, construction_support::SWORD, 1, 400, 250);
    issue(
        &mut s,
        vec![
            combat_common::attack(&[a], Target::Unit(enemy)),
            SimCommand::Build {
                units: UnitSet::single(a),
                def: construction_support::WALL,
                at: Some(Vec2::from_ints(700, 200)),
                queue: QueueMode::Append,
            },
        ],
    );
    until(&mut s, 800, |s| {
        let enemy_alive = s.world().unit(enemy).is_some_and(|u| !u.combat.is_dying());
        if enemy_alive {
            let u = s.world().unit(a).unwrap();
            assert!(
                u.build.job.is_none(),
                "queued build started while still attacking"
            );
            assert!(u.orders.iter().any(|o| matches!(o, Order::Build { .. })));
        }
        !enemy_alive
    });
    until(&mut s, 2000, |s| {
        s.world().unit(a).unwrap().orders.is_empty()
    });
    assert!(
        s.world()
            .units()
            .any(|(_, u)| u.def == construction_support::WALL && u.build.is_complete())
    );
}

#[test]
fn attack_move_at_the_current_position_scans_before_releasing_its_queue() {
    let mut s = sim(&scenario(45));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 350, 450);
    let destination = Vec2::from_ints(200, 400);
    issue(
        &mut s,
        vec![
            attack_move(a, destination, QueueMode::Replace),
            walk(a, Vec2::from_ints(100, 400), QueueMode::Append),
        ],
    );
    let u = s.world().unit(a).unwrap();
    assert_eq!(
        u.orders.front(),
        Some(&Order::AttackMove { to: destination })
    );
    assert_eq!(u.combat.attack_move.as_ref().unwrap().target, Some(enemy));
    until(&mut s, 1600, |s| {
        s.world().unit(a).unwrap().orders.is_empty()
    });
    assert!(s.world().unit(enemy).is_none());
    assert!(
        s.world()
            .unit(a)
            .unwrap()
            .pos
            .distance(Vec2::from_ints(100, 400))
            <= ARRIVAL_TOLERANCE
    );
}

#[test]
fn avoided_targets_are_pruned_and_new_orders_clear_their_history() {
    let mut s = sim(&scenario(46));
    let a = place(&mut s, SWORD, 0, 200, 400);
    let enemy = place(&mut s, DUMMY_TARGET, 1, 350, 450);
    let other = place(&mut s, DUMMY_TARGET, 1, 360, 350);
    let destination = Vec2::from_ints(700, 400);
    issue(
        &mut s,
        vec![attack_move(a, destination, QueueMode::Replace)],
    );
    let stale = UnitId::new(999, 4);
    {
        let state = s
            .world_mut()
            .unit_mut(a)
            .unwrap()
            .combat
            .attack_move
            .as_mut()
            .unwrap();
        state.target = None;
        state.avoided.insert(enemy, Vec2::from_ints(350, 450));
        state.avoided.insert(stale, Vec2::ZERO);
    }
    s.world_mut().despawn(enemy).unwrap();
    let friendly = place(&mut s, DUMMY_TARGET, 0, 350, 450);
    assert_eq!(enemy.index, friendly.index);
    assert_ne!(enemy.generation, friendly.generation);
    s.step(&[]);
    let state = s
        .world()
        .unit(a)
        .unwrap()
        .combat
        .attack_move
        .as_ref()
        .unwrap();
    assert!(
        state.avoided.is_empty(),
        "dead and stale IDs must not accumulate"
    );
    assert_eq!(state.target, Some(other));
    {
        let state = s
            .world_mut()
            .unit_mut(a)
            .unwrap()
            .combat
            .attack_move
            .as_mut()
            .unwrap();
        state.target = None;
        state.avoided.insert(other, Vec2::from_ints(360, 350));
    }
    issue(
        &mut s,
        vec![attack_move(a, destination, QueueMode::Replace)],
    );
    let state = s
        .world()
        .unit(a)
        .unwrap()
        .combat
        .attack_move
        .as_ref()
        .unwrap();
    assert!(state.avoided.is_empty());
    assert_eq!(state.target, Some(other));
    issue(
        &mut s,
        vec![SimCommand::Stop {
            units: UnitSet::single(a),
        }],
    );
    assert!(s.world().unit(a).unwrap().combat.attack_move.is_none());
}

#[test]
fn guard_assist_walk_still_reaches_a_distant_frame_and_preserves_the_queue() {
    let mut s = sim(&builder_combat_scenario());
    let a = s.world().players()[0].monarch.unwrap();
    let frame = place(&mut s, construction_support::WALL, 0, 600, 200);
    s.world_mut().unit_mut(frame).unwrap().build.frame =
        Some(tak_sim::construction::Frame::new(1950 << 16));
    let tail = Vec2::from_ints(300, 400);
    issue(
        &mut s,
        vec![
            SimCommand::Guard {
                units: UnitSet::single(a),
                target: frame,
                queue: QueueMode::Replace,
            },
            attack_move(a, tail, QueueMode::Append),
        ],
    );
    until(&mut s, 1600, |s| {
        let u = s.world().unit(a).unwrap();
        assert_eq!(u.orders.back(), Some(&Order::AttackMove { to: tail }));
        s.world().unit(frame).unwrap().build.is_complete()
    });
    assert!(s.world().unit(a).unwrap().pos.x > Fx::from_int(450));
    assert_eq!(
        s.world().unit(a).unwrap().combat.order,
        Some(CombatOrder::Guard(frame))
    );
}

#[test]
fn guard_replace_cancels_an_old_build_job_before_assisting() {
    let mut s = sim(&builder_combat_scenario());
    let a = s.world().players()[0].monarch.unwrap();
    issue(
        &mut s,
        vec![SimCommand::Build {
            units: UnitSet::single(a),
            def: construction_support::WALL,
            at: Some(Vec2::from_ints(260, 200)),
            queue: QueueMode::Replace,
        }],
    );
    assert!(s.world().unit(a).unwrap().build.job.is_some());
    let frame = place(&mut s, construction_support::WALL, 0, 600, 200);
    s.world_mut().unit_mut(frame).unwrap().build.frame =
        Some(tak_sim::construction::Frame::new(1950 << 16));
    issue(
        &mut s,
        vec![SimCommand::Guard {
            units: UnitSet::single(a),
            target: frame,
            queue: QueueMode::Replace,
        }],
    );
    assert!(s.world().unit(a).unwrap().build.job.is_none());
    until(&mut s, 1600, |s| {
        s.world().unit(frame).unwrap().build.is_complete()
    });
}

#[test]
fn guard_assist_target_change_consumes_only_its_temporary_walk() {
    use tak_sim::construction::{Assist, BuilderJob, Frame};
    for new_x in [260, 700] {
        for tail_is_move in [false, true] {
            let mut s = sim(&builder_combat_scenario());
            let a = s.world().players()[0].monarch.unwrap();
            let guarded = place(&mut s, construction_support::MASON, 0, new_x, 300);
            let old = place(&mut s, construction_support::WALL, 0, 600, 200);
            let new = place(&mut s, construction_support::WALL, 0, new_x, 200);
            let old_pos = Vec2::from_ints(600, 200);
            let new_pos = Vec2::from_ints(new_x, 200);
            for frame in [old, new] {
                s.world_mut().unit_mut(frame).unwrap().build.frame = Some(Frame::new(1950 << 16));
            }
            let tail = if tail_is_move {
                Order::Move { to: old_pos }
            } else {
                Order::AttackMove {
                    to: Vec2::from_ints(300, 400),
                }
            };
            let u = s.world_mut().unit_mut(a).unwrap();
            u.combat.order = Some(CombatOrder::Guard(guarded));
            u.build.assist = Some(Assist {
                target: old,
                walk_to: Some(old_pos),
                walking: true,
                building: false,
            });
            u.orders.push_back(Order::Move { to: old_pos });
            u.orders.push_back(tail);
            let g = s.world_mut().unit_mut(guarded).unwrap();
            g.orders.push_back(Order::Build {
                def: construction_support::WALL,
                at: Some(new_pos),
            });
            g.build.job = Some(BuilderJob {
                def: construction_support::WALL,
                site: new_pos,
                target: Some(new),
                walking: false,
                building: false,
            });
            s.step(&[]);
            let u = s.world().unit(a).unwrap();
            assert_eq!(
                u.orders.back(),
                Some(&tail),
                "lost accepted tail at x={new_x}"
            );
            assert_eq!(u.build.assist.unwrap().target, new);
            if new_x == 700 {
                assert_eq!(u.orders.len(), 2);
                assert_eq!(u.orders.front(), Some(&Order::Move { to: new_pos }));
                assert_eq!(u.build.assist.unwrap().walk_to, Some(new_pos));
            } else {
                assert_eq!(u.orders.len(), 1);
                assert_eq!(u.build.assist.unwrap().walk_to, None);
            }
        }
    }
}

#[test]
fn replacing_guard_assist_preserves_new_move_at_old_walk_coordinates() {
    let mut s = sim(&builder_combat_scenario());
    let a = s.world().players()[0].monarch.unwrap();
    let frame = place(&mut s, construction_support::WALL, 0, 600, 200);
    s.world_mut().unit_mut(frame).unwrap().build.frame =
        Some(tak_sim::construction::Frame::new(1950 << 16));
    let goal = Vec2::from_ints(600, 200);
    let tail = Vec2::from_ints(300, 400);
    issue(
        &mut s,
        vec![SimCommand::Guard {
            units: UnitSet::single(a),
            target: frame,
            queue: QueueMode::Replace,
        }],
    );
    assert_eq!(
        s.world().unit(a).unwrap().build.assist.unwrap().walk_to,
        Some(goal)
    );
    issue(
        &mut s,
        vec![
            walk(a, goal, QueueMode::Replace),
            attack_move(a, tail, QueueMode::Append),
        ],
    );
    let u = s.world().unit(a).unwrap();
    assert_eq!(u.orders.front(), Some(&Order::Move { to: goal }));
    assert_eq!(u.orders.back(), Some(&Order::AttackMove { to: tail }));
    assert_eq!(u.orders.len(), 2);
    assert!(u.build.assist.is_none());
}

#[test]
fn failed_assist_walk_does_not_claim_identical_queued_player_move() {
    let mut sc = builder_combat_scenario();
    let (cols, rows) = (64, 64);
    let mut flags = vec![0; (cols * rows) as usize];
    for z in 0..rows {
        flags[(z * cols + 30) as usize] = terrain::FEATURE;
    }
    sc.map.terrain = Some(TerrainGrid {
        cols,
        rows,
        cell_size: 16,
        flags,
    });
    let mut s = sim(&sc);
    let a = s.world().players()[0].monarch.unwrap();
    s.world_mut().unit_mut(a).unwrap().pos = Vec2::from_ints(400, 400);
    let frame = place(&mut s, construction_support::WALL, 0, 700, 400);
    s.world_mut().unit_mut(frame).unwrap().build.frame =
        Some(tak_sim::construction::Frame::new(1950 << 16));
    let goal = Vec2::from_ints(700, 400);
    let tail = Vec2::from_ints(300, 700);
    issue(
        &mut s,
        vec![
            SimCommand::Guard {
                units: UnitSet::single(a),
                target: frame,
                queue: QueueMode::Replace,
            },
            walk(a, goal, QueueMode::Append),
            attack_move(a, tail, QueueMode::Append),
        ],
    );
    for _ in 0..2400 {
        s.step(&[]);
    }
    let u = s.world().unit(a).unwrap();
    assert_eq!(u.combat.order, Some(CombatOrder::Guard(frame)));
    assert_eq!(
        u.orders.iter().copied().collect::<Vec<_>>(),
        vec![Order::Move { to: goal }, Order::AttackMove { to: tail }]
    );
    let assist = u.build.assist.unwrap();
    assert_eq!(assist.walk_to, Some(goal));
    assert!(
        !assist.walking,
        "failed steering must not retry or claim queued moves"
    );
    let prior_hash = s.state_hash();
    s.world_mut()
        .unit_mut(a)
        .unwrap()
        .build
        .assist
        .as_mut()
        .unwrap()
        .walking = true;
    assert_ne!(
        s.state_hash(),
        prior_hash,
        "steering ownership affects future behavior and must be hashed"
    );
}
