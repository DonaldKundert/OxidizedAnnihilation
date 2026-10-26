//! Bounded synthetic production/replay stress, not an owned-install gameplay
//! claim. All placements and economy values belong to the serialized Scenario;
//! after tick zero only ordinary recorded commands can change the world. In
//! particular a deliberately free synthetic income building makes recovery
//! from an empty mana pool possible without editing running economy state.
#[path = "../../tak-sim/tests/factory_controls_support/mod.rs"]
mod factory_controls_support;

use factory_controls_support::{KEEP, KING, MASON, SWORD, at};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tak_commands::{
    CommandLog, FactoryRallyOrder, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId,
    UnitSet,
};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_math::{Angle, Rng};
use tak_sim::campaign::InitialUnit;
use tak_sim::{Order, Scenario, economy};

const OWNER: PlayerId = PlayerId(0);
const FACTORIES: [UnitId; 3] = [UnitId::new(0, 0), UnitId::new(1, 0), UnitId::new(2, 0)];
const BUILDER: UnitId = UnitId::new(3, 0);
const INCOME: UnitDefId = UnitDefId(5);
const MAX_TICKS: u32 = 700;
const STRESS_TICKS: u32 = 216;
const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");
static NEXT: AtomicU64 = AtomicU64::new(0);

fn scenario(seed: u64) -> Scenario {
    let mut sc = factory_controls_support::scenario();
    sc.seed = seed;
    sc.economy.rules.start_mana = 12;
    sc.unit_defs[KING.0 as usize].economy.mogriumincome = 0;
    sc.unit_defs[KING.0 as usize].build.canbuild = vec![INCOME];
    let mut income = sc.unit_defs[1].clone();
    income.name = "SYNTHETIC_RECOVERY_INCOME".into();
    income.build.buildcost = 0;
    income.build.buildtime = 10 << 16;
    income.economy.mogriumincome = 300;
    sc.unit_defs.push(income);
    // Arena IDs, including all three friendly factories, are reconstructible
    // initial conditions rather than out-of-band world_mut().spawn calls.
    sc.initial_units = Some(
        [
            (KEEP, OWNER, at(192, 192)),
            (KEEP, OWNER, at(480, 192)),
            (KEEP, OWNER, at(768, 192)),
            (KING, OWNER, at(192, 480)),
            (KEEP, PlayerId(1), at(800, 800)),
        ]
        .into_iter()
        .map(|(def, owner, pos)| InitialUnit {
            def,
            owner,
            pos,
            heading: Angle::ZERO,
            health_percentage: 100,
            mana_percentage: 0,
            kills: 0,
        })
        .collect(),
    );
    sc.validate().unwrap();
    sc
}

fn units(ids: &[UnitId]) -> UnitSet {
    let mut units = UnitSet::new();
    for &id in ids {
        units.insert(id);
    }
    units
}

fn build(ids: &[UnitId], def: UnitDefId) -> SimCommand {
    SimCommand::Build {
        units: units(ids),
        def,
        at: None,
        queue: QueueMode::Append,
    }
}

fn repeat(ids: &[UnitId], def: Option<UnitDefId>) -> SimCommand {
    SimCommand::SetFactoryRepeat {
        units: units(ids),
        def,
    }
}

fn cancel(ids: &[UnitId], def: UnitDefId, count: u32) -> SimCommand {
    SimCommand::CancelProduction {
        units: units(ids),
        def,
        count,
    }
}

fn rally(ids: &[UnitId], order: Option<FactoryRallyOrder>, queue: QueueMode) -> SimCommand {
    SimCommand::SetFactoryRally {
        units: units(ids),
        order,
        queue,
    }
}

struct Trace {
    source: CheckpointSource,
    session: ReplaySession,
    commands: CommandLog,
    hashes: Vec<u64>,
    boundaries: BTreeSet<u32>,
}

