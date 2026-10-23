//! Replay checkpoints reconstruct target-owned fractional repair credit.
use tak_commands::{CommandLog, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId, UnitSet};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_sim::campaign::InitialUnit;
use tak_sim::construction::BuildDef;
use tak_sim::{MapInfo, PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario, StartPos};

#[test]
fn mid_repair_checkpoint_restores_fractional_credit_and_every_suffix_hash() {
    let mut healer = tak_sim::testing::monarch_def("synthetic_healer");
    healer.combat.max_damage = 100;
    healer.build = BuildDef {
        builder: true,
        mobile: true,
        workertime: 10,
        builddistance: 200,
        ..Default::default()
    };
    healer.economy.mogriumstorage = 100;
    let mut target = tak_sim::testing::monarch_def("synthetic_target");
    target.combat.max_damage = 100;
    target.build = BuildDef {
        buildtime: 1000 << 16,
        buildcost: 90,
        ..Default::default()
    };
    let position =
        |x| serde_json::from_value(serde_json::json!({"x":x*65536,"y":200*65536})).unwrap();
    let initial = |def, x, hp| InitialUnit {
        def: UnitDefId(def),
        owner: PlayerId(0),
        pos: position(x),
        heading: Default::default(),
        health_percentage: hp,
        mana_percentage: 0,
        kills: 0,
    };
    let mut scenario = Scenario {
        version: SCENARIO_FORMAT_VERSION,
        seed: 83,
        initial_units: Some(vec![initial(0, 200, 100), initial(1, 260, 50)]),
        map: MapInfo::flat(1024, 1024, vec![StartPos { x: 200, y: 200 }]),
        unit_defs: vec![healer, target],
        players: vec![PlayerSpec {
            start: 0,
            monarch: UnitDefId(0),
        }],
        economy: Default::default(),
        features: Default::default(),
        victory: Default::default(),
    };
    scenario.economy.rules.start_mana = 1; // The suffix also crosses a mana shortage.
    let mut session = ReplaySession::new(CheckpointSource::synthetic(scenario), None).unwrap();
    let mut log = CommandLog::new();
    log.record(
        Tick(1),
        PlayerId(0),
        SimCommand::Heal {
            units: UnitSet::single(UnitId::new(0, 0)),
            target: UnitId::new(1, 0),
            queue: QueueMode::Replace,
        },
    );
    session.advance(&log, 17).unwrap();
    let repair = session
        .sim()
        .world()
        .unit(UnitId::new(1, 0))
        .unwrap()
        .build
        .repair
        .unwrap();
    assert!(repair.hp_remainder > 0 && repair.mana_remainder > 0);
    let checkpoint = session.checkpoint().unwrap();
    let decoded = Checkpoint::from_json(&checkpoint.to_json_pretty().unwrap()).unwrap();
    let mut restored = decoded.restore(None, ReplayBudget::default()).unwrap();
    assert_eq!(session.sim().state_hash(), restored.sim().state_hash());
    assert_eq!(
        session.advance(&log, 120).unwrap(),
        restored.advance(&log, 120).unwrap()
    );
    assert_eq!(
        session
            .sim()
            .world()
            .unit(UnitId::new(1, 0))
            .unwrap()
            .build
            .repair,
        restored
            .sim()
            .world()
            .unit(UnitId::new(1, 0))
            .unwrap()
            .build
            .repair
    );
}
