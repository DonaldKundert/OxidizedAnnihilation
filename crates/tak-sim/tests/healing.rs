//! Explicit builder Heal: synthetic fixtures only. Chosen repair throughput and
//! payment are intentionally distinct from claims about original-game fidelity.
mod construction_support;

use construction_support::*;
use tak_commands::{FireState, PlayerId, QueueMode, SimCommand, UnitId, UnitSet};
use tak_math::{Angle, Fx};
use tak_sim::campaign::InitialUnit;
use tak_sim::construction::{self, progress::Frame};
use tak_sim::{Order, Scenario, Sim, combat, lifecycle, systems};

const HEALER: UnitId = UnitId::new(0, 0);
const TARGET: UnitId = UnitId::new(1, 0);
const ENEMY: UnitId = UnitId::new(2, 0);

fn setup() -> Scenario {
    let mut sc = scenario();
    for d in &mut sc.unit_defs {
        d.combat.max_damage = 100;
        d.combat.heal_rate = Fx::ZERO;
        d.economy.mogriumincome = 0;
    }
    sc.unit_defs[0].economy.maxmana = 100;
    sc.unit_defs[3].build.workertime = 1000;
    sc.unit_defs[1].build.buildtime = 3 << 16;
    sc.unit_defs[1].build.buildcost = 90;
    let initial = |def, owner, pos, health_percentage| InitialUnit {
        def,
        owner: PlayerId(owner),
        pos,
        heading: Angle::ZERO,
        health_percentage,
        mana_percentage: 0,
        kills: 0,
    };
    sc.initial_units = Some(vec![
        initial(KING, 0, at(200, 200), 100),
        initial(WALL, 0, at(260, 200), 50),
        initial(WALL, 1, at(800, 800), 50),
    ]);
    sc
}

fn healing(actor: UnitId, target: UnitId, queue: QueueMode) -> SimCommand {
    SimCommand::Heal {
        units: UnitSet::single(actor),
        target,
        queue,
    }
}

fn issue(sim: &mut Sim, command: SimCommand) {
    sim.step(&[cmd(sim.tick().0, 0, command)]);
}

fn init() -> (Sim, Recorder) {
    let rec = Recorder::default();
    let mut sim = Sim::with_scripts(&setup(), Box::new(rec.clone())).unwrap();
    sim.step(&[]);
    (sim, rec)
}

#[test]
fn weaponless_builder_repairs_exactly_caps_and_pairs_callbacks() {
    let (mut sim, rec) = init();
    let before = tak_sim::economy::player(sim.world(), PlayerId(0))
        .unwrap()
        .mana;
    let attacker = sim.world().unit(TARGET).unwrap().combat.last_attacker;
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    for _ in 0..4 {
        sim.step(&[]);
    }
    let t = sim.world().unit(TARGET).unwrap();
    assert_eq!(t.combat.health, 100);
    assert_eq!(t.combat.last_attacker, attacker);
    assert_eq!(t.combat.kills, 0);
    assert_eq!(t.combat.heal_acc, 0);
    assert!(t.build.is_complete());
    assert_eq!(
        before
            - tak_sim::economy::player(sim.world(), PlayerId(0))
                .unwrap()
                .mana,
        45 << 16
    );
    let a = sim.world().unit(HEALER).unwrap();
    assert!(a.build.heal.is_none() && a.orders.is_empty());
    assert_eq!(a.economy.mana().raw(), 0, "no personal mana needed");
    let l = rec.log.borrow();
    assert_eq!(l.calls_of(HEALER, "StartBuilding").len(), 1);
    assert_eq!(l.calls_of(HEALER, "StopBuilding").len(), 1);
    assert!(
        l.calls
            .iter()
            .all(|(_, _, name)| !matches!(name.as_str(), "HitByWeapon" | "Dying" | "Killed"))
    );
}

