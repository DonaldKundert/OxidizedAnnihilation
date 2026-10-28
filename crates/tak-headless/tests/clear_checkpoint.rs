//! Command-only synthetic Clear traces: target-owned progress, cancellation,
//! feature generations, mana accounting and portable replay. No retail data.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tak_commands::{
    CommandLog, FeatureId, PlayerId, QueueMode, SimCommand, Target, UnitId, UnitSet,
};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_math::{Fx, Vec2};
use tak_sim::combat::{DamageStats, WeaponKind, WeaponStats};
use tak_sim::construction::{BuildDef, reclaim};
use tak_sim::features::{FeatureDef, FeatureSetup, PlacedFeature};
use tak_sim::map::TerrainGrid;
use tak_sim::{Order, Scenario, economy};

const OWNER: PlayerId = PlayerId(0);
const BUILDER: UnitId = UnitId::new(0, 0);
const TREE: FeatureId = FeatureId::new(0, 0);
const SUCCESSOR: FeatureId = FeatureId::new(0, 1);
const YIELD: i32 = 13;
const SUCCESSOR_YIELD: i32 = 17;
const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");

fn scenario(weapon_successor: bool) -> Scenario {
    let mut scenario = tak_sim::testing::example_scenario(0xc1ea_2026, 1);
    let builder = &mut scenario.unit_defs[0];
    builder.build = BuildDef {
        builder: true,
        canreclaim: true,
        mobile: true,
        workertime: 7,
        builddistance: 200,
        ..Default::default()
    };
    builder.combat.max_damage = 100;
    builder.economy.mogriumstorage = 100;
    builder.economy.maxmana = 100;
    scenario.economy.rules.start_mana = 0;
    scenario.economy.rules.monarch_start_mana_percent = 37;
    if weapon_successor {
        builder.combat.weapons.push(WeaponStats {
            kind: WeaponKind::RemoteEffect,
            range: 500,
            reload_ticks: 3,
            aim_tolerance: Some(u16::MAX),
            damage: DamageStats {
                default: 10,
                ..Default::default()
            },
            ..Default::default()
        });
    }
    scenario.map.terrain = Some(TerrainGrid {
        cols: 128,
        rows: 128,
        cell_size: 16,
        flags: vec![0; 128 * 128],
    });
    *scenario.features = FeatureSetup {
        defs: vec![
            FeatureDef {
                name: "synthetic_clear_tree".into(),
                footprint_x: 1,
                footprint_z: 1,
                blocking: true,
                reclaimable: true,
                energy: YIELD,
                damage: 10,
                feature_dead: weapon_successor.then_some(1),
                ..Default::default()
            },
            FeatureDef {
                name: "synthetic_clear_stump".into(),
                footprint_x: 1,
                footprint_z: 1,
                blocking: true,
                reclaimable: true,
                energy: SUCCESSOR_YIELD,
                damage: 23,
                ..Default::default()
            },
        ],
        placed: vec![PlacedFeature {
            def: 0,
            x: 20,
            z: 16,
        }],
        ..Default::default()
    };
    Scenario::from_json(&scenario.to_json_pretty()).unwrap()
}

fn clear(target: FeatureId) -> SimCommand {
    SimCommand::Clear {
        units: UnitSet::single(BUILDER),
        target,
        queue: QueueMode::Replace,
    }
}

struct Trace {
    scenario: Scenario,
    session: ReplaySession,
    commands: CommandLog,
    hashes: Vec<u64>,
    boundaries: Vec<u32>,
}

impl Trace {
    fn new(weapon_successor: bool) -> Self {
        Self::from_scenario(scenario(weapon_successor))
    }

    fn from_scenario(scenario: Scenario) -> Self {
        Self {
            session: ReplaySession::new(CheckpointSource::synthetic(scenario.clone()), None)
                .unwrap(),
            scenario,
            commands: CommandLog::new(),
            hashes: vec![],
            boundaries: vec![0],
        }
    }

    fn command(&mut self, command: SimCommand) {
        self.commands
            .record(self.session.sim().tick(), OWNER, command);
    }

