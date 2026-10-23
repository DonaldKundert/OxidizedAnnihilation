//! Separate-process replay-checkpoint contract tests. All written fixtures are
//! synthetic; the one installed-content test is gated on TAK_DATA_DIR.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use tak_commands::{CommandLog, Tick};
use tak_headless::checkpoint::{Checkpoint, ReplayBudget};
use tak_headless::parse_hashes;
use tak_sim::testing::{example_scenario, walk_log};

const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new(label: &str) -> Self {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "checkpoint-{label}-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("spawn tak-headless")
}
fn ok(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn synthetic_files(dir: &Scratch) -> (PathBuf, PathBuf, CommandLog) {
    let scenario = example_scenario(0x1234, 3);
    let log = walk_log(&scenario, 600, 11, 77);
    let scenario_path = dir.path("scenario.json");
    let log_path = dir.path("commands.json");
    std::fs::write(&scenario_path, scenario.to_json_pretty()).unwrap();
    std::fs::write(&log_path, log.to_json()).unwrap();
    (scenario_path, log_path, log)
}
fn continuation(path: &Path, first_tick: u32) -> Vec<u64> {
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.starts_with("# tak-headless continuation hashes v1\n"));
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .enumerate()
        .map(|(offset, line)| {
            let (tick, hash) = line.split_once(' ').unwrap();
            assert_eq!(tick.parse::<u32>().unwrap(), first_tick + offset as u32);
            u64::from_str_radix(hash, 16).unwrap()
        })
        .collect()
}

#[test]
fn separate_process_resume_matches_continuous_and_can_resume_again() {
    let dir = Scratch::new("roundtrip");
    let (scenario, log, commands) = synthetic_files(&dir);
    let whole = dir.path("whole.txt");
    ok(&run(&[
        "run",
        "--scenario",
        s(&scenario),
        "--log",
        s(&log),
        "--ticks",
        "600",
        "--hashes",
        s(&whole),
    ]));
    let expected = parse_hashes(&std::fs::read_to_string(whole).unwrap()).unwrap();
    let saved = dir.path("save.json");
    ok(&run(&[
        "checkpoint",
        "save",
        "--scenario",
        s(&scenario),
        "--log",
        s(&log),
        "--ticks",
        "301",
        "--out",
        s(&saved),
    ]));
    let checkpoint = Checkpoint::read(&saved).unwrap();
    assert_eq!(checkpoint.next_tick(), Tick(301));
    assert_eq!(
        checkpoint.command_count(),
        commands.iter().filter(|e| e.tick < Tick(301)).count()
    );
    let final_save = dir.path("final.json");
    let hashes = dir.path("continuation.txt");
    ok(&run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        s(&saved),
        "--log",
        s(&log),
        "--ticks",
        "299",
        "--out",
        s(&final_save),
        "--hashes",
        s(&hashes),
    ]));
    assert_eq!(continuation(&hashes, 301), expected[301..]);
    let final_checkpoint = Checkpoint::read(&final_save).unwrap();
    assert_eq!(
        final_checkpoint.state_hash(),
        format!("{:016x}", expected[599])
    );
    assert_eq!(final_checkpoint.command_count(), commands.len());
    let verified = run(&["checkpoint", "resume", "--checkpoint", s(&final_save)]);
    ok(&verified);
    assert!(
        String::from_utf8_lossy(&verified.stdout).contains("verified 600 ticks, next tick 600")
    );
    assert!(String::from_utf8_lossy(&verified.stderr).contains("match: running"));
    let info = run(&["checkpoint", "info", "--checkpoint", s(&final_save)]);
    ok(&info);
    let info = String::from_utf8_lossy(&info.stdout);
    assert!(info.contains("next tick: 600"));
    assert!(info.contains("replay: not checked"));
}

#[test]
fn legacy_v1_scenario_and_command_fixture_survive_checkpoint_resume() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let source = tak_headless::checkpoint::CheckpointSource::from_scenario_file(
        &fixtures.join("two_players.json"),
    )
    .unwrap();
    let log = tak_headless::checkpoint::read_command_log(Some(
        &fixtures.join("two_players.commands.json"),
    ))
    .unwrap();
    let mut full = tak_headless::checkpoint::ReplaySession::new(source.clone(), None).unwrap();
    let expected = full.advance(&log, 600).unwrap();
    let mut first = tak_headless::checkpoint::ReplaySession::new(source, None).unwrap();
    first.advance(&log, 31).unwrap();
    let json = first.checkpoint().unwrap().to_json_pretty().unwrap();
    let mut resumed = Checkpoint::from_json(&json)
        .unwrap()
        .restore(None, ReplayBudget::default())
        .unwrap();
    assert_eq!(resumed.advance(&log, 569).unwrap(), expected[31..]);
}

