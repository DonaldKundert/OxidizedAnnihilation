//! Authored startup is explicit, strict, and identical to the headless source.
//! All committed fixtures here are hand-authored, not extracted game data.

use std::path::{Path, PathBuf};

use clap::Parser;
use tak_client::cli::Args;
use tak_client::game::{Session, open_vfs};
use tak_commands::{CommandLog, PlayerId, QueueMode, SimCommand, Tick, UnitId, UnitSet};
use tak_headless::campaign;
use tak_math::{Fx, Vec2};
use tak_vfs::testing::HapiBuilder;

const MAP: &str = "missions/synthetic";

struct Install(PathBuf);

impl Install {
    fn new(name: &str, owner: u8, condition: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("client-campaign-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ota = format!(
            "[GLOBALHEADER]\n{{\n{condition}\nplayer1=not-interpreted;\n[MAP DATA]\n{{\n[SPECIALS]\n{{\n[SPECIAL0]\n{{\nspecialwhat=StartPos1;\nxpos=2;\nzpos=3;\n}}\n[SPECIAL1]\n{{\nspecialwhat=StartPos2;\nxpos=7;\nzpos=8;\n}}\n}}\n[UNITS]\n{{\n[UNIT0]\n{{\nunitname=TESTSOLDIER;\nplayer={owner};\nxpos=7;\nzpos=8;\nangle=90;\nhealthpercentage=25;\ninitialmission=patrol;\naiignore=1;\n}}\n[UNIT1]\n{{\nunitname=TESTKING;\nplayer=1;\nxpos=2;\nzpos=3;\n}}\n}}\n}}\n}}\n"
        );
        let archive = HapiBuilder::new()
            .file("missions/synthetic.tnt", &flat_tnt(), false)
            .file("missions/synthetic.ota", ota.as_bytes(), false)
            .file(
                "units/testsoldier.fbi",
                b"[UNITINFO]\n{\nunitname=TESTSOLDIER;\nname=Test Soldier;\nobjectname=soldier;\nmaxdamage=200;\nmaxvelocity=1;\nturnrate=400;\n}\n",
                false,
            )
            .file(
                "units/testking.fbi",
                b"[UNITINFO]\n{\nunitname=TESTKING;\nmaxdamage=200;\nmaxvelocity=1;\nturnrate=400;\n}\n",
                false,
            )
            .file(
                "gamedata/sidedata.tdf",
                b"[SIDE0]\n{\nname=Test;\nnameprefix=TST;\ncommander=TESTKING;\n}\n",
                false,
            )
            .build();
        std::fs::write(dir.join("synthetic.hpi"), archive).unwrap();
        Self(dir)
    }

    fn args(&self, owners: &str) -> Args {
        Args::try_parse_from([
            "tak-client",
            "--data",
            self.0.to_str().unwrap(),
            "--campaign",
            MAP,
            "--campaign-players",
            owners,
        ])
        .unwrap()
    }

