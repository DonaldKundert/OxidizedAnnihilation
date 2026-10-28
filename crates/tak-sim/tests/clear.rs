//! Synthetic Clear lifecycle, targeting, accounting and occupancy regressions.
mod construction_support;
use construction_support::*;
use tak_commands::{FeatureId, PlayerId, QueueMode, SimCommand, UnitId, UnitSet};
use tak_math::{Angle, Fx};
use tak_sim::construction::{self, progress::Frame};
use tak_sim::features::{self, FeatureDef, FeatureSetup, PlacedFeature};
use tak_sim::map::TerrainGrid;
use tak_sim::{Order, Scenario, Sim, combat, lifecycle};

const ACTOR: UnitId = UnitId::new(0, 0);
const OTHER: UnitId = UnitId::new(1, 0);
const TREE: FeatureId = FeatureId::new(0, 0);

fn setup() -> Scenario {
    let mut sc = scenario();
    sc.economy.rules.start_mana = 0;
    for d in &mut sc.unit_defs {
        d.economy.mogriumincome = 0;
        d.economy.mogriumstorage = 1000;
        d.combat.max_damage = 100;
        d.combat.heal_rate = Fx::ZERO;
    }
    sc.unit_defs[0].build.canreclaim = true;
    sc.unit_defs[0].build.workertime = 11;
    sc.map.terrain = Some(TerrainGrid {
        cols: 64,
        rows: 64,
        cell_size: 16,
        flags: vec![0; 64 * 64],
    });
    *sc.features = FeatureSetup {
        defs: vec![FeatureDef {
            name: "synthetic_tree".into(),
            footprint_x: 1,
            footprint_z: 1,
            blocking: true,
            damage: 10,
            energy: 7,
            reclaimable: true,
            ..Default::default()
        }],
        placed: vec![PlacedFeature {
            def: 0,
            x: 16,
            z: 12,
        }],
        ..Default::default()
    };
    sc
}

fn clearing(actor: UnitId, target: FeatureId, queue: QueueMode) -> SimCommand {
    SimCommand::Clear {
        units: UnitSet::single(actor),
        target,
        queue,
    }
}
fn issue(sim: &mut Sim, c: SimCommand) {
    sim.step(&[cmd(sim.tick().0, 0, c)]);
}
fn init(sc: &Scenario) -> (Sim, Recorder) {
    let rec = Recorder::default();
    let mut sim = Sim::with_scripts(sc, Box::new(rec.clone())).unwrap();
    sim.step(&[]);
    (sim, rec)
}
fn mana(sim: &Sim, owner: u8) -> i64 {
    tak_sim::economy::player(sim.world(), PlayerId(owner))
        .unwrap()
        .mana
}
fn finish(sim: &mut Sim) {
    for _ in 0..3000 {
        if sim.world().features.get(TREE).is_none() {
            return;
        }
        sim.step(&[]);
    }
    panic!("Clear did not finish");
}

#[test]
fn exact_progress_reward_and_callbacks_remove_obstacle_not_weapon_successor() {
    let mut sc = setup();
    sc.features.defs[0].feature_dead = Some(0); // cyclic weapon death is irrelevant to Clear
    let total = construction::reclaim::total_work(&sc.features.defs[0]);
    let step = construction::progress::worker_step(11);
    let ticks = (total + step - 1) / step;
    let (mut sim, rec) = init(&sc);
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    assert_eq!(sim.world().features.get(TREE).unwrap().reclaim_done, step);
    assert_eq!(sim.world().features.get(TREE).unwrap().health, 10);
    for _ in 1..ticks - 1 {
        sim.step(&[]);
    }
    assert!(sim.world().features.get(TREE).is_some());
    sim.step(&[]);
    assert!(sim.world().features.is_empty());
    assert_eq!(mana(&sim, 0), 7 << 16);
    assert!(!tak_sim::pathfinding::is_obstacle(sim.world(), 16, 12));
    assert!(sim.world().unit(ACTOR).unwrap().orders.is_empty());
    assert!(sim.world().unit(ACTOR).unwrap().build.clear.is_none());
    assert_eq!(rec.log.borrow().calls_of(ACTOR, "StartBuilding").len(), 1);
    assert_eq!(rec.log.borrow().calls_of(ACTOR, "StopBuilding").len(), 1);
    assert!(
        rec.log
            .borrow()
            .calls
            .iter()
            .all(|(_, _, name)| !matches!(name.as_str(), "HitByWeapon" | "Killed" | "Dying"))
    );
}