#[test]
fn future_only_log_new_commands_and_in_place_save_are_supported() {
    let dir = Scratch::new("new-orders");
    let (scenario, log, commands) = synthetic_files(&dir);
    let saved = dir.path("save.json");
    ok(&run(&[
        "checkpoint",
        "save",
        "--scenario",
        s(&scenario),
        "--log",
        s(&log),
        "--ticks",
        "300",
        "--out",
        s(&saved),
    ]));
    let mut future = CommandLog::new();
    for entry in commands.iter().filter(|e| e.tick >= Tick(300)) {
        future.insert(entry.clone()).unwrap();
    }
    let future_path = dir.path("future.json");
    std::fs::write(&future_path, future.to_json()).unwrap();
    ok(&run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        s(&saved),
        "--log",
        s(&future_path),
        "--ticks",
        "300",
        "--out",
        s(&saved),
    ]));
    assert_eq!(Checkpoint::read(&saved).unwrap().next_tick(), Tick(600));
    let resumed = Checkpoint::read(&saved)
        .unwrap()
        .restore(None, ReplayBudget::default())
        .unwrap();
    let expected = tak_sim::run_scenario(&example_scenario(0x1234, 3), &commands, 600).unwrap();
    assert_eq!(resumed.sim().state_hash(), expected[599]);
}

#[test]
fn zero_tick_process_roundtrip_ignores_future_commands() {
    let dir = Scratch::new("zero");
    let (scenario, log, _) = synthetic_files(&dir);
    let saved = dir.path("zero.json");
    ok(&run(&[
        "checkpoint",
        "save",
        "--scenario",
        s(&scenario),
        "--log",
        s(&log),
        "--ticks",
        "0",
        "--out",
        s(&saved),
    ]));
    let checkpoint = Checkpoint::read(&saved).unwrap();
    assert_eq!(checkpoint.next_tick(), Tick(0));
    assert_eq!(checkpoint.command_count(), 0);
    ok(&run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        s(&saved),
        "--max-replay-ticks",
        "0",
    ]));
}

#[test]
fn invalid_checkpoint_or_budget_never_touches_existing_outputs() {
    let dir = Scratch::new("reject");
    let (scenario, log, _) = synthetic_files(&dir);
    let saved = dir.path("save.json");
    ok(&run(&[
        "checkpoint",
        "save",
        "--scenario",
        s(&scenario),
        "--log",
        s(&log),
        "--ticks",
        "30",
        "--out",
        s(&saved),
    ]));
    let output = dir.path("output.json");
    let hashes = dir.path("hashes.txt");
    for (variant, budget) in [
        ("valid", "29"),
        ("bad-version", "100"),
        ("bad-checksum", "100"),
    ] {
        let input = dir.path(variant);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
        match variant {
            "bad-version" => value["version"] = 99.into(),
            "bad-checksum" => value["checksum"] = "0000000000000000".into(),
            _ => {}
        }
        std::fs::write(&input, value.to_string()).unwrap();
        std::fs::write(&output, b"keep checkpoint").unwrap();
        std::fs::write(&hashes, b"keep hashes").unwrap();
        let result = run(&[
            "checkpoint",
            "resume",
            "--checkpoint",
            s(&input),
            "--ticks",
            "1",
            "--max-replay-ticks",
            budget,
            "--out",
            s(&output),
            "--hashes",
            s(&hashes),
        ]);
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(std::fs::read(&output).unwrap(), b"keep checkpoint");
        assert_eq!(std::fs::read(&hashes).unwrap(), b"keep hashes");
    }
    let original = std::fs::read(&saved).unwrap();
    let result = run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        s(&saved),
        "--hashes",
        s(&saved),
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(std::fs::read(&saved).unwrap(), original);
}

#[test]
fn bad_arguments_and_invalid_source_leave_destination_untouched() {
    let dir = Scratch::new("arguments");
    let output = dir.path("save.json");
    std::fs::write(&output, b"preserve").unwrap();
    for args in [
        vec!["checkpoint", "save", "--ticks", "0", "--out", s(&output)],
        vec![
            "checkpoint",
            "save",
            "--scenario",
            "missing.json",
            "--ticks",
            "0",
            "--out",
            s(&output),
        ],
        vec![
            "checkpoint",
            "save",
            "--scenario",
            "a.json",
            "--map",
            "kmap/test",
            "--ticks",
            "0",
            "--out",
            s(&output),
        ],
        vec![
            "checkpoint",
            "save",
            "--scenario",
            "a.json",
            "--ticks",
            "4294967295",
            "--out",
            s(&output),
        ],
    ] {
        assert_eq!(run(&args).status.code(), Some(2));
        assert_eq!(std::fs::read(&output).unwrap(), b"preserve");
    }
}

