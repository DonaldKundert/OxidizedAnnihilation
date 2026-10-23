//! Hand-authored content fixtures only. Playable client resume uses the canonical
//! source, history, script state and absolute ticks, with no renderer dependency.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use tak_client::cli::Args;
use tak_client::game::{Session, open_vfs};
use tak_client::saves;
use tak_commands::{CommandLog, PlayerId, QueueMode, SimCommand, Tick, UnitId, UnitSet};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_math::{Fx, Vec2};
use tak_vfs::testing::HapiBuilder;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAP: &str = "missions/client-save-test";
struct Scratch(PathBuf);
impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "tak-client-saves-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn install(&self, name: &str, damage: u32) {
        self.install_named(name, damage, "Test Monarch");
    }
    fn install_named(&self, name: &str, damage: u32, display: &str) {
        let ota = b"[GLOBALHEADER]\n{\n[MAP DATA]\n{\n[SPECIALS]\n{\n[SPECIAL0]\n{\nspecialwhat=StartPos1;\nxpos=2;\nzpos=2;\n}\n[SPECIAL1]\n{\nspecialwhat=StartPos2;\nxpos=25;\nzpos=25;\n}\n}\n[UNITS]\n{\n[UNIT0]\n{\nunitname=MONARCH;\nplayer=10;\nxpos=2;\nzpos=2;\n}\n[UNIT1]\n{\nunitname=MONARCH;\nplayer=1;\nxpos=25;\nzpos=25;\n}\n}\n}\n}\n";
        let fbi = format!(
            "[UNITINFO]\n{{\nunitname=MONARCH;\nname={display};\nobjectname=monarch;\nmaxdamage={damage};\nmaxvelocity=1;\nturnrate=400;\n}}\n"
        );
        let archive = HapiBuilder::new()
            .file(&format!("{MAP}.tnt"), &flat_tnt(), false)
            .file(&format!("{MAP}.ota"), ota, false)
            .file("units/monarch.fbi", fbi.as_bytes(), false)
            .file(
                "gamedata/sidedata.tdf",
                b"[SIDE0]\n{\nnameprefix=TST;\ncommander=MONARCH;\n}\n",
                false,
            )
            .file("scripts/monarch.cob", &sleeping_script(), false)
            .build();
        std::fs::write(self.0.join(name), archive).unwrap();
    }
    fn args(&self, campaign: bool) -> Args {
        let mut args =
            Args::try_parse_from(["tak-client", "--data", self.0.to_str().unwrap()]).unwrap();
        if campaign {
            args.campaign = Some(MAP.into());
            args.campaign_players = Some("10:TST,1:TST".into());
        } else {
            args.map = MAP.into();
            args.players = "TST,TST".into();
        }
        args
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn flat_tnt() -> Vec<u8> {
    let mut body = Vec::new();
    let mut pointers = Vec::new();
    for block in [
        vec![20u8; 1024],
        vec![0xff; 2048],
        vec![0; 1024],
        vec![0; 256],
        vec![0; 256],
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
fn sleeping_script() -> Vec<u8> {
    // Original synthetic COB v6: Create sleeps 2s, then writes static[0] = 7.
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
fn move_order(x: i32) -> SimCommand {
    SimCommand::Move {
        units: UnitSet::single(UnitId::new(0, 0)),
        to: Vec2::new(Fx::from_int(x), Fx::from_int(110)),
        queue: QueueMode::Replace,
    }
}
fn resume_args(data: &Path, save: &Path) -> Args {
    Args::try_parse_from([
        "tak-client",
        "--data",
        data.to_str().unwrap(),
        "--load-checkpoint",
        save.to_str().unwrap(),
    ])
    .unwrap()
}

#[test]
fn scripted_skirmish_and_sparse_campaign_continue_new_orders_resave_and_relocate() {
    for campaign in [false, true] {
        let original = Scratch::new(if campaign { "campaign" } else { "skirmish" });
        original.install("original.hpi", 200);
        let saves_dir = Scratch::new("save-dir");
        let args = original.args(campaign);
        let vfs = open_vfs(&original.0).unwrap();
        let mut live = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
        assert!(live.sim.cob().is_some());
        live.step();
        live.submit(move_order(160));
        for _ in 1..31 {
            live.step();
        }
        let saved = saves::quick_save(&live, &saves_dir.0).unwrap();
        let saved_bytes = std::fs::read(&saved).unwrap();
        let checkpoint = Checkpoint::read(&saved).unwrap();
        assert_eq!(checkpoint.next_tick(), Tick(31));
        assert_eq!(checkpoint.command_count(), 1);
        assert_eq!(
            matches!(checkpoint.source(), CheckpointSource::Campaign { .. }),
            campaign
        );
        let text = std::str::from_utf8(&saved_bytes).unwrap();
        assert!(!text.contains(original.0.to_str().unwrap()));
        assert!(!text.contains("unit_defs"));
        let relocated = Scratch::new("relocated");
        std::fs::rename(
            original.0.join("original.hpi"),
            relocated.0.join("renamed.hpi"),
        )
        .unwrap();
        let args = resume_args(&relocated.0, &saved);
        let mut resumed =
            Session::load_from_args(&open_vfs(&relocated.0).unwrap(), &args, CommandLog::new())
                .unwrap();
        assert_eq!(resumed.sim.state_hash(), live.sim.state_hash());
        let vm = resumed
            .sim
            .cob()
            .unwrap()
            .instance(UnitId::new(0, 0))
            .unwrap();
        assert_eq!(vm.statics(), &[0]);
        assert_eq!(
            vm.thread_count(),
            1,
            "Create sleeps across the save boundary"
        );
        assert_eq!(resumed.hashes_first_tick, Tick(31));
        assert!(resumed.hashes.is_empty());
        // A new local player order executes at the absolute resumed tick.
        live.submit(move_order(220));
        resumed.submit(move_order(220));
        for _ in 31..90 {
            assert_eq!(resumed.step(), live.step());
        }
        assert_eq!(resumed.log, live.log);
        assert_eq!(
            resumed
                .sim
                .cob()
                .unwrap()
                .instance(UnitId::new(0, 0))
                .unwrap()
                .statics(),
            &[7]
        );
        let suffix = saves::format_hashes(&resumed);
        assert!(suffix.starts_with("# tak-headless continuation hashes v1\n31 "));
        assert_eq!(resumed.hashes, live.hashes[31..]);
        let second = saves::quick_save(&resumed, &saves_dir.0).unwrap();
        assert_ne!(second, saved);
        assert_eq!(std::fs::read(&saved).unwrap(), saved_bytes);
        let args = resume_args(&relocated.0, &second);
        let mut twice =
            Session::load_from_args(&open_vfs(&relocated.0).unwrap(), &args, CommandLog::new())
                .unwrap();
        twice.submit(SimCommand::Stop {
            units: UnitSet::single(UnitId::new(0, 0)),
        });
        resumed.submit(SimCommand::Stop {
            units: UnitSet::single(UnitId::new(0, 0)),
        });
        for _ in 0..20 {
            assert_eq!(twice.step(), resumed.step());
        }
        assert_eq!(saves::capture(&twice).unwrap().command_count(), 3);
    }
}

#[test]
fn failures_preserve_live_session_and_preexisting_saves_and_never_write_assets() {
    let install = Scratch::new("fail-install");
    install.install("test.hpi", 200);
    let output = Scratch::new("fail-output");
    let args = install.args(false);
    let vfs = open_vfs(&install.0).unwrap();
    let mut live = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
    live.step();
    live.submit(move_order(180));
    for _ in 1..12 {
        live.step();
    }
    let save = saves::quick_save(&live, &output.0).unwrap();
    let bytes = std::fs::read(&save).unwrap();
    let hash = live.sim.state_hash();
    let bad = output.0.join("corrupt.save.json");
    std::fs::write(&bad, b"{broken").unwrap();
    assert!(
        Session::load_from_args(&vfs, &resume_args(&install.0, &bad), CommandLog::new()).is_err()
    );
    let changed = Scratch::new("changed");
    changed.install("changed.hpi", 201);
    let error = Session::load_from_args(
        &open_vfs(&changed.0).unwrap(),
        &resume_args(&changed.0, &save),
        CommandLog::new(),
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains("content mismatch"), "{error}");
    assert_eq!(std::fs::read(&save).unwrap(), bytes);
    assert_eq!(live.sim.state_hash(), hash);
    assert!(saves::quick_save(&live, &install.0.join("forbidden")).is_err());
    assert!(!install.0.join("forbidden").exists());
    let repo = Scratch::new("repository");
    std::fs::write(repo.0.join(".git"), b"gitdir: /not-used").unwrap();
    assert!(saves::save_directory(Some(&repo.0.join("saves")), &install.0).is_err());
    assert!(!repo.0.join("saves").exists());
    // Changing executed history must not silently produce a different save.
    live.log = CommandLog::new();
    let error = saves::quick_save(&live, &output.0).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("executed command history was edited"),
        "{error}"
    );
    assert_eq!(std::fs::read(&save).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(&output.0).unwrap().count(), 2);
}

#[test]
fn no_script_source_and_future_logs_keep_exact_semantics() {
    let install = Scratch::new("no-scripts");
    install.install("test.hpi", 200);
    let output = Scratch::new("no-script-saves");
    let mut replay = ReplaySession::new(
        CheckpointSource::map(MAP, "TST,TST", 9, false),
        Some(&install.0),
    )
    .unwrap();
    let mut log = CommandLog::new();
    log.record(Tick(2), PlayerId(0), move_order(120));
    replay.advance(&log, 12).unwrap();
    let save = output.0.join("headless.save.json");
    replay.checkpoint().unwrap().write_atomic(&save).unwrap();
    let mut future = CommandLog::new();
    future.record(Tick(14), PlayerId(0), move_order(190));
    let mut resumed = Session::load_from_args(
        &open_vfs(&install.0).unwrap(),
        &resume_args(&install.0, &save),
        future.clone(),
    )
    .unwrap();
    assert!(resumed.sim.cob().is_none());
    assert_eq!(
        saves::capture(&resumed).unwrap().command_count(),
        1,
        "future fixture commands excluded"
    );
    let expected = replay.advance(&future, 10).unwrap();
    for h in expected {
        assert_eq!(resumed.step(), h);
    }
    assert_eq!(saves::capture(&resumed).unwrap().command_count(), 2);
    let mut bad = CommandLog::new();
    bad.record(Tick(2), PlayerId(0), move_order(121));
    assert!(
        Session::load_from_args(
            &open_vfs(&install.0).unwrap(),
            &resume_args(&install.0, &save),
            bad
        )
        .is_err()
    );
    let mut args = resume_args(&install.0, &save);
    args.max_replay_ticks = 11;
    assert!(
        Session::load_from_args(&open_vfs(&install.0).unwrap(), &args, CommandLog::new()).is_err()
    );
}

#[test]
fn cli_conflicts_and_runtime_unsupported_modes_are_clear() {
    for extra in [
        vec!["--demo"],
        vec!["--ghost", "ghost.json"],
        vec!["--campaign", MAP, "--campaign-players", "1:TST"],
        vec!["--map", MAP],
        vec!["--players", "TST"],
        vec!["--world-seed", "3"],
        vec!["--record-session", "session.json"],
    ] {
        assert!(
            Args::try_parse_from(
                [
                    "tak-client",
                    "--data",
                    "unused",
                    "--load-checkpoint",
                    "save.json"
                ]
                .into_iter()
                .chain(extra)
            )
            .is_err()
        );
    }
    let install = Scratch::new("mode-guards");
    install.install("test.hpi", 200);
    let vfs = open_vfs(&install.0).unwrap();
    for mode in ["demo", "ghost", "record-session", "synth-events"] {
        let mut args = install.args(false);
        match mode {
            "demo" => args.demo = true,
            "ghost" => args.ghost = Some("ignored".into()),
            "record-session" => args.record_session = Some("ignored".into()),
            "synth-events" => args.synth_events = Some("ignored".into()),
            _ => unreachable!(),
        }
        let session = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
        assert!(
            saves::capture(&session)
                .unwrap_err()
                .to_string()
                .contains("quick-save unavailable")
        );
    }
    let mut args = install.args(false);
    args.max_replay_ticks = 1;
    let mut live = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
    live.step();
    live.step();
    let output = Scratch::new("budget-output");
    let error = saves::quick_save(&live, &output.0).unwrap_err();
    assert!(
        error.to_string().contains("raise --max-replay-ticks"),
        "{error}"
    );
    assert_eq!(std::fs::read_dir(&output.0).unwrap().count(), 0);
}

#[test]
fn installed_skirmish_and_campaign_resume_accept_player_orders_and_resave() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client checkpoints") else {
        return;
    };
    let vfs = open_vfs(&data).unwrap();
    let map = vfs
        .list("missions")
        .into_iter()
        .find_map(|f| {
            f.path
                .strip_suffix(".ota")
                .filter(|stem| vfs.stat(&format!("{stem}.tnt")).is_some())
                .map(str::to_owned)
        })
        .unwrap();
    let probe_players = tak_headless::campaign::parse_players(
        &(1..=16)
            .map(|p| format!("{p}:ARA"))
            .collect::<Vec<_>>()
            .join(","),
    )
    .unwrap();
    let probe = tak_sim::campaign::load_campaign(&vfs, &map, &probe_players, 1).unwrap();
    let owners = probe
        .game
        .map
        .ota
        .units
        .iter()
        .map(|u| u.player.unwrap())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|p| format!("{p}:ARA"))
        .collect::<Vec<_>>()
        .join(",");
    for campaign in [false, true] {
        let mut args =
            Args::try_parse_from(["tak-client", "--data", data.to_str().unwrap()]).unwrap();
        if campaign {
            args.campaign = Some(map.clone());
            args.campaign_players = Some(owners.clone());
        }
        let mut live = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
        for _ in 0..31 {
            live.step();
        }
        let output = Scratch::new("retail-output");
        let save = saves::quick_save(&live, &output.0).unwrap();
        let args = resume_args(&data, &save);
        let mut resumed = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
        assert_eq!(resumed.sim.state_hash(), live.sim.state_hash());
        let local_unit = live
            .sim
            .world()
            .units()
            .find(|(_, u)| u.owner == PlayerId(0))
            .map(|(id, _)| id)
            .unwrap();
        let order = SimCommand::Stop {
            units: UnitSet::single(local_unit),
        };
        live.submit(order.clone());
        resumed.submit(order);
        for _ in 0..30 {
            assert_eq!(resumed.step(), live.step());
        }
        let resave = saves::quick_save(&resumed, &output.0).unwrap();
        assert_ne!(save, resave);
        assert_eq!(Checkpoint::read(&resave).unwrap().next_tick(), Tick(61));
        eprintln!(
            "retail client {} save/load, order, continue, resave: tick61 {:016x}",
            if campaign {
                "explicit campaign placeholder owner sides"
            } else {
                "skirmish"
            },
            live.sim.state_hash()
        );
    }
}