#[test]
fn invalid_actor_target_ownership_and_capacity_preserve_orders() {
    let (mut sim, _) = init(&setup());
    let w = sim.world_mut();
    let incapable = w
        .spawn(MASON, PlayerId(0), at(200, 240), Angle::ZERO)
        .unwrap();
    for id in [ACTOR, OTHER, incapable] {
        w.unit_mut(id)
            .unwrap()
            .orders
            .push_back(Order::Move { to: at(220, 220) });
    }
    for (id, target) in [
        (incapable, TREE),
        (OTHER, TREE),
        (ACTOR, FeatureId::new(0, 99)),
    ] {
        let before = w.unit(id).unwrap().orders.clone();
        construction::apply_clear_command(
            w,
            PlayerId(0),
            &clearing(id, target, QueueMode::Replace),
        );
        assert_eq!(w.unit(id).unwrap().orders, before);
    }
    w.unit_mut(ACTOR).unwrap().build.frame = Some(Frame::new(1));
    assert!(!construction::can_clear(w, ACTOR));
    construction::apply_clear_command(w, PlayerId(0), &clearing(ACTOR, TREE, QueueMode::Replace));
    assert_eq!(w.unit(ACTOR).unwrap().orders.len(), 1);
    w.unit_mut(ACTOR).unwrap().build.frame = None;
    w.unit_mut(ACTOR)
        .unwrap()
        .orders
        .resize(tak_sim::world::MAX_ORDERS, Order::Move { to: at(220, 220) });
    construction::apply_clear_command(w, PlayerId(0), &clearing(ACTOR, TREE, QueueMode::Append));
    assert_eq!(
        w.unit(ACTOR).unwrap().orders.len(),
        tak_sim::world::MAX_ORDERS
    );
    let mut sc = setup();
    sc.features.defs[0].reclaimable = false;
    let (sim, _) = init(&sc);
    assert!(!construction::can_clear_target(sim.world(), TREE));
    for (worker, distance) in [(0, 100), (10, 0)] {
        let mut sc = setup();
        sc.unit_defs[0].build.workertime = worker;
        sc.unit_defs[0].build.builddistance = distance;
        let (sim, _) = init(&sc);
        assert!(!construction::can_clear(sim.world(), ACTOR));
    }
}

#[test]
fn stop_replace_and_new_worker_resume_target_progress_without_duplicate_reward() {
    let (mut sim, rec) = init(&setup());
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    let done = sim.world().features.get(TREE).unwrap().reclaim_done;
    issue(
        &mut sim,
        SimCommand::Stop {
            units: UnitSet::single(ACTOR),
        },
    );
    for _ in 0..5 {
        sim.step(&[]);
    }
    assert_eq!(sim.world().features.get(TREE).unwrap().reclaim_done, done);
    assert_eq!(rec.log.borrow().calls_of(ACTOR, "StopBuilding").len(), 2);
    let helper = sim
        .world_mut()
        .spawn(KING, PlayerId(0), at(220, 240), Angle::ZERO)
        .unwrap();
    issue(&mut sim, clearing(helper, TREE, QueueMode::Replace));
    finish(&mut sim);
    assert_eq!(mana(&sim, 0), 7 << 16);
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    assert_eq!(mana(&sim, 0), 7 << 16);
}

