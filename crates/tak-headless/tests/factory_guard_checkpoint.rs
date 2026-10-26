//! Synthetic command-only coverage of factory Guard recipes across replay and
//! save boundaries. No running world mutations, scripts or retail data are used.
#[path = "../../tak-sim/tests/factory_controls_support/mod.rs"]
mod factory_controls_support;

use factory_controls_support::{FACTORY, KEEP, KING, MASON, SWORD, at, build, rally};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tak_commands::{
    CommandLog, FactoryRallyOrder, FireState, PlayerId, QueueMode, SimCommand, Target, Tick,
    UnitDefId, UnitId, UnitSet,
};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_math::{Angle, Fx};
use tak_sim::campaign::InitialUnit;
use tak_sim::combat::{CombatOrder, DamageStats, WeaponKind, WeaponStats};
use tak_sim::{Order, Scenario, Unit};

const OWNER: PlayerId = PlayerId(0);
const COMMANDER: UnitId = UnitId::new(1, 0);
const ENEMY: UnitId = UnitId::new(2, 0);
const ENEMY_DEF: UnitDefId = UnitDefId(5);
const MAX_TICKS: u32 = 1000;
const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");
static NEXT: AtomicU64 = AtomicU64::new(0);

fn scenario() -> Scenario {
    let mut scenario = factory_controls_support::scenario();
    scenario.seed = 0x6a17_d00d;
    // Expendable commander: its combat death must not defeat the owner and
    // self-destruct the factory, which would make release coverage vacuous.
    scenario.victory.defeat = tak_sim::victory::DefeatRule::AllUnitsKilled;
    let commander = &mut scenario.unit_defs[KING.0 as usize];
    commander.maxvelocity = Fx::ONE;
    commander.combat.max_damage = 100;
    commander.economy.mogriumincome = 0;
    for (def, buildtime) in [(SWORD, 30), (MASON, 18)] {
        let unit = &mut scenario.unit_defs[def.0 as usize];
        unit.combat.max_damage = 100;
        unit.build.buildtime = buildtime << 16;
    }
    let mut enemy = tak_sim::testing::monarch_def("SYNTHETIC_GUARD_TARGET_KILLER");
    enemy.maxvelocity = Fx::ZERO;
    enemy.combat.max_damage = 100;
    enemy.combat.weapons.push(WeaponStats {
        name: "SYNTHETIC_INSTANT_SHOT".into(),
        kind: WeaponKind::RemoteEffect,
        range: 2000,
        reload_ticks: 30,
        aim_tolerance: Some(u16::MAX),
        damage: DamageStats {
            default: 200,
            ..Default::default()
        },
        ..Default::default()
    });
    scenario.unit_defs.push(enemy);
    scenario.initial_units = Some(
        [
            (KEEP, OWNER, at(192, 192)),
            (KING, OWNER, at(320, 360)),
            (ENEMY_DEF, PlayerId(1), at(850, 750)),
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
    Scenario::from_json(&scenario.to_json_pretty()).unwrap()
}

fn guard(target: UnitId, queue: QueueMode) -> SimCommand {
    rally(Some(FactoryRallyOrder::Guard { target }), queue)
}

fn repeat(def: Option<UnitDefId>) -> SimCommand {
    SimCommand::SetFactoryRepeat {
        units: UnitSet::single(FACTORY),
        def,
    }
}

struct Trace {
    session: ReplaySession,
    commands: CommandLog,
    hashes: Vec<u64>,
    boundaries: BTreeSet<u32>,
}

impl Trace {
    fn new() -> Self {
        Self {
            session: ReplaySession::new(CheckpointSource::synthetic(scenario()), None).unwrap(),
            commands: CommandLog::new(),
            hashes: vec![],
            boundaries: BTreeSet::from([0]),
        }
    }

    fn tick(&self) -> u32 {
        self.session.sim().tick().0
    }

    fn unit(&self, id: UnitId) -> &Unit {
        self.session.sim().world().unit(id).unwrap()
    }

    fn command(&mut self, player: PlayerId, command: SimCommand) {
        self.commands.record(Tick(self.tick()), player, command);
    }

    fn step(&mut self) {
        assert!(
            self.tick() < MAX_TICKS,
            "Guard trace stalled at tick {}",
            self.tick()
        );
        let unfinished = self
            .session
            .sim()
            .world()
            .unit(FACTORY)
            .and_then(|factory| factory.build.factory.product)
            .filter(|&id| !self.unit(id).build.is_complete());
        self.hashes
            .extend(self.session.advance(&self.commands, 1).unwrap());
        if unfinished.is_some_and(|id| self.unit(id).build.is_complete()) {
            self.around_last_step();
        }
    }

    fn boundary(&mut self) {
        self.boundaries.insert(self.tick());
    }

    fn around_last_step(&mut self) {
        self.boundaries.extend([self.tick() - 1, self.tick()]);
    }

    fn queue(&self) -> Vec<UnitDefId> {
        self.unit(FACTORY)
            .orders
            .iter()
            .filter_map(|order| match order {
                Order::Build { def, at: None } => Some(*def),
                _ => None,
            })
            .collect()
    }

    fn assert_active(&self, id: UnitId, from_repeat: bool) {
        let factory = &self.unit(FACTORY).build.factory;
        assert_eq!(factory.product, Some(id));
        assert_eq!(factory.product_from_repeat, from_repeat);
        let frame = self
            .unit(id)
            .build
            .frame
            .expect("product must remain unfinished");
        assert!(
            frame.paid > 0 && frame.done > 0,
            "product must really be paid"
        );
    }

    fn paid_product(&mut self, def: UnitDefId, from_repeat: bool) -> UnitId {
        loop {
            if let Some(id) = self.unit(FACTORY).build.factory.product {
                assert_eq!(self.unit(id).def, def, "finite FIFO must precede repeat");
                if self
                    .unit(id)
                    .build
                    .frame
                    .is_some_and(|frame| frame.paid > 0 && frame.done > 0)
                {
                    self.assert_active(id, from_repeat);
                    self.boundary();
                    return id;
                }
            }
            self.step();
        }
    }

    fn finish(&mut self, id: UnitId) {
        while !self.unit(id).build.is_complete() {
            self.step();
        }
    }
}

fn has_guard(unit: &Unit, target: UnitId) -> bool {
    unit.combat.order == Some(CombatOrder::Guard(target))
        || unit
            .orders
            .iter()
            .any(|order| *order == Order::Guard { target })
}

fn trace() -> Trace {
    let mut trace = Trace::new();
    trace.command(
        PlayerId(1),
        SimCommand::SetFireState {
            units: UnitSet::single(ENEMY),
            state: FireState::HoldFire,
        },
    );
    trace.step();
    trace.command(OWNER, build(SWORD));
    trace.command(OWNER, build(MASON));
    trace.command(OWNER, repeat(Some(SWORD)));
    trace.command(OWNER, guard(COMMANDER, QueueMode::Replace));
    trace.command(
        OWNER,
        SimCommand::Move {
            units: UnitSet::single(COMMANDER),
            to: at(800, 400),
            queue: QueueMode::Replace,
        },
    );
    let first = trace.paid_product(SWORD, false);
    let fallback = at(448, 512);
    trace.command(OWNER, guard(COMMANDER, QueueMode::Replace));
    trace.command(
        OWNER,
        rally(
            Some(FactoryRallyOrder::Move { to: fallback }),
            QueueMode::Append,
        ),
    );
    trace.step();
    trace.assert_active(first, false);
    assert_eq!(trace.queue(), vec![SWORD, MASON]);
    assert_eq!(trace.unit(FACTORY).build.factory_repeat, Some(SWORD));
    let recipe = trace.unit(FACTORY).build.factory_rally.clone();
    // Rejected replacements must survive serialization too, without clearing
    // the accepted standing recipe or changing paid finite/repeated work.
    for invalid in [ENEMY, UnitId::new(COMMANDER.index, 1)] {
        trace.command(OWNER, guard(invalid, QueueMode::Replace));
        trace.step();
        trace.assert_active(first, false);
        assert_eq!(trace.unit(FACTORY).build.factory_rally, recipe);
        assert_eq!(trace.queue(), vec![SWORD, MASON]);
        assert_eq!(trace.unit(FACTORY).build.factory_repeat, Some(SWORD));
        trace.boundary();
    }
    trace.finish(first);
    assert!(has_guard(trace.unit(first), COMMANDER));
    assert_eq!(
        trace
            .unit(first)
            .orders
            .front()
            .map(|order| matches!(order, Order::Move { .. })),
        Some(true),
        "a leading Guard must leave the yard before becoming active"
    );
    assert_eq!(trace.queue(), vec![MASON]);
    assert_eq!(trace.unit(FACTORY).build.factory_repeat, Some(SWORD));

    let second = trace.paid_product(MASON, false);
    while trace.unit(first).combat.order != Some(CombatOrder::Guard(COMMANDER)) {
        trace.step();
    }
    let escort_start = trace.unit(first).pos;
    let commander_start = trace.unit(COMMANDER).pos;
    let distance_start = escort_start.distance(commander_start);
    trace.boundary();
    for _ in 0..40 {
        trace.step();
    }
    let escort_end = trace.unit(first).pos;
    let commander_end = trace.unit(COMMANDER).pos;
    assert_eq!(
        trace.unit(first).combat.order,
        Some(CombatOrder::Guard(COMMANDER))
    );
    assert!(
        commander_end.x > commander_start.x + Fx::from_int(20),
        "the commander must actually move"
    );
    assert!(
        escort_end.x > escort_start.x + Fx::from_int(20),
        "the finished product must actually escort it"
    );
    assert!(
        escort_end.distance(commander_end) < distance_start,
        "the faster escort must close distance toward the moving commander"
    );
    trace.boundary();

    trace.finish(second);
    assert!(has_guard(trace.unit(second), COMMANDER));
    assert!(trace.queue().is_empty());
    let repeated = trace.paid_product(SWORD, true);
    // Replace the standing recipe during paid repeated work, then disable
    // repetition and enqueue a finite product. None may interrupt this frame.
    trace.command(OWNER, guard(COMMANDER, QueueMode::Replace));
    trace.command(OWNER, repeat(None));
    trace.command(OWNER, build(SWORD));
    trace.step();
    trace.assert_active(repeated, true);
    assert_eq!(trace.queue(), vec![SWORD]);
    assert_eq!(trace.unit(FACTORY).build.factory_repeat, None);
    assert_eq!(
        trace.unit(FACTORY).build.factory_rally,
        Some(vec![FactoryRallyOrder::Guard { target: COMMANDER }])
    );
    trace.boundary();

    // The target dies through an ordinary enemy attack in the serialized log,
    // not remove_unit/world_mut. Assert both combat evidence and arena removal.
    assert_eq!(trace.session.sim().world().combat.deaths, 0);
    trace.command(
        PlayerId(1),
        SimCommand::Attack {
            units: UnitSet::single(ENEMY),
            target: Target::Unit(COMMANDER),
            queue: QueueMode::Replace,
        },
    );
    while trace.session.sim().world().combat.deaths == 0 {
        trace.step();
    }
    trace.around_last_step();
    assert!(trace.unit(COMMANDER).combat.is_dying());
    assert_eq!(trace.unit(ENEMY).combat.kills, 1);
    assert!(
        trace
            .session
            .sim()
            .world()
            .combat
            .impacts
            .iter()
            .any(|impact| impact.shooter == Some(ENEMY))
    );
    trace.assert_active(repeated, true);
    while trace.session.sim().world().unit(COMMANDER).is_some() {
        trace.step();
    }
    trace.around_last_step();
    assert!(
        trace
            .session
            .sim()
            .world()
            .combat
            .corpses
            .iter()
            .any(|corpse| corpse.def == KING)
    );
    assert_eq!(trace.session.sim().world().combat.deaths, 1);
    assert!(!has_guard(trace.unit(first), COMMANDER));
    let released_from = trace.unit(repeated).pos;
    trace.finish(repeated);
    assert!(
        !has_guard(trace.unit(repeated), COMMANDER),
        "dead targets must be skipped at completion"
    );
    assert!(
        matches!(
            trace.unit(repeated).orders.front(),
            Some(Order::Move { .. })
        ),
        "an empty effective recipe must still release the product from the yard"
    );
    assert_eq!(trace.queue(), vec![SWORD]);

    let after_death = trace.paid_product(SWORD, false);
    assert_eq!(
        after_death.index, COMMANDER.index,
        "the trace must exercise actual arena-slot reuse"
    );
    assert_ne!(after_death.generation, COMMANDER.generation);
    // The old target still names the dead generation, not the new product.
    trace.command(OWNER, guard(COMMANDER, QueueMode::Replace));
    trace.step();
    trace.assert_active(after_death, false);
    trace.finish(after_death);
    assert!(!has_guard(trace.unit(after_death), COMMANDER));
    assert!(!has_guard(trace.unit(after_death), after_death));
    assert!(trace.queue().is_empty());
    for _ in 0..100 {
        trace.step();
    }
    assert_eq!(trace.unit(FACTORY).build.factory.product, None);
    assert_eq!(trace.unit(FACTORY).build.factory_repeat, None);
    assert!(
        trace.unit(repeated).pos.distance(released_from) > Fx::from_int(16),
        "fallback egress must actually move rather than merely queue an exit"
    );
    assert!(
        trace.unit(first).pos.distance(fallback) < Fx::from_int(24),
        "ordinary Guard completion must release its appended fallback Move"
    );
    assert!(!has_guard(trace.unit(second), COMMANDER));
    assert_eq!(trace.session.sim().world().combat.deaths, 1);
    trace.boundary();
    trace
}

fn restore(session: &ReplaySession) -> ReplaySession {
    let checkpoint =
        Checkpoint::from_json(&session.checkpoint().unwrap().to_json_pretty().unwrap()).unwrap();
    let restored = checkpoint.restore(None, ReplayBudget::default()).unwrap();
    assert_eq!(restored.sim().world(), session.sim().world());
    assert_eq!(restored.sim().state_hash(), session.sim().state_hash());
    assert_eq!(restored.executed_log(), session.executed_log());
    restored
}

#[test]
fn guard_recipe_paid_completion_and_target_death_survive_checkpoint_splits_and_chains() {
    let trace = trace();
    let commands = CommandLog::from_json(&trace.commands.to_json_pretty()).unwrap();
    assert_eq!(
        commands, trace.commands,
        "Guard UnitIds and queue modes must round-trip"
    );
    let source = CheckpointSource::synthetic(scenario());
    let mut raw = ReplaySession::new(source.clone(), None).unwrap();
    assert_eq!(raw.advance(&commands, trace.tick()).unwrap(), trace.hashes);
    assert_eq!(raw.sim().world(), trace.session.sim().world());
    let final_save = trace
        .session
        .checkpoint()
        .unwrap()
        .to_json_pretty()
        .unwrap();
    for &split in &trace.boundaries {
        let mut prefix = ReplaySession::new(source.clone(), None).unwrap();
        assert_eq!(
            prefix.advance(&commands, split).unwrap(),
            trace.hashes[..split as usize]
        );
        let mut resumed = restore(&prefix);
        assert_eq!(
            resumed.advance(&commands, trace.tick() - split).unwrap(),
            trace.hashes[split as usize..],
            "Guard continuation diverged at split {split}"
        );
        assert_eq!(
            resumed.checkpoint().unwrap().to_json_pretty().unwrap(),
            final_save
        );
    }
    let mut chained = ReplaySession::new(source, None).unwrap();
    let mut previous = 0;
    for &split in &trace.boundaries {
        assert_eq!(
            chained.advance(&commands, split - previous).unwrap(),
            trace.hashes[previous as usize..split as usize]
        );
        chained = restore(&chained);
        previous = split;
    }
    assert_eq!(
        chained.checkpoint().unwrap().to_json_pretty().unwrap(),
        final_save
    );
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "factory-guard-{}-{}",
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
fn guard_two_fresh_cli_processes_and_serialized_resume_chain_match_absolute_tick_hashes() {
    let trace = trace();
    let dir = Scratch::new();
    let scenario_file = dir.path("scenario.json");
    let commands = dir.path("commands.json");
    std::fs::write(&scenario_file, scenario().to_json_pretty()).unwrap();
    std::fs::write(&commands, trace.commands.to_json_pretty()).unwrap();
    let a = dir.path("a.hashes");
    let b = dir.path("b.hashes");
    for output in [&a, &b] {
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
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
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
    for &split in trace.boundaries.iter().filter(|&&tick| tick > 0) {
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