#[test]
fn unlogged_world_mutation_and_synthetic_render_sources_fail_closed() {
    let install = Scratch::new("unlogged");
    install.install("test.hpi", 200);
    let output = Scratch::new("unlogged-output");
    let vfs = open_vfs(&install.0).unwrap();
    let mut live = Session::load_from_args(&vfs, &install.args(false), CommandLog::new()).unwrap();
    live.step();
    let def = live.sim.world().players()[0].monarch_def;
    live.sim
        .world_mut()
        .spawn(
            def,
            PlayerId(0),
            Vec2::new(Fx::from_int(200), Fx::from_int(200)),
            tak_math::Angle::from_raw(0),
        )
        .unwrap();
    assert!(
        saves::quick_save(&live, &output.0)
            .unwrap_err()
            .to_string()
            .contains("unlogged mutations")
    );
    assert_eq!(std::fs::read_dir(&output.0).unwrap().count(), 0);
    let replay = ReplaySession::new(
        CheckpointSource::synthetic(tak_sim::testing::example_scenario(42, 2)),
        None,
    )
    .unwrap();
    let save = output.0.join("synthetic.save.json");
    replay.checkpoint().unwrap().write_atomic(&save).unwrap();
    let error = Session::load_from_args(&vfs, &resume_args(&install.0, &save), CommandLog::new())
        .err()
        .unwrap();
    assert!(error.to_string().contains("no renderable map"), "{error}");
    assert_eq!(
        Checkpoint::read(&save)
            .unwrap()
            .restore(None, ReplayBudget::default())
            .unwrap()
            .sim()
            .tick(),
        Tick(0)
    );
}

