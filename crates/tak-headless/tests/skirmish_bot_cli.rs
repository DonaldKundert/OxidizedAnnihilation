//! Separate-process controller determinism and save/resume verification.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use tak_headless::checkpoint::{Checkpoint, ReplayBudget};

const BIN: &str = env!("CARGO_BIN_EXE_tak-skirmish");
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "skirmish-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
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
    Command::new(BIN).args(args).output().unwrap()
}
fn ok(output: &Output) {
    assert!(
        output.status.success(),
        "stdout {}\nstderr {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn hashes(path: &Path) -> Vec<(u32, u64)> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let (tick, hash) = line.split_once(' ').unwrap();
            (
                tick.parse().unwrap(),
                u64::from_str_radix(hash, 16).unwrap(),
            )
        })
        .collect()
}

#[test]
fn two_processes_emit_identical_logs_hashes_and_checkpoint_bytes_for_both_recipes() {
    for direct in [false, true] {
        let dir = Scratch::new();
        for suffix in ["a", "b"] {
            let record = dir.path(&format!("{suffix}.log.json"));
            let hashes = dir.path(&format!("{suffix}.hashes"));
            let save = dir.path(&format!("{suffix}.save.json"));
            let mut args = vec![
                "demo",
                "--ticks",
                "1200",
                "--record",
                s(&record),
                "--hashes",
                s(&hashes),
                "--out",
                s(&save),
            ];
            if direct {
                args.push("--direct-builder");
            }
            ok(&run(&args));
        }
        for extension in ["log.json", "hashes", "save.json"] {
            assert_eq!(
                std::fs::read(dir.path(&format!("a.{extension}"))).unwrap(),
                std::fs::read(dir.path(&format!("b.{extension}"))).unwrap()
            );
        }
        let expected = hashes(&dir.path("a.hashes"));
        for boundary in [0, 1, 2, 31, 91, 92, 200] {
            let split = dir.path("split.json");
            let boundary_text = boundary.to_string();
            let mut args = vec!["demo", "--ticks", &boundary_text, "--out", s(&split)];
            if direct {
                args.push("--direct-builder");
            }
            ok(&run(&args));
            let suffix = dir.path("suffix.hashes");
            let record = dir.path("resumed.log.json");
            let save = dir.path("resumed.save.json");
            ok(&run(&[
                "resume",
                "--checkpoint",
                s(&split),
                "--ticks",
                &(1200 - boundary).to_string(),
                "--hashes",
                s(&suffix),
                "--record",
                s(&record),
                "--out",
                s(&save),
            ]));
            assert_eq!(hashes(&suffix), expected[boundary as usize..]);
            assert_eq!(
                std::fs::read(record).unwrap(),
                std::fs::read(dir.path("a.log.json")).unwrap()
            );
            assert_eq!(
                std::fs::read(save).unwrap(),
                std::fs::read(dir.path("a.save.json")).unwrap()
            );
        }
        let final_save = Checkpoint::read(&dir.path("a.save.json")).unwrap();
        let restored = final_save.restore(None, ReplayBudget::default()).unwrap();
        assert_eq!(restored.sim().state_hash(), expected.last().unwrap().1);
    }
}

#[test]
fn invalid_aliases_or_competing_owner_input_preserve_outputs() {
    let dir = Scratch::new();
    let save = dir.path("save.json");
    ok(&run(&["demo", "--ticks", "32", "--out", s(&save)]));
    let original = std::fs::read(&save).unwrap();
    let result = run(&[
        "resume",
        "--checkpoint",
        s(&save),
        "--ticks",
        "1",
        "--hashes",
        s(&save),
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(std::fs::read(&save).unwrap(), original);
    let bad_log = dir.path("competing.json");
    let mut log = tak_commands::CommandLog::new();
    log.record(
        tak_commands::Tick(40),
        tak_commands::PlayerId(0),
        tak_commands::SimCommand::Stop {
            units: tak_commands::UnitSet::new(),
        },
    );
    std::fs::write(&bad_log, log.to_json()).unwrap();
    let output = dir.path("keep.json");
    std::fs::write(&output, b"keep existing output").unwrap();
    let result = run(&[
        "resume",
        "--checkpoint",
        s(&save),
        "--ticks",
        "100",
        "--log",
        s(&bad_log),
        "--out",
        s(&output),
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(std::fs::read(&output).unwrap(), b"keep existing output");
    assert_eq!(std::fs::read(&save).unwrap(), original);
}

#[test]
fn nonexistent_case_variant_outputs_are_rejected_before_any_write() {
    let dir = Scratch::new();
    let record = dir.path("run.json");
    let save = dir.path("RUN.JSON");
    let result = run(&[
        "demo",
        "--ticks",
        "1",
        "--record",
        s(&record),
        "--out",
        s(&save),
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert!(!record.exists());
    assert!(!save.exists());
}