#[test]
fn invalid_capabilities_targets_and_handles_preserve_existing_orders() {
    let (mut sim, _) = init();
    let w = sim.world_mut();
    let builder = w
        .spawn(MASON, PlayerId(0), at(200, 240), Angle::ZERO)
        .unwrap();
    let nonbuilder = w
        .spawn(SWORD, PlayerId(0), at(200, 260), Angle::ZERO)
        .unwrap();
    let actors = [HEALER, TARGET, nonbuilder];
    for &a in &actors {
        w.unit_mut(a)
            .unwrap()
            .orders
            .push_back(Order::Move { to: at(222, 222) });
    }
    for (a, t) in [
        (HEALER, HEALER),
        (HEALER, ENEMY),
        (HEALER, UnitId::new(1, 99)),
        (TARGET, HEALER),
        (nonbuilder, TARGET),
    ] {
        let before = w.unit(a).unwrap().orders.clone();
        construction::apply_heal_command(w, PlayerId(0), &healing(a, t, QueueMode::Replace));
        assert_eq!(w.unit(a).unwrap().orders, before);
    }
    w.unit_mut(builder).unwrap().build.frame = Some(Frame::new(1));
    construction::apply_heal_command(
        w,
        PlayerId(0),
        &healing(builder, TARGET, QueueMode::Replace),
    );
    assert!(w.unit(builder).unwrap().orders.is_empty());
    assert!(!construction::can_heal(w, nonbuilder));
    // A non-builder's authored workertime must not grant Heal.
    assert!(!construction::can_heal(w, TARGET));
}

#[test]
fn assists_existing_frame_with_identical_payment_then_repairs_damage() {
    let (mut sim, _) = init();
    let w = sim.world_mut();
    let total = construction::progress::total_work(3 << 16);
    let mut f = Frame::new(total);
    f.done = total / 2;
    f.paid = 45 << 16;
    w.unit_mut(TARGET).unwrap().build.frame = Some(f);
    w.unit_mut(TARGET).unwrap().combat.health = 30; // 20 HP construction damage.
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert!(!sim.world().unit(TARGET).unwrap().build.is_complete());
    sim.step(&[]);
    let t = sim.world().unit(TARGET).unwrap();
    assert!(t.build.is_complete());
    assert_eq!(t.combat.health, 80);
    assert!(sim.world().unit(HEALER).unwrap().build.heal.is_some());
    sim.step(&[]);
    sim.step(&[]);
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 100);
    assert!(sim.world().unit(HEALER).unwrap().orders.is_empty());
}

#[test]
fn replacing_same_target_stop_actor_death_and_target_loss_cleanup_once() {
    let (mut sim, rec) = init();
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    issue(
        &mut sim,
        SimCommand::Stop {
            units: UnitSet::single(HEALER),
        },
    );
    assert_eq!(rec.log.borrow().calls_of(HEALER, "StartBuilding").len(), 2);
    assert_eq!(rec.log.borrow().calls_of(HEALER, "StopBuilding").len(), 2);
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    let now = sim.tick().0;
    combat::apply_damage(
        sim.world_mut(),
        &mut rec.clone(),
        HEALER,
        1000,
        Some(ENEMY),
        1,
        at(800, 800),
        now,
    );
    assert_eq!(rec.log.borrow().calls_of(HEALER, "StopBuilding").len(), 3);
    assert!(sim.world().unit(HEALER).unwrap().build.heal.is_none());
    // Stale generation after target removal cannot retarget its replacement.
    let (mut sim, rec) = init();
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    lifecycle::remove_unit(sim.world_mut(), &mut rec.clone(), TARGET);
    let replacement = sim
        .world_mut()
        .spawn(WALL, PlayerId(0), at(260, 200), Angle::ZERO)
        .unwrap();
    assert_ne!(replacement, TARGET);
    sim.step(&[]);
    assert!(sim.world().unit(HEALER).unwrap().orders.is_empty());
    assert_eq!(rec.log.borrow().calls_of(HEALER, "StopBuilding").len(), 1);
}

#[test]
fn queued_heal_waits_for_move_and_preserves_equal_coordinate_tail() {
    let (mut sim, _) = init();
    sim.world_mut().unit_mut(TARGET).unwrap().pos = at(800, 200);
    let tail = Order::Move { to: at(800, 200) };
    let u = sim.world_mut().unit_mut(HEALER).unwrap();
    u.orders.extend([
        Order::Move { to: at(220, 200) },
        Order::Heal { target: TARGET },
        tail,
    ]);
    let mut started = false;
    for _ in 0..1000 {
        sim.step(&[]);
        let a = sim.world().unit(HEALER).unwrap();
        if a.build.heal.is_some() {
            started = true;
            assert!(a.orders.contains(&tail));
        }
        if started && a.build.heal.is_none() {
            assert_eq!(a.orders.front(), Some(&tail));
            break;
        }
    }
    assert!(started);
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 100);
}

