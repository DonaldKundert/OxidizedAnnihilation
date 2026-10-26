//! Factory controls replay/save contract. All scenario and command fixtures
//! are hand-built synthetic data; no installed game data is read.
#[path = "../../tak-sim/tests/factory_controls_support/mod.rs"]
mod factory_controls_support;
use factory_controls_support::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use tak_commands::{CommandLog, PlayerId, SimCommand, Tick, UnitDefId, UnitId, UnitSet};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_sim::{Order, Scenario, economy};

const TICKS: u32 = 300;
const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "factory-controls-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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
fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}
fn run(args: &[&str]) {
    let out = Command::new(BIN)
        .args(args)
        .env_remove("TAK_DATA_DIR")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
fn hashes(path: &Path) -> Vec<u64> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let (_, hash) = line.split_once(' ').unwrap();
            u64::from_str_radix(hash, 16).unwrap()
        })
        .collect()
}

#[test]
fn controls_round_trip_and_checkpoints_preserve_every_continuation_hash() {
    let scenario = scenario();
    let commands = log();
    let source = CheckpointSource::synthetic(scenario.clone());
    let mut full = ReplaySession::new(source.clone(), None).unwrap();
    let expected = full.advance(&commands, TICKS).unwrap();
    let mut roundtrip = ReplaySession::new(
        CheckpointSource::synthetic(Scenario::from_json(&scenario.to_json_pretty()).unwrap()),
        None,
    )
    .unwrap();
    let decoded = CommandLog::from_json(&commands.to_json()).unwrap();
    assert_eq!(roundtrip.advance(&decoded, TICKS).unwrap(), expected);
    assert!(
        full.sim()
            .world()
            .units()
            .filter(|(_, u)| [SWORD, MASON].contains(&u.def) && u.build.is_complete())
            .count()
            >= 3
    );
    assert_eq!(
        full.sim()
            .world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory_rally,
        Some(vec![])
    );
    // Before/after rally edits, active cancellation, release, replacement and
    // clear, including a prefix that ends with paid construction in progress.
    for split in [
        0, 1, 2, 5, 6, 7, 8, 10, 11, 12, 13, 26, 31, 40, 41, 42, 80, 81, 82, 150, 300,
    ] {
        let mut first = ReplaySession::new(source.clone(), None).unwrap();
        assert_eq!(
            first.advance(&commands, split).unwrap(),
            expected[..split as usize]
        );
        let saved = first.checkpoint().unwrap().to_json_pretty().unwrap();
        let checkpoint = Checkpoint::from_json(&saved).unwrap();
        let mut restored = checkpoint.restore(None, ReplayBudget::default()).unwrap();
        assert_eq!(restored.sim().state_hash(), first.sim().state_hash());
        assert_eq!(restored.sim().world(), first.sim().world());
        assert_eq!(
            restored.advance(&commands, TICKS - split).unwrap(),
            expected[split as usize..]
        );
        assert_eq!(
            restored.checkpoint().unwrap().to_json_pretty().unwrap(),
            full.checkpoint().unwrap().to_json_pretty().unwrap()
        );
    }
}

