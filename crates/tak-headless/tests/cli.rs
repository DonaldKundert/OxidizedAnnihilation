//! End-to-end tests of the `tak-headless` binary, including determinism
//! across two separate processes.

use std::path::{Path, PathBuf};
use std::process::Command;

use tak_commands::{CommandLog, SimCommand, Tick};
use tak_headless::{format_hashes, parse_hashes};
use tak_sim::run_scenario;
use tak_sim::testing::{example_scenario, random_log};

const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");
const TICKS: u32 = 10_000;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("tak-headless-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run_bin(args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("spawn tak-headless")
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn two_processes_agree_over_ten_thousand_ticks() {
    let dir = scratch("two-proc");
    let scenario = example_scenario(0xBADC0DE, 4);
    let log = random_log(&scenario, TICKS, 2024);
    let sc_path = dir.join("scenario.json");
    let log_path = dir.join("commands.json");
    std::fs::write(&sc_path, scenario.to_json_pretty()).unwrap();
    std::fs::write(&log_path, log.to_json()).unwrap();

    let ticks = TICKS.to_string();
    let mut outputs = Vec::new();
    for name in ["a.txt", "b.txt"] {
        let out = dir.join(name);
        let r = run_bin(&[
            "run",
            "--scenario",
            s(&sc_path),
            "--log",
            s(&log_path),
            "--ticks",
            &ticks,
            "--hashes",
            s(&out),
        ]);
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        outputs.push(std::fs::read_to_string(&out).unwrap());
    }
    assert_eq!(outputs[0], outputs[1], "two processes disagree");
    let hashes = parse_hashes(&outputs[0]).unwrap();
    assert_eq!(hashes.len(), TICKS as usize);
    // And they agree with an in-process run.
    assert_eq!(hashes, run_scenario(&scenario, &log, TICKS).unwrap());
    assert_eq!(outputs[0], format_hashes(&hashes));

    // `verify` accepts the recording ...
    let hashes_path = dir.join("a.txt");
    let r = run_bin(&[
        "verify",
        "--scenario",
        s(&sc_path),
        "--log",
        s(&log_path),
        "--hashes",
        s(&hashes_path),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
    assert!(String::from_utf8_lossy(&r.stdout).starts_with("ok: 10000 ticks match"));

    // ... and pinpoints a tampered command.
    let victim = log
        .iter()
        .find(|e| {
            // A move by a player that includes their own monarch, so turning
            // it into a Stop certainly changes the state.
            matches!(e.command, SimCommand::Move { .. })
                && e.tick > Tick(100)
                && e.command
                    .units()
                    .contains(tak_commands::UnitId::new(u32::from(e.player.0), 0))
                && (e.player.0 as usize) < scenario.players.len()
        })
        .map(|e| e.key());
    let (vt, vp, vs) = victim.expect("log has a move");
    let mut tampered = CommandLog::new();
    for e in log.iter() {
        let mut e = e.clone();
        if e.key() == (vt, vp, vs) {
            e.command = SimCommand::Stop {
                units: e.command.units().clone(),
            };
        }
        tampered.insert(e).unwrap();
    }
    let tampered_hashes = run_scenario(&scenario, &tampered, TICKS).unwrap();
    let first_diff = hashes
        .iter()
        .zip(&tampered_hashes)
        .position(|(a, b)| a != b);
    let tampered_path = dir.join("tampered.json");
    std::fs::write(&tampered_path, tampered.to_json()).unwrap();
    let r = run_bin(&[
        "verify",
        "--scenario",
        s(&sc_path),
        "--log",
        s(&tampered_path),
        "--hashes",
        s(&hashes_path),
    ]);
    let stdout = String::from_utf8_lossy(&r.stdout);
    let t = first_diff.expect("tampering changes the hashes");
    assert_eq!(t, vt.0 as usize, "divergence starts on the tampered tick");
    assert_eq!(r.status.code(), Some(1));
    assert!(
        stdout.starts_with(&format!("mismatch at tick {t}:")),
        "{stdout}"
    );
}

#[test]
fn fixture_scenario_runs_and_verifies() {
    let dir = scratch("fixture");
    let sc = fixture("two_players.json");
    let log = fixture("two_players.commands.json");
    let out = dir.join("hashes.txt");
    let r = run_bin(&[
        "run",
        "--scenario",
        s(&sc),
        "--log",
        s(&log),
        "--ticks",
        "600",
        "--hashes",
        s(&out),
    ]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let r = run_bin(&[
        "verify",
        "--scenario",
        s(&sc),
        "--log",
        s(&log),
        "--hashes",
        s(&out),
    ]);
    assert!(r.status.success());

    // Without the log the run diverges from tick 1 on (tick 0 has no
    // commands).
    let r = run_bin(&["verify", "--scenario", s(&sc), "--hashes", s(&out)]);
    assert_eq!(r.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&r.stdout).starts_with("mismatch at tick 1:"));

    // Printing to stdout gives the same text as the file.
    let r = run_bin(&[
        "run",
        "--scenario",
        s(&sc),
        "--log",
        s(&log),
        "--ticks",
        "600",
    ]);
    assert_eq!(
        String::from_utf8_lossy(&r.stdout),
        std::fs::read_to_string(&out).unwrap()
    );
}

#[test]
fn bad_inputs_fail_cleanly() {
    let dir = scratch("bad");
    let bad = dir.join("bad.json");
    std::fs::write(&bad, "{\"version\": 9}").unwrap();
    let r = run_bin(&["run", "--scenario", s(&bad), "--ticks", "5"]);
    assert_eq!(r.status.code(), Some(2));
    assert!(!r.stderr.is_empty());
    let r = run_bin(&[
        "run",
        "--scenario",
        s(&dir.join("missing.json")),
        "--ticks",
        "5",
    ]);
    assert_eq!(r.status.code(), Some(2));
}

#[test]
fn example_scenario_and_gen_log_round_trip() {
    let dir = scratch("example");
    let r = run_bin(&["example-scenario", "--players", "3", "--seed", "9"]);
    assert!(r.status.success());
    let sc_path = dir.join("example.json");
    std::fs::write(&sc_path, &r.stdout).unwrap();
    let log_path = dir.join("log.json");
    let r = run_bin(&[
        "gen-log",
        "--scenario",
        s(&sc_path),
        "--ticks",
        "300",
        "--out",
        s(&log_path),
    ]);
    assert!(r.status.success());
    let log = CommandLog::from_json(&std::fs::read_to_string(&log_path).unwrap()).unwrap();
    assert!(!log.is_empty());
}
