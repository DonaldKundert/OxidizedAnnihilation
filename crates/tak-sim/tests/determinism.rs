//! Same scenario and command log give the same per-tick hashes; changing one
//! command changes the hash from its tick on. (The two-process variant lives
//! in `crates/tak-headless/tests/cli.rs`.)

use tak_commands::{CommandLog, QueueMode, SimCommand, Tick};
use tak_math::Vec2;
use tak_sim::testing::{example_scenario, random_log};
use tak_sim::{Scenario, ScriptCtx, ScriptHost, Sim, StateHasher, run_scenario};

const TICKS: u32 = 10_000;

#[test]
fn ten_thousand_ticks_are_reproducible_in_process() {
    let scenario = example_scenario(0xC0FFEE, 4);
    let log = random_log(&scenario, TICKS, 99);
    assert!(log.len() > 1000, "log has {} commands", log.len());
    let a = run_scenario(&scenario, &log, TICKS).unwrap();
    let b = run_scenario(&scenario, &log, TICKS).unwrap();
    assert_eq!(a.len(), TICKS as usize);
    assert_eq!(a, b);
    // Hashes actually change over time (state is being hashed).
    let mut distinct = a.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(distinct.len() > (TICKS as usize) / 2);
}

#[test]
fn log_survives_json_round_trip_and_replays_identically() {
    let scenario = example_scenario(5, 3);
    let log = random_log(&scenario, 3000, 7);
    let replayed = CommandLog::from_json(&log.to_json()).unwrap();
    let scenario2 = Scenario::from_json(&scenario.to_json_pretty()).unwrap();
    assert_eq!(scenario, scenario2);
    let a = run_scenario(&scenario, &log, 3000).unwrap();
    let b = run_scenario(&scenario2, &replayed, 3000).unwrap();
    assert_eq!(a.last(), b.last());
    assert_eq!(a, b);
}

#[test]
fn changing_one_command_changes_hashes_from_that_tick_on() {
    let scenario = example_scenario(5, 2);
    let mut log = CommandLog::new();
    let u0 = tak_commands::UnitId::new(0, 0);
    let mv = |x, y| SimCommand::Move {
        units: tak_commands::UnitSet::single(u0),
        to: Vec2::from_ints(x, y),
        queue: QueueMode::Replace,
    };
    log.record(Tick(10), tak_commands::PlayerId(0), mv(600, 600));
    log.record(Tick(500), tak_commands::PlayerId(0), mv(1200, 300));
    // An order to a different unit later on must not mask the divergence.
    log.record(
        Tick(900),
        tak_commands::PlayerId(1),
        SimCommand::Move {
            units: tak_commands::UnitSet::single(tak_commands::UnitId::new(1, 0)),
            to: Vec2::from_ints(1000, 1000),
            queue: QueueMode::Append,
        },
    );

    let mut edited = CommandLog::new();
    for e in log.iter() {
        let mut e = e.clone();
        if e.tick == Tick(500) {
            e.command = mv(1201, 300);
        }
        edited.insert(e).unwrap();
    }

    let a = run_scenario(&scenario, &log, 1500).unwrap();
    let b = run_scenario(&scenario, &edited, 1500).unwrap();
    assert_eq!(a[..500], b[..500], "identical before the edited tick");
    for t in 500..1500 {
        assert_ne!(a[t], b[t], "tick {t} should differ");
    }
}

#[test]
fn seed_changes_the_hash() {
    let log = CommandLog::new();
    let a = run_scenario(&example_scenario(1, 2), &log, 5).unwrap();
    let b = run_scenario(&example_scenario(2, 2), &log, 5).unwrap();
    assert!(a.iter().zip(&b).all(|(x, y)| x != y));
}

/// A toy script host: counts callbacks and hashes the count, so the test can
/// check the hook is called in the documented pattern and feeds the hash.
#[derive(Default)]
struct Counter {
    created: u32,
    ticked: u32,
}

impl ScriptHost for Counter {
    fn on_create(&mut self, ctx: &ScriptCtx) {
        assert_eq!(ctx.tick, Tick(0));
        self.created += 1;
    }
    fn on_tick(&mut self, _ctx: &ScriptCtx) {
        self.ticked += 1;
    }
    fn hash_state(&self, h: &mut StateHasher) {
        h.u32(self.created);
        h.u32(self.ticked);
    }
}

#[test]
fn script_hook_is_called_and_hashed() {
    let scenario = example_scenario(3, 4);
    let mut with = Sim::with_scripts(&scenario, Box::new(Counter::default())).unwrap();
    let mut without = Sim::new(&scenario).unwrap();
    let log = CommandLog::new();
    let h1 = with.run(&log, 10);
    let h2 = without.run(&log, 10);
    assert!(h1.iter().zip(&h2).all(|(a, b)| a != b));
    // Same host twice gives the same hashes.
    let mut again = Sim::with_scripts(&scenario, Box::new(Counter::default())).unwrap();
    assert_eq!(h1, again.run(&log, 10));
}
