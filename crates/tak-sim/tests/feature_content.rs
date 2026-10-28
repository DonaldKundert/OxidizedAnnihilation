//! Synthetic content conversion: ownership, authored anchoring, and bounded input.
use tak_content::{FeatureAnchor, MapDef, PassGrid, PassRule, Ruleset};
use tak_sim::features::{self, FeatureSetup, MAX_FEATURE_DEFS, MAX_PLACED_FEATURES};
use tak_sim::map::terrain;
use tak_sim::{Scenario, Sim};

fn definition(name: &str) -> tak_content::FeatureDef {
    let text = format!("[{name}]\n{{\nfootprintx=3;\nfootprintz=3;\nblocking=1;\ndamage=40;\n}}\n");
    let doc = tak_formats::tdf::parse(text.as_bytes()).unwrap();
    tak_content::feature::features_from_tdf(&doc, "synthetic.tdf", &mut Default::default())
        .remove(0)
}

fn content(anchor: FeatureAnchor, blocks: bool) -> (MapDef, Ruleset) {
    let mut rs = Ruleset::default();
    rs.features.insert("test".into(), definition("test"));
    let rule = PassRule {
        anchor,
        features_block: blocks,
        ..PassRule::default()
    };
    let mut flags = vec![0; 256];
    let first = if anchor == FeatureAnchor::Center {
        3
    } else {
        4
    };
    if blocks {
        for z in first..first + 3 {
            for x in first..first + 3 {
                flags[z * 16 + x] =
                    terrain::FEATURE | terrain::WATER | terrain::STEEP | terrain::PAVED;
            }
        }
    }
    flags[0] = terrain::FEATURE; // authored/unmanaged cell must survive conversion
    let map = MapDef {
        name: "synthetic".into(),
        ota: Default::default(),
        width: 16,
        height: 16,
        sea_level: 0,
        heights: vec![0; 256],
        kingdom: None,
        palette: None,
        features: vec![
            tak_content::PlacedFeature {
                name: "test".into(),
                def: Some("test".into()),
                x: 4,
                z: 4
            };
            2
        ],
        paved_cells: 0,
        marker_cells: 0,
        start_positions: vec![],
        player_slots: 0,
        crt: None,
        passability: PassGrid {
            width: 16,
            height: 16,
            rule,
            cells: flags,
        },
        issues: vec![],
        dangling: vec![],
        coverage: Default::default(),
    };
    (map, rs)
}

#[test]
fn content_stamps_become_owned_without_erasing_other_terrain() {
    for anchor in [FeatureAnchor::TopLeft, FeatureAnchor::Center] {
        let (map, rs) = content(anchor, true);
        let before = map.passability.cells.clone();
        let sc = Scenario::from_content(&map, &rs, &[]).unwrap();
        let first = if anchor == FeatureAnchor::Center {
            3
        } else {
            4
        };
        assert_eq!(
            (sc.features.placed[0].x, sc.features.placed[0].z),
            (first, first)
        );
        let terrain = &sc.map.terrain.as_ref().unwrap().flags;
        assert_eq!(terrain[0], terrain::FEATURE);
        assert_eq!(
            terrain[(first * 16 + first) as usize],
            terrain::WATER | terrain::STEEP | terrain::PAVED
        );
        assert_eq!(map.passability.cells, before, "content remains immutable");
        let mut sim = Sim::new(&sc).unwrap();
        let ids: Vec<_> = sim.world().features.iter().map(|(id, _)| id).collect();
        assert_eq!(
            sim.world()
                .pathing
                .obstacles_at(first as i32, first as i32, 16),
            2
        );
        features::remove(sim.world_mut(), ids[0]);
        assert_eq!(
            sim.world()
                .pathing
                .obstacles_at(first as i32, first as i32, 16),
            1
        );
        features::remove(sim.world_mut(), ids[1]);
        assert_eq!(
            sim.world()
                .pathing
                .obstacles_at(first as i32, first as i32, 16),
            0
        );
        assert_eq!(&sim.world().map().terrain.as_ref().unwrap().flags, terrain);
    }
}

#[test]
fn disabled_content_feature_blocking_is_preserved_and_serialized() {
    let (map, rs) = content(FeatureAnchor::Center, false);
    let sc = Scenario::from_content(&map, &rs, &[]).unwrap();
    assert!(!sc.features.placed_features_block);
    assert_eq!(
        sc.map.terrain.as_ref().unwrap().flags,
        map.passability.cells
    );
    let restored = Scenario::from_json(&sc.to_json_pretty()).unwrap();
    let sim = Sim::new(&restored).unwrap();
    assert!(sim.world().features.iter().all(|(_, f)| !f.obstacle));
    assert_eq!(sim.world().pathing.obstacles_at(3, 3, 16), 0);
    let legacy: FeatureSetup = serde_json::from_str("{}").unwrap();
    assert!(
        legacy.placed_features_block,
        "omitted field retains authored scenario default"
    );
}

#[test]
fn too_many_placed_features_fail_before_terrain_normalization() {
    let (mut map, rs) = content(FeatureAnchor::TopLeft, true);
    map.features
        .resize(MAX_PLACED_FEATURES + 1, map.features[0].clone());
    let before = map.passability.cells.clone();
    let error = Scenario::from_content(&map, &rs, &[]).unwrap_err();
    assert!(error.to_string().contains("too many placed features"));
    assert_eq!(map.passability.cells, before);
}

