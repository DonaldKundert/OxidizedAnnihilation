//! Shared helpers for the `construction_*` tests: a synthetic tech tree and
//! a script host that records what construction asks of scripts.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use tak_commands::{
    LoggedCommand, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Fx, Rng, Vec2};
use tak_sim::construction::BuildDef;
use tak_sim::map::{MapInfo, StartPos};
use tak_sim::testing::monarch_def;
use tak_sim::{
    PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario, ScriptCtx, ScriptHost, ScriptReturn,
    StateHasher, UnitDef, WorldPos,
};

pub const KING: UnitDefId = UnitDefId(0);
pub const WALL: UnitDefId = UnitDefId(1);
pub const KEEP: UnitDefId = UnitDefId(2);
pub const SWORD: UnitDefId = UnitDefId(3);
pub const MASON: UnitDefId = UnitDefId(4);

fn secs(s: i64) -> i64 {
    s << 16
}

pub fn structure(name: &str, fx: u8, fz: u8, build: BuildDef) -> UnitDef {
    // Start from the shared helper so fields other lanes add keep their
    // defaults.
    let mut d = monarch_def(name);
    d.maxvelocity = Fx::ZERO;
    d.acceleration = Fx::from_raw(1);
    d.brakerate = Fx::from_raw(1);
    d.turnrate = 0;
    d.footprintx = fx;
    d.footprintz = fz;
    d.build = build;
    d
}

/// King (mobile builder, workertime 10, range 100) builds walls (2x2,
/// buildtime 65 = 195 ticks), keeps (4x4 factory, buildtime 10) and masons;
/// keeps build swords (buildtime 9 = 27 ticks) and masons.
pub fn scenario() -> Scenario {
    let mut king = monarch_def("KING");
    // Real monarch economy (WP-N, docs/economy.md): 10 mana/s, 5000 storage.
    king.economy.mogriumincome = 10;
    king.economy.mogriumstorage = 5000;
    king.build = BuildDef {
        buildcost: 100_000,
        buildtime: secs(10),
        workertime: 10,
        builddistance: 100,
        builder: true,
        mobile: true,
        canbuild: vec![WALL, KEEP, MASON],
        yardmap: String::new(),
    };
    let wall = structure(
        "WALL",
        2,
        2,
        BuildDef {
            buildcost: 75,
            buildtime: secs(65),
            yardmap: "oooo".into(),
            ..BuildDef::default()
        },
    );
    let keep = structure(
        "KEEP",
        4,
        4,
        BuildDef {
            buildcost: 500,
            buildtime: secs(10),
            workertime: 10,
            builddistance: 100,
            builder: true,
            mobile: false,
            canbuild: vec![SWORD, MASON],
            yardmap: "oooooooooooocccc".into(),
        },
    );
    let mut sword = monarch_def("SWORD");
    sword.footprintx = 1;
    sword.footprintz = 1;
    sword.build = BuildDef {
        buildcost: 30,
        buildtime: secs(9),
        mobile: true,
        ..BuildDef::default()
    };
    let mut mason = monarch_def("MASON");
    mason.footprintx = 1;
    mason.footprintz = 1;
    mason.build = BuildDef {
        buildcost: 40,
        buildtime: secs(6),
        workertime: 10,
        builddistance: 60,
        builder: true,
        mobile: true,
        canbuild: vec![WALL],
        yardmap: String::new(),
    };
    Scenario {
        version: SCENARIO_FORMAT_VERSION,
        initial_units: None,
        seed: 3,
        economy: Default::default(),
        features: Default::default(),
        victory: Default::default(),
        map: MapInfo::flat(
            1024,
            1024,
            vec![StartPos { x: 200, y: 200 }, StartPos { x: 800, y: 800 }],
        ),
        unit_defs: vec![king, wall, keep, sword, mason],
        players: vec![
            PlayerSpec {
                start: 0,
                monarch: KING,
            },
            PlayerSpec {
                start: 1,
                monarch: KING,
            },
        ],
    }
}