#[test]
fn multiple_owners_share_actual_work_only_and_completion_cannot_double_credit() {
    let mut sc = setup();
    sc.unit_defs[0].build.workertime = 90;
    let (mut sim, _) = init(&sc);
    sim.world_mut().unit_mut(OTHER).unwrap().pos = at(240, 240);
    let now = sim.tick().0;
    sim.step(&[
        cmd(now, 0, clearing(ACTOR, TREE, QueueMode::Replace)),
        cmd(now, 1, clearing(OTHER, TREE, QueueMode::Replace)),
    ]);
    finish(&mut sim);
    assert!(mana(&sim, 0) > 0 && mana(&sim, 1) > 0);
    assert_eq!(mana(&sim, 0) + mana(&sim, 1), 7 << 16);
    assert!(sim.world().unit(ACTOR).unwrap().build.clear.is_none());
    assert!(sim.world().unit(OTHER).unwrap().build.clear.is_none());
    for _ in 0..10 {
        sim.step(&[]);
    }
    assert_eq!(mana(&sim, 0) + mana(&sim, 1), 7 << 16);
}

#[test]
fn zero_yield_indestructible_and_zero_health_features_still_clear_without_mana() {
    for damage in [0, 10] {
        let mut sc = setup();
        let d = &mut sc.features.defs[0];
        d.energy = 0;
        d.damage = damage;
        d.indestructible = true;
        let (mut sim, _) = init(&sc);
        assert!(!features::apply_damage(sim.world_mut(), TREE, 100));
        issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
        finish(&mut sim);
        assert_eq!(mana(&sim, 0), 0);
    }
}

#[test]
fn actor_death_and_target_generation_loss_end_interval_once() {
    let (mut sim, rec) = init(&setup());
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    let before = sim.world().features.get(TREE).unwrap().reclaim_done;
    let now = sim.tick().0;
    combat::apply_damage(
        sim.world_mut(),
        &mut rec.clone(),
        ACTOR,
        1000,
        Some(OTHER),
        1,
        at(800, 800),
        now,
    );
    assert!(sim.world().unit(ACTOR).unwrap().build.clear.is_none());
    sim.step(&[]);
    assert_eq!(sim.world().features.get(TREE).unwrap().reclaim_done, before);
    assert_eq!(rec.log.borrow().calls_of(ACTOR, "StopBuilding").len(), 1);
    let mut sc = setup();
    sc.features.defs[0].feature_dead = Some(0);
    let (mut sim, rec) = init(&sc);
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    let paid = mana(&sim, 0);
    features::apply_damage(sim.world_mut(), TREE, 1000);
    let replacement = sim.world().features.iter().next().unwrap().0;
    assert_eq!(replacement.index, TREE.index);
    assert_ne!(replacement.generation, TREE.generation);
    sim.step(&[]);
    assert_eq!(
        sim.world().features.get(replacement).unwrap().reclaim_done,
        0
    );
    assert_eq!(mana(&sim, 0), paid);
    assert!(sim.world().unit(ACTOR).unwrap().orders.is_empty());
    assert_eq!(rec.log.borrow().calls_of(ACTOR, "StopBuilding").len(), 1);
    issue(&mut sim, clearing(ACTOR, replacement, QueueMode::Replace));
    lifecycle::remove_unit(sim.world_mut(), &mut rec.clone(), ACTOR);
    assert_eq!(rec.log.borrow().calls_of(ACTOR, "StopBuilding").len(), 2);
}

