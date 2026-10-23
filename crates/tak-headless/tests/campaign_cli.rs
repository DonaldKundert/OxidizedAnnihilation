//! CLI campaign fixtures are hand-authored, including the tiny map/archive.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tak_commands::CommandLog;
use tak_headless::{campaign, parse_hashes, run_match};
use tak_vfs::testing::HapiBuilder;

const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");
const MAP: &str = "missions/synthetic";

struct Install(PathBuf);

impl Install {
    fn new(name: &str, second_owner: u8, condition: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("campaign-cli-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ota = format!(
            "[GLOBALHEADER]\n{{\n{condition}\nplayer1=not-interpreted;\n[MAP DATA]\n{{\n[UNITS]\n{{\n[UNIT0]\n{{\nunitname=TESTSOLDIER;\nplayer={second_owner};\nxpos=7;\nzpos=8;\nhealthpercentage=25;\ninitialmission=patrol;\naiignore=1;\n}}\n[UNIT1]\n{{\nunitname=TESTKING;\nplayer=1;\nxpos=2;\nzpos=3;\n}}\n}}\n}}\n}}\n"
        );
        let soldier = b"[UNITINFO]\n{\nunitname=TESTSOLDIER;\nmaxdamage=200;\nmaxvelocity=1;\nturnrate=400;\n}\n";
        let king = b"[UNITINFO]\n{\nunitname=TESTKING;\nmaxdamage=200;\nmaxvelocity=1;\nturnrate=400;\n}\n";
        let archive = HapiBuilder::new()
            .file("missions/synthetic.tnt", &flat_tnt(), false)
            .file("missions/synthetic.ota", ota.as_bytes(), false)
            .file("units/testsoldier.fbi", soldier, false)
            .file("units/testking.fbi", king, false)
            .file(
                "gamedata/sidedata.tdf",
                b"[SIDE0]\n{\nname=Test;\nnameprefix=TST;\ncommander=TESTKING;\n}\n",
                false,
            )
            .build();
        std::fs::write(dir.join("synthetic.hpi"), archive).unwrap();
        Install(dir)
    }

    fn run(&self, command: &str, owners: &str, extra: &[&str]) -> Output {
        Command::new(BIN)
            .args([
                command,
                "--campaign",
                MAP,
                "--campaign-players",
                owners,
                "--data",
            ])
            .arg(&self.0)
            .arg("--no-scripts")
            .args(extra)
            .output()
            .unwrap()
    }
}