#[test]
fn separate_process_replay_and_mid_product_save_resume_match_all_hashes() {
    let dir = Scratch::new();
    let scenario = dir.path("scenario.json");
    let commands = dir.path("commands.json");
    std::fs::write(
        &scenario,
        factory_controls_support::scenario().to_json_pretty(),
    )
    .unwrap();
    std::fs::write(&commands, log().to_json_pretty()).unwrap();
    let a = dir.path("a.hashes");
    let b = dir.path("b.hashes");
    for output in [&a, &b] {
        run(&[
            "run",
            "--scenario",
            s(&scenario),
            "--log",
            s(&commands),
            "--ticks",
            "300",
            "--hashes",
            s(output),
        ]);
    }
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let expected = hashes(&a);
    let mut direct = ReplaySession::new(
        CheckpointSource::synthetic(factory_controls_support::scenario()),
        None,
    )
    .unwrap();
    assert_eq!(expected, direct.advance(&log(), TICKS).unwrap());
    for boundary in [8, 12, 13, 42, 81] {
        let save = dir.path("split.json");
        let resumed = dir.path("resumed.json");
        let suffix = dir.path("suffix.hashes");
        run(&[
            "checkpoint",
            "save",
            "--scenario",
            s(&scenario),
            "--log",
            s(&commands),
            "--ticks",
            &boundary.to_string(),
            "--out",
            s(&save),
        ]);
        let checkpoint = Checkpoint::read(&save).unwrap();
        assert_eq!(checkpoint.next_tick(), Tick(boundary));
        if boundary == 8 {
            let prefix = checkpoint.restore(None, ReplayBudget::default()).unwrap();
            let p = prefix
                .sim()
                .world()
                .unit(FACTORY)
                .unwrap()
                .build
                .factory
                .product
                .unwrap();
            assert!(
                prefix
                    .sim()
                    .world()
                    .unit(p)
                    .unwrap()
                    .build
                    .frame
                    .unwrap()
                    .paid
                    > 0
            );
            assert_eq!(
                prefix
                    .sim()
                    .world()
                    .unit(FACTORY)
                    .unwrap()
                    .build
                    .factory_rally
                    .as_ref()
                    .unwrap()
                    .len(),
                3
            );
        }
        run(&[
            "checkpoint",
            "resume",
            "--checkpoint",
            s(&save),
            "--log",
            s(&commands),
            "--ticks",
            &(TICKS - boundary).to_string(),
            "--out",
            s(&resumed),
            "--hashes",
            s(&suffix),
        ]);
        assert_eq!(hashes(&suffix), expected[boundary as usize..]);
        let final_save = Checkpoint::read(&resumed).unwrap();
        assert_eq!(
            final_save.state_hash(),
            format!("{:016x}", direct.sim().state_hash())
        );
        assert_eq!(
            final_save.to_json_pretty().unwrap(),
            direct.checkpoint().unwrap().to_json_pretty().unwrap()
        );
    }
}

fn repeat_command(def: Option<UnitDefId>) -> SimCommand {
    SimCommand::SetFactoryRepeat {
        units: UnitSet::single(FACTORY),
        def,
    }
}

/// A command-only synthetic trace. The fixture's ordinary starting mana is
/// defined by `scenario()`; no running world state is edited to reach a phase.
struct RepeatTrace {
    session: ReplaySession,
    commands: CommandLog,
    hashes: Vec<u64>,
    boundaries: Vec<u32>,
}

impl RepeatTrace {
    fn new() -> Self {
        Self {
            session: ReplaySession::new(CheckpointSource::synthetic(scenario()), None).unwrap(),
            commands: CommandLog::new(),
            hashes: vec![],
            boundaries: vec![0],
        }
    }

    fn command(&mut self, command: SimCommand) {
        self.commands
            .record(self.session.sim().tick(), PlayerId(0), command);
    }

    fn step(&mut self) {
        assert!(
            self.session.sim().tick().0 < 2000,
            "synthetic repeat production stalled at tick {}",
            self.session.sim().tick().0
        );
        self.hashes
            .extend(self.session.advance(&self.commands, 1).unwrap());
    }

    fn boundary(&mut self) {
        let tick = self.session.sim().tick().0;
        if self.boundaries.last() != Some(&tick) {
            self.boundaries.push(tick);
        }
    }

    fn paid_product(&mut self, def: UnitDefId, from_repeat: bool) -> UnitId {
        loop {
            let world = self.session.sim().world();
            let factory = world.unit(FACTORY).unwrap();
            if let Some(id) = factory.build.factory.product {
                let product = world.unit(id).unwrap();
                assert_eq!(
                    product.def, def,
                    "the next product must respect finite FIFO"
                );
                assert_eq!(factory.build.factory.product_from_repeat, from_repeat);
                if product
                    .build
                    .frame
                    .is_some_and(|f| f.paid > 0 && f.done > 0)
                {
                    return id;
                }
            }
            self.step();
        }
    }

    fn finish(&mut self, product: UnitId) {
        while !self
            .session
            .sim()
            .world()
            .unit(product)
            .expect("the active product must not be silently cancelled")
            .build
            .is_complete()
        {
            self.step();
        }
    }

