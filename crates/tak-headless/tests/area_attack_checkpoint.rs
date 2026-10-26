//! Synthetic, command-only replay of the actual explicit Attack box mapper.
//! Initial units are scenario data; combat, movement and stale-target promotion
//! use ordinary logged commands, never mutations of an already-running world.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tak_commands::{
    CommandLog, FireState, PlayerId, QueueMode, Scope, SimCommand, Target, Tick, UnitDefId, UnitId,
    UnitSet,
};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_input::{
    Action, Button, EventKind, InputEvent, Key, Mapper, Output, Profile, UnitInfo, WorldView,
};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::campaign::InitialUnit;
use tak_sim::combat::{CombatOrder, DamageStats, WeaponKind, WeaponStats};
use tak_sim::victory::DefeatRule;
use tak_sim::{Order, Scenario, Sim, World};

const OWNER: PlayerId = PlayerId(0);
const ARMY: [UnitId; 2] = [UnitId::new(0, 0), UnitId::new(1, 0)];
const TARGETS: [UnitId; 3] = [UnitId::new(2, 0), UnitId::new(3, 0), UnitId::new(4, 0)];
const HELPER: UnitId = UnitId::new(5, 0);
const SENTINEL: UnitId = UnitId::new(6, 0);
const TICKS: u32 = 360;
const BIN: &str = env!("CARGO_BIN_EXE_tak-headless");

fn army() -> UnitSet {
    UnitSet::from_iter(ARMY)
}

fn scenario() -> Scenario {
    let mut scenario = tak_sim::testing::example_scenario(0xa7ea_2026, 2);
    scenario.victory.defeat = DefeatRule::AllUnitsKilled;
    let mut soldier = tak_sim::testing::monarch_def("SYNTHETIC_AREA_ATTACK_SOLDIER");
    soldier.maxvelocity = Fx::from_int(8);
    soldier.acceleration = Fx::from_int(2);
    soldier.brakerate = Fx::from_int(2);
    soldier.turnrate = 8192;
    soldier.footprintx = 1;
    soldier.footprintz = 1;
    soldier.combat.max_damage = 100;
    soldier.combat.radius = 8;
    soldier.combat.weapons.push(WeaponStats {
        name: "SYNTHETIC_SHORT_RANGE_SHOT".into(),
        kind: WeaponKind::RemoteEffect,
        range: 48,
        reload_ticks: 5,
        aim_tolerance: Some(u16::MAX),
        damage: DamageStats {
            default: 20,
            ..Default::default()
        },
        ..Default::default()
    });
    let mut target = soldier.clone();
    target.name = "SYNTHETIC_AREA_ATTACK_TARGET".into();
    target.maxvelocity = Fx::from_int(2);
    target.combat.weapons.clear();
    let mut helper = soldier.clone();
    helper.name = "SYNTHETIC_AREA_ATTACK_HELPER".into();
    helper.maxvelocity = Fx::ZERO;
    helper.combat.weapons[0].range = 4096;
    helper.combat.weapons[0].damage.default = 1000;
    scenario.unit_defs = vec![soldier, target, helper];
    scenario.initial_units = Some(
        [
            (0, 0, 160, 224),
            (0, 0, 160, 256),
            (1, 1, 440, 224),
            (1, 1, 620, 224),
            (1, 1, 800, 384),
            (2, 0, 1600, 1600),
            // This living enemy keeps victory/defeat sweeps out of the trace.
            (1, 1, 1900, 1900),
        ]
        .into_iter()
        .map(|(def, owner, x, y)| InitialUnit {
            def: UnitDefId(def),
            owner: PlayerId(owner),
            pos: Vec2::from_ints(x, y),
            heading: Angle::ZERO,
            health_percentage: 100,
            mana_percentage: 0,
            kills: 0,
        })
        .collect(),
    );
    Scenario::from_json(&scenario.to_json_pretty()).unwrap()
}

// A read-only, fully visible synthetic world adapter. Actual client fog and
// picking contracts have their own adapter tests; this fixture tests replay.
struct View<'a>(&'a World);
impl WorldView for View<'_> {
    fn local_player(&self) -> PlayerId {
        OWNER
    }
    fn is_enemy(&self, owner: PlayerId) -> bool {
        self.0.are_hostile(OWNER, owner)
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo)) {
        let units: Vec<_> = self.0.units().collect();
        // Do not accidentally supply the mapper's required ordering for it.
        for (id, unit) in units.into_iter().rev() {
            f(&UnitInfo {
                id,
                pos: unit.pos,
                radius: Fx::from_int(8),
                owner: unit.owner,
                def: unit.def,
                selectable: !unit.combat.is_dying(),
                builder: None,
                under_construction: !unit.build.is_complete(),
            });
        }
    }
}

