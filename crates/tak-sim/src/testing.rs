//! Synthetic scenarios and command logs for tests, soak runs and the
//! headless runner. Everything here is deterministic in its seed.

use tak_commands::{
    CommandLog, FireState, PlayerId, QueueMode, SimCommand, Target, Tick, UnitDefId, UnitId,
    UnitSet,
};
use tak_math::{Fx, Rng, Vec2};

use crate::defs::UnitDef;
use crate::map::{MapInfo, StartPos};
use crate::scenario::{PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario};

/// A monarch-like walker: 1.5 units/tick top speed, about 4 degrees/tick
/// turning.
pub fn monarch_def(name: &str) -> UnitDef {
    UnitDef {
        name: name.into(),
        maxvelocity: Fx::from_raw(98_304),
        acceleration: Fx::from_raw(8_192),
        brakerate: Fx::from_raw(16_384),
        turnrate: 728,
        footprintx: 2,
        footprintz: 2,
    }
}

/// A flat 2048 x 2048 map with `players` (1..=4) players in the corners,
/// alternating between two monarch definitions.
pub fn example_scenario(seed: u64, players: usize) -> Scenario {
    let starts = vec![
        StartPos { x: 256, y: 256 },
        StartPos { x: 1792, y: 1792 },
        StartPos { x: 1792, y: 256 },
        StartPos { x: 256, y: 1792 },
    ];
    let mut slow = monarch_def("slow_monarch");
    slow.maxvelocity = Fx::ONE;
    slow.turnrate = 400;
    Scenario {
        version: SCENARIO_FORMAT_VERSION,
        seed,
        map: MapInfo::flat(2048, 2048, starts),
        unit_defs: vec![monarch_def("monarch"), slow],
        players: (0..players.clamp(1, 4))
            .map(|i| PlayerSpec {
                start: i as u32,
                monarch: UnitDefId((i % 2) as u32),
            })
            .collect(),
    }
}

/// A random but reproducible command log over `ticks` ticks for a scenario
/// whose units are the tick-0 monarchs (slot `i` belongs to player `i`).
/// Mixes valid orders with stale ids, other players' units, unknown players,
/// off-map targets and orders the skeleton ignores.
pub fn random_log(scenario: &Scenario, ticks: u32, seed: u64) -> CommandLog {
    let mut rng = Rng::with_stream(seed, 0x5eed);
    let mut log = CommandLog::new();
    let n = scenario.players.len() as u32;
    let w = scenario.map.width;
    let h = scenario.map.height;
    for t in 0..ticks {
        if !rng.chance(1, 6) {
            continue;
        }
        let player = PlayerId(rng.below(n + 1) as u8);
        let mut units = UnitSet::new();
        for _ in 0..=rng.below(2) {
            let index = rng.below(n + 2);
            let generation = u32::from(rng.chance(1, 10));
            units.insert(UnitId::new(index, generation));
        }
        let queue = if rng.chance(1, 3) {
            QueueMode::Append
        } else {
            QueueMode::Replace
        };
        let pos = Vec2::from_ints(rng.range_i32(-64, w + 64), rng.range_i32(-64, h + 64));
        let command = match rng.below(10) {
            0..=5 => SimCommand::Move {
                units,
                to: pos,
                queue,
            },
            6 => SimCommand::Stop { units },
            7 => SimCommand::SetFireState {
                units,
                state: match rng.below(3) {
                    0 => FireState::HoldFire,
                    1 => FireState::ReturnFire,
                    _ => FireState::FireAtWill,
                },
            },
            8 => SimCommand::Attack {
                units,
                target: Target::Ground(pos),
                queue,
            },
            _ => SimCommand::Patrol {
                units,
                to: pos,
                queue,
            },
        };
        log.record(Tick(t), player, command);
    }
    log
}
