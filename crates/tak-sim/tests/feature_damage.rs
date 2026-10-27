//! Synthetic feature damage, owned blockers and command-only replay coverage.
//! No retail data or snapshots are used here.

use tak_commands::{
    CommandLog, PlayerId, QueueMode, SimCommand, Target, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::combat::{DamageStats, WeaponKind, WeaponStats, area_damage};
use tak_sim::construction::{BuildDef, placement};
use tak_sim::features::{self, FeatureDef, FeatureId, FeatureSetup, Origin, PlacedFeature};
use tak_sim::map::{MapInfo, StartPos, TerrainGrid, terrain};
use tak_sim::pathfinding::{self, CellRect};
use tak_sim::testing::monarch_def;
use tak_sim::{PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario, Sim};

const SHOOTER: UnitDefId = UnitDefId(0);
const BUILDING: UnitDefId = UnitDefId(1);
const UNIT: UnitId = UnitId::new(0, 0);

fn at(x: i32, z: i32) -> Vec2 {
    Vec2::from_ints(x, z)
}

fn feature(name: &str, health: i32, blocking: bool) -> FeatureDef {
    FeatureDef {
        name: name.into(),
        footprint_x: 1,
        footprint_z: 1,
        blocking,
        damage: health,
        ..FeatureDef::default()
    }
}

fn placed(def: u32, x: u32, z: u32) -> PlacedFeature {
    PlacedFeature { def, x, z }
}

fn scenario(defs: Vec<FeatureDef>, placed: Vec<PlacedFeature>) -> Scenario {
    let mut shooter = monarch_def("synthetic_shooter");
    shooter.footprintx = 1;
    shooter.footprintz = 1;
    shooter.maxvelocity = Fx::from_int(2);
    shooter.acceleration = Fx::ONE;
    shooter.brakerate = Fx::ONE;
    shooter.turnrate = 32767;
    shooter.combat.max_damage = 1000;
    shooter.combat.radius = 8;
    shooter.combat.weapons = vec![WeaponStats {
        kind: WeaponKind::RemoteEffect,
        range: 512,
        reload_ticks: 1000,
        damage: DamageStats {
            default: 100,
            ..DamageStats::default()
        },
        ..WeaponStats::default()
    }];
    shooter.economy.mogriumstorage = 1000;
    let mut building = monarch_def("synthetic_building");
    building.footprintx = 1;
    building.footprintz = 1;
    building.maxvelocity = Fx::ZERO;
    building.build = BuildDef {
        yardmap: "o".into(),
        ..BuildDef::default()
    };
    let mut map = MapInfo::flat(512, 512, vec![StartPos { x: 64, y: 64 }]);
    map.terrain = Some(TerrainGrid {
        cols: 32,
        rows: 32,
        cell_size: 16,
        flags: vec![0; 32 * 32],
    });
    Scenario {
        version: SCENARIO_FORMAT_VERSION,
        seed: 71,
        map,
        unit_defs: vec![shooter, building],
        players: vec![PlayerSpec {
            start: 0,
            monarch: SHOOTER,
        }],
        initial_units: None,
        economy: Default::default(),
        features: Box::new(FeatureSetup {
            defs,
            placed,
            corpses: vec![Some(0)],
            ..FeatureSetup::default()
        }),
        victory: Default::default(),
    }
}

fn ids(sim: &Sim) -> Vec<FeatureId> {
    sim.world().features.iter().map(|(id, _)| id).collect()
}

fn command(sim: &mut Sim, command: SimCommand) {
    let mut log = CommandLog::new();
    log.record(sim.tick(), PlayerId(0), command);
    sim.step(log.for_tick(sim.tick()));
}

fn attack(point: Vec2) -> SimCommand {
    SimCommand::Attack {
        units: UnitSet::single(UNIT),
        target: Target::Ground(point),
        queue: QueueMode::Replace,
    }
}

fn fire_one(sim: &mut Sim, point: Vec2) {
    if sim.tick() == Tick(0) {
        sim.step(&[]);
    }
    let before = sim.world().combat.impacts.len();
    command(sim, attack(point));
    for _ in 0..60 {
        if sim.world().combat.impacts.len() > before {
            return;
        }
        sim.step(&[]);
    }
    panic!("ground attack never produced an impact");
}

#[test]
fn damage_is_hashed_without_awarding_reclaim_mana() {
    let mut f = feature("valuable", 250, true);
    f.reclaimable = true;
    f.energy = 500;
    let mut sim = Sim::new(&scenario(vec![f], vec![placed(0, 10, 10)])).unwrap();
    sim.step(&[]);
    let id = ids(&sim)[0];
    let before = sim.state_hash();
    let economy = sim.world().economy.clone();
    assert!(features::apply_damage(sim.world_mut(), id, 75));
    assert_eq!(sim.world().features.get(id).unwrap().health, 175);
    assert_ne!(sim.state_hash(), before);
    assert!(features::apply_damage(sim.world_mut(), id, i32::MAX));
    assert!(sim.world().features.get(id).is_none());
    assert_eq!(sim.world().economy, economy, "destruction is not reclaim");
    assert!(!features::apply_damage(sim.world_mut(), id, 1), "stale id");
}

#[test]
fn indestructible_zero_health_and_nonpositive_hits_are_noops() {
    let mut stone = feature("indestructible", 20, true);
    stone.indestructible = true;
    let sc = scenario(
        vec![
            stone,
            feature("no_health", 0, true),
            feature("ordinary", 20, true),
        ],
        vec![placed(0, 8, 8), placed(1, 9, 8), placed(2, 10, 8)],
    );
    let mut sim = Sim::new(&sc).unwrap();
    let ids = ids(&sim);
    let before = sim.state_hash();
    assert!(!features::apply_damage(sim.world_mut(), ids[0], i32::MAX));
    assert!(!features::apply_damage(sim.world_mut(), ids[1], i32::MAX));
    for amount in [0, -1, i32::MIN] {
        assert!(!features::apply_damage(sim.world_mut(), ids[2], amount));
    }
    assert_eq!(sim.state_hash(), before);
}

#[test]
fn successor_has_fresh_generation_full_health_and_its_own_footprint() {
    let mut first = feature("large_wreck", 20, true);
    first.footprint_x = 4;
    first.footprint_z = 2;
    first.feature_dead = Some(1);
    let mut next = feature("small_wreck", 70, true);
    next.footprint_x = 2;
    next.footprint_z = 2;
    next.feature_dead = Some(1); // A cycle must not recurse in a single hit.
    let mut sim = Sim::new(&scenario(vec![first, next], vec![placed(0, 10, 10)])).unwrap();
    sim.step(&[]);
    let old_id = ids(&sim)[0];
    let old = *sim.world().features.get(old_id).unwrap();
    let now = sim.tick().0;
    assert!(features::apply_damage(sim.world_mut(), old_id, 1_000_000));
    let new_id = ids(&sim)[0];
    let new = sim.world().features.get(new_id).unwrap();
    assert_eq!(new_id.index, old_id.index);
    assert_ne!(new_id.generation, old_id.generation);
    assert!(sim.world().features.get(old_id).is_none());
    assert_eq!(
        (new.def, new.health, new.origin, new.born),
        (1, 70, Origin::Map, now)
    );
    assert_eq!(new.center(), old.center());
    assert_eq!((new.rect.w, new.rect.h), (2, 2));
    assert!(!pathfinding::is_obstacle(sim.world(), 10, 10));
    assert!(pathfinding::is_obstacle(sim.world(), 11, 10));
    assert!(!features::apply_damage(sim.world_mut(), old_id, 1_000_000));
    assert_eq!(sim.world().features.get(new_id).unwrap().health, 70);
    assert!(features::apply_damage(sim.world_mut(), new_id, i32::MAX));
    let repeated = ids(&sim)[0];
    assert_ne!(repeated, new_id, "self-cycle creates only one new lifetime");
    assert_eq!(sim.world().features.get(repeated).unwrap().health, 70);
    let after = sim.state_hash();
    assert!(!features::apply_damage(sim.world_mut(), new_id, i32::MAX));
    assert_eq!(
        sim.state_hash(),
        after,
        "retired generation cannot touch successor"
    );
}

#[test]
fn oversized_successors_at_map_edges_clip_blockers_without_losing_their_center() {
    for (x, z) in [(0, 0), (31, 31)] {
        let mut first = feature("edge_feature", 10, true);
        first.feature_dead = Some(1);
        let mut successor = feature("large_successor", 80, true);
        successor.footprint_x = 255;
        successor.footprint_z = 255;
        let sc = scenario(vec![first, successor], vec![placed(0, x, z)]);
        let mut sim = Sim::new(&sc).unwrap();
        sim.step(&[]);
        let old_id = ids(&sim)[0];
        let center = sim.world().features.get(old_id).unwrap().center();
        features::apply_damage(sim.world_mut(), old_id, i32::MAX);
        let next_id = ids(&sim)[0];
        let next = sim.world().features.get(next_id).unwrap();
        assert_eq!(
            next.center(),
            center,
            "odd-sized footprint keeps exact center"
        );
        assert_eq!((next.rect.w, next.rect.h), (255, 255));
        assert_eq!(next.rect.clip(32, 32), (0, 32, 0, 32));
        for cell in [(0, 0), (0, 31), (31, 0), (31, 31)] {
            assert!(pathfinding::is_obstacle(sim.world(), cell.0, cell.1));
            assert_eq!(sim.world().pathing.grid_for(SHOOTER).unwrap().tier(cell), 0);
        }
        assert!(!pathfinding::is_obstacle(sim.world(), -1, -1));
        assert!(!pathfinding::is_obstacle(sim.world(), 32, 32));
        features::apply_damage(sim.world_mut(), next_id, 80);
        for cell in [(0, 0), (0, 31), (31, 0), (31, 31)] {
            assert!(!pathfinding::is_obstacle(sim.world(), cell.0, cell.1));
        }
    }
}

#[test]
fn alternating_successor_footprints_do_not_drift_after_five_thousand_hits() {
    let mut small = feature("small_cycle", 10, true);
    small.feature_dead = Some(1);
    let mut large = feature("large_cycle", 20, true);
    large.footprint_x = 2;
    large.footprint_z = 2;
    large.feature_dead = Some(0);
    let sc = scenario(vec![small, large], vec![placed(0, 31, 31)]);
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    let original = *sim.world().features.iter().next().unwrap().1;
    let mut id = ids(&sim)[0];
    for hit in 1..=5000 {
        assert!(features::apply_damage(sim.world_mut(), id, i32::MAX));
        assert!(sim.world().features.get(id).is_none());
        id = ids(&sim)[0];
        assert_eq!(id.generation, hit, "exactly one lifetime per hit");
        let f = sim.world().features.get(id).unwrap();
        let size = if hit % 2 == 0 { 1 } else { 2 };
        assert_eq!(
            f.rect,
            CellRect {
                x: 31,
                z: 31,
                w: size,
                h: size
            }
        );
        assert_eq!(f.health, if size == 1 { 10 } else { 20 });
        assert_eq!(f.distance_to(original.center()), Fx::ZERO);
        let far_distance = f.distance_to(at(0, 0));
        assert!(far_distance > Fx::ZERO && far_distance < Fx::from_int(1024));
        assert!((f.center().x - original.center().x).abs() <= Fx::from_int(8));
        assert!((f.center().y - original.center().y).abs() <= Fx::from_int(8));
        assert!(pathfinding::is_obstacle(sim.world(), 31, 31));
        assert!(!pathfinding::is_obstacle(sim.world(), 30, 31));
        assert_eq!(
            sim.world()
                .pathing
                .grid_for(SHOOTER)
                .unwrap()
                .tier((31, 31)),
            0
        );
    }
    assert_eq!(sim.world().features.get(id).unwrap().rect, original.rect);
}

#[test]
fn corpse_successor_keeps_origin_and_heading() {
    let mut corpse = feature("corpse", 20, true);
    corpse.feature_dead = Some(1);
    let mut sim = Sim::new(&scenario(
        vec![corpse, feature("debris", 80, false)],
        vec![],
    ))
    .unwrap();
    sim.step(&[]);
    let heading = Angle::from_raw(12345);
    let id = features::spawn_corpse(sim.world_mut(), SHOOTER, at(168, 168), heading, 1).unwrap();
    assert!(features::apply_damage(sim.world_mut(), id, 100));
    let (_, next) = sim.world().features.iter().next().unwrap();
    assert_eq!(next.origin, Origin::Corpse(SHOOTER));
    assert_eq!(next.heading, heading);
    assert_eq!(next.health, 80);
    assert!(!pathfinding::is_obstacle(sim.world(), 10, 10));
}

#[test]
fn overlapping_features_clear_independently_and_preserve_static_terrain() {
    let mut f = feature("overlap", 10, true);
    f.footprint_x = 2;
    f.footprint_z = 2;
    let mut sc = scenario(vec![f], vec![placed(0, 8, 8), placed(0, 8, 8)]);
    let flags = &mut sc.map.terrain.as_mut().unwrap().flags;
    flags[8 * 32 + 9] = terrain::FEATURE;
    flags[9 * 32 + 8] = terrain::STEEP;
    flags[9 * 32 + 9] = terrain::WATER | terrain::DEEP;
    let original_flags = flags.clone();
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    let ids = ids(&sim);
    features::apply_damage(sim.world_mut(), ids[0], 10);
    assert!(pathfinding::is_obstacle(sim.world(), 8, 8));
    assert_eq!(
        sim.world().pathing.grid_for(SHOOTER).unwrap().tier((8, 8)),
        0
    );
    features::apply_damage(sim.world_mut(), ids[1], 10);
    assert!(!pathfinding::is_obstacle(sim.world(), 8, 8));
    let grid = sim.world().pathing.grid_for(SHOOTER).unwrap();
    assert_ne!(grid.tier((8, 8)), 0);
    for cell in [(9, 8), (8, 9), (9, 9)] {
        assert_eq!(grid.tier(cell), 0, "unrelated terrain at {cell:?} survived");
    }
    assert_eq!(
        sim.world().map().terrain.as_ref().unwrap().flags,
        original_flags
    );
    assert!(placement::check_site(sim.world(), BUILDING, at(136, 136), None).is_ok());
    assert_eq!(
        placement::check_site(sim.world(), BUILDING, at(152, 136), None),
        Err(placement::SiteError::Terrain)
    );
}

#[test]
fn removal_before_layers_build_and_structure_overlap_are_safe() {
    let sc = scenario(vec![feature("blocker", 10, true)], vec![placed(0, 10, 10)]);
    let mut sim = Sim::new(&sc).unwrap();
    let id = ids(&sim)[0];
    assert!(
        pathfinding::is_obstacle(sim.world(), 10, 10),
        "blocked before first path tick"
    );
    features::apply_damage(sim.world_mut(), id, 10);
    sim.step(&[]);
    assert_ne!(
        sim.world()
            .pathing
            .grid_for(SHOOTER)
            .unwrap()
            .tier((10, 10)),
        0
    );

    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    sim.spawn_unit(BUILDING, PlayerId(0), at(168, 168), Angle::ZERO)
        .unwrap();
    sim.step(&[]);
    let id = ids(&sim)[0];
    features::apply_damage(sim.world_mut(), id, 10);
    assert!(!pathfinding::is_obstacle(sim.world(), 10, 10));
    assert_eq!(
        sim.world()
            .pathing
            .grid_for(SHOOTER)
            .unwrap()
            .tier((10, 10)),
        0,
        "structure still owns this cell"
    );
    assert_eq!(
        placement::check_site(sim.world(), BUILDING, at(168, 168), None),
        Err(placement::SiteError::Occupied)
    );
}

#[test]
fn sacred_yard_exemption_applies_to_placed_features_not_corpses() {
    let mut sc = scenario(
        vec![feature("sacred_stone", 20, true)],
        vec![placed(0, 10, 10)],
    );
    sc.unit_defs[BUILDING.0 as usize].build.yardmap = "S".into();
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    assert!(placement::check_site(sim.world(), BUILDING, at(168, 168), None).is_ok());
    let corpse =
        features::spawn_corpse(sim.world_mut(), SHOOTER, at(168, 168), Angle::ZERO, 1).unwrap();
    assert_eq!(
        placement::check_site(sim.world(), BUILDING, at(168, 168), None),
        Err(placement::SiteError::Feature)
    );
    features::remove(sim.world_mut(), corpse).unwrap();
    assert!(placement::check_site(sim.world(), BUILDING, at(168, 168), None).is_ok());
}

#[test]
fn feature_obstacle_count_does_not_saturate_at_sixteen_bits() {
    let sc = scenario(vec![feature("unused", 10, true)], vec![]);
    let mut sim = Sim::new(&sc).unwrap();
    let r = CellRect {
        x: 10,
        z: 10,
        w: 1,
        h: 1,
    };
    for _ in 0..=u16::MAX {
        pathfinding::add_obstacle(sim.world_mut(), r);
    }
    for _ in 0..u16::MAX {
        pathfinding::remove_obstacle(sim.world_mut(), r);
    }
    assert!(pathfinding::is_obstacle(sim.world(), 10, 10));
    pathfinding::remove_obstacle(sim.world_mut(), r);
    assert!(!pathfinding::is_obstacle(sim.world(), 10, 10));
}

#[test]
fn disabling_placed_blocking_does_not_disable_corpse_blocking() {
    let mut sc = scenario(
        vec![feature("optional_blocker", 10, true)],
        vec![placed(0, 10, 10)],
    );
    sc.features.placed_features_block = false;
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    assert!(!pathfinding::is_obstacle(sim.world(), 10, 10));
    assert_ne!(
        sim.world()
            .pathing
            .grid_for(SHOOTER)
            .unwrap()
            .tier((10, 10)),
        0
    );
    let corpse =
        features::spawn_corpse(sim.world_mut(), SHOOTER, at(168, 168), Angle::ZERO, 1).unwrap();
    assert!(pathfinding::is_obstacle(sim.world(), 10, 10));
    features::apply_damage(sim.world_mut(), corpse, 10);
    assert!(!pathfinding::is_obstacle(sim.world(), 10, 10));
}

#[test]
fn feature_destruction_invalidates_suspended_search_within_tick_budget() {
    let mut sc = scenario(
        vec![feature("search_obstacle", 10, true)],
        vec![placed(0, 110, 50)],
    );
    sc.map = MapInfo::flat(2048, 2048, vec![StartPos { x: 40, y: 40 }]);
    let mut flags = vec![0; 128 * 128];
    for (i, x) in [20usize, 40, 60, 80, 100].into_iter().enumerate() {
        for z in 0..128 {
            let gap = if i % 2 == 0 { 116..124 } else { 4..12 };
            if !gap.contains(&z) {
                flags[z * 128 + x] = terrain::FEATURE;
            }
        }
    }
    sc.map.terrain = Some(TerrainGrid {
        cols: 128,
        rows: 128,
        cell_size: 16,
        flags,
    });
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    command(
        &mut sim,
        SimCommand::Move {
            units: UnitSet::single(UNIT),
            to: at(1900, 1900),
            queue: QueueMode::Replace,
        },
    );
    assert_eq!(
        sim.world().unit(UNIT).unwrap().path.status,
        pathfinding::PathStatus::Searching,
        "fixture must suspend a search"
    );
    let requests = sim.world().pathing.stats.requests;
    let id = ids(&sim)[0];
    features::apply_damage(sim.world_mut(), id, 10);
    assert_eq!(
        sim.world().unit(UNIT).unwrap().path.status,
        pathfinding::PathStatus::Pending
    );
    sim.step(&[]);
    assert!(sim.world().pathing.stats.requests > requests);
    assert!(sim.world().pathing.stats.steps_last_tick <= pathfinding::STEPS_PER_TICK);
}

#[test]
fn direct_ground_hit_chooses_one_feature_and_respects_units_only() {
    for units_only in [false, true] {
        let mut sc = scenario(
            vec![feature("overlapping", 250, false)],
            vec![placed(0, 10, 10), placed(0, 10, 10)],
        );
        sc.unit_defs[0].combat.weapons[0].units_only = units_only;
        let mut sim = Sim::new(&sc).unwrap();
        fire_one(&mut sim, at(168, 168));
        let health: Vec<_> = sim.world().features.iter().map(|(_, f)| f.health).collect();
        assert_eq!(
            health,
            if units_only {
                vec![250, 250]
            } else {
                vec![150, 250]
            }
        );
    }
}

#[test]
fn direct_unit_hit_keeps_priority_over_a_feature_underneath() {
    let mut sim = Sim::new(&scenario(
        vec![feature("under_unit", 250, false)],
        vec![placed(0, 10, 10)],
    ))
    .unwrap();
    sim.step(&[]);
    let victim = sim
        .spawn_unit(SHOOTER, PlayerId(0), at(168, 168), Angle::ZERO)
        .unwrap();
    sim.step(&[]);
    fire_one(&mut sim, at(168, 168));
    assert_eq!(sim.world().unit(victim).unwrap().combat.health, 900);
    assert_eq!(sim.world().features.iter().next().unwrap().1.health, 250);
}

#[test]
fn units_only_blast_spares_features_but_still_damages_units() {
    let mut sc = scenario(
        vec![feature("under_unit", 250, false)],
        vec![placed(0, 10, 10)],
    );
    sc.unit_defs[0].combat.weapons[0].units_only = true;
    sc.unit_defs[0].combat.weapons[0].area_of_effect = 128;
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    let victim = sim
        .spawn_unit(SHOOTER, PlayerId(0), at(168, 168), Angle::ZERO)
        .unwrap();
    sim.step(&[]);
    fire_one(&mut sim, at(168, 168));
    assert_eq!(sim.world().unit(victim).unwrap().combat.health, 900);
    assert_eq!(sim.world().features.iter().next().unwrap().1.health, 250);
}

#[test]
fn blast_uses_footprint_distance_and_does_not_hit_fresh_successor() {
    let mut f = feature("fragile", 20, false);
    f.feature_dead = Some(1);
    let mut sc = scenario(
        vec![f, feature("remaining", 250, false)],
        vec![
            placed(0, 10, 10),
            placed(1, 12, 10),
            placed(1, 14, 10),
            placed(1, 15, 10),
        ],
    );
    sc.unit_defs[0].combat.weapons[0].area_of_effect = 128;
    sc.unit_defs[0].combat.weapons[0].edge_effectiveness = Fx::HALF;
    let mut sim = Sim::new(&sc).unwrap();
    let initial = ids(&sim);
    fire_one(&mut sim, at(160, 160));
    assert!(sim.world().features.get(initial[0]).is_none());
    let successor = sim
        .world()
        .features
        .iter()
        .find(|(id, _)| id.index == initial[0].index)
        .unwrap()
        .1;
    assert_eq!(
        successor.health, 250,
        "one blast never hits the new generation"
    );
    assert_eq!(
        sim.world().features.get(initial[1]).unwrap().health,
        250 - area_damage(100, 32, 64, Fx::HALF)
    );
    assert_eq!(
        sim.world().features.get(initial[2]).unwrap().health,
        250,
        "exactly on blast boundary"
    );
    assert_eq!(
        sim.world().features.get(initial[3]).unwrap().health,
        250,
        "outside blast"
    );
}

fn replay_hashes() -> String {
    let mut f = feature("tree", 150, true);
    f.feature_dead = Some(1);
    let mut sc = scenario(
        vec![f, feature("stump", 80, true)],
        vec![placed(0, 10, 10), placed(0, 14, 10)],
    );
    sc.unit_defs[0].combat.weapons[0].reload_ticks = 10;
    let sc = Scenario::from_json(&sc.to_json_pretty()).unwrap();
    let mut log = CommandLog::new();
    log.record(Tick(1), PlayerId(0), attack(at(168, 168)));
    log.record(
        Tick(55),
        PlayerId(0),
        SimCommand::Move {
            units: UnitSet::single(UNIT),
            to: at(200, 168),
            queue: QueueMode::Replace,
        },
    );
    log.record(Tick(180), PlayerId(0), attack(at(232, 168)));
    log.record(
        Tick(240),
        PlayerId(0),
        SimCommand::Move {
            units: UnitSet::single(UNIT),
            to: at(280, 168),
            queue: QueueMode::Replace,
        },
    );
    let log = CommandLog::from_json(&log.to_json()).unwrap();
    let mut sim = Sim::new(&sc).unwrap();
    let hashes = sim.run(&log, 400);
    assert!(
        sim.world().features.is_empty(),
        "commands cleared both trees and successors"
    );
    assert!(
        sim.world().unit(UNIT).unwrap().pos.x > Fx::from_int(250),
        "walked through cleared terrain"
    );
    hashes.iter().map(|h| format!("{h:016x}\n")).collect()
}

const CHILD_OUT: &str = "OXA_FEATURE_DAMAGE_CHILD_OUT";

#[test]
fn child_writes_feature_damage_hashes() {
    let Ok(out) = std::env::var(CHILD_OUT) else {
        return;
    };
    std::fs::write(out, replay_hashes()).unwrap();
}

#[test]
fn feature_damage_command_replay_agrees_across_processes() {
    let dir = std::env::temp_dir().join(format!("oxa-feature-damage-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("child.txt");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "child_writes_feature_damage_hashes",
            "--exact",
            "--test-threads=1",
        ])
        .env(CHILD_OUT, &out)
        .status()
        .unwrap();
    assert!(status.success());
    let child = std::fs::read_to_string(&out).unwrap();
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(child.lines().count(), 400);
    assert_eq!(child, replay_hashes());
}

#[test]
fn immune_or_zero_health_features_do_not_shield_eligible_direct_target() {
    let mut immune = feature("immune", 500, false);
    immune.indestructible = true;
    let mut sim = Sim::new(&scenario(
        vec![
            immune,
            feature("zero", 0, false),
            feature("eligible", 250, false),
        ],
        vec![placed(0, 10, 10), placed(1, 10, 10), placed(2, 10, 10)],
    ))
    .unwrap();
    fire_one(&mut sim, at(168, 168));
    let health: Vec<_> = sim.world().features.iter().map(|(_, f)| f.health).collect();
    assert_eq!(health, vec![500, 0, 150]);
}
