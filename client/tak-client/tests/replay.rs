//! Replay equivalence (gated on `TAK_DATA_DIR`): the client's sim driver
//! and `tak-headless`'s runner, given the same match and command log,
//! produce identical per-tick hashes. The log is the scripted demo session
//! (a monarch builds a wall, then both monarchs fight), round-tripped
//! through JSON as a recorded session would be.

use tak_client::game::{Session, demo_log, open_vfs};
use tak_commands::CommandLog;
use tak_sim::PlayerSetup;
use tak_sim::load_match;

const MAP: &str = "kmap/Adamantine Gate";
const TICKS: u32 = 2400;

#[test]
fn client_session_replays_identically_in_headless() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("client replay") else {
        return;
    };
    let vfs = open_vfs(&dir).expect("open install");
    let mut s = Session::load(&vfs, MAP, "ARA,ZON", 1, CommandLog::new()).expect("load");
    s.log = demo_log(&s);
    assert!(s.log.len() >= 3, "demo log: {}", s.log.to_json());
    let json = s.log.to_json();
    // The committed fixture is this log (regenerate with
    // `tak-client --demo --screenshot x.png --at-tick 1 --record ...`).
    let fixture = CommandLog::from_json(include_str!("../../fixtures/demo_session.json"))
        .expect("fixture parses");
    assert_eq!(fixture, s.log, "client/fixtures/demo_session.json is stale");
    for _ in 0..TICKS {
        s.step();
    }
    // The monarch actually built something and the two fought.
    let w = s.sim.world();
    assert!(w.unit_count() >= 2);

    let log = CommandLog::from_json(&json).expect("log round trip");
    let m = load_match(&vfs, MAP, &PlayerSetup::parse_list("ARA,ZON"), 1).expect("load match");
    let run = tak_headless::run_match(m, &log, TICKS, true).expect("headless run");
    assert_eq!(run.hashes.len(), s.hashes.len());
    if let Some((tick, a, b)) = tak_headless::first_mismatch(&run.hashes, &s.hashes) {
        panic!("hash mismatch at tick {tick}: headless {a:016x}, client {b:016x}");
    }
    eprintln!(
        "{} ticks, final hash {:016x}",
        TICKS,
        s.last_hash().unwrap_or(0)
    );
}