impl Trace {
    fn new(seed: u64) -> Self {
        let source = CheckpointSource::synthetic(scenario(seed));
        Self {
            session: ReplaySession::new(source.clone(), None).unwrap(),
            source,
            commands: CommandLog::new(),
            hashes: vec![],
            boundaries: BTreeSet::from([0]),
        }
    }

    fn command(&mut self, command: SimCommand) {
        self.commands
            .record(self.session.sim().tick(), OWNER, command);
    }

    fn step(&mut self) {
        assert!(self.tick() < MAX_TICKS, "bounded production trace stalled");
        self.hashes
            .extend(self.session.advance(&self.commands, 1).unwrap());
    }

    fn tick(&self) -> u32 {
        self.session.sim().tick().0
    }

    fn boundary(&mut self) {
        self.boundaries.insert(self.tick());
    }

    fn product(&self, factory: UnitId) -> Option<UnitId> {
        self.session
            .sim()
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product
    }

    fn queue(&self, factory: UnitId) -> Vec<UnitDefId> {
        self.session
            .sim()
            .world()
            .unit(factory)
            .unwrap()
            .orders
            .iter()
            .filter_map(|order| match order {
                Order::Build { def, at: None } => Some(*def),
                _ => None,
            })
            .collect()
    }
}

fn trace(seed: u64) -> Trace {
    let mut trace = Trace::new(seed);
    trace.step();
    for (i, &factory) in FACTORIES.iter().enumerate() {
        trace.command(rally(
            &[factory],
            Some(FactoryRallyOrder::Move {
                to: at(192 + i as i32 * 288, 600),
            }),
            QueueMode::Replace,
        ));
        trace.command(repeat(&[factory], Some(SWORD)));
    }
    for def in [SWORD, MASON, SWORD] {
        trace.command(build(&FACTORIES[..1], def));
    }
    for def in [MASON, SWORD, MASON] {
        trace.command(build(&FACTORIES[1..2], def));
    }
    trace.boundary();
    for _ in 0..24 {
        trace.step();
    }
    trace.boundary();
    let products = FACTORIES.map(|f| trace.product(f).unwrap());
    let frames = products.map(|id| {
        trace
            .session
            .sim()
            .world()
            .unit(id)
            .unwrap()
            .build
            .frame
            .unwrap()
    });
    assert!(frames.iter().all(|frame| frame.paid > 0 && frame.done > 0));
    for ((factory, product), (def, repeated)) in
        FACTORIES
            .into_iter()
            .zip(products)
            .zip([(SWORD, false), (MASON, false), (SWORD, true)])
    {
        let world = trace.session.sim().world();
        assert_eq!(world.unit(product).unwrap().def, def);
        assert_eq!(
            world
                .unit(factory)
                .unwrap()
                .build
                .factory
                .product_from_repeat,
            repeated
        );
    }
    // Fixed-point sharing can leave a few raw sub-mana units that cannot
    // buy another work step; the unchanged frames below prove starvation.
    assert!(economy::mana(trace.session.sim().world(), OWNER).0 < 256);
    assert_eq!(
        economy::income_per_second(trace.session.sim().world(), OWNER).0,
        0
    );
    for _ in 0..12 {
        trace.step();
        for (id, frame) in products.into_iter().zip(frames) {
            assert_eq!(
                trace.session.sim().world().unit(id).unwrap().build.frame,
                Some(frame),
                "paid factory work must really stall during the synthetic shortage"
            );
        }
    }
    trace.boundary();

    // Pending-first finite cancellation and repeat edits while all paid frames
    // are frozen. Recipe changes must not silently cancel or replace a frame.
    trace.command(cancel(&FACTORIES[..1], SWORD, 1));
    trace.command(cancel(&FACTORIES[1..2], MASON, 0));
    trace.command(repeat(&FACTORIES[..1], Some(MASON)));
    for recipe in [Some(MASON), None, Some(MASON)] {
        trace.command(repeat(&FACTORIES[2..], recipe));
        trace.step();
        assert_eq!(FACTORIES.map(|f| trace.product(f).unwrap()), products);
        assert_eq!(
            trace
                .session
                .sim()
                .world()
                .unit(FACTORIES[2])
                .unwrap()
                .build
                .factory_repeat,
            recipe
        );
        trace.boundary();
    }
    assert_eq!(trace.queue(FACTORIES[0]), vec![SWORD, MASON]);
    assert_eq!(trace.queue(FACTORIES[1]), vec![MASON, SWORD, MASON]);
    trace.command(rally(&FACTORIES[1..2], None, QueueMode::Replace));
    trace.command(rally(
        &FACTORIES[..1],
        Some(FactoryRallyOrder::Move { to: at(320, 600) }),
        QueueMode::Replace,
    ));
    trace.command(rally(
        &FACTORIES[..1],
        Some(FactoryRallyOrder::Patrol { to: at(320, 720) }),
        QueueMode::Append,
    ));
    trace.command(SimCommand::Build {
        units: UnitSet::single(BUILDER),
        def: INCOME,
        at: Some(at(240, 480)),
        queue: QueueMode::Replace,
    });
    trace.boundary();
    while economy::income_per_second(trace.session.sim().world(), OWNER).0 == 0 {
        trace.step();
        if economy::income_per_second(trace.session.sim().world(), OWNER).0 == 0 {
            for (id, frame) in products.into_iter().zip(frames) {
                assert_eq!(
                    trace.session.sim().world().unit(id).unwrap().build.frame,
                    Some(frame),
                    "the free income building must not fund factory progress before completion"
                );
            }
        }
    }
    assert_eq!(
        trace
            .session
            .sim()
            .world()
            .unit(FACTORIES[0])
            .unwrap()
            .build
            .factory_rally,
        Some(vec![
            FactoryRallyOrder::Move { to: at(320, 600) },
            FactoryRallyOrder::Patrol { to: at(320, 720) }
        ])
    );
    assert_eq!(
        trace
            .session
            .sim()
            .world()
            .unit(FACTORIES[1])
            .unwrap()
            .build
            .factory_rally,
        Some(vec![])
    );
    trace.boundary();
    // Income is aggregated before the next shortage-sharing ratio becomes
    // available to every worker, so observe actual resumed progress as well.
    while products.into_iter().zip(frames).any(|(id, frame)| {
        trace
            .session
            .sim()
            .world()
            .unit(id)
            .unwrap()
            .build
            .frame
            .unwrap()
            .done
            == frame.done
    }) {
        trace.step();
    }
    trace.boundary();
    for (id, frame) in products.into_iter().zip(frames) {
        assert!(
            trace
                .session
                .sim()
                .world()
                .unit(id)
                .unwrap()
                .build
                .frame
                .unwrap()
                .done
                > frame.done,
            "ordinary construction of the income source must resume paid work"
        );
    }
    // Observe finite queues finish and actual repeated paid products at all
    // factories before fuzzing, so a vacuous no-production trace cannot pass.
    while !FACTORIES.iter().all(|&factory| {
        let world = trace.session.sim().world();
        let unit = world.unit(factory).unwrap();
        unit.orders.is_empty()
            && unit.build.factory.product_from_repeat
            && unit.build.factory.product.is_some_and(|id| {
                world
                    .unit(id)
                    .unwrap()
                    .build
                    .frame
                    .is_some_and(|f| f.paid > 0 && f.done > 0)
            })
    }) {
        trace.step();
    }
    for (factory, expected) in FACTORIES.into_iter().zip([MASON, SWORD, MASON]) {
        let product = trace.product(factory).unwrap();
        assert_eq!(
            trace.session.sim().world().unit(product).unwrap().def,
            expected
        );
    }
    for product in products {
        assert!(
            trace
                .session
                .sim()
                .world()
                .unit(product)
                .unwrap()
                .build
                .is_complete()
        );
    }
    trace.boundary();

    // An explicit paid repeated cancellation guarantees arena removal and
    // fresh-product creation are crossed by checkpoints, independent of seed.
    let cancelled = trace.product(FACTORIES[2]).unwrap();
    assert_eq!(
        trace.session.sim().world().unit(cancelled).unwrap().def,
        MASON
    );
    trace.command(cancel(&FACTORIES[2..], MASON, 1));
    trace.step();
    assert!(trace.session.sim().world().unit(cancelled).is_none());
    assert_eq!(trace.product(FACTORIES[2]), None);
    assert_eq!(
        trace
            .session
            .sim()
            .world()
            .unit(FACTORIES[2])
            .unwrap()
            .build
            .factory_repeat,
        None
    );
    trace.boundary();
    trace.command(repeat(&FACTORIES[2..], Some(SWORD)));
    while trace.product(FACTORIES[2]).is_none() {
        trace.step();
    }
    assert_ne!(trace.product(FACTORIES[2]), Some(cancelled));
    trace.boundary();

    // Fixed-seed, bounded interleavings stress simultaneous commands, recipe
    // changes, finite additions, 0/1/oversized cancellation, rally lists and Stop.
    let mut rng = Rng::with_stream(seed, 0x0fac_70a7);
    let mut command_kinds_seen = [false; 10];
    for offset in 0..STRESS_TICKS {
        if offset % 3 == 0 {
            for _ in 0..2 {
                let selected = if rng.below(4) == 0 {
                    FACTORIES.to_vec()
                } else {
                    vec![FACTORIES[rng.below(3) as usize]]
                };
                let def = if rng.below(2) == 0 { SWORD } else { MASON };
                let kind = rng.below(10);
                command_kinds_seen[kind as usize] = true;
                let command = match kind {
                    0..=2 => build(&selected, def),
                    3 => repeat(&selected, Some(def)),
                    4 => repeat(&selected, None),
                    5 => cancel(&selected, def, [0, 1, u32::MAX][rng.below(3) as usize]),
                    6 => rally(
                        &selected,
                        Some(FactoryRallyOrder::Move {
                            to: at(rng.range_i32(128, 896), rng.range_i32(480, 720)),
                        }),
                        QueueMode::Replace,
                    ),
                    7 => rally(
                        &selected,
                        Some(FactoryRallyOrder::Patrol {
                            to: at(rng.range_i32(128, 896), rng.range_i32(480, 720)),
                        }),
                        QueueMode::Append,
                    ),
                    8 => rally(&selected, None, QueueMode::Replace),
                    _ => SimCommand::Stop {
                        units: units(&selected),
                    },
                };
                trace.command(command);
            }
        }
        if offset % 31 == 0 {
            trace.boundary();
        }
        trace.step();
    }
    trace.boundary();
    assert!(command_kinds_seen.into_iter().all(|seen| seen));
    trace.command(SimCommand::Stop {
        units: units(&FACTORIES),
    });
    trace.step();
    trace.boundary();
    for _ in 0..20 {
        trace.step();
        for factory in FACTORIES {
            let unit = trace.session.sim().world().unit(factory).unwrap();
            assert_eq!(unit.build.factory.product, None);
            assert_eq!(unit.build.factory_repeat, None);
            assert!(trace.queue(factory).is_empty());
        }
    }
    trace.boundary();
    trace
}