    fn advance(&mut self, ticks: u32) {
        assert!(self.session.sim().tick().0 + ticks < 2000, "Clear stalled");
        self.hashes
            .extend(self.session.advance(&self.commands, ticks).unwrap());
    }

    fn boundary(&mut self) {
        self.boundaries.push(self.session.sim().tick().0);
    }

    fn mana(&self) -> i64 {
        economy::mana(self.session.sim().world(), OWNER).0
    }

    fn claimed(&self, target: FeatureId) -> i64 {
        self.session
            .sim()
            .world()
            .features
            .get(target)
            .unwrap()
            .reclaim_claimed
    }

    fn partial(&self, target: FeatureId) -> i64 {
        let features = &self.session.sim().world().features;
        let feature = features
            .get(target)
            .expect("feature is still being cleared");
        assert!(feature.reclaim_done > 0);
        assert!(feature.reclaim_done < reclaim::total_work(features.def(feature).unwrap()));
        feature.reclaim_done
    }

    fn finish(&mut self, target: FeatureId) {
        while self.session.sim().world().features.get(target).is_some() {
            let before = self.mana();
            self.advance(1);
            assert!(self.mana() >= before, "Clear never spends player mana");
        }
        self.boundaries.push(self.session.sim().tick().0 - 1);
        self.boundary();
    }

    fn finish_trace(&mut self) {
        self.advance(8);
        self.boundary();
        self.boundaries.sort_unstable();
        self.boundaries.dedup();
        assert!(self.session.sim().world().features.is_empty());
        for feature in &self.scenario.features.placed {
            assert!(!tak_sim::pathfinding::is_obstacle(
                self.session.sim().world(),
                feature.x as i32,
                feature.z as i32
            ));
        }
        assert_eq!(
            economy::unit_mana(self.session.sim().world(), BUILDER)
                .unwrap()
                .mana,
            Fx::from_int(37),
            "Clear credits the player's pool, never the worker's spell mana"
        );
    }
}

fn cancellation_trace() -> Trace {
    let mut trace = Trace::new(false);
    trace.advance(1);
    trace.command(clear(TREE));
    trace.advance(5);
    let first_progress = trace.partial(TREE);
    let first_mana = trace.mana();
    assert!(first_mana > 0 && first_mana < (i64::from(YIELD) << 16));
    assert_ne!(first_mana % (1 << 16), 0, "fixture needs fractional mana");
    assert_eq!(
        trace
            .session
            .sim()
            .world()
            .features
            .get(TREE)
            .unwrap()
            .health,
        10
    );
    trace.boundary();
    // Stop preserves the target's paid work, including fractional mana credit.
    trace.command(SimCommand::Stop {
        units: UnitSet::single(BUILDER),
    });
    trace.advance(1);
    trace.boundary();
    trace.advance(7);
    assert_eq!(trace.partial(TREE), first_progress);
    assert_eq!(trace.mana(), first_mana);
    assert!(
        trace
            .session
            .sim()
            .world()
            .unit(BUILDER)
            .unwrap()
            .build
            .clear
            .is_none()
    );
    trace.boundary();
    trace.command(clear(TREE));
    trace.advance(5);
    assert!(trace.partial(TREE) > first_progress);
    let second_progress = trace.partial(TREE);
    let second_mana = trace.mana();
    trace.boundary();
    // A replacing movement order must tear down work just like Stop.
    trace.command(SimCommand::Move {
        units: UnitSet::single(BUILDER),
        to: Vec2::from_ints(256, 256),
        queue: QueueMode::Replace,
    });
    trace.advance(1);
    trace.boundary();
    trace.advance(7);
    assert_eq!(trace.partial(TREE), second_progress);
    assert_eq!(trace.mana(), second_mana);
    trace.boundary();
    trace.command(clear(TREE));
    trace.advance(1);
    trace.boundary();
    trace.finish(TREE);
    assert_eq!(trace.mana(), i64::from(YIELD) << 16);
    trace.command(clear(TREE)); // Repeating a completed lifetime earns nothing.
    trace.finish_trace();
    assert_eq!(trace.mana(), i64::from(YIELD) << 16);
    trace
}

