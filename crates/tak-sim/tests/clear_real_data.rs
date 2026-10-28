//! Owned-data conversion and original COB callbacks, no retail fixtures stored.
use tak_commands::{FeatureId, PlayerId, QueueMode, SimCommand, Tick, UnitId, UnitSet};
use tak_sim::construction::reclaim::total_work;
use tak_sim::features::{FeatureSetup, PlacedFeature};
use tak_sim::{MapInfo, PlayerSetup, Sim, StartPos};

#[test]
fn authored_tree_and_zero_yield_wreck_clear_with_original_builder_script() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("Clear owned data") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    let rs = tak_content::Ruleset::load(&vfs, tak_content::RulesetKind::Base);
    for want_energy in [true, false] {
        let m = tak_sim::construction::content::load_match_with_builds(
            &vfs,
            "kmap/Adamantine Gate",
            &PlayerSetup::parse_list("ARA,TAR"),
            93,
        )
        .unwrap();
        let mut sc = m.scenario.clone();
        let authored = rs
            .feature(if want_energy {
                "AraTree01"
            } else {
                "arabuild_dead"
            })
            .expect("authored tree/wreck");
        let mut f = tak_sim::features::FeatureDef::from_content(authored);
        assert!(f.reclaimable);
        assert_eq!(f.energy > 0, want_energy);
        f.feature_dead = None; // isolated definition, no unrelated graph indices
        let expected = i64::from(f.energy) << 16;
        let total = total_work(&f);
        let d = &mut sc.unit_defs[0];
        assert!(d.build.canreclaim && d.build.workertime > 0 && d.build.builddistance > 0);
        let step = tak_sim::construction::progress::worker_step(d.build.workertime);
        let ticks = (total + step - 1) / step;
        d.economy.mogriumincome = 0; // isolate Clear income, leave work/range intact
        assert!(i64::from(d.economy.mogriumstorage) << 16 >= expected);
        sc.players.truncate(1);
        sc.map = MapInfo::flat(512, 512, vec![StartPos { x: 256, y: 256 }]);
        sc.economy.rules.start_mana = 0;
        *sc.features = FeatureSetup {
            defs: vec![f],
            placed: vec![PlacedFeature {
                def: 0,
                x: 18,
                z: 16,
            }],
            ..Default::default()
        };
        let mut sim = Sim::with_scripts(&sc, Box::new(m.scripts)).unwrap();
        sim.step(&[]);
        assert!(tak_sim::construction::can_clear(
            sim.world(),
            UnitId::new(0, 0)
        ));
        let mut log = tak_commands::CommandLog::new();
        log.record(
            Tick(1),
            PlayerId(0),
            SimCommand::Clear {
                units: UnitSet::single(UnitId::new(0, 0)),
                target: FeatureId::new(0, 0),
                queue: QueueMode::Replace,
            },
        );
        for _ in 0..ticks {
            sim.step(log.for_tick(sim.tick()));
        }
        assert!(sim.world().features.is_empty());
        assert_eq!(
            tak_sim::economy::player(sim.world(), PlayerId(0))
                .unwrap()
                .mana,
            expected
        );
        assert!(sim.script_stats().errors.is_empty());
        assert!(sim.script_stats().runaway.is_empty());
    }
}
