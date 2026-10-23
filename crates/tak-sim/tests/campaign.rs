//! Hand-authored synthetic mission fixtures only; no retail data.
use std::process::Command;
use tak_content::{MapDef, PassGrid, Ruleset};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::campaign::{CampaignPlayer, enable_objectives, scenario_from_campaign};
use tak_sim::{PlayerId, Scenario, Sim, UnitDefId, UnitId, World};

fn fixture() -> (MapDef, Ruleset, Vec<CampaignPlayer>) {
    let text = "[GLOBALHEADER]\n{\ndestroyallunits=1;\n[MAP DATA]\n{\n[UNITS]\n{\n[UNIT0]\n{\nunitname=TESTSOLDIER;\nplayer=10;\nxpos=7;\nzpos=8;\nangle=90;\nhealthpercentage=25;\nmanapercentage=50;\nkills=3;\ninitialmission=patrol;\n}\n[UNIT1]\n{\nunitname=TESTKING;\nplayer=1;\nxpos=2;\nzpos=3;\nangle=-90;\n}\n}\n}\n}\n";
    let doc = tak_formats::tdf::parse(text.as_bytes()).unwrap();
    let mut ctx = tak_content::fields::Ctx::default();
    let ota = tak_content::ota::ota_from_tdf(&doc, "synthetic.ota", &mut ctx);
    assert!(ctx.issues.is_empty());
    let map = MapDef {
        name: "synthetic".into(),
        ota,
        width: 32,
        height: 32,
        sea_level: 0,
        heights: vec![0; 1024],
        kingdom: None,
        palette: None,
        features: vec![],
        paved_cells: 0,
        marker_cells: 0,
        start_positions: vec![],
        player_slots: 0,
        crt: None,
        passability: PassGrid {
            width: 32,
            height: 32,
            rule: Default::default(),
            cells: vec![0; 1024],
        },
        issues: vec![],
        dangling: vec![],
        coverage: Default::default(),
    };
    let mut rs = Ruleset::default();
    for name in ["TESTKING", "TESTSOLDIER"] {
        let text = format!("[UNITINFO]\n{{\nunitname={name};\nmaxdamage=200;\nmaxmana=80;\n}}\n");
        let doc = tak_formats::tdf::parse(text.as_bytes()).unwrap();
        let unit = tak_content::unit::unit_from_fbi(&doc, "synthetic.fbi", &mut ctx).unwrap();
        rs.units.insert(name.to_ascii_lowercase(), unit);
    }
    let sides = tak_formats::tdf::parse(
        b"[SIDE0]\n{\nname=Test;\nnameprefix=TST;\ncommander=TESTKING;\n}\n",
    )
    .unwrap();
    rs.sides = tak_content::gamedata::sides_from_tdf(&sides, "synthetic.tdf", &mut ctx);
    (
        map,
        rs,
        vec![
            CampaignPlayer {
                ota_slot: 1,
                side: "TST".into(),
            },
            CampaignPlayer {
                ota_slot: 10,
                side: "TST".into(),
            },
        ],
    )
}

fn scenario() -> Scenario {
    let (map, rs, players) = fixture();
    scenario_from_campaign(&map, &rs, &players).unwrap().0
}

#[test]
fn authored_units_replace_monarchs_and_keep_file_order() {
    let (map, rs, players) = fixture();
    let (sc, warnings) = scenario_from_campaign(&map, &rs, &players).unwrap();
    assert!(warnings.iter().any(|s| s == "unit 0: initialmission"));
    assert!(sc.victory.mission.is_none());
    let roundtrip = Scenario::from_json(&sc.to_json_pretty()).unwrap();
    assert_eq!(sc, roundtrip);
    let mut world = World::new(&sc).unwrap();
    let ids = tak_sim::systems::spawn_start_units(&mut world);
    assert_eq!(ids, vec![UnitId::new(0, 0), UnitId::new(1, 0)]);
    assert_eq!(world.unit_count(), 2);
    let first = world.unit(ids[0]).unwrap();
    assert_eq!(world.defs().get(first.def).unwrap().name, "TESTSOLDIER");
    assert_eq!(first.owner, PlayerId(1));
    assert_eq!(first.pos, Vec2::from_ints(120, 136));
    assert_eq!(first.heading, Angle::QUARTER);
    assert_eq!(first.combat.health, 50);
    assert_eq!(first.combat.kills, 3);
    assert_eq!(first.economy.mana(), Fx::from_int(40));
    assert_eq!(world.players()[0].monarch, Some(ids[1]));
    assert_eq!(world.players()[1].monarch, None);
    assert_eq!(
        world.unit(ids[1]).unwrap().heading,
        Angle::from_degrees(270)
    );
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    assert_eq!(sim.world().unit_count(), 2);
    sim.step(&[]);
    assert_eq!(sim.world().unit_count(), 2);
}

#[test]
fn skirmish_json_and_empty_explicit_start_are_distinct() {
    let sc = tak_sim::testing::example_scenario(7, 2);
    assert!(!sc.to_json_pretty().contains("initial_units"));
    let sc = Scenario::from_json(&sc.to_json_pretty()).unwrap();
    let mut sim = Sim::new(&sc).unwrap();
    // Independently measured on unmodified e1c0018, not regenerated here.
    assert_eq!(sim.state_hash(), 0x312263619e949bfa);
    assert_eq!(sim.step(&[]), 0xcd79724308262668);
    assert_eq!(sim.world().unit_count(), 2);
    let mut empty = sc.clone();
    empty.initial_units = Some(vec![]);
    let mut sim_empty = Sim::new(&empty).unwrap();
    assert_ne!(sim_empty.state_hash(), Sim::new(&sc).unwrap().state_hash());
    sim_empty.step(&[]);
    assert_eq!(sim_empty.world().unit_count(), 0);
}