#[test]
fn moved_target_regoals_and_range_loss_pairs_work_intervals() {
    let (mut sim, rec) = init();
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    sim.world_mut().unit_mut(TARGET).unwrap().pos = at(800, 200);
    sim.step(&[]);
    assert_eq!(rec.log.borrow().calls_of(HEALER, "StopBuilding").len(), 1);
    assert_eq!(
        sim.world().unit(HEALER).unwrap().orders.front(),
        Some(&Order::Move { to: at(800, 200) })
    );
    sim.world_mut().unit_mut(TARGET).unwrap().pos = at(700, 400);
    sim.step(&[]);
    assert_eq!(
        sim.world().unit(HEALER).unwrap().orders.front(),
        Some(&Order::Move { to: at(700, 400) })
    );
    for _ in 0..2000 {
        sim.step(&[]);
        if sim.world().unit(HEALER).unwrap().build.heal.is_none() {
            break;
        }
    }
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 100);
    assert_eq!(rec.log.borrow().calls_of(HEALER, "StartBuilding").len(), 2);
    assert_eq!(rec.log.borrow().calls_of(HEALER, "StopBuilding").len(), 2);
}

#[test]
fn zero_time_refuses_free_targets_work_and_damage_does_not_change_billing() {
    let mut sc = setup();
    sc.unit_defs[1].build.buildtime = 0;
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 50);
    assert!(sim.world().unit(HEALER).unwrap().orders.is_empty());
    let mut sc = setup();
    sc.unit_defs[1].build.buildcost = 0;
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    assert_eq!(
        sim.world().unit(TARGET).unwrap().combat.health,
        61,
        "free repair still takes work"
    );
    let t = sim.world_mut().unit_mut(TARGET).unwrap();
    t.combat.health -= 10;
    for _ in 0..5 {
        sim.step(&[]);
    }
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 100);
    assert_eq!(
        tak_sim::economy::player(sim.world(), PlayerId(0))
            .unwrap()
            .mana,
        1000 << 16
    );
}

#[test]
fn no_mana_cannot_heal_indefinitely_and_two_healers_do_not_overpay() {
    let (mut sim, _) = init();
    tak_sim::economy::drain(sim.world_mut(), PlayerId(0), Fx::from_int(1000));
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    for _ in 0..100 {
        sim.step(&[]);
    }
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 50);
    let (mut sim, _) = init();
    let other = sim
        .world_mut()
        .spawn(MASON, PlayerId(0), at(280, 240), Angle::ZERO)
        .unwrap();
    let before = tak_sim::economy::player(sim.world(), PlayerId(0))
        .unwrap()
        .mana;
    let command = SimCommand::Heal {
        units: vec![HEALER, other].into(),
        target: TARGET,
        queue: QueueMode::Replace,
    };
    issue(&mut sim, command);
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 100);
    assert_eq!(
        before
            - tak_sim::economy::player(sim.world(), PlayerId(0))
                .unwrap()
                .mana,
        45 << 16
    );
    assert!(sim.world().unit(HEALER).unwrap().orders.is_empty());
    assert!(sim.world().unit(other).unwrap().orders.is_empty());
}

#[test]
fn reached_work_move_consumes_token_without_popping_identical_player_move() {
    let (mut sim, _) = init();
    let u = sim.world_mut().unit_mut(HEALER).unwrap();
    let to = u.pos;
    u.orders.extend([
        Order::Move { to },
        Order::Heal { target: TARGET },
        Order::Move { to },
    ]);
    u.build.heal = Some(construction::HealJob {
        work: construction::Assist {
            target: TARGET,
            walk_to: Some(to),
            walking: true,
            building: false,
        },
        cancelled: false,
    });
    systems::orders(sim.world_mut());
    let u = sim.world().unit(HEALER).unwrap();
    assert_eq!(u.orders.len(), 2);
    assert!(!u.build.heal.unwrap().work.walking);
    assert_eq!(u.orders.front(), Some(&Order::Heal { target: TARGET }));
}