#[cfg(unix)]
#[test]
fn save_destination_symlinks_cannot_route_into_the_install_or_repository() {
    use std::os::unix::fs::symlink;
    let install = Scratch::new("destination-install");
    install.install("test.hpi", 200);
    let output = Scratch::new("destination-output");
    let alias = output.0.join("install-alias");
    symlink(&install.0, &alias).unwrap();
    assert!(saves::save_directory(Some(&alias.join("saves")), &install.0).is_err());
    assert!(!install.0.join("saves").exists());
}

#[test]
fn controller_menu_then_west_requests_save_without_submitting_world_orders() {
    use tak_input::{Button, EventKind, InputEvent, Mapper, Output, Profile};
    let install = Scratch::new("controller-route");
    install.install("test.hpi", 200);
    let vfs = open_vfs(&install.0).unwrap();
    let session = Session::load_from_args(&vfs, &install.args(false), CommandLog::new()).unwrap();
    let mut mapper = Mapper::new(Profile::default_combined());
    let output = mapper.handle(
        &InputEvent::new(
            0,
            EventKind::ButtonDown {
                button: Button::Menu,
            },
        ),
        &session.view(),
        session.tick(),
    );
    assert_eq!(output, vec![Output::ToggleOverlay]);
    assert!(saves::quick_save_shortcut(false, true, true));
    assert!(!saves::quick_save_shortcut(false, false, true));
    assert!(saves::quick_save_shortcut(true, false, false));
    assert!(session.log.is_empty());
}

