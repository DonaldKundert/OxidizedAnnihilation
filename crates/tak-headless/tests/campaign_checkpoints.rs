//! Campaign checkpoint fixtures, including all archive/map/script bytes, are
//! hand-authored. Optional retail tests retain only descriptors and hashes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use tak_commands::{CommandLog, PlayerId, SimCommand, Tick, UnitId, UnitSet};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget};
use tak_headless::parse_hashes;
use tak_vfs::testing::HapiBuilder;

const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");
const MAP: &str = "missions/checkpoint-synthetic";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new(label: &str) -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "campaign-checkpoint-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn install(&self, second_owner: u8, condition: &str) {
        let ota = format!(
            "[GLOBALHEADER]\n{{\n{condition}\n[MAP DATA]\n{{\n[UNITS]\n{{\n[UNIT0]\n{{\nunitname=MONARCH;\nplayer={second_owner};\nxpos=2;\nzpos=2;\nhealthpercentage=75;\ninitialmission=DO_NOT_EMBED_AUTHORED_ORDER;\naiignore=1;\n}}\n[UNIT1]\n{{\nunitname=MONARCH;\nplayer=1;\nxpos=1;\nzpos=1;\n}}\n}}\n}}\n}}\n"
        );
        let archive = HapiBuilder::new()
            .file(&format!("{MAP}.tnt"), &tiny_map(), false)
            .file(&format!("{MAP}.ota"), ota.as_bytes(), false)
            .file("units/monarch.fbi", b"[UNITINFO]\n{\nunitname=MONARCH;\nmaxdamage=200;\nmaxvelocity=1;\nturnrate=400;\n}\n", false)
            .file("gamedata/sidedata.tdf", b"[SIDE0]\n{\nnameprefix=TST;\ncommander=MONARCH;\n}\n", false)
            .file("scripts/monarch.cob", &sleeping_script(), false)
            .build();
        std::fs::write(self.path("synthetic.hpi"), archive).unwrap();
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
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn tiny_map() -> Vec<u8> {
    // Flat 4x4 attribute cells; no retail graphics/features/minimap.
    let header: [u32; 13] = [0x4000, 4, 4, 0, 52, 68, 0, 0, 100, 116, 120, 0, 0];
    let mut bytes: Vec<_> = header.into_iter().flat_map(u32::to_le_bytes).collect();
    bytes.extend([0u8; 16]);
    for _ in 0..16 {
        bytes.extend(0xffffu16.to_le_bytes());
    }
    bytes.extend([0u8; 24]);
    bytes
}
fn sleeping_script() -> Vec<u8> {
    // COB v6 Create sleeps two seconds, then writes static[0] = 7 and returns.
    // A split at tick 31 therefore crosses a live VM thread's wakeup.
    let code: [u32; 10] = [
        0x1002_1001,
        2000,
        0x1001_3000,
        0x1002_1001,
        7,
        0x1002_3004,
        0,
        0x1002_1001,
        0,
        0x1006_5000,
    ];
    let header: [u32; 13] = [6, 1, 0, 10, 1, 0, 92, 96, 100, 52, 100, 100, 0];
    let mut bytes: Vec<_> = header
        .into_iter()
        .chain(code)
        .flat_map(u32::to_le_bytes)
        .collect();
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(100u32.to_le_bytes());
    bytes.extend(b"Create\0");
    bytes
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
fn scripted_sparse_campaign_resumes_exactly_across_processes_without_embedded_content() {
    let dir = Scratch::new("scripted");
    dir.install(10, "");
    let log_path = dir.path("commands.json");
    let mut log = CommandLog::new();
    for tick in [0, 0, 30, 31, 90, 179, 180] {
        log.record(
            Tick(tick),
            PlayerId(1),
            SimCommand::Stop {
                units: UnitSet::single(UnitId::new(0, 0)),
            },
        );
    }
    std::fs::write(&log_path, log.to_json()).unwrap();
    let whole = run(&[
        "run",
        "--campaign",
        MAP,
        "--campaign-players",
        "1:TST,10:TST",
        "--data",
        s(&dir.0),
        "--ticks",
        "180",
        "--log",
        s(&log_path),
    ]);
    ok(&whole);
    let expected = parse_hashes(std::str::from_utf8(&whole.stdout).unwrap()).unwrap();
    let save = dir.path("save.json");
    let first = run(&[
        "checkpoint",
        "save",
        "--campaign",
        MAP,
        "--campaign-players",
        "1:TST,10:TST",
        "--data",
        s(&dir.0),
        "--ticks",
        "31",
        "--log",
        s(&log_path),
        "--out",
        s(&save),
    ]);
    ok(&first);
    let report = String::from_utf8_lossy(&first.stderr);
    for warning in [
        "2 authored units",
        "ignores initialmission",
        "ignores AI flags",
        "alliances and neutrality are not implemented",
        "authored objectives are disabled",
    ] {
        assert!(report.contains(warning), "missing {warning}: {report}");
    }
    let text = std::fs::read_to_string(&save).unwrap();
    for absent in [
        s(&dir.0),
        "unit_defs",
        "initial_units",
        "DO_NOT_EMBED_AUTHORED_ORDER",
        "synthetic.hpi",
        "Create",
        "maxdamage",
    ] {
        assert!(!text.contains(absent), "must not embed {absent:?}");
    }
    let saved = Checkpoint::read(&save).unwrap();
    assert_eq!(saved.next_tick(), Tick(31));
    assert_eq!(saved.command_count(), 3);
    let halfway = saved
        .restore(Some(&dir.0), ReplayBudget::default())
        .unwrap();
    for slot in 0..2 {
        let vm = halfway
            .sim()
            .cob()
            .unwrap()
            .instance(UnitId::new(slot, 0))
            .unwrap();
        assert_eq!(vm.statics(), &[0]);
        assert_eq!(
            vm.thread_count(),
            1,
            "Create must still be sleeping at tick 31"
        );
        assert!(vm.errors().is_empty());
    }
    drop(halfway);
    assert!(
        matches!(saved.source(), CheckpointSource::Campaign { scripts: true, objectives: false, players, .. } if players[1].ota_slot == 10)
    );
    let info = Command::new(BIN)
        .args(["checkpoint", "info", "--checkpoint", s(&save)])
        .env_remove("TAK_DATA_DIR")
        .output()
        .unwrap();
    ok(&info);
    let info = String::from_utf8_lossy(&info.stdout);
    for expected in [
        "campaign (external install required)",
        "OTA owner 10, side TST",
        "strict objectives: false",
        "initial orders",
        "replay: not checked",
    ] {
        assert!(info.contains(expected), "{info}");
    }
    // Portability includes a different install root and archive filename.
    // Remove the original archive so successful replay cannot fall back to it.
    let relocated = Scratch::new("relocated-install");
    std::fs::copy(dir.path("synthetic.hpi"), relocated.path("renamed.hpi")).unwrap();
    std::fs::remove_file(dir.path("synthetic.hpi")).unwrap();
    let hashes = dir.path("continuation.txt");
    let resumed = run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        s(&save),
        "--data",
        s(&relocated.0),
        "--ticks",
        "149",
        "--log",
        s(&log_path),
        "--hashes",
        s(&hashes),
        "--out",
        s(&save),
    ]);
    ok(&resumed);
    assert!(String::from_utf8_lossy(&resumed.stderr).contains("ignores initialmission"));
    assert_eq!(continuation(&hashes, 31), expected[31..]);
    let saved = Checkpoint::read(&save).unwrap();
    assert_eq!(saved.command_count(), 6);
    assert_eq!(saved.state_hash(), format!("{:016x}", expected[179]));
    let restored = saved
        .restore(Some(&relocated.0), ReplayBudget::default())
        .unwrap();
    assert_eq!(restored.sim().script_stats().units, 2);
    assert!(restored.sim().script_stats().missing.is_empty());
    assert_eq!(restored.sim().state_hash(), expected[179]);
    for slot in 0..2 {
        let vm = restored
            .sim()
            .cob()
            .unwrap()
            .instance(UnitId::new(slot, 0))
            .unwrap();
        assert_eq!(vm.statics(), &[7]);
        assert_eq!(vm.thread_count(), 0);
        assert!(vm.errors().is_empty());
    }
}