#[test]
fn malformed_placements_are_rejected_not_clamped_or_reassigned() {
    let (map, rs, players) = fixture();
    for mutation in 0..9 {
        let mut bad = map.clone();
        let u = &mut bad.ota.units[0];
        match mutation {
            0 => u.player = None,
            1 => u.player = Some(9),
            2 => u.unitname = "MISSING".into(),
            3 => u.x = -1,
            4 => u.z = 32,
            5 => u.health_percentage = Some(0),
            6 => u.mana_percentage = Some(101),
            7 => u.health_percentage = Some(-1),
            _ => u.kills = Some(-1),
        }
        assert!(
            scenario_from_campaign(&bad, &rs, &players).is_err(),
            "mutation {mutation}"
        );
    }
    let mut duplicated = players.clone();
    duplicated[1].ota_slot = 1;
    assert!(scenario_from_campaign(&map, &rs, &duplicated).is_err());
    let mut sc = scenario();
    sc.initial_units.as_mut().unwrap()[0].owner = PlayerId(15);
    assert!(sc.validate().is_err());
    sc.initial_units.as_mut().unwrap()[0].owner = PlayerId(1);
    sc.initial_units.as_mut().unwrap()[0].def = UnitDefId(u32::MAX);
    assert!(sc.validate().is_err());
}

#[test]
fn pending_placements_are_hashed_before_tick_zero() {
    let a = scenario();
    let mut b = a.clone();
    b.initial_units.as_mut().unwrap()[0].mana_percentage = 49;
    assert_ne!(
        Sim::new(&a).unwrap().state_hash(),
        Sim::new(&b).unwrap().state_hash()
    );
}

#[test]
fn objectives_are_explicit_and_refuse_unsupported_owner_layouts() {
    let (mut map, rs, mut players) = fixture();
    let mut sc = scenario();
    assert!(enable_objectives(&mut sc, &map, &players).is_err());
    assert!(sc.victory.mission.is_none());
    players[1].ota_slot = 2;
    map.ota.units[0].player = Some(2);
    let (mut sc, _) = scenario_from_campaign(&map, &rs, &players).unwrap();
    enable_objectives(&mut sc, &map, &players).unwrap();
    assert!(sc.victory.mission.is_some());
}

fn hashes() -> Vec<u64> {
    let mut sim = Sim::new(&scenario()).unwrap();
    (0..300).map(|_| sim.step(&[])).collect()
}

#[test]
fn campaign_child() {
    if std::env::var_os("CAMPAIGN_HASH_CHILD").is_some() {
        println!("CAMPAIGN_HASHES={:?}", hashes());
    }
}

#[test]
fn campaign_hashes_match_across_processes() {
    let expected = format!("CAMPAIGN_HASHES={:?}", hashes());
    for _ in 0..2 {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "campaign_child", "--nocapture"])
            .env("CAMPAIGN_HASH_CHILD", "1")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8(out.stdout).unwrap().contains(&expected));
    }
}

/// Corpus coverage of placement conversion only. Using ARA as an explicit
/// placeholder side here is NOT evidence that mission ownership/AI is decoded.
#[test]
fn retail_campaign_placements_load_without_dropping_units() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("campaign_placements") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    let rs = Ruleset::load(&vfs, tak_content::RulesetKind::Base);
    let (mut maps, mut placements) = (0, 0);
    for f in vfs.list("") {
        if !f.path.starts_with("missions/") || !f.path.ends_with(".ota") {
            continue;
        }
        let name = f.path.strip_suffix(".ota").unwrap();
        let map = MapDef::load(&vfs, &rs, name).unwrap();
        if map.ota.units.is_empty() {
            continue;
        }
        let slots: std::collections::BTreeSet<_> =
            map.ota.units.iter().map(|u| u.player.unwrap()).collect();
        let players: Vec<_> = slots
            .into_iter()
            .map(|slot| CampaignPlayer {
                ota_slot: u8::try_from(slot).unwrap(),
                side: "ARA".into(),
            })
            .collect();
        let (sc, _) =
            scenario_from_campaign(&map, &rs, &players).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut world = World::new(&sc).unwrap();
        tak_sim::systems::spawn_start_units(&mut world);
        assert_eq!(world.unit_count(), map.ota.units.len(), "{name}");
        maps += 1;
        placements += world.unit_count();
    }
    assert_eq!((maps, placements), (74, 7341));
}

#[test]
fn scripts_receive_one_create_per_authored_unit_with_initial_health() {
    use std::sync::{Arc, Mutex};
    struct Host(Arc<Mutex<Vec<(UnitId, i32)>>>);
    impl tak_sim::ScriptHost for Host {
        fn on_create(&mut self, ctx: &tak_sim::ScriptCtx) {
            self.0.lock().unwrap().push((ctx.unit, ctx.health_percent));
        }
    }
    let created = Arc::new(Mutex::new(Vec::new()));
    let mut sim = Sim::with_scripts(&scenario(), Box::new(Host(created.clone()))).unwrap();
    sim.step(&[]);
    sim.step(&[]);
    assert_eq!(
        *created.lock().unwrap(),
        vec![(UnitId::new(0, 0), 25), (UnitId::new(1, 0), 100)]
    );
}