impl Drop for Install {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn flat_tnt() -> Vec<u8> {
    let cells = 32 * 32;
    let tiles = cells / 4;
    let blocks = [
        vec![20; cells],
        vec![0xff; cells * 2],
        vec![0; tiles * 4],
        vec![0; tiles],
        vec![0; tiles],
    ];
    let mut pointers = Vec::new();
    let mut body = Vec::new();
    for block in blocks {
        pointers.push((52 + body.len()) as u32);
        body.extend(block);
    }
    let header = [
        0x4000,
        32,
        32,
        0,
        pointers[0],
        pointers[1],
        0,
        0,
        pointers[2],
        pointers[3],
        pointers[4],
        0,
        0,
    ];
    let mut bytes: Vec<_> = header.into_iter().flat_map(u32::to_le_bytes).collect();
    bytes.extend(body);
    bytes
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn explicit_campaign_runs_replays_and_reports_fidelity_gaps() {
    let install = Install::new("run", 10, "destroyallunits=1;");
    let first = install.run("run", "1:TST,10:TST", &["--ticks", "90"]);
    assert!(first.status.success(), "{}", stderr(&first));
    let report = stderr(&first);
    for expected in [
        "2 authored units",
        "not full campaign support",
        "player 0: OTA owner 1, side TST",
        "player 1: OTA owner 10, side TST",
        "ignores initialmission on 1 placement",
        "ignores AI flags on 1 placement",
        "alliances and neutrality are not implemented",
        "authored objectives are disabled",
    ] {
        assert!(report.contains(expected), "missing {expected:?}: {report}");
    }
    let second = install.run("run", "1:TST,10:TST", &["--ticks", "90"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert_eq!(first.stdout, second.stdout, "separate processes must agree");
    let actual = parse_hashes(std::str::from_utf8(&first.stdout).unwrap()).unwrap();
    let players = campaign::parse_players("1:TST,10:TST").unwrap();
    let loaded = campaign::load_campaign_map(&install.0, MAP, &players, 1, false).unwrap();
    assert_eq!(
        loaded.game.scenario.initial_units.as_ref().unwrap().len(),
        2
    );
    assert_eq!(
        actual,
        run_match(loaded.game, &CommandLog::new(), 90, false)
            .unwrap()
            .hashes
    );
    let hashes = install.0.join("hashes.txt");
    std::fs::write(&hashes, &first.stdout).unwrap();
    let verified = install.run(
        "verify",
        "1:TST,10:TST",
        &["--hashes", hashes.to_str().unwrap()],
    );
    assert!(verified.status.success(), "{}", stderr(&verified));
    assert!(String::from_utf8_lossy(&verified.stdout).starts_with("ok: 90 ticks match"));
}

#[test]
fn campaign_arguments_do_not_silently_guess_or_ignore_mappings() {
    for args in [
        vec!["run", "--campaign", MAP, "--ticks", "1"],
        vec![
            "run",
            "--map",
            MAP,
            "--campaign-players",
            "1:TST",
            "--ticks",
            "1",
        ],
        vec![
            "run",
            "--scenario",
            "missing.json",
            "--campaign",
            MAP,
            "--campaign-players",
            "1:TST",
            "--ticks",
            "1",
        ],
        vec![
            "run",
            "--campaign",
            MAP,
            "--campaign-players",
            "1:TST",
            "--players",
            "TST",
            "--ticks",
            "1",
        ],
        vec!["run", "--map", MAP, "--campaign-objectives", "--ticks", "1"],
    ] {
        let output = Command::new(BIN)
            .args(&args)
            .env_remove("TAK_DATA_DIR")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            stderr(&output)
        );
    }
    let install = Install::new("mapping", 10, "destroyallunits=1;");
    for (owners, message) in [
        ("1:TST", "unmapped owner"),
        ("1:TST,1:TST", "duplicate"),
        ("1:TST,10:MISSING", "unknown side"),
    ] {
        let output = install.run("run", owners, &["--ticks", "1"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(stderr(&output).contains(message), "{}", stderr(&output));
    }
    let output = Command::new(BIN)
        .args([
            "run",
            "--campaign",
            MAP,
            "--campaign-players",
            "1:TST,10:TST",
            "--ticks",
            "1",
        ])
        .env_remove("TAK_DATA_DIR")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--campaign needs --data or $TAK_DATA_DIR"));
}

#[test]
fn objectives_are_opt_in_strict_and_owner_layout_bounded() {
    let supported = Install::new("objectives", 2, "victorytimerrunsout=1;");
    let output = supported.run(
        "run",
        "1:TST,2:TST",
        &["--ticks", "31", "--campaign-objectives"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("condition evaluator enabled"));
    assert!(stderr(&output).contains("match: over at tick 30: winners [0]"));
    let output = supported.run("run", "1:TST,2:TST", &["--ticks", "31"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("match: running"));
    let output = supported.run(
        "run",
        "2:TST,1:TST",
        &["--ticks", "1", "--campaign-objectives"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("exactly OTA slots 1,2 in that order"));
    let unsupported = Install::new("unsupported-objectives", 2, "useonlyunits=TESTKING;");
    let output = unsupported.run(
        "run",
        "1:TST,2:TST",
        &["--ticks", "1", "--campaign-objectives"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("unsupported or malformed conditions: useonlyunits"));
    let unknown = Install::new("unresolved-objectives", 2, "killalloftype=MISSING;");
    let output = unknown.run(
        "run",
        "1:TST,2:TST",
        &["--ticks", "1", "--campaign-objectives"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("absent from the loaded definitions"));
}

#[test]
fn skirmish_only_log_generators_refuse_explicit_placements() {
    let install = Install::new("gen-log", 10, "destroyallunits=1;");
    let output = install.run(
        "gen-log",
        "1:TST,10:TST",
        &["--ticks", "30", "--kind", "walk"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("gen-log only supports automatic monarch startup"));
}

/// Placeholder sides prove CLI loading/replay only, not correct authored
/// player-header semantics or original mission behavior. No retail snapshots.
#[test]
fn retail_campaign_cli_runs_authored_setup_with_scripts_deterministically() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("retail_campaign_cli") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&data).unwrap();
    let map = vfs
        .list("missions")
        .into_iter()
        .find_map(|file| {
            let name = file.path.strip_suffix(".ota")?;
            vfs.stat(&format!("{name}.tnt")).map(|_| name.to_string())
        })
        .expect("retail mission map");
    let all_players: Vec<_> = (1..=16)
        .map(|ota_slot| tak_sim::campaign::CampaignPlayer {
            ota_slot,
            side: "ARA".into(),
        })
        .collect();
    let loaded = tak_sim::campaign::load_campaign(&vfs, &map, &all_players, 1).unwrap();
    let count = loaded.game.map.ota.units.len();
    assert!(count > 0);
    let slots: std::collections::BTreeSet<_> = loaded
        .game
        .map
        .ota
        .units
        .iter()
        .map(|unit| unit.player.expect("authored owner"))
        .collect();
    let owners = slots
        .into_iter()
        .map(|slot| format!("{slot}:ARA"))
        .collect::<Vec<_>>()
        .join(",");
    let mut outputs = Vec::new();
    for _ in 0..2 {
        let output = Command::new(BIN)
            .args([
                "run",
                "--campaign",
                &map,
                "--campaign-players",
                &owners,
                "--ticks",
                "30",
                "--data",
            ])
            .arg(&data)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(stderr(&output).contains(&format!("{count} authored units")));
        assert!(
            stderr(&output).contains(&format!("script units: {count}")),
            "{}",
            stderr(&output)
        );
        let hashes = parse_hashes(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
        assert_eq!(hashes.len(), 30);
        outputs.push(hashes);
    }
    assert_eq!(outputs[0], outputs[1]);
}