#[test]
fn walk_into_range_preserves_appended_equal_coordinate_move_and_cancelled_walk() {
    let mut sc = setup();
    sc.features.placed[0].x = 40;
    let goal = at(648, 200);
    let (mut sim, _) = init(&sc);
    issue(
        &mut sim,
        SimCommand::Move {
            units: UnitSet::single(ACTOR),
            to: at(220, 200),
            queue: QueueMode::Replace,
        },
    );
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Append));
    issue(
        &mut sim,
        SimCommand::Move {
            units: UnitSet::single(ACTOR),
            to: goal,
            queue: QueueMode::Append,
        },
    );
    finish(&mut sim);
    assert_eq!(
        sim.world().unit(ACTOR).unwrap().orders.front(),
        Some(&Order::Move { to: goal })
    );
    assert!(sim.world().unit(ACTOR).unwrap().pos.x > Fx::from_int(400));
    let (mut sim, _) = init(&sc);
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    assert!(
        sim.world()
            .unit(ACTOR)
            .unwrap()
            .build
            .clear
            .unwrap()
            .walking
    );
    issue(
        &mut sim,
        SimCommand::Move {
            units: UnitSet::single(ACTOR),
            to: goal,
            queue: QueueMode::Replace,
        },
    );
    assert_eq!(
        sim.world().unit(ACTOR).unwrap().orders.front(),
        Some(&Order::Move { to: goal })
    );
    assert!(sim.world().unit(ACTOR).unwrap().build.clear.is_none());
    assert_eq!(sim.world().features.get(TREE).unwrap().reclaim_done, 0);
}

#[test]
fn overlap_keeps_other_feature_and_static_terrain_blockers() {
    let mut sc = setup();
    sc.features.placed.push(sc.features.placed[0]);
    sc.map.terrain.as_mut().unwrap().flags[12 * 64 + 16] = tak_sim::map::terrain::FEATURE;
    let (mut sim, _) = init(&sc);
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    finish(&mut sim);
    assert_eq!(sim.world().pathing.obstacles_at(16, 12, 64), 1);
    assert!(sim.world().features.get(FeatureId::new(1, 0)).is_some());
    features::remove(sim.world_mut(), FeatureId::new(1, 0));
    assert_eq!(sim.world().pathing.obstacles_at(16, 12, 64), 0);
    assert_ne!(
        sim.world().map().terrain.as_ref().unwrap().flags[12 * 64 + 16]
            & tak_sim::map::terrain::FEATURE,
        0
    );
}

#[test]
fn nonbuilder_stationary_capability_works_in_range_and_ends_out_of_range() {
    let mut sc = setup();
    sc.unit_defs[0].build.builder = false;
    sc.unit_defs[0].build.mobile = false;
    sc.unit_defs[0].maxvelocity = Fx::ZERO;
    let (mut sim, _) = init(&sc);
    assert!(construction::can_clear(sim.world(), ACTOR));
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    finish(&mut sim);
    sc.features.placed[0].x = 50;
    let (mut sim, _) = init(&sc);
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    assert!(sim.world().unit(ACTOR).unwrap().orders.is_empty());
    assert_eq!(sim.world().features.get(TREE).unwrap().reclaim_done, 0);
}

#[test]
fn tick_zero_payout_uses_live_storage_and_large_yields_never_overflow() {
    for tick in [0, 1] {
        let mut sc = setup();
        sc.unit_defs[0].build.workertime = i32::MAX;
        sc.unit_defs[0].economy.mogriumstorage = 1_000_000;
        sc.features.defs[0].energy = 100_000;
        let mut sim = Sim::new(&sc).unwrap();
        if tick == 1 {
            sim.step(&[]);
        }
        issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
        assert!(sim.world().features.is_empty());
        assert_eq!(mana(&sim, 0), 100_000i64 << 16);
    }
    let mut sc = setup();
    sc.unit_defs[0].economy.mogriumstorage = 2;
    let (mut sim, _) = init(&sc);
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    finish(&mut sim);
    assert_eq!(mana(&sim, 0), 2 << 16);
    assert!(sim.world().construction.reclaim_credits.is_empty());
}

