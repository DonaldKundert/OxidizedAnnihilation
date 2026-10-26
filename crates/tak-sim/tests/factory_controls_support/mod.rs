//! Shared synthetic production controls fixture. No game data.
#![allow(dead_code)]
#[path = "../construction_support/mod.rs"]
mod construction;

pub use construction::{KEEP, MASON, SWORD, at};
#[allow(unused_imports)]
pub use construction::{Recorder, step};
pub const KING: tak_commands::UnitDefId = construction::KING;
use tak_commands::{
    CommandLog, FactoryRallyOrder, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId,
    UnitSet,
};
use tak_sim::{Scenario, Sim};

pub const FACTORY: UnitId = UnitId::new(0, 0);
pub const ENEMY_FACTORY: UnitId = UnitId::new(1, 0);

pub fn scenario() -> Scenario {
    let mut sc = construction::scenario();
    for p in &mut sc.players {
        p.monarch = KEEP;
    }
    sc.unit_defs[KEEP.0 as usize].economy.mogriumstorage = 5000;
    sc.economy.rules.start_mana = 1000;
    sc
}

pub fn sim() -> Sim {
    let mut sim = Sim::new(&scenario()).unwrap();
    sim.step(&[]);
    sim
}

pub fn build(def: UnitDefId) -> SimCommand {
    SimCommand::Build {
        units: UnitSet::single(FACTORY),
        def,
        at: None,
        queue: QueueMode::Append,
    }
}

pub fn rally(order: Option<FactoryRallyOrder>, queue: QueueMode) -> SimCommand {
    SimCommand::SetFactoryRally {
        units: UnitSet::single(FACTORY),
        order,
        queue,
    }
}

pub fn cancel(def: UnitDefId, count: u32) -> SimCommand {
    SimCommand::CancelProduction {
        units: UnitSet::single(FACTORY),
        def,
        count,
    }
}

pub fn issue(sim: &mut Sim, commands: impl IntoIterator<Item = SimCommand>) {
    let mut log = CommandLog::new();
    for command in commands {
        log.record(sim.tick(), PlayerId(0), command);
    }
    sim.step(log.for_tick(sim.tick()));
}

pub fn log() -> CommandLog {
    use FactoryRallyOrder::{Move, Patrol};
    let mut log = CommandLog::new();
    let mut add = |tick, command| log.record(Tick(tick), PlayerId(0), command);
    for _ in 0..4 {
        add(1, build(SWORD));
    }
    for _ in 0..2 {
        add(1, build(MASON));
    }
    add(
        2,
        rally(Some(Move { to: at(350, 350) }), QueueMode::Replace),
    );
    add(
        5,
        rally(Some(Move { to: at(450, 350) }), QueueMode::Replace),
    );
    add(6, rally(Some(Move { to: at(600, 350) }), QueueMode::Append));
    add(
        7,
        rally(Some(Patrol { to: at(600, 650) }), QueueMode::Append),
    );
    add(10, cancel(SWORD, 1));
    add(11, cancel(MASON, 1));
    add(12, cancel(SWORD, 3));
    add(12, build(SWORD));
    add(
        40,
        rally(Some(Move { to: at(500, 450) }), QueueMode::Replace),
    );
    add(
        41,
        rally(Some(Patrol { to: at(700, 450) }), QueueMode::Append),
    );
    add(80, rally(None, QueueMode::Replace));
    add(81, build(SWORD));
    log
}