#[test]
fn strict_campaign_objectives_and_zero_tick_saves_match_uninterrupted_run() {
    let dir = Scratch::new("objectives");
    dir.install(2, "victorytimerrunsout=2;");
    let whole = run(&[
        "run",
        "--campaign",
        MAP,
        "--campaign-players",
        "1:TST,2:TST",
        "--campaign-objectives",
        "--data",
        s(&dir.0),
        "--ticks",
        "90",
        "--no-scripts",
    ]);
    ok(&whole);
    assert!(String::from_utf8_lossy(&whole.stderr).contains("over at tick 60"));
    let expected = parse_hashes(std::str::from_utf8(&whole.stdout).unwrap()).unwrap();
    for split in [0, 31, 75] {
        let save = dir.path(&format!("save-{split}.json"));
        let output = run(&[
            "checkpoint",
            "save",
            "--campaign",
            MAP,
            "--campaign-players",
            "1:TST,2:TST",
            "--campaign-objectives",
            "--data",
            s(&dir.0),
            "--ticks",
            &split.to_string(),
            "--no-scripts",
            "--out",
            s(&save),
        ]);
        ok(&output);
        let saved = Checkpoint::read(&save).unwrap();
        assert!(matches!(
            saved.source(),
            CheckpointSource::Campaign {
                objectives: true,
                scripts: false,
                ..
            }
        ));
        let mut restored = saved
            .restore(Some(&dir.0), ReplayBudget::default())
            .unwrap();
        assert!(restored.sim().cob().is_none());
        assert_eq!(
            restored.advance(&CommandLog::new(), 90 - split).unwrap(),
            expected[split as usize..]
        );
    }
}

