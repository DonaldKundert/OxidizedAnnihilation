//! Real data (gated on `TAK_DATA_DIR`): Adamantine Gate with an Aramon and
//! a Zhon monarch, COB scripts on, 10,000 ticks, identical hashes from two
//! separate processes, and `verify` accepts the recording.

use std::path::{Path, PathBuf};
use std::process::Command;

use tak_headless::parse_hashes;

const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");
const MAP: &str = "kmap/Adamantine Gate";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("tak-headless-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn run_bin(data: &Path, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .args(["--data", s(data)])
        .output()
        .expect("spawn tak-headless")
}

#[test]
fn real_map_two_processes_agree_over_ten_thousand_ticks() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("real_map_two_processes") else {
        return;
    };
    let dir = scratch("real-map");
    let log = dir.join("walk.json");
    let r = run_bin(
        &data,
        &[
            "gen-log",
            "--map",
            MAP,
            "--players",
            "ARA,ZON",
            "--ticks",
            "10000",
            "--kind",
            "walk",
            "--every",
            "450",
            "--seed",
            "3",
            "--out",
            s(&log),
        ],
    );
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));

    let mut outputs = Vec::new();
    for name in ["a.txt", "b.txt"] {
        let out = dir.join(name);
        let r = run_bin(
            &data,
            &[
                "run",
                "--map",
                MAP,
                "--players",
                "ARA,ZON",
                "--log",
                s(&log),
                "--ticks",
                "10000",
                "--hashes",
                s(&out),
            ],
        );
        let stderr = String::from_utf8_lossy(&r.stderr).to_string();
        assert!(r.status.success(), "{stderr}");
        assert!(stderr.contains("ran 10000 ticks"), "{stderr}");
        assert!(stderr.contains("script units: 2"), "{stderr}");
        assert!(stderr.contains("other thread errors: 0"), "{stderr}");
        outputs.push(std::fs::read_to_string(&out).unwrap());
    }
    assert_eq!(outputs[0], outputs[1], "two processes disagree");
    let hashes = parse_hashes(&outputs[0]).unwrap();
    assert_eq!(hashes.len(), 10_000);
    let mut distinct = hashes.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(
        distinct.len() > 9_000,
        "state (and script state) changes every tick"
    );

    let r = run_bin(
        &data,
        &[
            "verify",
            "--map",
            MAP,
            "--players",
            "ARA,ZON",
            "--log",
            s(&log),
            "--hashes",
            s(&dir.join("a.txt")),
        ],
    );
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));

    // Scripts are part of the hash: the same run without them differs.
    let r = run_bin(
        &data,
        &[
            "verify",
            "--map",
            MAP,
            "--players",
            "ARA,ZON",
            "--log",
            s(&log),
            "--hashes",
            s(&dir.join("a.txt")),
            "--no-scripts",
        ],
    );
    assert_eq!(r.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&r.stdout).starts_with("mismatch at tick 0:"));
}

#[test]
fn map_runs_need_an_install_and_valid_sides() {
    let r = Command::new(BIN)
        .args(["run", "--map", MAP, "--ticks", "5"])
        .env_remove("TAK_DATA_DIR")
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(2));
    let Some(data) = tak_vfs::testing::data_dir_or_skip("map_runs_need_valid_sides") else {
        return;
    };
    let r = run_bin(
        &data,
        &["run", "--map", MAP, "--players", "XYZ", "--ticks", "5"],
    );
    assert_eq!(r.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&r.stderr).contains("unknown side"));
}
