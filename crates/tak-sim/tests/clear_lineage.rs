//! Resource conservation through weapon-death successors, including cycles.
use tak_commands::{
    FeatureId, PlayerId, QueueMode, SimCommand, Target, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Angle, Vec2};
use tak_sim::combat::{DamageStats, WeaponKind, WeaponStats};
use tak_sim::features::{self, FeatureDef, FeatureSetup, PlacedFeature};
use tak_sim::{Scenario, Sim};
const ACTOR: UnitId = UnitId::new(0, 0);

fn setup(values: &[i32], damage: i32, worker: i32, storage: i32) -> Scenario {
    let mut sc = tak_sim::testing::example_scenario(83, 1);
    let actor = &mut sc.unit_defs[0];
    actor.build.canreclaim = true;
    actor.build.workertime = worker;
    actor.build.builddistance = 200;
    actor.combat.max_damage = 100;
    actor.combat.weapons = vec![WeaponStats {
        kind: WeaponKind::RemoteEffect,
        range: 500,
        reload_ticks: 1,
        aim_tolerance: Some(u16::MAX),
        damage: DamageStats {
            default: 1_000_000,
            ..Default::default()
        },
        ..Default::default()
    }];
    actor.economy.mogriumstorage = storage;
    sc.unit_defs[1].economy.mogriumstorage = 1000;
    sc.economy.rules.start_mana = 0;
    *sc.features = FeatureSetup {
        defs: values
            .iter()
            .enumerate()
            .map(|(i, &energy)| FeatureDef {
                name: format!("synthetic_stage_{i}"),
                footprint_x: 1,
                footprint_z: 1,
                reclaimable: true,
                damage,
                energy,
                feature_dead: Some(((i + 1) % values.len()) as u32),
                ..Default::default()
            })
            .collect(),
        placed: vec![PlacedFeature {
            def: 0,
            x: 20,
            z: 16,
        }],
        corpses: vec![Some(0)],
        ..Default::default()
    };
    sc
}
fn issue(sim: &mut Sim, command: SimCommand) {
    sim.step(&[tak_commands::LoggedCommand {
        tick: sim.tick(),
        player: PlayerId(0),
        seq: 0,
        command,
    }]);
}
fn target(sim: &Sim) -> FeatureId {
    sim.world().features.iter().next().unwrap().0
}
fn clear(sim: &mut Sim) {
    let target = target(sim);
    issue(
        sim,
        SimCommand::Clear {
            units: UnitSet::single(ACTOR),
            target,
            queue: QueueMode::Replace,
        },
    );
}
fn attack(sim: &mut Sim) {
    let old = target(sim);
    let point = sim.world().features.get(old).unwrap().center();
    issue(
        sim,
        SimCommand::Attack {
            units: UnitSet::single(ACTOR),
            target: Target::Ground(point),
            queue: QueueMode::Replace,
        },
    );
    assert!(sim.world().features.get(old).is_none());
    assert_eq!(target(sim).generation, old.generation + 1);
}
fn mana(sim: &Sim) -> i64 {
    tak_sim::economy::player(sim.world(), PlayerId(0))
        .unwrap()
        .mana
}
fn finish(sim: &mut Sim) {
    for _ in 0..1000 {
        if sim.world().features.is_empty() {
            return;
        }
        sim.step(&[]);
    }
    panic!("Clear stalled");
}

#[test]
fn command_only_self_and_two_stage_weapon_cycles_cannot_mint_mana() {
    for values in [&[30][..], &[30, 60][..]] {
        let sc = setup(values, 10, 90, 1000);
        let mut sim = Sim::new(&sc).unwrap();
        sim.step(&[]);
        let maximum = i64::from(*values.iter().max().unwrap()) << 16;
        for _ in 0..100 {
            clear(&mut sim);
            assert!(!sim.world().features.is_empty(), "partial work fixture");
            assert!(mana(&sim) <= maximum);
            let claimed = sim
                .world()
                .features
                .get(target(&sim))
                .unwrap()
                .reclaim_claimed;
            attack(&mut sim);
            let f = sim.world().features.get(target(&sim)).unwrap();
            assert_eq!(f.reclaim_done, 0);
            assert_eq!(f.reclaim_claimed, claimed);
        }
        assert_eq!(mana(&sim), maximum);
        clear(&mut sim);
        finish(&mut sim);
        assert_eq!(mana(&sim), maximum);
    }
}

#[test]
fn lower_stage_cannot_repay_prior_work_higher_stage_only_adds_difference() {
    let mut sim = Sim::new(&setup(&[20, 5, 30], 100, 300, 1000)).unwrap();
    sim.step(&[]);
    clear(&mut sim);
    sim.step(&[]);
    sim.step(&[]);
    assert_eq!(mana(&sim), 6 << 16);
    attack(&mut sim);
    clear(&mut sim);
    sim.step(&[]);
    assert_eq!(
        mana(&sim),
        6 << 16,
        "lower-valued stage is already consumed"
    );
    attack(&mut sim);
    clear(&mut sim);
    finish(&mut sim);
    assert_eq!(mana(&sim), 30 << 16, "only higher-value difference remains");
}

#[test]
fn overflow_consumes_lineage_value_but_new_corpses_start_fresh() {
    let mut sim = Sim::new(&setup(&[20], 100, 300, 0)).unwrap();
    sim.step(&[]);
    clear(&mut sim);
    sim.step(&[]);
    sim.step(&[]);
    assert_eq!(mana(&sim), 0);
    assert_eq!(
        sim.world()
            .features
            .get(target(&sim))
            .unwrap()
            .reclaim_claimed,
        6 << 16
    );
    attack(&mut sim);
    sim.world_mut()
        .spawn(
            UnitDefId(1),
            PlayerId(0),
            Vec2::from_ints(450, 450),
            Angle::ZERO,
        )
        .unwrap();
    clear(&mut sim);
    finish(&mut sim);
    assert_eq!(
        mana(&sim),
        14 << 16,
        "overflow cannot be recovered via replacement"
    );
    let corpse = features::spawn_corpse(
        sim.world_mut(),
        UnitDefId(0),
        Vec2::from_ints(328, 264),
        Angle::ZERO,
        1,
    )
    .unwrap();
    assert_eq!(sim.world().features.get(corpse).unwrap().reclaim_claimed, 0);
    clear(&mut sim);
    finish(&mut sim);
    assert_eq!(
        mana(&sim),
        34 << 16,
        "a genuinely new corpse has its own budget"
    );
}