#[test]
fn stationary_clear_blocks_later_factory_production() {
    let mut sc = setup();
    sc.unit_defs[0].build.mobile = false;
    sc.unit_defs[0].maxvelocity = Fx::ZERO;
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    issue(
        &mut sim,
        SimCommand::Build {
            units: UnitSet::single(ACTOR),
            def: MASON,
            at: None,
            queue: QueueMode::Append,
        },
    );
    assert!(
        sim.world()
            .unit(ACTOR)
            .unwrap()
            .build
            .factory
            .product
            .is_none()
    );
    finish(&mut sim);
    for _ in 0..3 {
        sim.step(&[]);
    }
    assert!(
        sim.world()
            .unit(ACTOR)
            .unwrap()
            .build
            .factory
            .product
            .is_some()
    );
}

#[test]
fn unreachable_clear_ends_without_consuming_accepted_tail() {
    let mut sc = setup();
    sc.features.placed[0].x = 40;
    for z in 0..64 {
        sc.map.terrain.as_mut().unwrap().flags[z * 64 + 30] = tak_sim::map::terrain::FEATURE;
    }
    let (mut sim, _) = init(&sc);
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Replace));
    let tail = Order::Move { to: at(648, 200) };
    sim.world_mut()
        .unit_mut(ACTOR)
        .unwrap()
        .orders
        .push_back(tail);
    for _ in 0..3000 {
        sim.step(&[]);
        if sim.world().unit(ACTOR).unwrap().build.clear.is_none() {
            break;
        }
    }
    assert!(sim.world().unit(ACTOR).unwrap().build.clear.is_none());
    assert_eq!(sim.world().unit(ACTOR).unwrap().orders.front(), Some(&tail));
    assert_eq!(sim.world().features.get(TREE).unwrap().reclaim_done, 0);
}

#[test]
fn freshly_completed_storage_receives_this_ticks_reclaim_yield() {
    let mut sc = setup();
    sc.unit_defs[0].economy.mogriumstorage = 0;
    sc.unit_defs[0].build.workertime = 1000;
    let (mut sim, _) = init(&sc);
    let helper = sim
        .world_mut()
        .spawn(KING, PlayerId(0), at(180, 240), Angle::ZERO)
        .unwrap();
    let storage = sim
        .world_mut()
        .spawn(WALL, PlayerId(0), at(160, 240), Angle::ZERO)
        .unwrap();
    sim.world_mut().unit_mut(storage).unwrap().build.frame = Some(Frame {
        done: 0,
        total: 1,
        paid: 75 << 16,
    });
    assert_eq!(
        tak_sim::economy::player(sim.world(), PlayerId(0))
            .unwrap()
            .storage,
        0
    );
    let now = sim.tick().0;
    let mut heal = cmd(
        now,
        0,
        SimCommand::Heal {
            units: UnitSet::single(helper),
            target: storage,
            queue: QueueMode::Replace,
        },
    );
    heal.seq = 1;
    sim.step(&[cmd(now, 0, clearing(ACTOR, TREE, QueueMode::Replace)), heal]);
    assert!(sim.world().unit(storage).unwrap().build.is_complete());
    assert!(sim.world().features.is_empty());
    assert_eq!(mana(&sim, 0), 7 << 16);
}

#[test]
fn completed_clear_then_combat_death_in_same_tick_pays_and_tears_down_once() {
    let mut sc = setup();
    sc.unit_defs[0].build.workertime = 1000;
    // Only the enemy instance receives a weapon by using a separate definition.
    let mut enemy = sc.unit_defs[0].clone();
    enemy.combat.weapons = vec![tak_sim::combat::WeaponStats {
        kind: tak_sim::combat::WeaponKind::RemoteEffect,
        range: 2000,
        reload_ticks: 1,
        aim_tolerance: Some(u16::MAX),
        damage: tak_sim::combat::DamageStats {
            default: 1000,
            ..Default::default()
        },
        ..Default::default()
    }];
    sc.players[1].monarch = tak_commands::UnitDefId(sc.unit_defs.len() as u32);
    sc.unit_defs.push(enemy);
    let rec = Recorder::default();
    let mut sim = Sim::with_scripts(&sc, Box::new(rec.clone())).unwrap();
    sim.step(&[cmd(
        0,
        1,
        SimCommand::SetFireState {
            units: UnitSet::single(OTHER),
            state: tak_commands::FireState::HoldFire,
        },
    )]);
    let now = sim.tick().0;
    sim.step(&[
        cmd(now, 0, clearing(ACTOR, TREE, QueueMode::Replace)),
        cmd(
            now,
            1,
            SimCommand::Attack {
                units: UnitSet::single(OTHER),
                target: tak_commands::Target::Unit(ACTOR),
                queue: QueueMode::Replace,
            },
        ),
    ]);
    assert!(sim.world().features.is_empty());
    assert_eq!(mana(&sim, 0), 7 << 16);
    assert!(sim.world().unit(ACTOR).is_none_or(|u| u.combat.is_dying()));
    assert_eq!(rec.log.borrow().calls_of(ACTOR, "StopBuilding").len(), 1);
}

