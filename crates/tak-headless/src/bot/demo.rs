//! Hand-authored synthetic training/demo data, never retail assets.
use tak_commands::{PlayerId, UnitDefId};
use tak_math::Fx;
use tak_sim::combat::{CombatDef, DamageStats, WeaponKind, WeaponStats};
use tak_sim::construction::BuildDef;
use tak_sim::testing::{example_scenario, monarch_def};
use tak_sim::{MapInfo, Scenario, StartPos, World};

use super::{
    BotError, ControllerConfig, Recipe, SIMPLE_SKIRMISH_V1, SkirmishController, Targeting,
};

/// Small income -> factory/direct builder -> armed troop -> defeat loop.
/// All production is ordinary construction paid from normal mana.
pub fn scenario(direct_builder: bool) -> Scenario {
    let mut scenario = example_scenario(0x5c1, 2);
    scenario.map = MapInfo::flat(
        640,
        320,
        vec![StartPos { x: 112, y: 160 }, StartPos { x: 528, y: 160 }],
    );
    let mut root = monarch_def("training_builder");
    root.build = BuildDef {
        builder: true,
        mobile: true,
        workertime: 30,
        builddistance: 160,
        canbuild: vec![UnitDefId(1), UnitDefId(if direct_builder { 3 } else { 2 })],
        ..Default::default()
    };
    root.combat.max_damage = 1000;
    root.economy.mogriumstorage = 1000;
    let mut income = monarch_def("training_income");
    income.maxvelocity = Fx::ZERO;
    income.build = BuildDef {
        buildcost: 100,
        buildtime: 10 << 16,
        ..Default::default()
    };
    income.economy.mogriumincome = 30;
    income.combat.max_damage = 200;
    let mut factory = monarch_def("training_factory");
    factory.maxvelocity = Fx::ZERO;
    factory.build = BuildDef {
        builder: true,
        workertime: 30,
        canbuild: vec![UnitDefId(3)],
        buildcost: 60,
        buildtime: 15 << 16,
        ..Default::default()
    };
    factory.combat.max_damage = 200;
    let mut troop = monarch_def("training_troop");
    troop.footprintx = 1;
    troop.footprintz = 1;
    troop.turnrate = 4096;
    troop.build = BuildDef {
        mobile: true,
        buildcost: 30,
        buildtime: 10 << 16,
        ..Default::default()
    };
    troop.combat = CombatDef {
        max_damage: 100,
        sight: 48,
        leash: 48,
        radius: 8,
        weapons: vec![WeaponStats {
            kind: WeaponKind::LineOfSight,
            range: 80,
            reload_ticks: 8,
            damage: DamageStats {
                default: 20,
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut enemy = monarch_def("training_target");
    enemy.maxvelocity = Fx::ZERO;
    enemy.combat.max_damage = 240;
    enemy.economy.mogriumstorage = 100;
    scenario.unit_defs = vec![root, income, factory, troop, enemy];
    scenario.players[1].monarch = UnitDefId(4);
    scenario.economy.rules.start_mana = 100;
    scenario
}

/// Explicitly enable player zero only; player one is protected human input.
pub fn config(world: &World, direct_builder: bool) -> Result<ControllerConfig, BotError> {
    let config = ControllerConfig {
        human_players: vec![PlayerId(1)],
        controllers: vec![SkirmishController {
            policy: SIMPLE_SKIRMISH_V1.into(),
            player: PlayerId(0),
            recipe: Recipe::resolve(
                world,
                "training_builder",
                Some("training_income"),
                (!direct_builder).then_some("training_factory"),
                "training_troop",
            )?,
            cadence_ticks: 30,
            command_budget: 8,
            site_budget: 64,
            army_cap: 3,
            mana_reserve: 0,
            targeting: Targeting::OmniscientTrainingV1,
        }],
    };
    config.validate(world)?;
    Ok(config)
}