    fn finite_queue(&self) -> Vec<UnitDefId> {
        self.session
            .sim()
            .world()
            .unit(FACTORY)
            .unwrap()
            .orders
            .iter()
            .filter_map(|order| match order {
                Order::Build { def, at: None } => Some(*def),
                _ => None,
            })
            .collect()
    }

    fn recipe(&self) -> Option<UnitDefId> {
        self.session
            .sim()
            .world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory_repeat
    }

    fn assert_active(&self, product: UnitId, from_repeat: bool) {
        let factory = &self
            .session
            .sim()
            .world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory;
        assert_eq!(factory.product, Some(product));
        assert_eq!(factory.product_from_repeat, from_repeat);
    }
}

fn repeat_trace() -> RepeatTrace {
    let mut trace = RepeatTrace::new();
    trace.step();
    trace.command(repeat_command(Some(SWORD)));
    let first = trace.paid_product(SWORD, true);
    trace.boundary();
    assert!(trace.finite_queue().is_empty());
    trace.command(cancel(SWORD, 0));
    trace.step();
    trace.assert_active(first, true);
    assert_eq!(trace.recipe(), Some(SWORD));
    trace.finish(first);
    let second = trace.paid_product(SWORD, true);
    assert_ne!(
        first, second,
        "repeat must create a new unit, not recycle the frame"
    );
    trace.finish(second);
    let third = trace.paid_product(SWORD, true);
    assert_ne!(second, third);
    trace.boundary();

    // Finite additions must not erase or interrupt a paid repeated product.
    trace.command(build(MASON));
    trace.command(build(SWORD));
    trace.step();
    trace.assert_active(third, true);
    assert_eq!(trace.finite_queue(), vec![MASON, SWORD]);
    trace.boundary();
    trace.command(repeat_command(Some(MASON)));
    trace.step();
    trace.assert_active(third, true);
    assert_eq!(trace.recipe(), Some(MASON));
    trace.boundary();
    trace.command(repeat_command(None));
    trace.step();
    trace.assert_active(third, true);
    assert_eq!(trace.recipe(), None);
    trace.boundary();
    trace.command(repeat_command(Some(MASON)));
    trace.step();
    trace.assert_active(third, true);
    trace.finish(third);
    assert_eq!(trace.finite_queue(), vec![MASON, SWORD]);
    trace.boundary();

    let finite_mason = trace.paid_product(MASON, false);
    trace.boundary();
    trace.finish(finite_mason);
    assert_eq!(trace.finite_queue(), vec![SWORD]);
    let finite_sword = trace.paid_product(SWORD, false);
    trace.finish(finite_sword);
    assert!(trace.finite_queue().is_empty());
    let repeated_mason = trace.paid_product(MASON, true);
    trace.command(repeat_command(None));
    trace.step();
    trace.assert_active(repeated_mason, true);
    trace.boundary();
    trace.finish(repeated_mason);
    for _ in 0..40 {
        trace.step();
        assert_eq!(
            trace
                .session
                .sim()
                .world()
                .unit(FACTORY)
                .unwrap()
                .build
                .factory
                .product,
            None,
            "disabled repeat must finish once, then stay idle"
        );
    }

    trace.command(repeat_command(Some(SWORD)));
    let cancelled = trace.paid_product(SWORD, true);
    trace.command(build(SWORD));
    trace.command(build(SWORD));
    trace.step();
    trace.command(cancel(SWORD, 1));
    trace.step();
    trace.assert_active(cancelled, true);
    assert_eq!(
        trace.recipe(),
        None,
        "positive cancellation disables the matching recipe"
    );
    assert_eq!(trace.finite_queue(), vec![SWORD]);
    trace.boundary();
    trace.command(cancel(SWORD, 1));
    trace.step();
    trace.assert_active(cancelled, true);
    assert!(trace.finite_queue().is_empty());
    trace.boundary();
    let world = trace.session.sim().world();
    let paid = world.unit(cancelled).unwrap().build.frame.unwrap().paid;
    let mana = economy::mana(world, PlayerId(0)).0;
    assert_eq!(economy::income_per_second(world, PlayerId(0)).0, 0);
    trace.command(cancel(SWORD, 1));
    trace.step();
    let world = trace.session.sim().world();
    assert!(world.unit(cancelled).is_none());
    assert_eq!(economy::mana(world, PlayerId(0)).0, mana + paid);
    assert_eq!(world.unit(FACTORY).unwrap().build.factory.product, None);
    trace.boundary();

    trace.command(repeat_command(Some(SWORD)));
    let stopped = trace.paid_product(SWORD, true);
    trace.command(build(MASON));
    trace.step();
    trace.command(SimCommand::Stop {
        units: UnitSet::single(FACTORY),
    });
    trace.step();
    assert_eq!(trace.recipe(), None);
    assert!(trace.finite_queue().is_empty());
    assert!(trace.session.sim().world().unit(stopped).is_none());
    trace.boundary();
    for _ in 0..40 {
        trace.step();
        assert_eq!(
            trace
                .session
                .sim()
                .world()
                .unit(FACTORY)
                .unwrap()
                .build
                .factory
                .product,
            None,
            "Stop must not leave an infinite recipe armed"
        );
    }
    trace.boundary();
    trace
}