#[test]
fn healing_helper_does_not_resurrect_or_change_passive_credit() {
    let (mut sim, _) = init();
    let t = sim.world_mut().unit_mut(TARGET).unwrap();
    t.combat.heal_acc = 123;
    t.combat.last_attacker = Some(ENEMY);
    t.fire_state = FireState::HoldFire;
    assert_eq!(combat::apply_healing(sim.world_mut(), TARGET, -1), 0);
    assert_eq!(combat::apply_healing(sim.world_mut(), TARGET, 1000), 50);
    let t = sim.world().unit(TARGET).unwrap();
    assert_eq!(t.combat.heal_acc, 123);
    assert_eq!(t.combat.last_attacker, Some(ENEMY));
    sim.world_mut().unit_mut(TARGET).unwrap().combat.health = 0;
    assert_eq!(combat::apply_healing(sim.world_mut(), TARGET, 100), 0);
}

#[test]
fn healing_every_tick_hash_matches_across_json_and_processes() {
    use tak_commands::CommandLog;
    let sc = setup();
    let mut log = CommandLog::new();
    let mut add = |tick, command| log.record(tak_commands::Tick(tick), PlayerId(0), command);
    add(1, healing(HEALER, TARGET, QueueMode::Replace));
    add(
        2,
        SimCommand::Move {
            units: UnitSet::single(HEALER),
            to: at(400, 400),
            queue: QueueMode::Append,
        },
    );
    add(
        3,
        SimCommand::Build {
            units: UnitSet::single(HEALER),
            def: WALL,
            at: Some(at(450, 400)),
            queue: QueueMode::Append,
        },
    );
    add(4, healing(HEALER, TARGET, QueueMode::Append));
    add(
        5,
        SimCommand::Move {
            units: UnitSet::single(HEALER),
            to: at(500, 400),
            queue: QueueMode::Append,
        },
    );
    add(200, healing(HEALER, TARGET, QueueMode::Append));
    add(
        201,
        SimCommand::Patrol {
            units: UnitSet::single(HEALER),
            to: at(600, 400),
            queue: QueueMode::Append,
        },
    );
    let run = |sc: &Scenario, log: &CommandLog| Sim::new(sc).unwrap().run(log, 500);
    let hashes = run(&sc, &log);
    if std::env::var_os("TAK_HEAL_DETERMINISM_CHILD").is_some() {
        println!("HEAL_HASHES {}", serde_json::to_string(&hashes).unwrap());
        return;
    }
    assert_eq!(hashes, run(&sc, &log));
    assert_eq!(
        hashes,
        run(
            &Scenario::from_json(&sc.to_json_pretty()).unwrap(),
            &CommandLog::from_json(&log.to_json()).unwrap()
        )
    );
    for _ in 0..2 {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "healing_every_tick_hash_matches_across_json_and_processes",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TAK_HEAL_DETERMINISM_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout
            .split("HEAL_HASHES ")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap();
        let child: Vec<u64> = serde_json::from_str(line).unwrap();
        assert_eq!(hashes, child);
    }
}

#[test]
fn in_range_heal_brakes_immediately_even_without_own_steering() {
    let (mut sim, _) = init();
    let pos = sim.world().unit(HEALER).unwrap().pos;
    sim.world_mut().unit_mut(HEALER).unwrap().speed = Fx::from_int(2);
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    let a = sim.world().unit(HEALER).unwrap();
    assert_eq!(a.speed, Fx::ZERO);
    assert_eq!(a.pos, pos);
}

#[test]
fn reaching_full_by_another_source_discards_hp_but_keeps_billing_credit() {
    let mut sc = setup();
    sc.unit_defs[1].build.buildtime = 1000 << 16;
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    let before = sim.world().unit(TARGET).unwrap().build.repair.unwrap();
    assert!(before.hp_remainder > 0);
    assert_eq!(combat::apply_healing(sim.world_mut(), TARGET, 100), 50);
    sim.step(&[]);
    let after = sim.world().unit(TARGET).unwrap().build.repair.unwrap();
    assert_eq!(after.hp_remainder, 0);
    assert_eq!(after.mana_remainder, before.mana_remainder);
    assert_eq!(after.prepaid, before.prepaid);
    assert!(sim.world().unit(HEALER).unwrap().build.heal.is_none());
}