#[test]
fn invalid_campaign_arguments_and_objectives_preserve_save_output() {
    let dir = Scratch::new("reject");
    dir.install(2, "useonlyunits=MONARCH;");
    let save = dir.path("preserve.json");
    std::fs::write(&save, b"preserve existing checkpoint").unwrap();
    let common = [
        "checkpoint",
        "save",
        "--campaign",
        MAP,
        "--data",
        s(&dir.0),
        "--ticks",
        "0",
        "--out",
        s(&save),
    ];
    for extra in [
        vec![],
        vec!["--campaign-players", "1:TST,1:TST"],
        vec!["--campaign-players", "1:TST,2:TST", "--players", "TST,TST"],
        vec!["--campaign-players", "1:TST,2:TST", "--map", MAP],
        vec![
            "--campaign-players",
            "1:TST,2:TST",
            "--scenario",
            "missing.json",
        ],
        vec!["--campaign-players", "2:TST,1:TST", "--campaign-objectives"],
        vec!["--campaign-players", "1:TST,2:TST", "--campaign-objectives"],
    ] {
        let result = Command::new(BIN).args(common).args(extra).output().unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert_eq!(
            std::fs::read(&save).unwrap(),
            b"preserve existing checkpoint"
        );
    }
}

#[test]
fn installed_authored_campaign_resumes_scripts_and_stores_only_descriptor() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("campaign_checkpoint_scripts") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&data).unwrap();
    let map = vfs
        .list("missions")
        .into_iter()
        .find_map(|file| {
            let stem = file.path.strip_suffix(".ota")?;
            vfs.stat(&format!("{stem}.tnt")).map(|_| stem.to_string())
        })
        .expect("private authored mission");
    let all_players: Vec<_> = (1..=16)
        .map(|ota_slot| tak_sim::campaign::CampaignPlayer {
            ota_slot,
            side: "ARA".into(),
        })
        .collect();
    let loaded = tak_sim::campaign::load_campaign(&vfs, &map, &all_players, 1).unwrap();
    let count = loaded.game.map.ota.units.len();
    assert!(count > 0);
    let owners: std::collections::BTreeSet<_> = loaded
        .game
        .map
        .ota
        .units
        .iter()
        .map(|unit| unit.player.unwrap())
        .collect();
    let players = owners
        .iter()
        .map(|slot| format!("{slot}:ARA"))
        .collect::<Vec<_>>()
        .join(",");
    let dir = Scratch::new("private");
    let whole = run(&[
        "run",
        "--campaign",
        &map,
        "--campaign-players",
        &players,
        "--data",
        s(&data),
        "--ticks",
        "60",
    ]);
    ok(&whole);
    let expected = parse_hashes(std::str::from_utf8(&whole.stdout).unwrap()).unwrap();
    let save = dir.path("save.json");
    let saved = run(&[
        "checkpoint",
        "save",
        "--campaign",
        &map,
        "--campaign-players",
        &players,
        "--data",
        s(&data),
        "--ticks",
        "31",
        "--out",
        s(&save),
    ]);
    ok(&saved);
    let text = std::fs::read_to_string(&save).unwrap();
    assert!(!text.contains(s(&data)));
    assert!(!text.contains("unit_defs"));
    assert!(!text.contains("initial_units"));
    assert!(!text.contains("unsupported"));
    assert!(text.len() < 4000, "descriptor and empty command log only");
    let hashes = dir.path("continued.txt");
    ok(&run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        s(&save),
        "--data",
        s(&data),
        "--ticks",
        "29",
        "--hashes",
        s(&hashes),
        "--out",
        s(&save),
    ]));
    assert_eq!(continuation(&hashes, 31), expected[31..]);
    let restored = Checkpoint::read(&save)
        .unwrap()
        .restore(Some(&data), ReplayBudget::default())
        .unwrap();
    assert_eq!(restored.sim().state_hash(), expected[59]);
    assert_eq!(restored.sim().script_stats().units, count as u64);
    assert!(restored.sim().cob().is_some());
}