#[test]
fn repeat_raw_replay_and_checkpoints_preserve_paid_source_and_finite_fifo() {
    let trace = repeat_trace();
    let commands = CommandLog::from_json(&trace.commands.to_json()).unwrap();
    assert_eq!(commands, trace.commands);
    let source =
        CheckpointSource::synthetic(Scenario::from_json(&scenario().to_json_pretty()).unwrap());
    let ticks = trace.session.sim().tick().0;
    let mut raw = ReplaySession::new(source.clone(), None).unwrap();
    assert_eq!(raw.advance(&commands, ticks).unwrap(), trace.hashes);
    assert_eq!(raw.sim().world(), trace.session.sim().world());
    for split in trace.boundaries {
        let mut first = ReplaySession::new(source.clone(), None).unwrap();
        assert_eq!(
            first.advance(&commands, split).unwrap(),
            trace.hashes[..split as usize]
        );
        let encoded = first.checkpoint().unwrap().to_json_pretty().unwrap();
        let mut restored = Checkpoint::from_json(&encoded)
            .unwrap()
            .restore(None, ReplayBudget::default())
            .unwrap();
        assert_eq!(restored.sim().world(), first.sim().world());
        assert_eq!(restored.sim().state_hash(), first.sim().state_hash());
        assert_eq!(
            restored.advance(&commands, ticks - split).unwrap(),
            trace.hashes[split as usize..],
            "repeat continuation diverged after tick {split}"
        );
        assert_eq!(
            restored.checkpoint().unwrap().to_json_pretty().unwrap(),
            trace
                .session
                .checkpoint()
                .unwrap()
                .to_json_pretty()
                .unwrap()
        );
    }
}

#[test]
fn repeat_separate_process_replay_and_mid_repeat_save_resume_match_every_hash() {
    let trace = repeat_trace();
    let dir = Scratch::new();
    let scenario_path = dir.path("repeat-scenario.json");
    let commands = dir.path("repeat-commands.json");
    std::fs::write(&scenario_path, scenario().to_json_pretty()).unwrap();
    std::fs::write(&commands, trace.commands.to_json_pretty()).unwrap();
    let ticks = trace.session.sim().tick().0;
    let a = dir.path("repeat-a.hashes");
    let b = dir.path("repeat-b.hashes");
    for output in [&a, &b] {
        run(&[
            "run",
            "--scenario",
            s(&scenario_path),
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
    for boundary in trace.boundaries {
        let save = dir.path("repeat-split.json");
        let resumed = dir.path("repeat-resumed.json");
        let suffix = dir.path("repeat-suffix.hashes");
        run(&[
            "checkpoint",
            "save",
            "--scenario",
            s(&scenario_path),
            "--log",
            s(&commands),
            "--ticks",
            &boundary.to_string(),
            "--out",
            s(&save),
        ]);
        let checkpoint = Checkpoint::read(&save).unwrap();
        assert_eq!(checkpoint.next_tick(), Tick(boundary));
        run(&[
            "checkpoint",
            "resume",
            "--checkpoint",
            s(&save),
            "--log",
            s(&commands),
            "--ticks",
            &(ticks - boundary).to_string(),
            "--out",
            s(&resumed),
            "--hashes",
            s(&suffix),
        ]);
        assert_eq!(hashes(&suffix), trace.hashes[boundary as usize..]);
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