#[test]
#[cfg(unix)]
fn hardlinked_hash_output_cannot_destroy_input_checkpoint() {
    let dir = Scratch::new("hardlink");
    let (scenario, _, _) = synthetic_files(&dir);
    let saved = dir.path("save.json");
    ok(&run(&[
        "checkpoint",
        "save",
        "--scenario",
        s(&scenario),
        "--ticks",
        "0",
        "--out",
        s(&saved),
    ]));
    let hashes = dir.path("hashes.txt");
    std::fs::hard_link(&saved, &hashes).unwrap();
    let original = std::fs::read(&saved).unwrap();
    let result = run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        s(&saved),
        "--hashes",
        s(&hashes),
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(std::fs::read(&saved).unwrap(), original);
    assert_eq!(std::fs::read(&hashes).unwrap(), original);
    assert!(Checkpoint::read(&saved).is_ok());
}

#[test]
fn new_output_aliases_are_rejected_before_creating_either_file() {
    let dir = Scratch::new("new-alias");
    let (scenario, _, _) = synthetic_files(&dir);
    let saved = dir.path("save.json");
    ok(&run(&[
        "checkpoint",
        "save",
        "--scenario",
        s(&scenario),
        "--ticks",
        "0",
        "--out",
        s(&saved),
    ]));
    let output = dir.path("new.json");
    let hashes = dir.0.join(".").join("new.json");
    let result = run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        s(&saved),
        "--out",
        s(&output),
        "--hashes",
        s(&hashes),
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert!(!output.exists());
    assert!(Checkpoint::read(&saved).is_ok());
    #[cfg(unix)]
    {
        let link = dir.path("parent-alias");
        std::os::unix::fs::symlink(&dir.0, &link).unwrap();
        let hashes = link.join("new.json");
        let result = run(&[
            "checkpoint",
            "resume",
            "--checkpoint",
            s(&saved),
            "--out",
            s(&output),
            "--hashes",
            s(&hashes),
        ]);
        assert_eq!(result.status.code(), Some(2));
        assert!(!output.exists());
    }
}

#[test]
fn installed_scripted_map_resumes_across_processes_without_embedded_game_data() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("checkpoint_scripted_map") else {
        return;
    };
    let dir = Scratch::new("real-map");
    let map = "kmap/Adamantine Gate";
    let log = dir.path("walk.json");
    ok(&run(&[
        "gen-log",
        "--data",
        s(&data),
        "--map",
        map,
        "--players",
        "ARA,ZON",
        "--ticks",
        "600",
        "--kind",
        "walk",
        "--every",
        "51",
        "--out",
        s(&log),
    ]));
    let whole = dir.path("whole.txt");
    ok(&run(&[
        "run",
        "--data",
        s(&data),
        "--map",
        map,
        "--players",
        "ARA,ZON",
        "--log",
        s(&log),
        "--ticks",
        "600",
        "--hashes",
        s(&whole),
    ]));
    let expected = parse_hashes(&std::fs::read_to_string(&whole).unwrap()).unwrap();
    let saved = dir.path("map.json");
    ok(&run(&[
        "checkpoint",
        "save",
        "--data",
        s(&data),
        "--map",
        map,
        "--players",
        "ARA,ZON",
        "--log",
        s(&log),
        "--ticks",
        "301",
        "--out",
        s(&saved),
    ]));
    let text = std::fs::read_to_string(&saved).unwrap();
    assert!(!text.contains(s(&data)), "install path must remain local");
    assert!(
        !text.contains("unit_defs"),
        "real-map checkpoint must not embed retail-derived scenario"
    );
    assert!(
        text.len() < 20_000,
        "only descriptor and executed commands expected"
    );
    let info = Command::new(BIN)
        .args(["checkpoint", "info", "--checkpoint", s(&saved)])
        .env_remove("TAK_DATA_DIR")
        .output()
        .unwrap();
    ok(&info); // Metadata inspection does not need the install.
    let missing = Command::new(BIN)
        .args(["checkpoint", "resume", "--checkpoint", s(&saved)])
        .env_remove("TAK_DATA_DIR")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    let hashes = dir.path("continued.txt");
    let final_save = dir.path("final.json");
    ok(&run(&[
        "checkpoint",
        "resume",
        "--data",
        s(&data),
        "--checkpoint",
        s(&saved),
        "--log",
        s(&log),
        "--ticks",
        "299",
        "--hashes",
        s(&hashes),
        "--out",
        s(&final_save),
    ]));
    assert_eq!(continuation(&hashes, 301), expected[301..]);
    let resumed = Checkpoint::read(&final_save)
        .unwrap()
        .restore(Some(&data), ReplayBudget::default())
        .unwrap();
    assert_eq!(resumed.sim().state_hash(), expected[599]);
    assert_eq!(resumed.sim().script_stats().units, 2);
    assert!(resumed.sim().cob().is_some());
}