#[test]
fn failed_unchanged_walk_finishes_heal_and_preserves_accepted_tail() {
    let mut sc = setup();
    sc.unit_defs[0].maxvelocity = Fx::ZERO;
    sc.initial_units.as_mut().unwrap()[1].pos = at(800, 200);
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    let tail = Order::Move { to: at(200, 500) };
    sim.world_mut()
        .unit_mut(HEALER)
        .unwrap()
        .orders
        .push_back(tail);
    for _ in 0..1500 {
        sim.step(&[]);
        if sim.world().unit(HEALER).unwrap().build.heal.is_none() {
            break;
        }
    }
    let a = sim.world().unit(HEALER).unwrap();
    assert!(a.build.heal.is_none(), "failed goal is not retried forever");
    assert_eq!(a.orders.front(), Some(&tail));
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 50);
}

#[test]
fn heal_frame_work_is_the_same_as_build_assistance_and_needs_no_menu_entry() {
    let mut sc = setup();
    sc.initial_units.as_mut().unwrap()[1].pos = at(256, 208);
    let mut a = Sim::new(&sc).unwrap();
    let mut b = Sim::new(&sc).unwrap();
    a.step(&[]);
    b.step(&[]);
    for sim in [&mut a, &mut b] {
        let t = sim.world_mut().unit_mut(TARGET).unwrap();
        t.build.frame = Some(Frame::new(construction::progress::total_work(3 << 16)));
        t.combat.health = 0;
    }
    issue(&mut a, healing(HEALER, TARGET, QueueMode::Replace));
    issue(
        &mut b,
        SimCommand::Build {
            units: UnitSet::single(HEALER),
            def: WALL,
            at: Some(at(256, 208)),
            queue: QueueMode::Replace,
        },
    );
    for _ in 0..8 {
        assert_eq!(
            a.world().unit(TARGET).unwrap().build.frame,
            b.world().unit(TARGET).unwrap().build.frame
        );
        assert_eq!(
            a.world().unit(TARGET).unwrap().combat.health,
            b.world().unit(TARGET).unwrap().combat.health
        );
        assert_eq!(
            tak_sim::economy::player(a.world(), PlayerId(0)),
            tak_sim::economy::player(b.world(), PlayerId(0))
        );
        a.step(&[]);
        b.step(&[]);
    }
    assert!(a.world().unit(TARGET).unwrap().build.is_complete());
    // A weaponless builder may Heal a unit absent from its own build menu.
    let mut sc = setup();
    sc.unit_defs[0].build.canbuild.clear();
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 61);
}

#[test]
fn steering_preserves_a_full_previously_admitted_order_queue() {
    let (mut sim, _) = init();
    sim.world_mut().unit_mut(TARGET).unwrap().pos = at(800, 200);
    let w = sim.world_mut();
    construction::apply_heal_command(w, PlayerId(0), &healing(HEALER, TARGET, QueueMode::Replace));
    for _ in 1..tak_sim::world::MAX_ORDERS {
        w.unit_mut(HEALER)
            .unwrap()
            .orders
            .push_back(Order::Move { to: at(800, 200) });
    }
    let rest = w.unit(HEALER).unwrap().orders.clone();
    construction::tick(w, &mut tak_sim::NoScripts);
    let a = w.unit(HEALER).unwrap();
    assert_eq!(a.orders.len(), tak_sim::world::MAX_ORDERS + 1);
    assert_eq!(
        a.orders
            .iter()
            .skip(1)
            .copied()
            .collect::<std::collections::VecDeque<_>>(),
        rest
    );
}