#[test]
fn successor_definition_capacity_fails_closed() {
    let (mut map, mut rs) = content(FeatureAnchor::TopLeft, true);
    map.features.clear();
    rs.features.clear();
    for i in 0..MAX_FEATURE_DEFS {
        let name = format!("f{i}");
        let mut def = definition(&name);
        if i + 1 == MAX_FEATURE_DEFS {
            def.feature_dead = Some("overflow".into());
        }
        rs.features.insert(name.clone(), def);
        map.features.push(tak_content::PlacedFeature {
            name: name.clone(),
            def: Some(name),
            x: 4,
            z: 4,
        });
    }
    rs.features
        .insert("overflow".into(), definition("overflow"));
    assert!(
        Scenario::from_content(&map, &rs, &[])
            .unwrap_err()
            .to_string()
            .contains("too many feature definitions")
    );
}

#[test]
fn units_only_is_parsed_and_hashed() {
    let unit = |only| {
        let text = format!(
            "[UNITINFO]\n{{\nUnitName=Test;\nMaxDamage=100;\n}}\n[WEAPON1]\n{{\ntype=Ballistic;\nunitsonly={only};\n[DAMAGE]\n{{\ndefault=100;\n}}\n}}\n"
        );
        let doc = tak_formats::tdf::parse(text.as_bytes()).unwrap();
        tak_content::unit::unit_from_fbi(&doc, "synthetic.fbi", &mut Default::default()).unwrap()
    };
    let a = tak_sim::content::unit_def_from_fbi(&unit(0));
    let b = tak_sim::content::unit_def_from_fbi(&unit(1));
    assert!(!a.combat.weapons[0].units_only);
    assert!(b.combat.weapons[0].units_only);
    let mut sc = tak_sim::testing::example_scenario(1, 1);
    sc.unit_defs[0] = a;
    let h = Sim::new(&sc).unwrap().state_hash();
    sc.unit_defs[0] = b;
    assert_ne!(Sim::new(&sc).unwrap().state_hash(), h);
}

#[test]
fn extreme_blast_falloff_uses_wide_intermediates() {
    for edge in [
        tak_math::Fx::MIN,
        tak_math::Fx::ZERO,
        tak_math::Fx::ONE,
        tak_math::Fx::MAX,
    ] {
        let amount = tak_sim::combat::area_damage(i32::MAX, 12_000, i32::MAX / 2, edge);
        if edge == tak_math::Fx::ONE {
            assert_eq!(amount, i32::MAX);
        }
    }
}

#[test]
fn clear_capability_is_authored_hashed_and_independent_of_builder_flag() {
    let unit = |clear| {
        let text = format!(
            "[UNITINFO]\n{{\nUnitName=Test;\ncanreclaim={clear};\nworkertime=10;\nbuilddistance=80;\n}}\n"
        );
        let doc = tak_formats::tdf::parse(text.as_bytes()).unwrap();
        let u = tak_content::unit::unit_from_fbi(&doc, "synthetic.fbi", &mut Default::default())
            .unwrap();
        tak_sim::content::unit_def_from_fbi(&u)
    };
    let a = unit(0);
    let b = unit(1);
    assert!(!a.build.canreclaim && b.build.canreclaim);
    assert!(!b.build.builder);
    let mut sc = tak_sim::testing::example_scenario(1, 1);
    sc.unit_defs[0] = a;
    let before = Sim::new(&sc).unwrap().state_hash();
    sc.unit_defs[0] = b;
    assert_ne!(Sim::new(&sc).unwrap().state_hash(), before);
    let sc = Scenario::from_json(&sc.to_json_pretty()).unwrap();
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    assert!(tak_sim::construction::can_clear(
        sim.world(),
        tak_sim::UnitId::new(0, 0)
    ));
}

#[test]
fn authored_reclamate_self_and_cycle_links_do_not_spawn_clear_successors() {
    let src = "[synthetic_a]\n{\nreclaimable=1;\nenergy=1;\nfeaturereclamate=synthetic_a;\n}\n[synthetic_b]\n{\nreclaimable=1;\nenergy=1;\nfeaturereclamamate=synthetic_c;\n}\n[synthetic_c]\n{\nreclaimable=1;\nenergy=1;\nfeaturereclamate=synthetic_b;\n}\n";
    let doc = tak_formats::tdf::parse(src.as_bytes()).unwrap();
    let defs =
        tak_content::feature::features_from_tdf(&doc, "synthetic.tdf", &mut Default::default());
    assert!(defs.iter().all(|d| d.feature_reclamate.is_some()));
    for def in defs {
        let mut sc = tak_sim::testing::example_scenario(1, 1);
        let d = &mut sc.unit_defs[0];
        d.build.canreclaim = true;
        d.build.workertime = 1000;
        d.build.builddistance = 100;
        d.economy.mogriumstorage = 100;
        sc.economy.rules.start_mana = 0;
        *sc.features = FeatureSetup {
            defs: vec![features::FeatureDef::from_content(&def)],
            placed: vec![features::PlacedFeature {
                def: 0,
                x: 18,
                z: 16,
            }],
            ..Default::default()
        };
        let mut sim = Sim::new(&sc).unwrap();
        let mut log = tak_commands::CommandLog::new();
        log.record(
            tak_commands::Tick(0),
            tak_commands::PlayerId(0),
            tak_commands::SimCommand::Clear {
                units: tak_commands::UnitSet::single(tak_sim::UnitId::new(0, 0)),
                target: tak_commands::FeatureId::new(0, 0),
                queue: tak_commands::QueueMode::Replace,
            },
        );
        for _ in 0..3 {
            sim.step(log.for_tick(sim.tick()));
        }
        assert!(sim.world().features.is_empty());
        assert_eq!(
            tak_sim::economy::player(sim.world(), tak_commands::PlayerId(0))
                .unwrap()
                .mana,
            1 << 16
        );
    }
}