#[test]
fn appended_clear_waits_for_active_repeat_then_pauses_recipe_without_double_work() {
    let mut sc = setup();
    sc.economy.rules.start_mana = 1000;
    sc.unit_defs[0].build.mobile = false;
    sc.unit_defs[0].maxvelocity = Fx::ZERO;
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    issue(
        &mut sim,
        SimCommand::SetFactoryRepeat {
            units: UnitSet::single(ACTOR),
            def: Some(MASON),
        },
    );
    let product = sim
        .world()
        .unit(ACTOR)
        .unwrap()
        .build
        .factory
        .product
        .expect("active repeat");
    issue(&mut sim, clearing(ACTOR, TREE, QueueMode::Append));
    assert!(sim.world().unit(ACTOR).unwrap().build.clear.is_none());
    assert_eq!(sim.world().features.get(TREE).unwrap().reclaim_done, 0);
    let mut worked = false;
    for _ in 0..300 {
        sim.step(&[]);
        let u = sim.world().unit(ACTOR).unwrap();
        if u.build.clear.is_some() {
            worked = true;
            assert!(u.build.factory.product.is_none());
            assert!(sim.world().unit(product).unwrap().build.is_complete());
        }
        if sim.world().features.is_empty() {
            break;
        }
    }
    assert!(worked && sim.world().features.is_empty());
    assert_eq!(
        sim.world().unit(ACTOR).unwrap().build.factory_repeat,
        Some(MASON)
    );
    for _ in 0..100 {
        sim.step(&[]);
        if sim
            .world()
            .unit(ACTOR)
            .unwrap()
            .build
            .factory
            .product
            .is_some()
        {
            return;
        }
    }
    panic!("repeat did not resume after Clear");
}

#[test]
fn corpse_clear_and_natural_decay_retire_exact_target_lifetime() {
    for decay in [0, 1] {
        let mut sc = setup();
        sc.features.placed.clear();
        sc.features.corpses = vec![Some(0)];
        sc.features.defs[0].damage = 100;
        sc.features.defs[0].decompose_time = decay;
        let (mut sim, rec) = init(&sc);
        let corpse =
            features::spawn_corpse(sim.world_mut(), KING, at(264, 200), Angle::ZERO, 1).unwrap();
        assert_eq!(corpse, TREE);
        issue(&mut sim, clearing(ACTOR, corpse, QueueMode::Replace));
        finish(&mut sim);
        sim.step(&[]); // expire lane runs after construction; cleanup is next tick
        assert!(sim.world().unit(ACTOR).unwrap().build.clear.is_none());
        assert!(!tak_sim::pathfinding::is_obstacle(sim.world(), 16, 12));
        if decay == 0 {
            assert_eq!(mana(&sim, 0), 7 << 16);
        } else {
            assert!(mana(&sim, 0) > 0 && mana(&sim, 0) < 7 << 16);
        }
        assert_eq!(rec.log.borrow().calls_of(ACTOR, "StopBuilding").len(), 1);
    }
}
