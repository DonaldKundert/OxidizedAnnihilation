//! Construction is deterministic: a busy build log (walls, a keep, a
//! factory queue, an interruption, a cancel) gives identical per-tick
//! hashes in-process, after a JSON round trip, and in two separate
//! processes (this test binary re-runs itself).

mod construction_support;

use construction_support::*;
use tak_commands::{CommandLog, QueueMode, SimCommand, UnitId, UnitSet};
use tak_sim::{Scenario, Sim};

const TICKS: u32 = 2400;
const CHILD_ENV: &str = "TAK_CONSTRUCTION_DETERMINISM_CHILD";

fn king(p: u32) -> UnitId {
    UnitId::new(p, 0)
}

fn log() -> CommandLog {
    let mut log = CommandLog::new();
    let mut add = |c: tak_commands::LoggedCommand| log.record(c.tick, c.player, c.command);
    add(build(1, 0, &[king(0)], KEEP, Some(at(336, 208))));
    add(build(2, 1, &[king(1)], WALL, Some(at(600, 820))));
    add(cmd(
        40,
        1,
        SimCommand::Move {
            units: UnitSet::single(king(1)),
            to: at(700, 700),
            queue: QueueMode::Replace,
        },
    ));
    add(build(90, 1, &[king(1)], WALL, Some(at(600, 820))));
    // Player 0's keep is slot 2 once placed on tick 1.
    let keep = UnitId::new(2, 0);
    for (t, q) in [
        (400, QueueMode::Replace),
        (401, QueueMode::Append),
        (402, QueueMode::Append),
    ] {
        add(cmd(
            t,
            0,
            SimCommand::Build {
                units: UnitSet::single(keep),
                def: SWORD,
                at: None,
                queue: q,
            },
        ));
    }
    add(build(403, 0, &[keep], MASON, None));
    add(build(500, 0, &[king(0)], WALL, Some(at(150, 400))));
    add(cmd(
        800,
        0,
        SimCommand::Stop {
            units: UnitSet::single(keep),
        },
    ));
    add(build(900, 0, &[keep], SWORD, None));
    log
}

fn sim_after(sc: &Scenario, log: &CommandLog) -> (Sim, Vec<u64>) {
    let rec = Recorder::default();
    let clock = rec.log.clone();
    let mut sim = Sim::with_scripts(sc, Box::new(rec)).unwrap();
    let hashes = (0..TICKS)
        .map(|_| {
            let t = sim.tick();
            clock.borrow_mut().tick = t.0;
            sim.step(log.for_tick(t))
        })
        .collect();
    (sim, hashes)
}

fn run(sc: &Scenario, log: &CommandLog) -> Vec<u64> {
    sim_after(sc, log).1
}

fn digest(h: &[u64]) -> String {
    format!(
        "{:016x} {:016x} {}",
        h[h.len() / 2],
        h[h.len() - 1],
        h.len()
    )
}

#[test]
fn construction_is_deterministic_across_runs_and_processes() {
    let sc = scenario();
    let log = log();
    let a = run(&sc, &log);
    if std::env::var_os(CHILD_ENV).is_some() {
        println!("DIGEST {}", digest(&a));
        return;
    }
    let b = run(&sc, &log);
    assert_eq!(a, b);
    let sc2 = Scenario::from_json(&sc.to_json_pretty()).unwrap();
    assert_eq!(sc, sc2, "build data survives JSON");
    let log2 = CommandLog::from_json(&log.to_json()).unwrap();
    assert_eq!(a, run(&sc2, &log2));

    // Construction actually happened (and changed the hash).
    let (sim, _) = sim_after(&sc, &log);
    let w = sim.world();
    let count = |d| {
        w.units()
            .filter(|(_, u)| u.def == d && u.build.is_complete())
            .count()
    };
    assert_eq!(count(KEEP), 1);
    assert_eq!(count(WALL), 2);
    assert!(count(SWORD) >= 2, "{}", count(SWORD));
    let idle = run(&sc, &CommandLog::new());
    assert_ne!(a[10], idle[10]);

    let exe = std::env::current_exe().unwrap();
    let mut digests = Vec::new();
    for _ in 0..2 {
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "construction_is_deterministic_across_runs_and_processes",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .unwrap();
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let line = text
            .split("DIGEST ")
            .nth(1)
            .and_then(|rest| rest.lines().next())
            .unwrap_or_else(|| panic!("child printed no digest: {text}"))
            .to_string();
        digests.push(line);
    }
    assert_eq!(digests[0], digests[1]);
    assert_eq!(digests[0], digest(&a));
}
