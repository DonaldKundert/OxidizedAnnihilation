//! Command-only checkpoint continuation through feature damage and replacement.
use tak_commands::{CommandLog, PlayerId, QueueMode, SimCommand, Target, Tick, UnitId, UnitSet};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_math::{Fx, Vec2};
use tak_sim::combat::{DamageStats, WeaponKind, WeaponStats};
use tak_sim::features::{FeatureDef, FeatureSetup, PlacedFeature};
use tak_sim::map::TerrainGrid;

#[test]
fn partial_damage_successor_and_cleared_checkpoints_resume_identically() {
    let mut sc = tak_sim::testing::example_scenario(0xc1ea, 1);
    let shooter = &mut sc.unit_defs[0];
    shooter.combat.max_damage = 1000;
    shooter.combat.weapons = vec![WeaponStats {
        kind: WeaponKind::RemoteEffect,
        range: 500,
        reload_ticks: 3,
        aim_tolerance: Some(u16::MAX),
        damage: DamageStats {
            default: 100,
            ..Default::default()
        },
        ..Default::default()
    }];
    sc.map.terrain = Some(TerrainGrid {
        cols: 128,
        rows: 128,
        cell_size: 16,
        flags: vec![0; 128 * 128],
    });
    sc.features = Box::new(FeatureSetup {
        defs: vec![
            FeatureDef {
                name: "synthetic_tree".into(),
                footprint_x: 1,
                footprint_z: 1,
                blocking: true,
                damage: 250,
                feature_dead: Some(1),
                ..Default::default()
            },
            FeatureDef {
                name: "synthetic_stump".into(),
                footprint_x: 2,
                footprint_z: 2,
                blocking: true,
                damage: 150,
                ..Default::default()
            },
        ],
        placed: vec![PlacedFeature {
            def: 0,
            x: 20,
            z: 16,
        }],
        ..Default::default()
    });
    let source = CheckpointSource::synthetic(sc);
    let mut log = CommandLog::new();
    log.record(
        Tick(1),
        PlayerId(0),
        SimCommand::Attack {
            units: UnitSet::single(UnitId::new(0, 0)),
            target: Target::Ground(Vec2::from_ints(328, 264)),
            queue: QueueMode::Replace,
        },
    );
    let log = CommandLog::from_json(&log.to_json()).unwrap();
    let mut session = ReplaySession::new(source.clone(), None).unwrap();
    let mut seen = [false; 3];
    for _ in 0..60 {
        session.advance(&log, 1).unwrap();
        let fs = &session.sim().world().features;
        let stage = match fs.iter().next() {
            Some((_, f)) if f.def == 0 && f.health < 250 => Some(0),
            Some((id, f)) if f.def == 1 && id.generation > 0 => Some(1),
            None => Some(2),
            _ => None,
        };
        if let Some(stage) = stage.filter(|&n| !seen[n]) {
            seen[stage] = true;
            let encoded = session.checkpoint().unwrap().to_json_pretty().unwrap();
            let mut resumed = Checkpoint::from_json(&encoded)
                .unwrap()
                .restore(None, ReplayBudget::default())
                .unwrap();
            assert_eq!(resumed.sim().world(), session.sim().world());
            // Compare to a fresh run from tick zero, not two copies of restore.
            let mut uninterrupted = ReplaySession::new(source.clone(), None).unwrap();
            uninterrupted.advance(&log, resumed.sim().tick().0).unwrap();
            let expected = uninterrupted.advance(&log, 40).unwrap();
            assert_eq!(resumed.advance(&log, 40).unwrap(), expected);
            assert!(resumed.sim().world().features.is_empty());
            assert!(!tak_sim::pathfinding::is_obstacle(
                resumed.sim().world(),
                20,
                16
            ));
            assert!(
                resumed
                    .sim()
                    .world()
                    .units()
                    .all(|(_, u)| u.pos.x >= Fx::ZERO)
            );
        }
    }
    assert_eq!(
        seen, [true; 3],
        "fixture must pass all three checkpoint stages"
    );
}