#[cfg(unix)]
#[test]
fn exit_outputs_cannot_alias_the_loaded_checkpoint_by_path_symlink_or_hardlink() {
    let root = Scratch::new("alias-outputs");
    let input = root.0.join("input.save.json");
    std::fs::write(&input, b"preserve even corrupt bytes").unwrap();
    let alias = root.0.join("hardlink.json");
    std::fs::hard_link(&input, &alias).unwrap();
    let symlink = root.0.join("symlink.json");
    std::os::unix::fs::symlink(&input, &symlink).unwrap();
    for path in [&input, &alias, &symlink] {
        let mut args = resume_args(&root.0, &input);
        args.record = Some(path.clone());
        assert!(saves::protect_checkpoint_outputs(&args).is_err());
        args.record = None;
        args.hashes = Some(path.clone());
        assert!(saves::protect_checkpoint_outputs(&args).is_err());
        args.hashes = None;
        args.input_profile = Some(path.clone());
        assert!(saves::protect_checkpoint_outputs(&args).is_err());
    }
    assert_eq!(
        std::fs::read(&input).unwrap(),
        b"preserve even corrupt bytes"
    );
}

#[test]
fn tick_zero_round_trip_does_not_spawn_early_and_loaded_zero_is_a_full_hash_run() {
    for campaign in [false, true] {
        let install = Scratch::new("zero-tick");
        install.install("test.hpi", 200);
        let output = Scratch::new("zero-save");
        let vfs = open_vfs(&install.0).unwrap();
        let mut live =
            Session::load_from_args(&vfs, &install.args(campaign), CommandLog::new()).unwrap();
        let save = saves::quick_save(&live, &output.0).unwrap();
        let mut restored =
            Session::load_from_args(&vfs, &resume_args(&install.0, &save), CommandLog::new())
                .unwrap();
        assert_eq!(live.sim.state_hash(), restored.sim.state_hash());
        assert_eq!(
            live.sim.world().unit_count(),
            restored.sim.world().unit_count()
        );
        live.submit(move_order(150));
        restored.submit(move_order(150));
        for _ in 0..4 {
            assert_eq!(live.step(), restored.step());
        }
        assert_eq!(
            saves::format_hashes(&restored),
            tak_headless::format_hashes(&live.hashes)
        );
    }
}