fn restore(session: &ReplaySession) -> ReplaySession {
    let checkpoint =
        Checkpoint::from_json(&session.checkpoint().unwrap().to_json_pretty().unwrap()).unwrap();
    let restored = checkpoint.restore(None, ReplayBudget::default()).unwrap();
    assert_eq!(restored.sim().state_hash(), session.sim().state_hash());
    assert_eq!(restored.sim().world(), session.sim().world());
    assert_eq!(restored.executed_log(), session.executed_log());
    restored
}

#[test]
fn bounded_multi_factory_controls_survive_repeated_serialized_reload_chains() {
    for seed in [0x5eed, 0xc0ffee] {
        let trace = trace(seed);
        eprintln!(
            "factory checkpoint stress seed {seed:x}: {} ticks, {} commands, {} reload boundaries",
            trace.tick(),
            trace.commands.len(),
            trace.boundaries.len()
        );
        let commands = CommandLog::from_json(&trace.commands.to_json()).unwrap();
        let encoded = match &trace.source {
            CheckpointSource::Synthetic { scenario } => scenario.to_json_pretty(),
            _ => unreachable!(),
        };
        let mut resumed = ReplaySession::new(
            CheckpointSource::synthetic(Scenario::from_json(&encoded).unwrap()),
            None,
        )
        .unwrap();
        let mut previous = 0;
        for &split in &trace.boundaries {
            assert_eq!(
                resumed.advance(&commands, split - previous).unwrap(),
                trace.hashes[previous as usize..split as usize],
                "seed {seed:x}, split {split}"
            );
            resumed = restore(&resumed);
            previous = split;
        }
        assert_eq!(resumed.sim().world(), trace.session.sim().world());
        assert_eq!(
            resumed.checkpoint().unwrap().to_json_pretty().unwrap(),
            trace
                .session
                .checkpoint()
                .unwrap()
                .to_json_pretty()
                .unwrap()
        );
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "factory-stress-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
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
fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn run(args: &[&str]) {
    // Capture to regular files rather than unread pipes, so a verbose child
    // cannot deadlock while we poll its wall-clock bound. Scratch removes both
    // captures only after exit (or kill + wait) and after diagnostics are read.
    let scratch = Scratch::new();
    let stdout = scratch.path("stdout.txt");
    let stderr = scratch.path("stderr.txt");
    let started = Instant::now();
    let mut child = Command::new(BIN)
        .args(args)
        .env_remove("TAK_DATA_DIR")
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&stdout).unwrap())
        .stderr(std::fs::File::create(&stderr).unwrap())
        .spawn()
        .unwrap();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < Duration::from_secs(30) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            state => {
                let reason = match state {
                    Ok(None) => "subprocess exceeded its 30-second wall-clock bound".to_owned(),
                    Err(error) => format!("could not poll subprocess: {error}"),
                    Ok(Some(_)) => unreachable!(),
                };
                let killed = child.kill();
                let waited = child.wait();
                break Err(format!("{reason}; kill: {killed:?}; wait: {waited:?}"));
            }
        }
    };
    let stdout = std::fs::read(stdout).unwrap();
    let stderr = std::fs::read(stderr).unwrap();
    assert!(
        result.as_ref().is_ok_and(|status| status.success()),
        "{args:?}\n{result:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
}
fn hashes(path: &Path, first_tick: u32) -> Vec<u64> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .enumerate()
        .map(|(offset, line)| {
            let (tick, hash) = line.split_once(' ').unwrap();
            let expected = first_tick
                .checked_add(u32::try_from(offset).unwrap())
                .unwrap();
            assert_eq!(
                tick.parse::<u32>().unwrap(),
                expected,
                "{} must contain consecutive absolute tick labels",
                path.display()
            );
            u64::from_str_radix(hash, 16).unwrap()
        })
        .collect()
}