fn generation_trace() -> Trace {
    let mut trace = Trace::new(true);
    trace.advance(1);
    trace.command(clear(TREE));
    trace.advance(5);
    trace.partial(TREE);
    let partial_mana = trace.mana();
    assert!(partial_mana > 0);
    assert_eq!(trace.claimed(TREE), partial_mana);
    trace.boundary();
    // Weapon replacement invalidates the old Clear lifetime. Its new occupant
    // starts at zero work but retains the lineage's already-claimed bounty.
    trace.command(SimCommand::Attack {
        units: UnitSet::single(BUILDER),
        target: Target::Ground(Vec2::from_ints(328, 264)),
        queue: QueueMode::Replace,
    });
    while trace.session.sim().world().features.get(TREE).is_some() {
        trace.advance(1);
    }
    let successor = trace.session.sim().world().features.get(SUCCESSOR).unwrap();
    assert_eq!(successor.def, 1);
    assert_eq!(successor.reclaim_done, 0);
    assert_eq!(successor.reclaim_claimed, partial_mana);
    assert_eq!(trace.mana(), partial_mana, "weapon damage gives no bounty");
    trace.boundary();
    trace.command(SimCommand::Stop {
        units: UnitSet::single(BUILDER),
    });
    trace.advance(1);
    trace.command(clear(SUCCESSOR));
    trace.command(clear(TREE)); // Invalid Replace must not erase the valid order.
    trace.advance(1);
    trace.partial(SUCCESSOR);
    assert!(trace.claimed(SUCCESSOR) > partial_mana);
    assert_eq!(trace.claimed(SUCCESSOR), trace.mana());
    assert_eq!(
        trace
            .session
            .sim()
            .world()
            .unit(BUILDER)
            .unwrap()
            .build
            .clear
            .unwrap()
            .target,
        SUCCESSOR
    );
    trace.boundary();
    trace.finish(SUCCESSOR);
    assert_eq!(trace.mana(), i64::from(SUCCESSOR_YIELD) << 16);
    trace.command(clear(TREE));
    trace.command(clear(SUCCESSOR));
    trace.finish_trace();
    assert_eq!(trace.mana(), i64::from(SUCCESSOR_YIELD) << 16);
    trace
}

fn cyclic_replacement_trace() -> Trace {
    let mut scenario = scenario(true);
    scenario.features.defs[0].feature_dead = Some(0);
    let mut trace = Trace::from_scenario(scenario);
    trace.advance(1);
    assert_eq!(trace.claimed(TREE), 0);
    let bounty = i64::from(YIELD) << 16;
    let mut target = TREE;
    let mut capped_rounds = 0;
    loop {
        let before = trace.mana();
        trace.command(clear(target));
        trace.advance(5);
        trace.partial(target);
        let claimed = trace.claimed(target);
        assert_eq!(claimed, trace.mana());
        assert!((before..=bounty).contains(&claimed));
        if before == bounty {
            assert_eq!(trace.mana(), before, "a new generation is not a new bounty");
        }
        let capture = target.generation == 0 || target.generation == 5 || claimed == bounty;
        if capture {
            trace.boundary();
        }
        trace.command(SimCommand::Attack {
            units: UnitSet::single(BUILDER),
            target: Target::Ground(Vec2::from_ints(328, 264)),
            queue: QueueMode::Replace,
        });
        while trace.session.sim().world().features.get(target).is_some() {
            trace.advance(1);
        }
        target = FeatureId::new(target.index, target.generation + 1);
        let successor = trace.session.sim().world().features.get(target).unwrap();
        assert_eq!(successor.reclaim_done, 0);
        assert_eq!(successor.reclaim_claimed, claimed);
        assert_eq!(trace.mana(), claimed);
        if capture {
            trace.boundary();
        }
        if claimed == bounty {
            capped_rounds += 1;
            if capped_rounds == 4 {
                break;
            }
        }
    }
    // Even after several further weapon replacements, exhausted resources
    // still clear normally; neither restore nor completion creates more mana.
    trace.command(clear(target));
    trace.advance(1);
    trace.partial(target);
    assert_eq!(trace.claimed(target), bounty);
    trace.boundary();
    trace.finish(target);
    trace.finish_trace();
    assert_eq!(trace.mana(), bounty);
    trace
}