#[test]
fn content_changes_after_start_refuse_capture_before_creating_save_directory() {
    let install = Scratch::new("content-change");
    install.install("test.hpi", 200);
    let vfs = open_vfs(&install.0).unwrap();
    let mut live = Session::load_from_args(&vfs, &install.args(false), CommandLog::new()).unwrap();
    live.step();
    let state = live.sim.state_hash();
    install.install("test.hpi", 201);
    let output = Scratch::new("changed-save-dir");
    let destination = output.0.join("not-created");
    let error = saves::quick_save(&live, &destination).unwrap_err();
    assert!(error.to_string().contains("content mismatch"), "{error}");
    assert!(!destination.exists());
    assert_eq!(state, live.sim.state_hash());
}

#[test]
fn canonical_source_owns_renderer_metadata_even_if_caller_supplies_another_vfs() {
    let source = Scratch::new("source-a");
    source.install_named("a.hpi", 200, "Canonical render source A");
    let other = Scratch::new("source-b");
    other.install_named("b.hpi", 900, "Unrelated render source B");
    let other_vfs = open_vfs(&other.0).unwrap();
    let mut live =
        Session::load_from_args(&other_vfs, &source.args(false), CommandLog::new()).unwrap();
    assert_eq!(live.defs[0].display, "Canonical render source A");
    live.step();
    let output = Scratch::new("source-owned-save");
    let save = saves::quick_save(&live, &output.0).unwrap();
    let loaded = Session::load_from_args(
        &other_vfs,
        &resume_args(&source.0, &save),
        CommandLog::new(),
    )
    .unwrap();
    assert_eq!(loaded.defs[0].display, "Canonical render source A");
    assert_eq!(live.sim.state_hash(), loaded.sim.state_hash());
}