pub fn build(
    tick: u32,
    player: u8,
    units: &[UnitId],
    def: UnitDefId,
    at: Option<WorldPos>,
) -> LoggedCommand {
    let mut set = UnitSet::new();
    for &u in units {
        set.insert(u);
    }
    LoggedCommand {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: 0,
        command: SimCommand::Build {
            units: set,
            def,
            at,
            queue: QueueMode::Replace,
        },
    }
}

pub fn cmd(tick: u32, player: u8, command: SimCommand) -> LoggedCommand {
    LoggedCommand {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: 0,
        command,
    }
}

pub fn at(x: i32, y: i32) -> WorldPos {
    Vec2::from_ints(x, y)
}

/// What the recorder saw.
#[derive(Default, Debug)]
pub struct Log {
    /// Set by the test before each step.
    pub tick: u32,
    /// `(tick, unit, function)` for every `start_script` and `call_script`.
    pub calls: Vec<(u32, UnitId, String)>,
    /// Port 17 as each unit's script saw it, once per tick.
    pub port17: BTreeMap<UnitId, Vec<(u32, i32)>>,
    /// Units created, with the tick.
    pub created: Vec<(u32, UnitId)>,
}

impl Log {
    pub fn calls_of(&self, unit: UnitId, name: &str) -> Vec<u32> {
        self.calls
            .iter()
            .filter(|(_, u, n)| *u == unit && n == name)
            .map(|(t, _, _)| *t)
            .collect()
    }
}

/// A script host with no VM: records calls and the port 17 values, and
/// plays a factory script's door handshake (`INBUILDSTANCE` reads 1 from
/// the tick after `Activate` until `Deactivate`).
#[derive(Clone, Default)]
pub struct Recorder {
    pub log: Rc<RefCell<Log>>,
    stance: BTreeMap<UnitId, (bool, u32)>,
}

impl ScriptHost for Recorder {
    fn create_unit(&mut self, ctx: &ScriptCtx, _rng: &mut Rng) {
        let mut l = self.log.borrow_mut();
        let t = l.tick;
        l.created.push((t, ctx.unit));
    }

    fn tick_unit(&mut self, ctx: &ScriptCtx, _rng: &mut Rng) {
        let mut l = self.log.borrow_mut();
        let t = l.tick;
        l.port17
            .entry(ctx.unit)
            .or_default()
            .push((t, ctx.build_percent_left));
    }

    fn call_script(
        &mut self,
        ctx: &ScriptCtx,
        _rng: &mut Rng,
        name: &str,
        _args: &[i32],
    ) -> Option<ScriptReturn> {
        let mut l = self.log.borrow_mut();
        let t = l.tick;
        l.calls.push((t, ctx.unit, name.to_string()));
        None
    }

    fn start_script(&mut self, unit: UnitId, name: &str, _args: &[i32]) -> bool {
        let t = {
            let mut l = self.log.borrow_mut();
            let t = l.tick;
            l.calls.push((t, unit, name.to_string()));
            t
        };
        match name {
            "Activate" => {
                self.stance.insert(unit, (true, t));
            }
            "Deactivate" => {
                self.stance.insert(unit, (false, t));
            }
            _ => {}
        }
        true
    }

    fn port_value(&self, unit: UnitId, port: i32) -> Option<i32> {
        if port != 5 {
            return Some(0);
        }
        let now = self.log.borrow().tick;
        Some(match self.stance.get(&unit) {
            Some(&(true, t)) if t < now => 1,
            _ => 0,
        })
    }

    fn hash_state(&self, h: &mut StateHasher) {
        h.len(self.stance.len());
    }
}

/// Run one tick with the recorder's clock set.
pub fn step(sim: &mut tak_sim::Sim, log: &Rc<RefCell<Log>>, cmds: &[LoggedCommand]) -> u64 {
    log.borrow_mut().tick = sim.tick().0;
    let now = sim.tick();
    let these: Vec<LoggedCommand> = cmds.iter().filter(|c| c.tick == now).cloned().collect();
    sim.step(&these)
}