    fn load(&self, owners: &str, objectives: bool) -> Result<Session, String> {
        Session::load_campaign(&self.0, MAP, owners, 1, objectives, CommandLog::new())
            .map_err(|e| e.to_string())
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
    let mut pointers = Vec::new();
    let mut body = Vec::new();
    for block in [
        vec![20; cells],
        vec![0xff; cells * 2],
        vec![0; tiles * 4],
        vec![0; tiles],
        vec![0; tiles],
    ] {
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

#[test]
fn cli_keeps_skirmish_defaults_and_requires_explicit_campaign_configuration() {
    let defaults = Args::try_parse_from(["tak-client", "--data", "unused"]).unwrap();
    assert_eq!(defaults.map, "kmap/Adamantine Gate");
    assert_eq!(defaults.players, "ARA,ZON");
    assert!(defaults.campaign.is_none());
    assert!(!defaults.campaign_objectives);
    defaults.validate_source().unwrap();
    for extra in [
        vec!["--demo"],
        vec!["--ghost", "ghost.json"],
        vec!["--demo", "--record-session", "session.json"],
        vec![
            "--synth-events",
            "pad.json",
            "--record-session",
            "session.json",
        ],
    ] {
        let args =
            Args::try_parse_from(["tak-client", "--data", "unused"].into_iter().chain(extra))
                .unwrap();
        args.validate_source().unwrap();
    }

    for extra in [
        vec!["--campaign", MAP],
        vec!["--campaign-players", "1:TST,2:TST"],
        vec!["--campaign-objectives"],
    ] {
        assert!(
            Args::try_parse_from(["tak-client", "--data", "unused"].into_iter().chain(extra))
                .is_err()
        );
    }
    let args = Args::try_parse_from([
        "tak-client",
        "--data",
        "unused",
        "--campaign",
        MAP,
        "--campaign-players",
        "10:TST,1:TST",
        "--log",
        "orders.json",
        "--record",
        "record.json",
        "--hashes",
        "hashes.txt",
        "--screenshot",
        "mission.png",
    ])
    .unwrap();
    assert_eq!(args.campaign.as_deref(), Some(MAP));
    assert_eq!(args.campaign_players.as_deref(), Some("10:TST,1:TST"));
    args.validate_source().unwrap();
}

#[test]
fn cli_rejects_conflicting_skirmish_and_unrepresentable_recording_modes() {
    for extra in [
        vec!["--map", MAP],
        vec!["--players", "TST,TST"],
        vec!["--demo"],
        vec!["--ghost", "ghost.json"],
        vec!["--record-session", "session.json"],
        vec![
            "--synth-events",
            "pad.json",
            "--record-session",
            "session.json",
        ],
    ] {
        let parsed = Args::try_parse_from(
            [
                "tak-client",
                "--data",
                "unused",
                "--campaign",
                MAP,
                "--campaign-players",
                "1:TST,2:TST",
            ]
            .into_iter()
            .chain(extra.iter().copied()),
        );
        let error = parsed.unwrap_err().to_string();
        assert!(error.contains("cannot be used with"), "{extra:?}: {error}");
    }
}

#[test]
fn runtime_source_guard_explains_unsupported_modes_before_any_loading() {
    let install = Install::new("guards", 2, "");
    let vfs = open_vfs(&install.0).unwrap();
    for mode in ["demo", "ghost", "record-session", "synth-events"] {
        let mut args = install.args("1:TST,2:TST");
        match mode {
            "demo" => args.demo = true,
            "ghost" => args.ghost = Some("unused.json".into()),
            "record-session" => args.record_session = Some("unused.json".into()),
            "synth-events" => args.synth_events = Some("unused.json".into()),
            _ => unreachable!(),
        }
        let error = Session::load_from_args(&vfs, &args, CommandLog::new())
            .err()
            .expect("unsupported mode must error")
            .to_string();
        assert!(error.contains(mode), "{error}");
        assert!(error.contains("skirmish"), "{error}");
    }
}

#[test]
fn session_routes_campaign_map_and_preserves_authored_order_and_replay_hashes() {
    let install = Install::new("session", 10, "destroyallunits=1;");
    let args = install.args("10:TST,1:TST");
    let vfs = open_vfs(&install.0).unwrap();
    let mut log = CommandLog::new();
    log.record(
        Tick(2),
        PlayerId(0),
        SimCommand::Move {
            units: UnitSet::single(UnitId::new(0, 0)),
            to: Vec2::new(Fx::from_int(180), Fx::from_int(160)),
            queue: QueueMode::Replace,
        },
    );
    let log = CommandLog::from_json(&log.to_json()).unwrap();
    let mut session = Session::load_from_args(&vfs, &args, log.clone()).unwrap();
    assert_eq!(session.map_name, MAP, "renderer must not use default --map");
    assert_ne!(session.map_name, args.map);
    assert_eq!((session.map.width, session.map.height), (32, 32));
    assert_eq!(session.local, PlayerId(0));
    assert_eq!(session.log, log);
    assert!(session.recorded.is_empty());
    let report = session.campaign_report.as_ref().unwrap();
    for text in [
        "2 authored units",
        "not full campaign support",
        "player 0: OTA owner 10",
        "player 1: OTA owner 1",
        "ignores initialmission",
        "ignores AI flags",
        "alliances and neutrality are not implemented",
        "authored objectives are disabled",
    ] {
        assert!(report.contains(text), "missing {text}: {report}");
    }
    let players = campaign::parse_players("10:TST,1:TST").unwrap();
    let loaded = campaign::load_campaign_map(&install.0, MAP, &players, 1, false).unwrap();
    let mut initial =
        tak_sim::Sim::with_scripts(&loaded.game.scenario, Box::new(loaded.game.scripts)).unwrap();
    assert_eq!(session.sim.state_hash(), initial.state_hash());
    assert_eq!(session.step(), initial.step(&[]));
    let world = session.sim.world();
    assert_eq!(world.unit_count(), 2, "no automatic bonus monarchs");
    let first = world.unit(UnitId::new(0, 0)).unwrap();
    assert_eq!(first.owner, PlayerId(0));
    assert_eq!(first.pos, Vec2::new(Fx::from_int(120), Fx::from_int(136)));
    assert_eq!(first.combat.health, 50);
    assert_eq!(first.heading.raw(), 16384);
    assert_eq!(world.unit(UnitId::new(1, 0)).unwrap().owner, PlayerId(1));
    assert_eq!(world.players()[1].monarch, Some(UnitId::new(1, 0)));
    let info = session.def(first.def).unwrap();
    assert_eq!(info.display, "Test Soldier");
    assert_eq!(info.object, "soldier");
    for _ in 1..90 {
        session.step();
    }
    let loaded = campaign::load_campaign_map(&install.0, MAP, &players, 1, false).unwrap();
    let run = tak_headless::run_match(loaded.game, &log, 90, true).unwrap();
    assert_eq!(session.hashes, run.hashes);

    // The same map remains a skirmish when authored startup was not requested.
    let mut skirmish = Session::load(&vfs, MAP, "TST,TST", 1, log.clone()).unwrap();
    assert!(skirmish.campaign_report.is_none());
    assert_eq!(skirmish.log, log);
    let loaded =
        tak_sim::load_match(&vfs, MAP, &tak_sim::PlayerSetup::parse_list("TST,TST"), 1).unwrap();
    let run = tak_headless::run_match(loaded, &log, 30, true).unwrap();
    for _ in 0..30 {
        skirmish.step();
    }
    assert_eq!(skirmish.hashes, run.hashes);
}

#[test]
fn session_reuses_explicit_mapping_errors_without_guessing_sides() {
    let install = Install::new("mapping", 10, "");
    for (owners, expected) in [
        ("1:TST", "unmapped owner"),
        ("1:TST,1:TST", "duplicate"),
        ("1:TST,10:MISSING", "unknown side"),
        ("1:TST,10:", "nonempty side"),
        ("0:TST,10:TST", "1..=255"),
    ] {
        let error = install.load(owners, false).err().expect("invalid mapping");
        assert!(error.contains(expected), "{owners}: {error}");
    }
}

#[test]
fn objective_opt_in_uses_headless_strict_validation() {
    let supported = Install::new("objectives", 2, "victorytimerrunsout=1;");
    let mut args = supported.args("1:TST,2:TST");
    args.campaign_objectives = true;
    let mut enabled =
        Session::load_from_args(&open_vfs(&supported.0).unwrap(), &args, CommandLog::new())
            .unwrap();
    let mut disabled = supported.load("1:TST,2:TST", false).unwrap();
    assert!(
        enabled
            .campaign_report
            .as_ref()
            .unwrap()
            .contains("condition evaluator enabled")
    );
    for _ in 0..31 {
        enabled.step();
        disabled.step();
    }
    assert!(matches!(
        enabled.sim.match_state(),
        tak_sim::victory::MatchState::Over { .. }
    ));
    assert!(matches!(
        disabled.sim.match_state(),
        tak_sim::victory::MatchState::Running
    ));
    let players = campaign::parse_players("1:TST,2:TST").unwrap();
    let loaded = campaign::load_campaign_map(&supported.0, MAP, &players, 1, true).unwrap();
    assert_eq!(
        enabled.hashes,
        tak_headless::run_match(loaded.game, &CommandLog::new(), 31, true)
            .unwrap()
            .hashes
    );
    let error = supported.load("2:TST,1:TST", true).err().unwrap();
    assert!(
        error.contains("exactly OTA slots 1,2 in that order"),
        "{error}"
    );
    for (name, condition, expected) in [
        (
            "unsupported",
            "useonlyunits=TESTKING;",
            "unsupported or malformed conditions",
        ),
        (
            "unresolved",
            "killalloftype=MISSING;",
            "absent from the loaded definitions",
        ),
        (
            "malformed",
            "victorytimerrunsout=not-a-number;",
            "unsupported or malformed conditions",
        ),
    ] {
        let install = Install::new(name, 2, condition);
        assert!(
            install.load("1:TST,2:TST", false).is_ok(),
            "objectives remain opt-in"
        );
        let error = install.load("1:TST,2:TST", true).err().unwrap();
        assert!(error.contains(expected), "{error}");
    }
}

/// Placeholder mappings prove startup and script replay, not retail ownership
/// semantics, alliances, AI, campaign parity or mission completion.
#[test]
fn retail_campaign_startup_and_script_hashes_match_headless() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client_campaign_replay") else {
        return;
    };
    let vfs = open_vfs(&data).unwrap();
    let map = vfs
        .list("missions")
        .into_iter()
        .find_map(|file| {
            let name = file.path.strip_suffix(".ota")?;
            vfs.stat(&format!("{name}.tnt")).map(|_| name.to_string())
        })
        .expect("retail mission");
    let all_players = campaign::parse_players(
        &(1..=16)
            .map(|owner| format!("{owner}:ARA"))
            .collect::<Vec<_>>()
            .join(","),
    )
    .unwrap();
    let loaded = tak_sim::campaign::load_campaign(&vfs, &map, &all_players, 1).unwrap();
    let count = loaded.game.map.ota.units.len();
    assert!(count > 0);
    let owners = loaded
        .game
        .map
        .ota
        .units
        .iter()
        .map(|unit| unit.player.unwrap())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|owner| format!("{owner}:ARA"))
        .collect::<Vec<_>>()
        .join(",");
    let players = campaign::parse_players(&owners).unwrap();
    let loaded = campaign::load_campaign_map(&data, &map, &players, 1, false).unwrap();
    let expected = tak_sim::Sim::with_scripts(&loaded.game.scenario, Box::new(loaded.game.scripts))
        .unwrap()
        .state_hash();
    let mut session =
        Session::load_campaign(&data, &map, &owners, 1, false, CommandLog::new()).unwrap();
    assert_eq!(session.sim.state_hash(), expected, "pre-tick startup hash");
    session.step();
    assert_eq!(session.sim.world().unit_count(), count);
    for _ in 1..30 {
        session.step();
    }
    let loaded = campaign::load_campaign_map(&data, &map, &players, 1, false).unwrap();
    let run = tak_headless::run_match(loaded.game, &CommandLog::new(), 30, true).unwrap();
    assert_eq!(session.hashes, run.hashes);
    eprintln!(
        "retail campaign client/headless: {count} authored units, 30 identical scripted ticks"
    );
}
