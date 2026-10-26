//! Spatial Append builds use the ordinary paid construction path, including
//! queue preservation, terrain rejection, serialized replay and checkpoints.
#[path = "../../tak-sim/tests/construction_support/mod.rs"]
mod construction_support;

use construction_support::{KING, WALL, at};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tak_commands::{CommandLog, PlayerId, QueueMode, SimCommand, Tick, UnitId, UnitSet};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_math::Fx;
use tak_sim::construction::placement::{self, SiteError};
use tak_sim::map::{TerrainGrid, terrain};
use tak_sim::{Order, Scenario, Sim, WorldPos};

const OWNER: PlayerId = PlayerId(0);
const BUILDER: UnitId = UnitId::new(0, 0);
const TICKS: u32 = 200;
const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");

fn scenario() -> Scenario {
    let mut scenario = construction_support::scenario();
    scenario.seed = 0x51f7_d2a6;
    scenario.players.truncate(1);
    scenario.map.start_positions.truncate(1);
    scenario.economy.rules.start_mana = 1;
    let builder = &mut scenario.unit_defs[KING.0 as usize];
    builder.maxvelocity = Fx::from_int(4);
    builder.acceleration = Fx::ONE;
    builder.economy.mogriumincome = 30;
    let wall = &mut scenario.unit_defs[WALL.0 as usize];
    // Nine ticks with full funding, but only one mana arrives per tick.
    wall.build.buildtime = 3 << 16;
    wall.build.buildcost = 30;
    let mut terrain = TerrainGrid {
        cols: 64,
        rows: 64,
        cell_size: 16,
        flags: vec![0; 64 * 64],
    };
    terrain.flags[16 * 64 + 19] = terrain::STEEP;
    scenario.map.terrain = Some(terrain);
    Scenario::from_json(&scenario.to_json_pretty()).unwrap()
}

fn append(log: &mut CommandLog, tick: u32, site: WorldPos) {
    log.record(
        Tick(tick),
        OWNER,
        SimCommand::Build {
            units: UnitSet::single(BUILDER),
            def: WALL,
            at: Some(site),
            queue: QueueMode::Append,
        },
    );
}

fn restore(session: &ReplaySession) -> ReplaySession {
    let encoded = session.checkpoint().unwrap().to_json_pretty().unwrap();
    let restored = Checkpoint::from_json(&encoded)
        .unwrap()
        .restore(None, ReplayBudget::default())
        .unwrap();
    assert_eq!(restored.sim().world(), session.sim().world());
    assert_eq!(restored.executed_log(), session.executed_log());
    assert_eq!(restored.sim().state_hash(), session.sim().state_hash());
    restored
}

