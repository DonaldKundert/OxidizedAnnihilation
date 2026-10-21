//! Cross-process determinism with combat on: this test binary re-runs
//! itself as a child process, which computes the same brawl (and, with
//! `TAK_DATA_DIR`, the real-data skirmish) and writes its per-tick hashes to
//! a file; the parent compares them with its own.

mod combat_common;

use std::path::PathBuf;
use std::process::Command;

const CHILD_ENV: &str = "OXA_COMBAT_CHILD_OUT";
const TICKS: u32 = 3000;

fn hashes_text() -> String {
    let mut s = String::new();
    for h in combat_common::brawl_hashes(77, TICKS) {
        s.push_str(&format!("{h:016x}\n"));
    }
    // With game data: the real-map skirmish with the original scripts too.
    if let Some(dir) = tak_vfs::testing::data_dir_or_skip("combat_cross_process_skirmish") {
        let r = combat_common::skirmish(&dir, "ARASWORD", "ZONWOLF2", 5, 3000);
        s.push_str("skirmish\n");
        for h in r.hashes {
            s.push_str(&format!("{h:016x}\n"));
        }
    }
    s
}

/// Child half: only does something when the parent asks.
#[test]
fn child_writes_hashes() {
    let Ok(out) = std::env::var(CHILD_ENV) else {
        return;
    };
    std::fs::write(out, hashes_text()).expect("write hashes");
}

#[test]
fn brawl_hashes_agree_across_processes() {
    let dir = std::env::temp_dir().join(format!("oxa-combat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out: PathBuf = dir.join("child.txt");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["child_writes_hashes", "--exact", "--test-threads=1"])
        .env(CHILD_ENV, &out)
        .status()
        .expect("spawn child");
    assert!(status.success());
    let child = std::fs::read_to_string(&out).expect("child output");
    let _ = std::fs::remove_dir_all(&dir);
    let mine = hashes_text();
    assert!(child.lines().count() >= (TICKS - 1) as usize);
    assert_eq!(child, mine, "child process diverged");
}