#[test]
fn executed_noop_history_is_immutable_but_future_fixture_edits_are_allowed() {
    let install = Scratch::new("exact-history");
    install.install("test.hpi", 200);
    let vfs = open_vfs(&install.0).unwrap();
    let mut live = Session::load_from_args(&vfs, &install.args(false), CommandLog::new()).unwrap();
    live.step();
    live.submit(SimCommand::Stop {
        units: UnitSet::single(UnitId::new(0, 0)),
    });
    live.step();
    let hash = live.sim.state_hash();
    let original = live.log.clone();
    assert_eq!(saves::capture(&live).unwrap().command_count(), 1);
    live.log = CommandLog::new();
    let error = saves::capture(&live).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("executed command history was edited"),
        "{error}"
    );
    assert_eq!(hash, live.sim.state_hash());
    live.log = original;
    live.log.record(Tick(12), PlayerId(0), move_order(190));
    assert_eq!(saves::capture(&live).unwrap().command_count(), 1);
    // Even a no-op direct Sim step bypasses Session's actual tick provenance.
    live.sim.step(&[]);
    assert!(
        saves::capture(&live)
            .unwrap_err()
            .to_string()
            .contains("outside the recorded session tick path")
    );
}

#[test]
fn default_profile_checkpoint_alias_probe() {
    if std::env::var_os("TAK_TEST_DEFAULT_PROFILE_ALIAS").is_none() {
        return;
    }
    let input = std::env::current_dir()
        .unwrap()
        .join(tak_client::feel::default_profile_path());
    std::fs::write(&input, b"preserve checkpoint bytes").unwrap();
    let args = resume_args(&std::env::current_dir().unwrap(), &input);
    assert!(args.input_profile.is_none());
    assert!(saves::protect_checkpoint_outputs(&args).is_err());
    assert_eq!(std::fs::read(&input).unwrap(), b"preserve checkpoint bytes");
}

