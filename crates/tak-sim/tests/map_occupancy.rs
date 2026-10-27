//! Synthetic regression coverage for the real-map fixtures' occupancy helper.

mod map_common;

use map_common::blocked;
use tak_commands::UnitDefId;
use tak_math::{Angle, Vec2};
use tak_sim::features::{self, FeatureDef, PlacedFeature};
use tak_sim::map::{TerrainGrid, terrain};
use tak_sim::testing::example_scenario;
use tak_sim::{Scenario, Sim};

fn scenario() -> Scenario {
    let mut sc = example_scenario(7, 1);
    sc.map.terrain = Some(TerrainGrid {
        cols: 128,
        rows: 128,
        cell_size: 16,
        flags: vec![0; 128 * 128],
    });
    sc.features.defs.push(FeatureDef {
        name: "synthetic_blocker".into(),
        footprint_x: 2,
        footprint_z: 2,
        blocking: true,
        ..FeatureDef::default()
    });
    sc.features.corpses = vec![Some(0)];
    sc
}

#[test]
fn ground_cell_selection_keeps_static_terrain_semantics() {
    let mut sc = scenario();
    let flags = &mut sc.map.terrain.as_mut().unwrap().flags;
    for (x, flag) in [terrain::DEEP, terrain::STEEP, terrain::FEATURE]
        .into_iter()
        .enumerate()
    {
        flags[x] = flag;
    }
    flags[3] = terrain::WATER | terrain::PAVED;
    let sim = Sim::new(&sc).unwrap();
    for x in 0..3 {
        assert!(blocked(sim.world(), Vec2::from_ints(x * 16 + 8, 8)));
    }
    assert!(!blocked(sim.world(), Vec2::from_ints(56, 8)));
    assert!(!blocked(sim.world(), Vec2::from_ints(72, 8)));
    let free = Sim::new(&example_scenario(7, 1)).unwrap();
    assert!(!blocked(free.world(), Vec2::from_ints(72, 8)));
}

#[test]
fn ground_cell_selection_tracks_live_feature_addition_and_removal() {
    let mut sim = Sim::new(&scenario()).unwrap();
    let p = Vec2::from_ints(160, 160);
    assert!(!blocked(sim.world(), p));
    // Works before movement layers exist, as fixture placement often does.
    assert!(!sim.world().pathing.is_built());
    let id = features::spawn_corpse(sim.world_mut(), UnitDefId(0), p, Angle::ZERO, 1)
        .expect("synthetic corpse");
    assert_eq!(sim.world().map().terrain_at(p), 0);
    assert!(blocked(sim.world(), p), "live feature is not open ground");
    sim.step(&[]);
    assert!(blocked(sim.world(), p));
    features::remove(sim.world_mut(), id).unwrap();
    assert!(
        !blocked(sim.world(), p),
        "removed feature releases its cell"
    );
}

#[test]
fn ground_cell_selection_keeps_overlapping_blockers_and_static_flags() {
    let mut sc = scenario();
    sc.features.placed = vec![
        PlacedFeature {
            def: 0,
            x: 10,
            z: 10,
        },
        PlacedFeature {
            def: 0,
            x: 11,
            z: 10,
        },
    ];
    sc.map.terrain.as_mut().unwrap().flags[11 * 128 + 11] = terrain::STEEP;
    let mut sim = Sim::new(&sc).unwrap();
    let ids: Vec<_> = sim.world().features.iter().map(|(id, _)| id).collect();
    let left = Vec2::from_ints(168, 168);
    let overlap = Vec2::from_ints(184, 168);
    let right = Vec2::from_ints(200, 168);
    let steep_overlap = Vec2::from_ints(184, 184);
    for p in [left, overlap, right, steep_overlap] {
        assert!(blocked(sim.world(), p));
    }
    assert_eq!(sim.world().map().terrain_at(overlap), 0);
    features::remove(sim.world_mut(), ids[0]).unwrap();
    assert!(!blocked(sim.world(), left));
    assert!(blocked(sim.world(), overlap));
    assert!(blocked(sim.world(), right));
    features::remove(sim.world_mut(), ids[1]).unwrap();
    assert!(!blocked(sim.world(), overlap));
    assert!(!blocked(sim.world(), right));
    assert!(blocked(sim.world(), steep_overlap));
}