fn assert_walking_with_equal_coordinate_tail(trace: &Trace, goal: Vec2) {
    let unit = trace.session.sim().world().unit(BUILDER).unwrap();
    let job = unit.build.clear.unwrap();
    assert!(job.walking && !job.building && !job.cancelled);
    assert_eq!(job.walk_to, Some(goal));
    assert_eq!(job.target, TREE);
    assert_eq!(
        unit.orders.iter().copied().collect::<Vec<_>>(),
        [
            Order::Move { to: goal }, // Owned by the active Clear interval.
            Order::Clear { target: TREE },
            Order::Move { to: goal }, // Equal coordinates, but owned by the player.
        ]
    );
    assert_eq!(
        trace
            .session
            .sim()
            .world()
            .features
            .get(TREE)
            .unwrap()
            .reclaim_done,
        0
    );
    assert_eq!(trace.mana(), 0);
}

fn walking_trace() -> Trace {
    let mut scenario = scenario(false);
    scenario.features.placed[0].x = 40;
    scenario.unit_defs[0].build.builddistance = 64;
    let goal = Vec2::from_ints(648, 264);
    let movement = |queue| SimCommand::Move {
        units: UnitSet::single(BUILDER),
        to: goal,
        queue,
    };
    let mut trace = Trace::from_scenario(scenario);
    trace.advance(1);
    trace.command(clear(TREE));
    trace.command(movement(QueueMode::Append));
    trace.advance(1);
    assert_walking_with_equal_coordinate_tail(&trace, goal);
    trace.boundary();
    trace.advance(5);
    assert_walking_with_equal_coordinate_tail(&trace, goal);
    trace.boundary();

    // Replacing a work-walk with a player Move to exactly the same coordinates
    // must invalidate the ownership token, not consume that new player order.
    trace.command(movement(QueueMode::Replace));
    trace.advance(1);
    let unit = trace.session.sim().world().unit(BUILDER).unwrap();
    assert!(unit.build.clear.is_none());
    assert_eq!(
        unit.orders.iter().copied().collect::<Vec<_>>(),
        [Order::Move { to: goal }]
    );
    trace.boundary();
    trace.advance(3);
    assert_eq!(trace.mana(), 0);
    assert_eq!(
        trace
            .session
            .sim()
            .world()
            .features
            .get(TREE)
            .unwrap()
            .reclaim_done,
        0
    );
    trace.command(clear(TREE));
    trace.command(movement(QueueMode::Append));
    trace.advance(1);
    assert_walking_with_equal_coordinate_tail(&trace, goal);
    trace.boundary();

    while trace
        .session
        .sim()
        .world()
        .features
        .get(TREE)
        .unwrap()
        .reclaim_done
        == 0
    {
        trace.advance(1);
    }
    trace.partial(TREE);
    let unit = trace.session.sim().world().unit(BUILDER).unwrap();
    let job = unit.build.clear.unwrap();
    assert!(job.building && !job.walking);
    assert_eq!(job.walk_to, None);
    assert_eq!(
        unit.orders.iter().copied().collect::<Vec<_>>(),
        [Order::Clear { target: TREE }, Order::Move { to: goal }]
    );
    trace.boundary();
    trace.finish(TREE);
    let unit = trace.session.sim().world().unit(BUILDER).unwrap();
    assert!(unit.build.clear.is_none());
    assert_eq!(
        unit.orders.iter().copied().collect::<Vec<_>>(),
        [Order::Move { to: goal }]
    );
    assert_eq!(trace.mana(), i64::from(YIELD) << 16);
    while !trace
        .session
        .sim()
        .world()
        .unit(BUILDER)
        .unwrap()
        .orders
        .is_empty()
    {
        trace.advance(1);
    }
    assert!(
        trace
            .session
            .sim()
            .world()
            .unit(BUILDER)
            .unwrap()
            .pos
            .distance(goal)
            < Fx::from_int(16)
    );
    trace.boundary();
    trace.finish_trace();
    assert_eq!(trace.mana(), i64::from(YIELD) << 16);
    trace
}