#[test]
fn default_profile_alias_is_protected_in_an_isolated_working_directory() {
    let root = Scratch::new("default-profile-cwd");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "default_profile_checkpoint_alias_probe",
            "--nocapture",
        ])
        .env("TAK_TEST_DEFAULT_PROFILE_ALIAS", "1")
        .current_dir(&root.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn profile_atomic_replacement_does_not_truncate_a_different_checkpoint_hardlink() {
    let root = Scratch::new("profile-hardlink");
    let save = root.0.join("save.json");
    let profile = root.0.join("profile.json");
    std::fs::write(&save, b"original checkpoint bytes").unwrap();
    std::fs::hard_link(&save, &profile).unwrap();
    tak_client::feel::save_profile(&profile, &tak_input::Profile::default_combined()).unwrap();
    assert_eq!(std::fs::read(&save).unwrap(), b"original checkpoint bytes");
    assert!(tak_input::Profile::from_json(&std::fs::read_to_string(profile).unwrap()).is_ok());
}

#[test]
fn authoritative_restored_history_budget_is_checked_before_content_or_replay() {
    let install = Scratch::new("history-budget");
    install.install("test.hpi", 200);
    let output = Scratch::new("history-budget-save");
    let vfs = open_vfs(&install.0).unwrap();
    let mut live = Session::load_from_args(&vfs, &install.args(false), CommandLog::new()).unwrap();
    live.step();
    live.submit(SimCommand::Stop {
        units: UnitSet::single(UnitId::new(0, 0)),
    });
    live.step();
    let save = saves::quick_save(&live, &output.0).unwrap();
    let mut args = resume_args(&install.0, &save);
    args.max_replay_commands = 1;
    let mut restored = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
    restored.log = CommandLog::new();
    assert!(
        saves::capture(&restored)
            .unwrap_err()
            .to_string()
            .contains("executed command history was edited")
    );
    restored.log = live.log.clone();
    restored.submit(SimCommand::Stop {
        units: UnitSet::single(UnitId::new(0, 0)),
    });
    restored.step();
    restored.log = CommandLog::new(); // authoritative prefix + continuation still count as 2
    std::fs::remove_file(install.0.join("test.hpi")).unwrap();
    let error = saves::capture(&restored).unwrap_err();
    assert!(
        error.to_string().contains("command replay budget exceeded"),
        "must fail before install access: {error}"
    );
}

#[test]
fn loaded_screenshots_require_fresh_paths_and_recheck_after_startup() {
    let root = Scratch::new("screenshot-fresh");
    let input = root.0.join("loaded.save.json");
    std::fs::write(&input, b"checkpoint bytes remain untouched").unwrap();
    let output = root.0.join("shot.png");
    let mut args = resume_args(&root.0, &input);
    args.screenshot = Some(output.to_str().unwrap().into());
    saves::protect_checkpoint_outputs(&args).unwrap();
    saves::check_screenshot_destination(true, &output).unwrap();
    // An output may appear during the frame-settle wait. Dispatch must refuse.
    std::fs::hard_link(&input, &output).unwrap();
    let error = saves::check_screenshot_destination(true, &output).unwrap_err();
    assert!(error.to_string().contains("fresh output path"), "{error}");
    assert!(saves::protect_checkpoint_outputs(&args).is_err());
    assert_eq!(
        std::fs::read(&input).unwrap(),
        b"checkpoint bytes remain untouched"
    );
    assert!(
        saves::check_screenshot_destination(false, &output).is_ok(),
        "non-loaded legacy behavior is explicit"
    );
    args.load_checkpoint = None;
    saves::protect_checkpoint_outputs(&args).unwrap();
    let directory = root.0.join("directory.png");
    std::fs::create_dir(&directory).unwrap();
    assert!(saves::check_screenshot_destination(true, &directory).is_err());
    #[cfg(unix)]
    {
        let dangling = root.0.join("dangling.png");
        std::os::unix::fs::symlink(root.0.join("missing"), &dangling).unwrap();
        assert!(saves::check_screenshot_destination(true, &dangling).is_err());
    }
}

#[test]
fn persisted_controller_metadata_keeps_local_human_and_requires_verified_content() {
    let install = Scratch::new("controller-save");
    install.install("test.hpi", 200);
    let output = Scratch::new("controller-save-output");
    let replay = ReplaySession::new_with_controllers(
        CheckpointSource::map(MAP, "TST,TST", 1, true),
        Some(&install.0),
        tak_headless::bot::ControllerConfig {
            human_players: vec![PlayerId(0)],
            controllers: Vec::new(),
        },
    )
    .unwrap();
    let checkpoint = replay.checkpoint().unwrap();
    assert!(!checkpoint.controllers().is_empty());
    let path = output.0.join("controlled.save.json");
    checkpoint.write_atomic(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let supported = Session::load_from_args(
        &open_vfs(&install.0).unwrap(),
        &resume_args(&install.0, &path),
        CommandLog::new(),
    )
    .unwrap();
    assert_eq!(supported.controllers(), checkpoint.controllers());
    // An unrelated caller VFS cannot replace missing canonical content.
    let args = resume_args(&output.0.join("missing-install"), &path);
    let error = Session::load_from_args(&tak_vfs::Vfs::new(), &args, CommandLog::new())
        .err()
        .unwrap();
    assert!(error.to_string().contains("not found"), "{error}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn explicit_training_configuration_refuses_incompatible_modes_and_protects_inputs() {
    for extra in [
        vec!["--load-checkpoint", "save.json"],
        vec!["--campaign", MAP, "--campaign-players", "1:TST"],
        vec!["--demo"],
        vec!["--ghost", "ghost.json"],
        vec!["--record-session", "session.json"],
        vec![
            "--synth-events",
            "events.json",
            "--record-session",
            "session.json",
        ],
    ] {
        assert!(
            Args::try_parse_from(
                [
                    "tak-client",
                    "--data",
                    "unused",
                    "--controller-config",
                    "bots.json"
                ]
                .into_iter()
                .chain(extra)
            )
            .is_err()
        );
    }
    let install = Scratch::new("training-config");
    install.install("test.hpi", 200);
    let vfs = open_vfs(&install.0).unwrap();
    let output = Scratch::new("training-config-output");
    let path = output.0.join("config.json");
    let mut args = install.args(false);
    args.controller_config = Some(path.clone());
    for text in [
        r#"{"human_players":[],"controllers":[]}"#,
        r#"{"human_players":[0],"controllers":[],"unknown":true}"#,
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(Session::load_from_args(&vfs, &args, CommandLog::new()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
    std::fs::write(&path, vec![b' '; 65_537]).unwrap();
    assert!(
        Session::load_from_args(&vfs, &args, CommandLog::new())
            .err()
            .unwrap()
            .to_string()
            .contains("64 KiB")
    );
    let valid = r#"{"human_players":[0],"controllers":[]}"#;
    std::fs::write(&path, valid).unwrap();
    let live = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
    assert_eq!(live.controllers().human_players, vec![PlayerId(0)]);
    args.record = Some(path.clone());
    assert!(saves::protect_checkpoint_outputs(&args).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), valid);
    // Direct Args mutation must be guarded as well as clap parsing.
    args.record = None;
    args.demo = true;
    assert!(Session::load_from_args(&vfs, &args, CommandLog::new()).is_err());
}