fn mapped_attacks(sim: &Sim, append: bool) -> Vec<SimCommand> {
    let before = sim.state_hash();
    let view = View(sim.world());
    let tick = sim.tick();
    let mut mapper = Mapper::new(Profile::classic_tak());
    mapper.perform(
        Action::SelectAll {
            scope: Scope::OnScreen,
        },
        &view,
        tick,
    );
    assert_eq!(
        mapper.selection(),
        &army(),
        "offscreen helper is not selected"
    );
    mapper.perform(Action::AttackMode, &view, tick);
    let mut events = Vec::new();
    if append {
        events.push(EventKind::KeyDown {
            key: Key::new("shift"),
        });
    }
    events.extend([
        EventKind::PointerMove { x: 400, y: 180 },
        EventKind::ButtonDown {
            button: Button::MouseLeft,
        },
        EventKind::PointerMove { x: 840, y: 440 },
        EventKind::ButtonUp {
            button: Button::MouseLeft,
        },
        EventKind::Frame,
    ]);
    let mut commands = Vec::new();
    for (index, event) in events.into_iter().enumerate() {
        for output in mapper.handle(
            &InputEvent::new(100 + index as u32 * 10, event),
            &view,
            tick,
        ) {
            if let Output::Sim {
                tick: stamped,
                command,
            } = output
            {
                assert_eq!(stamped, tick);
                commands.push(command);
            }
        }
    }
    assert_eq!(
        mapper.selection(),
        &army(),
        "targeting must preserve the army"
    );
    assert_eq!(
        sim.state_hash(),
        before,
        "input inspection must be read-only"
    );
    assert_eq!(
        commands,
        TARGETS
            .into_iter()
            .enumerate()
            .map(|(index, target)| SimCommand::Attack {
                units: army(),
                target: Target::Unit(target),
                queue: if append || index > 0 {
                    QueueMode::Append
                } else {
                    QueueMode::Replace
                },
            })
            .collect::<Vec<_>>()
    );
    commands
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

struct Trace {
    scenario: Scenario,
    log: CommandLog,
    hashes: Vec<u64>,
    final_save: String,
    combat_split: u32,
}

fn exercise(append: bool) -> Trace {
    let scenario = scenario();
    let source = CheckpointSource::synthetic(scenario.clone());
    let mut session = ReplaySession::new(source.clone(), None).unwrap();
    let mut log = CommandLog::new();
    // HoldFire prevents autonomous acquisition from disguising a missing
    // area order. Explicit Attack still fires through the normal combat path.
    log.record(
        Tick(0),
        OWNER,
        SimCommand::SetFireState {
            units: UnitSet::from_iter(ARMY.into_iter().chain([HELPER])),
            state: FireState::HoldFire,
        },
    );
    let waypoint = Vec2::from_ints(256, 256);
    log.record(
        Tick(0),
        OWNER,
        SimCommand::Move {
            units: army(),
            to: waypoint,
            queue: QueueMode::Replace,
        },
    );
    let mut expected_hashes = session.advance(&log, 1).unwrap();
    for target in TARGETS {
        assert_eq!(
            session.sim().world().unit(target).unwrap().combat.health,
            100
        );
    }
    for command in mapped_attacks(session.sim(), append) {
        log.record(Tick(1), OWNER, command);
    }
    // The captured first target dies through an ordinary combat command,
    // before either selected soldier can get into its short weapon range.
    log.record(
        Tick(1),
        OWNER,
        SimCommand::Attack {
            units: UnitSet::single(HELPER),
            target: Target::Unit(TARGETS[0]),
            queue: QueueMode::Replace,
        },
    );
    // A unit target is not a frozen box coordinate: the third target escapes
    // the original rectangle, and the army must follow that same handle.
    log.record(
        Tick(20),
        PlayerId(1),
        SimCommand::Move {
            units: UnitSet::single(TARGETS[2]),
            to: Vec2::from_ints(1040, 480),
            queue: QueueMode::Replace,
        },
    );
    let decoded = CommandLog::from_json(&log.to_json_pretty()).unwrap();
    assert_eq!(
        decoded, log,
        "target handles and queue modes must round-trip"
    );
    let mut cuts = BTreeSet::from([0, 1, 2, 20, 21, TICKS]);
    let mut seen_targets = [BTreeSet::new(), BTreeSet::new()];
    let mut last_orders = [None, None];
    let mut first_damage = BTreeSet::new();
    let mut deaths = BTreeSet::new();
    let mut combat_split = None;
    let mut stale_waited_in_queue = false;
    for tick in 1..TICKS {
        expected_hashes.extend(session.advance(&decoded, 1).unwrap());
        let world = session.sim().world();
        for (index, id) in ARMY.into_iter().enumerate() {
            let unit = world.unit(id).unwrap();
            if tick == 1 {
                if append {
                    assert_eq!(unit.orders.front(), Some(&Order::Move { to: waypoint }));
                    assert!(unit.combat.order.is_none());
                    assert_eq!(
                        unit.orders
                            .iter()
                            .filter_map(|order| match order {
                                Order::Attack { target } => Some(*target),
                                _ => None,
                            })
                            .collect::<Vec<_>>(),
                        TARGETS.map(Target::Unit)
                    );
                } else {
                    assert!(!unit.orders.contains(&Order::Move { to: waypoint }));
                    assert_eq!(
                        unit.combat.order,
                        Some(CombatOrder::Attack(Target::Unit(TARGETS[0])))
                    );
                }
            }
            if unit.combat.order != last_orders[index] {
                cuts.extend([tick, tick + 1]);
                last_orders[index] = unit.combat.order;
            }
            if let Some(CombatOrder::Attack(Target::Unit(target))) = unit.combat.order {
                seen_targets[index].insert(target);
            }
            let first_dead = world
                .unit(TARGETS[0])
                .is_none_or(|target| target.combat.is_dying());
            stale_waited_in_queue |= first_dead
                && unit.orders.contains(&Order::Attack {
                    target: Target::Unit(TARGETS[0]),
                });
            assert!(
                unit.combat.attack_move.is_none(),
                "area Attack is not attack-move"
            );
            assert_eq!(unit.fire_state, FireState::HoldFire);
        }
        for target in TARGETS {
            if let Some(unit) = world.unit(target) {
                if unit.combat.health < 100 && first_damage.insert(target) {
                    cuts.extend([tick, tick + 1]);
                    if target != TARGETS[0] {
                        assert!(
                            unit.combat
                                .last_attacker
                                .is_some_and(|id| ARMY.contains(&id))
                        );
                        if !unit.combat.is_dying() {
                            combat_split.get_or_insert(tick + 1);
                        }
                    }
                    if target == TARGETS[2] {
                        assert!(
                            unit.pos.x > Fx::from_int(840),
                            "the live unit target must have left the original box"
                        );
                    }
                }
            } else if deaths.insert(target) {
                cuts.extend([tick, tick + 1]);
            }
        }
    }
    assert_eq!(first_damage.len(), TARGETS.len());
    assert_eq!(
        deaths.len(),
        TARGETS.len(),
        "all three targets must really die"
    );
    assert_eq!(
        stale_waited_in_queue, append,
        "Shift keeps the dead first target behind prior work until promotion"
    );
    let world = session.sim().world();
    for (index, id) in ARMY.into_iter().enumerate() {
        assert!(seen_targets[index].contains(&TARGETS[1]));
        assert!(seen_targets[index].contains(&TARGETS[2]));
        let unit = world.unit(id).unwrap();
        assert!(unit.orders.is_empty() && unit.combat.order.is_none());
        assert!(
            unit.pos.x > Fx::from_int(900),
            "both soldiers must travel to the escaped target"
        );
    }
    assert_eq!(world.unit(HELPER).unwrap().combat.kills, 1);
    assert_eq!(
        ARMY.into_iter()
            .map(|id| world.unit(id).unwrap().combat.kills)
            .sum::<u32>(),
        2
    );
    assert_eq!(world.unit(SENTINEL).unwrap().combat.health, 100);
    assert_eq!(world.combat.deaths, 3);

    // Fresh direct Sim stepping independently consumes the serialized log.
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
            "suffix diverged at split {split}, append={append}"
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
    Trace {
        scenario,
        log: decoded,
        hashes: expected_hashes,
        final_save,
        combat_split: combat_split.expect("checkpoint must cut through a live damaged target"),
    }
}

// One test owns this directory. File-backed output avoids unread-pipe
// deadlocks; every fresh process is independently bounded by a real timeout.
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("area-attack-{}", std::process::id()));
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
                    std::thread::sleep(Duration::from_millis(10))
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
fn mapped_area_attacks_promote_dead_targets_and_replay_through_combat_checkpoints() {
    // Both queue contracts get command-only combat and boundary restores.
    // Cross-process coverage uses only the stronger prior-work/Append trace.
    exercise(false);
    let trace = exercise(true);
    let scratch = Scratch::new();
    let scenario = scratch.path("scenario.json");
    let log = scratch.path("commands.json");
    std::fs::write(&scenario, trace.scenario.to_json_pretty()).unwrap();
    std::fs::write(&log, trace.log.to_json_pretty()).unwrap();
    let a = scratch.path("a.hashes");
    let b = scratch.path("b.hashes");
    for output in [&a, &b] {
        scratch.run(&[
            "run",
            "--scenario",
            path(&scenario),
            "--log",
            path(&log),
            "--ticks",
            &TICKS.to_string(),
            "--hashes",
            path(output),
        ]);
        assert_eq!(hashes(output, 0), trace.hashes);
    }
    assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
    let prefix = scratch.path("prefix.json");
    let suffix = scratch.path("suffix.json");
    let output = scratch.path("suffix.hashes");
    scratch.run(&[
        "checkpoint",
        "save",
        "--scenario",
        path(&scenario),
        "--log",
        path(&log),
        "--ticks",
        &trace.combat_split.to_string(),
        "--out",
        path(&prefix),
    ]);
    scratch.run(&[
        "checkpoint",
        "resume",
        "--checkpoint",
        path(&prefix),
        "--log",
        path(&log),
        "--ticks",
        &(TICKS - trace.combat_split).to_string(),
        "--out",
        path(&suffix),
        "--hashes",
        path(&output),
    ]);
    assert_eq!(
        hashes(&output, trace.combat_split),
        trace.hashes[trace.combat_split as usize..]
    );
    assert_eq!(
        Checkpoint::read(&suffix).unwrap().to_json_pretty().unwrap(),
        trace.final_save
    );
}