#[test]
fn terrain_unreachable_heal_is_attempted_once_then_releases_tail() {
    let mut sc = setup();
    sc.initial_units.as_mut().unwrap()[1].pos = at(800, 200);
    let mut flags = vec![0; 64 * 64];
    for row in 0..64 {
        flags[row * 64 + 30] = tak_sim::map::terrain::STEEP;
    }
    sc.map.terrain = Some(tak_sim::TerrainGrid {
        cols: 64,
        rows: 64,
        cell_size: 16,
        flags,
    });
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    let tail = Order::Move { to: at(200, 500) };
    sim.world_mut()
        .unit_mut(HEALER)
        .unwrap()
        .orders
        .push_back(tail);
    for _ in 0..4000 {
        sim.step(&[]);
        if sim.world().unit(HEALER).unwrap().build.heal.is_none() {
            break;
        }
    }
    let a = sim.world().unit(HEALER).unwrap();
    assert!(a.build.heal.is_none());
    assert_eq!(a.orders.front(), Some(&tail));
    assert_eq!(sim.world().pathing.stats.unreachable, 1);
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 50);
}

#[test]
fn owned_base_and_cb_data_grant_heal_by_builder_not_workertime_alone() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("healing capability corpus") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    for kind in [tak_content::RulesetKind::Base, tak_content::RulesetKind::Cb] {
        let rs = tak_content::Ruleset::load(&vfs, kind);
        assert!(rs.issues.is_empty());
        let eligible = |name: &str| {
            let u = rs.units.get(&name.to_ascii_lowercase()).unwrap();
            let d = tak_sim::content::unit_def_from_fbi(u);
            d.build.builder && d.build.mobile && d.build.workertime > 0
        };
        assert!(eligible("VERLIEGE"));
        assert!(!eligible("ZONWOLF"));
        assert!(!eligible("VERPILL"));
        assert!(!eligible("ARAPRIE2"));
        let priestess = rs.units.get("verliege").unwrap();
        assert!(priestess.weapons.is_empty());
        let count = rs
            .units
            .values()
            .filter(|u| {
                let d = tak_sim::content::unit_def_from_fbi(u);
                d.build.builder && d.build.mobile && d.build.workertime > 0
            })
            .count();
        assert_eq!(count, 21);
    }
}

#[test]
fn immediately_started_appended_spell_suspends_heal_without_losing_durable_order() {
    let (mut sim, rec) = init();
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    let w = sim.world_mut();
    let health = w.unit(TARGET).unwrap().combat.health;
    let mana = tak_sim::economy::player(w, PlayerId(0)).unwrap().mana;
    // Current spell plumbing starts an appended spell immediately rather than
    // representing it in the durable queue. Heal must not work concurrently.
    combat::apply_command(
        w,
        PlayerId(0),
        &SimCommand::CastSpell {
            units: UnitSet::single(HEALER),
            spell: tak_commands::SpellId(0),
            target: Some(tak_commands::Target::Unit(ENEMY)),
            queue: QueueMode::Append,
        },
    );
    assert!(w.unit(HEALER).unwrap().combat.order.is_some());
    construction::tick(w, &mut rec.clone());
    assert_eq!(w.unit(TARGET).unwrap().combat.health, health);
    assert_eq!(tak_sim::economy::player(w, PlayerId(0)).unwrap().mana, mana);
    assert_eq!(
        w.unit(HEALER).unwrap().orders.front(),
        Some(&Order::Heal { target: TARGET })
    );
    assert!(w.unit(HEALER).unwrap().build.heal.is_none());
    assert_eq!(rec.log.borrow().calls_of(HEALER, "StopBuilding").len(), 1);
    w.unit_mut(HEALER).unwrap().combat.order = None; // The cast completed.
    construction::tick(w, &mut rec.clone());
    assert!(w.unit(TARGET).unwrap().combat.health > health);
    assert_eq!(rec.log.borrow().calls_of(HEALER, "StartBuilding").len(), 2);
}

#[test]
fn passive_full_health_then_same_tick_damage_cannot_reuse_old_repair_work() {
    let mut sc = setup();
    sc.unit_defs[1].build.buildtime = 1000 << 16;
    sc.unit_defs[1].combat.heal_rate = Fx::from_int(30);
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    sim.world_mut().unit_mut(TARGET).unwrap().combat.health = 99;
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, 100);
    assert_eq!(
        sim.world()
            .unit(TARGET)
            .unwrap()
            .build
            .repair
            .unwrap()
            .hp_remainder,
        0
    );
    let now = sim.tick().0;
    combat::apply_damage(
        sim.world_mut(),
        &mut tak_sim::NoScripts,
        TARGET,
        2,
        Some(ENEMY),
        1,
        at(800, 800),
        now,
    );
    assert_eq!(
        sim.world()
            .unit(TARGET)
            .unwrap()
            .build
            .repair
            .unwrap()
            .hp_remainder,
        0
    );
}