// There is one test/process fixture in this file. Capturing to files avoids
// unread-pipe deadlocks while each child has a real wall-clock timeout.
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("queued-placement-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn run(&self, args: &[&str]) {
        let stdout = self.path("stdout.txt");
        let stderr = self.path("stderr.txt");
        let mut child = Command::new(BIN)
            .args(args)
            .env_remove("TAK_DATA_DIR")
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let result = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if start.elapsed() < Duration::from_secs(30) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                state => {
                    let killed = child.kill();
                    let waited = child.wait();
                    break Err(format!(
                        "child timeout/poll failure: {state:?}; kill: {killed:?}; wait: {waited:?}"
                    ));
                }
            }
        };
        assert!(
            result.as_ref().is_ok_and(|status| status.success()),
            "{args:?}\n{result:?}\nstdout: {}\nstderr: {}",
            std::fs::read_to_string(stdout).unwrap(),
            std::fs::read_to_string(stderr).unwrap()
        );
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn hashes(path: &Path, first_tick: u32) -> Vec<u64> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
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
fn queued_spatial_builds_preserve_paid_work_and_replay_across_checkpoint_boundaries() {
    let scenario = scenario();
    let sites = [at(256, 208), at(320, 208), at(384, 208), at(448, 208)];
    let blocked = at(320, 272);
    let mut commands = CommandLog::new();
    // Existing paid work and an existing queued site precede the drag batch.
    for &site in &sites[..2] {
        append(&mut commands, 1, site);
    }
    for site in [blocked, sites[2], sites[3]] {
        append(&mut commands, 5, site);
    }
    let decoded = CommandLog::from_json(&commands.to_json_pretty()).unwrap();
    assert_eq!(decoded, commands, "site order and Append must round-trip");
    let source = CheckpointSource::synthetic(scenario.clone());
    let mut session = ReplaySession::new(source.clone(), None).unwrap();
    let mut expected_hashes = Vec::new();
    let mut cuts = BTreeSet::from([0, 1, 2, 5, 6, TICKS]);
    let mut completed = BTreeSet::new();
    let mut before_append = None;
    let mut shortage = false;
    let mut rejected = false;
    for tick in 0..TICKS {
        expected_hashes.extend(session.advance(&decoded, 1).unwrap());
        let world = session.sim().world();
        let builder = world.unit(BUILDER).unwrap();
        if tick == 0 {
            for &site in &sites {
                assert_eq!(
                    placement::site_center(&scenario.unit_defs[WALL.0 as usize], site),
                    site
                );
                assert_eq!(placement::check_site(world, WALL, site, None), Ok(()));
            }
            assert_eq!(
                placement::check_site(world, WALL, blocked, None),
                Err(SiteError::Terrain)
            );
        }
        if tick == 4 {
            let target = builder.build.job.unwrap().target.unwrap();
            let frame = world.unit(target).unwrap().build.frame.unwrap();
            assert!(frame.paid > 0 && frame.done > 0);
            before_append = Some((target, frame));
        }
        if tick == 5 {
            let (target, frame) = before_append.unwrap();
            assert_eq!(builder.build.job.unwrap().target, Some(target));
            let after = world.unit(target).unwrap().build.frame.unwrap();
            assert!(after.done > frame.done && after.paid >= frame.paid);
            assert_eq!(
                builder.orders.iter().copied().collect::<Vec<_>>(),
                [sites[0], sites[1], blocked, sites[2], sites[3]].map(|site| Order::Build {
                    def: WALL,
                    at: Some(site)
                })
            );
        }
        let mana = tak_sim::economy::player(world, OWNER).unwrap();
        if !shortage && mana.last_demand > mana.last_granted {
            shortage = true;
            cuts.extend([tick, tick + 1]);
        }
        if tick == 12 {
            let (target, _) = before_append.unwrap();
            let frame = world.unit(target).unwrap().build.frame.unwrap();
            assert!(
                frame.paid > 0 && !frame.is_complete(),
                "shortage must delay completion beyond nine ticks"
            );
        }
        for (id, unit) in world.units().filter(|(_, unit)| unit.def == WALL) {
            assert!(
                sites.contains(&unit.pos),
                "blocked terrain must never spawn a frame"
            );
            if unit.build.is_complete() && completed.insert(id) {
                cuts.extend([tick, tick + 1]);
            }
        }
        if !rejected && !world.construction.rejections.is_empty() {
            rejected = true;
            cuts.extend([tick, tick + 1]);
        }
    }
    assert!(
        shortage && rejected,
        "exercise both normal admission outcomes"
    );
    assert_eq!(completed.len(), sites.len());
    let world = session.sim().world();
    let builder = world.unit(BUILDER).unwrap();
    assert!(builder.orders.is_empty() && builder.build.job.is_none());
    assert!(
        builder.pos.x > Fx::from_int(200),
        "later sites must require the mobile builder to walk"
    );
    for site in sites {
        assert_eq!(
            world
                .units()
                .filter(|(_, unit)| unit.def == WALL
                    && unit.pos == site
                    && unit.build.is_complete())
                .count(),
            1
        );
    }
    let refusals = &world.construction.rejections;
    assert_eq!(refusals.len(), 1);
    assert_eq!(
        (
            refusals[0].builder,
            refusals[0].player,
            refusals[0].def,
            refusals[0].site,
            refusals[0].reason
        ),
        (BUILDER, OWNER, WALL, blocked, SiteError::Terrain)
    );

    // Independent Sim stepping does not use ReplaySession's advance path.
    let mut raw = Sim::new(&scenario).unwrap();
    let raw_hashes: Vec<_> = (0..TICKS)
        .map(|_| raw.step(decoded.for_tick(raw.tick())))
        .collect();
    assert_eq!(raw_hashes, expected_hashes);
    assert_eq!(raw.world(), world);
    let final_save = session.checkpoint().unwrap().to_json_pretty().unwrap();
    for &split in &cuts {
        let mut prefix = ReplaySession::new(source.clone(), None).unwrap();
        assert_eq!(
            prefix.advance(&decoded, split).unwrap(),
            expected_hashes[..split as usize]
        );
        let mut resumed = restore(&prefix);
        assert_eq!(
            resumed.advance(&decoded, TICKS - split).unwrap(),
            expected_hashes[split as usize..],
            "continuation diverged at split {split}"
        );
        assert_eq!(
            resumed.checkpoint().unwrap().to_json_pretty().unwrap(),
            final_save
        );
    }
    let mut chained = ReplaySession::new(source, None).unwrap();
    let mut previous = 0;
    for &split in &cuts {
        assert_eq!(
            chained.advance(&decoded, split - previous).unwrap(),
            expected_hashes[previous as usize..split as usize]
        );
        chained = restore(&chained);
        previous = split;
    }
    assert_eq!(
        chained.checkpoint().unwrap().to_json_pretty().unwrap(),
        final_save
    );

    let scratch = Scratch::new();
    let scenario_file = scratch.path("scenario.json");
    let log_file = scratch.path("commands.json");
    std::fs::write(&scenario_file, scenario.to_json_pretty()).unwrap();
    std::fs::write(&log_file, decoded.to_json_pretty()).unwrap();
    let a = scratch.path("a.hashes");
    let b = scratch.path("b.hashes");
    for output in [&a, &b] {
        scratch.run(&[
            "run",
            "--scenario",
            path(&scenario_file),
            "--log",
            path(&log_file),
            "--ticks",
            &TICKS.to_string(),
            "--hashes",
            path(output),
        ]);
        assert_eq!(hashes(output, 0), expected_hashes);
    }
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
    let prefix = scratch.path("prefix.json");
    let suffix = scratch.path("suffix.json");
    let output = scratch.path("suffix.hashes");
    scratch.run(&[
        "checkpoint",
        "save",
        "--scenario",
        path(&scenario_file),
        "--log",
        path(&log_file),
        "--ticks",
        "6",
        "--out",
        path(&prefix),
    ]);
    scratch.run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        path(&prefix),
        "--log",
        path(&log_file),
        "--ticks",
        &(TICKS - 6).to_string(),
        "--out",
        path(&suffix),
        "--hashes",
        path(&output),
    ]);
    assert_eq!(hashes(&output, 6), expected_hashes[6..]);
    assert_eq!(
        Checkpoint::read(&suffix).unwrap().to_json_pretty().unwrap(),
        final_save
    );
}