fn verify_checkpoints(trace: Trace) {
    let commands = CommandLog::from_json(&trace.commands.to_json()).unwrap();
    let source = CheckpointSource::synthetic(trace.scenario);
    let ticks = trace.session.sim().tick().0;
    let final_json = trace
        .session
        .checkpoint()
        .unwrap()
        .to_json_pretty()
        .unwrap();
    for split in trace.boundaries {
        let mut prefix = ReplaySession::new(source.clone(), None).unwrap();
        assert_eq!(
            prefix.advance(&commands, split).unwrap(),
            trace.hashes[..split as usize]
        );
        let encoded = prefix.checkpoint().unwrap().to_json_pretty().unwrap();
        let mut restored = Checkpoint::from_json(&encoded)
            .unwrap()
            .restore(None, ReplayBudget::default())
            .unwrap();
        assert_eq!(restored.sim().world(), prefix.sim().world());
        assert_eq!(
            restored.advance(&commands, ticks - split).unwrap(),
            trace.hashes[split as usize..],
            "continuation at next tick {split}"
        );
        assert_eq!(
            restored.checkpoint().unwrap().to_json_pretty().unwrap(),
            final_json
        );
    }
}

#[test]
fn partial_cancelled_resumed_and_completed_clear_checkpoints_match_every_hash() {
    verify_checkpoints(cancellation_trace());
}

#[test]
fn stale_clear_cannot_reclaim_or_reset_a_replacement_generation_after_restore() {
    verify_checkpoints(generation_trace());
}

#[test]
fn cyclic_weapon_replacement_checkpoints_conserve_the_lineage_bounty() {
    verify_checkpoints(cyclic_replacement_trace());
}

#[test]
fn walking_clear_checkpoints_preserve_work_move_ownership_and_player_tail() {
    verify_checkpoints(walking_trace());
}

// File-backed output avoids pipe deadlocks; every fresh process is bounded.
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("clear-checkpoint-{}", std::process::id()));
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
fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn hashes(path: &Path) -> Vec<u64> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| u64::from_str_radix(line.split_once(' ').unwrap().1, 16).unwrap())
        .collect()
}

#[test]
fn raw_clear_logs_and_checkpoint_suffixes_match_in_separate_processes() {
    let dir = Scratch::new();
    for trace in [
        cancellation_trace(),
        generation_trace(),
        cyclic_replacement_trace(),
        walking_trace(),
    ] {
        let scenario = dir.path("scenario.json");
        let commands = dir.path("commands.json");
        std::fs::write(&scenario, trace.scenario.to_json_pretty()).unwrap();
        std::fs::write(&commands, trace.commands.to_json_pretty()).unwrap();
        let ticks = trace.session.sim().tick().0;
        let a = dir.path("a.hashes");
        let b = dir.path("b.hashes");
        for output in [&a, &b] {
            dir.run(&[
                "run",
                "--scenario",
                s(&scenario),
                "--log",
                s(&commands),
                "--ticks",
                &ticks.to_string(),
                "--hashes",
                s(output),
            ]);
        }
        assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
        assert_eq!(hashes(&a), trace.hashes);
        for split in trace.boundaries {
            let save = dir.path("split.json");
            let resumed = dir.path("resumed.json");
            let suffix = dir.path("suffix.hashes");
            dir.run(&[
                "checkpoint",
                "save",
                "--scenario",
                s(&scenario),
                "--log",
                s(&commands),
                "--ticks",
                &split.to_string(),
                "--out",
                s(&save),
            ]);
            dir.run(&[
                "checkpoint",
                "resume",
                "--checkpoint",
                s(&save),
                "--log",
                s(&commands),
                "--ticks",
                &(ticks - split).to_string(),
                "--out",
                s(&resumed),
                "--hashes",
                s(&suffix),
            ]);
            assert_eq!(hashes(&suffix), trace.hashes[split as usize..]);
            assert_eq!(
                Checkpoint::read(&resumed)
                    .unwrap()
                    .to_json_pretty()
                    .unwrap(),
                trace
                    .session
                    .checkpoint()
                    .unwrap()
                    .to_json_pretty()
                    .unwrap()
            );
        }
    }
}