#[test]
fn owned_weaponless_priestess_heals_with_original_cob_and_no_script_errors() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("priestess explicit heal COB") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    let loaded = tak_sim::load_match(
        &vfs,
        "kmap/Adamantine Gate",
        &tak_sim::PlayerSetup::parse_list("VER,ARA"),
        19,
    )
    .unwrap();
    let mut sc = loaded.scenario;
    let priestess = tak_sim::construction::content::def_by_name(
        &tak_sim::UnitDefs::new(sc.unit_defs.clone()),
        "VERLIEGE",
    )
    .unwrap();
    let centre = sc.map.center();
    let unit = |pos, hp| InitialUnit {
        def: priestess,
        owner: PlayerId(0),
        pos,
        heading: Angle::ZERO,
        health_percentage: hp,
        mana_percentage: 0,
        kills: 0,
    };
    sc.initial_units = Some(vec![
        unit(centre, 100),
        unit(sc.map.clamp(centre + at(64, 0)), 50),
    ]);
    sc.economy.rules.start_mana = 5000;
    let mut sim = Sim::with_scripts(&sc, Box::new(loaded.scripts)).unwrap();
    sim.step(&[]);
    let max = sim.world().unit(TARGET).unwrap().combat.max_health;
    assert!(construction::can_heal(sim.world(), HEALER));
    assert!(
        sim.world()
            .defs()
            .get(priestess)
            .unwrap()
            .combat
            .weapons
            .is_empty()
    );
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    let started = sim.world().unit(HEALER).unwrap().build.heal.is_some();
    for _ in 0..2000 {
        sim.step(&[]);
        if sim.world().unit(HEALER).unwrap().build.heal.is_none() {
            break;
        }
    }
    assert!(started);
    assert_eq!(sim.world().unit(TARGET).unwrap().combat.health, max);
    assert!(sim.world().unit(HEALER).unwrap().build.heal.is_none());
    let stats = sim.script_stats();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert!(stats.runaway.is_empty(), "{:?}", stats.runaway);
    assert_eq!(stats.units, 2);
}

#[test]
fn heal_job_and_all_target_owned_credit_are_hashed() {
    let (mut sim, _) = init();
    issue(&mut sim, healing(HEALER, TARGET, QueueMode::Replace));
    let baseline = sim.state_hash();
    let original = sim.world().unit(TARGET).unwrap().build.repair.unwrap();
    for mutate in [
        |r: &mut construction::repair::RepairCredit| r.hp_remainder += 1,
        |r: &mut construction::repair::RepairCredit| r.mana_remainder += 1,
        |r: &mut construction::repair::RepairCredit| r.mana_remainder += 1i128 << 64,
        |r: &mut construction::repair::RepairCredit| r.prepaid += 1,
    ] {
        let r = &mut sim.world_mut().unit_mut(TARGET).unwrap().build.repair;
        *r = Some(original);
        mutate(r.as_mut().unwrap());
        assert_ne!(sim.state_hash(), baseline);
    }
    sim.world_mut().unit_mut(TARGET).unwrap().build.repair = Some(original);
    let original = sim.world().unit(HEALER).unwrap().build.heal.unwrap();
    for mutate in [
        |j: &mut construction::HealJob| j.work.target = ENEMY,
        |j: &mut construction::HealJob| j.work.walk_to = Some(at(800, 200)),
        |j: &mut construction::HealJob| j.work.walking = !j.work.walking,
        |j: &mut construction::HealJob| j.work.building = !j.work.building,
        |j: &mut construction::HealJob| j.cancelled = !j.cancelled,
    ] {
        let j = &mut sim.world_mut().unit_mut(HEALER).unwrap().build.heal;
        *j = Some(original);
        mutate(j.as_mut().unwrap());
        assert_ne!(sim.state_hash(), baseline);
    }
}