#[test]
fn separate_process_replay_and_serial_checkpoint_chain_match_every_hash() {
    let trace = trace(0x5eed);
    let dir = Scratch::new();
    let scenario_file = dir.path("scenario.json");
    let commands = dir.path("commands.json");
    std::fs::write(&scenario_file, scenario(0x5eed).to_json_pretty()).unwrap();
    std::fs::write(&commands, trace.commands.to_json_pretty()).unwrap();
    let full_a = dir.path("a.hashes");
    let full_b = dir.path("b.hashes");
    for output in [&full_a, &full_b] {
        run(&[
            "run",
            "--scenario",
            path(&scenario_file),
            "--log",
            path(&commands),
            "--ticks",
            &trace.tick().to_string(),
            "--hashes",
            path(output),
        ]);
        assert_eq!(hashes(output, 0), trace.hashes);
    }
    assert_eq!(
        std::fs::read(full_a).unwrap(),
        std::fs::read(full_b).unwrap()
    );
    let mut checkpoint = dir.path("checkpoint-0.json");
    run(&[
        "checkpoint",
        "save",
        "--scenario",
        path(&scenario_file),
        "--log",
        path(&commands),
        "--ticks",
        "0",
        "--out",
        path(&checkpoint),
    ]);
    let mut previous = 0;
    // Keep subprocess count small while chaining reloads across shortage,
    // recovery, random controls and final Stop rather than only one split.
    let boundaries: BTreeSet<_> = trace
        .boundaries
        .iter()
        .copied()
        .step_by(3)
        .filter(|&tick| tick > 0)
        .chain([trace.tick()])
        .collect();
    for split in boundaries {
        let next = dir.path(&format!("checkpoint-{split}.json"));
        let output = dir.path("segment.hashes");
        run(&[
            "checkpoint",
            "resume",
            "--checkpoint",
            path(&checkpoint),
            "--log",
            path(&commands),
            "--ticks",
            &(split - previous).to_string(),
            "--out",
            path(&next),
            "--hashes",
            path(&output),
        ]);
        assert_eq!(
            hashes(&output, previous),
            trace.hashes[previous as usize..split as usize]
        );
        let saved = Checkpoint::read(&next).unwrap();
        assert_eq!(saved.next_tick(), Tick(split));
        saved.restore(None, ReplayBudget::default()).unwrap();
        checkpoint = next;
        previous = split;
    }
    assert_eq!(
        Checkpoint::read(&checkpoint)
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

#[test]
fn future_controls_are_excluded_until_drained_then_save_and_replay_exactly() {
    // This is the headless executed-history contract, NOT the client browser's
    // pending-order rejection policy (covered by client/tests/checkpoints.rs).
    let source = CheckpointSource::synthetic(scenario(0x5eed));
    let mut live = ReplaySession::new(source, None).unwrap();
    let mut log = CommandLog::new();
    live.advance(&log, 1).unwrap();
    let before = live.checkpoint().unwrap().to_json_pretty().unwrap();
    for command in [
        build(&FACTORIES, SWORD),
        repeat(&FACTORIES, Some(MASON)),
        cancel(&FACTORIES, SWORD, 1),
        rally(
            &FACTORIES,
            Some(FactoryRallyOrder::Move { to: at(384, 600) }),
            QueueMode::Replace,
        ),
    ] {
        log.record(Tick(1), OWNER, command);
    }
    log.record(Tick(4), OWNER, repeat(&FACTORIES, None));
    log.record(Tick(4), OWNER, rally(&FACTORIES, None, QueueMode::Replace));
    live.advance(&log, 0).unwrap();
    assert_eq!(live.checkpoint().unwrap().command_count(), 0);
    assert_eq!(live.checkpoint().unwrap().to_json_pretty().unwrap(), before);
    let mut saved = restore(&live);
    assert_eq!(
        live.advance(&log, 1).unwrap(),
        saved.advance(&log, 1).unwrap()
    );
    assert_eq!(live.checkpoint().unwrap().command_count(), 4);
    let mut pending_future = restore(&live);
    assert_eq!(
        live.advance(&log, 3).unwrap(),
        pending_future.advance(&log, 3).unwrap()
    );
    assert_eq!(live.checkpoint().unwrap().command_count(), log.len());
    let mut drained = restore(&live);
    assert_eq!(
        live.advance(&log, 90).unwrap(),
        drained.advance(&log, 90).unwrap()
    );
    assert_eq!(live.sim().world(), drained.sim().world());
}
