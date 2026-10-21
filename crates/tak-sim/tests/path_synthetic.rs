//! Pathfinding on synthetic maps (no game data): routing through a gap,
//! unreachable goals, patrols, separation, group moves, the step budget,
//! and identical hashes across two processes.

use tak_commands::{
    CommandLog, LoggedCommand, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Angle, Fx, Rng, Vec2};
use tak_sim::map::{TerrainGrid, terrain};
use tak_sim::pathfinding::{Outcome, STEPS_PER_TICK};
use tak_sim::systems::ARRIVAL_TOLERANCE;
use tak_sim::testing::example_scenario;
use tak_sim::{Scenario, Sim, WorldPos};

const U0: UnitId = UnitId::new(0, 0);
const P0: PlayerId = PlayerId(0);

/// 2048 x 2048, 128 x 128 cells. A steep wall runs down column 30
/// (x in [480, 496)) with a gap at rows 60..66 (y in [960, 1056)); a
/// second wall down column 70 (x in [1120, 1136)) has no gap. Plus a box
/// of blocked cells around (1600, 400) with an open interior.
fn maze() -> Scenario {
    let mut sc = example_scenario(1, 2);
    let (cols, rows) = (128u32, 128u32);
    let mut flags = vec![0u8; (cols * rows) as usize];
    for r in 0..rows {
        if !(60..66).contains(&r) {
            flags[(r * cols + 30) as usize] = terrain::STEEP;
        }
        flags[(r * cols + 70) as usize] = terrain::STEEP;
    }
    for i in 95..106 {
        for &(c, r) in &[(i, 20), (i, 30), (95, i - 75), (105, i - 75)] {
            flags[(r * cols + c) as usize] = terrain::FEATURE;
        }
    }
    sc.map.terrain = Some(TerrainGrid {
        cols,
        rows,
        cell_size: 16,
        flags,
    });
    sc
}

fn wall_cell(p: WorldPos) -> bool {
    let (x, y) = (p.x.to_int_floor(), p.y.to_int_floor());
    ((480..496).contains(&x) && !(960..1056).contains(&y)) || (1120..1136).contains(&x)
}

fn cmd(tick: u32, seq: u32, command: SimCommand) -> LoggedCommand {
    LoggedCommand {
        tick: Tick(tick),
        player: P0,
        seq,
        command,
    }
}

fn mv(unit: UnitId, to: WorldPos) -> SimCommand {
    SimCommand::Move {
        units: UnitSet::single(unit),
        to,
        queue: QueueMode::Replace,
    }
}

fn started(sc: &Scenario) -> Sim {
    let mut sim = Sim::new(sc).unwrap();
    sim.step(&[]);
    sim
}

fn run_until(sim: &mut Sim, limit: u32, mut done: impl FnMut(&Sim) -> bool) -> Option<u32> {
    for i in 0..limit {
        if done(sim) {
            return Some(i);
        }
        sim.step(&[]);
    }
    None
}

#[test]
fn routes_through_the_gap_in_a_wall() {
    let mut sim = started(&maze());
    let to = Vec2::from_ints(900, 256);
    let t = sim.tick().0;
    sim.step(&[cmd(t, 0, mv(U0, to))]);
    let mut crossed_at = None;
    let took = run_until(&mut sim, 5000, |s| {
        let u = s.world().unit(U0).unwrap();
        assert!(!wall_cell(u.pos), "inside the wall at {:?}", u.pos);
        if crossed_at.is_none() && u.pos.x >= Fx::from_int(488) {
            crossed_at = Some(u.pos);
        }
        u.orders.is_empty()
    })
    .expect("finishes");
    let u = sim.world().unit(U0).unwrap();
    assert_eq!(u.path.outcome, Outcome::Arrived);
    assert!(u.pos.distance(to) <= ARRIVAL_TOLERANCE);
    let y = crossed_at.unwrap().y.to_int_floor();
    assert!((960..1056).contains(&y), "crossed the wall at y = {y}");
    // Down ~750 units to the gap, across, back up: well over the straight 644.
    assert!(took > 900, "took {took} ticks");
}

#[test]
fn unreachable_goals_stop_at_the_nearest_reachable_point() {
    let mut sim = started(&maze());
    // Behind the gapless second wall.
    let to = Vec2::from_ints(1400, 1000);
    let t = sim.tick().0;
    sim.step(&[cmd(t, 0, mv(U0, to))]);
    run_until(&mut sim, 8000, |s| {
        s.world().unit(U0).unwrap().orders.is_empty()
    })
    .expect("finishes");
    let u = sim.world().unit(U0).unwrap();
    assert_eq!(u.path.outcome, Outcome::Unreachable);
    assert_eq!(u.speed, Fx::ZERO);
    let x = u.pos.x.to_int_floor();
    assert!((1090..1120).contains(&x), "stopped at {:?}", u.pos);
    assert!(
        (u.pos.y - Fx::from_int(1000)).abs() < Fx::from_int(16),
        "{:?}",
        u.pos
    );
    assert_eq!(sim.world().pathing.stats.unreachable, 1);

    // A goal inside the closed box is unreachable too; the unit stops at
    // its wall.
    let t = sim.tick().0;
    sim.step(&[cmd(t, 0, mv(U0, Vec2::from_ints(1600, 400)))]);
    run_until(&mut sim, 8000, |s| {
        s.world().unit(U0).unwrap().orders.is_empty()
    })
    .expect("finishes");
    let u = sim.world().unit(U0).unwrap();
    assert_eq!(u.path.outcome, Outcome::Unreachable);
    assert!(u.pos.x < Fx::from_int(1120));
}

#[test]
fn patrol_legs_rotate_forever() {
    let mut sim = started(&maze());
    let start = sim.world().unit(U0).unwrap().pos;
    let far = Vec2::from_ints(400, 600);
    let t = sim.tick().0;
    sim.step(&[cmd(
        t,
        0,
        SimCommand::Patrol {
            units: UnitSet::single(U0),
            to: far,
            queue: QueueMode::Replace,
        },
    )]);
    let mut visits = [0u32; 2];
    let mut near = [false; 2];
    for _ in 0..6000 {
        let u = sim.world().unit(U0).unwrap();
        assert_eq!(u.orders.len(), 2, "two patrol legs stay queued");
        for (i, p) in [far, start].iter().enumerate() {
            let close = u.pos.distance(*p) <= Fx::from_int(4);
            if close && !near[i] {
                visits[i] += 1;
            }
            near[i] = close;
        }
        sim.step(&[]);
    }
    assert!(visits[0] >= 3 && visits[1] >= 3, "visits {visits:?}");
}

#[test]
fn separation_spreads_stacked_units() {
    let mut sim = started(&example_scenario(1, 1));
    let p = Vec2::from_ints(1000, 1000);
    let ids: Vec<_> = (0..9)
        .map(|_| {
            sim.world_mut()
                .spawn(UnitDefId(0), P0, p, Angle::ZERO)
                .unwrap()
        })
        .collect();
    for _ in 0..300 {
        sim.step(&[]);
    }
    let w = sim.world();
    for (i, &a) in ids.iter().enumerate() {
        for &b in &ids[i + 1..] {
            let d = w.unit(a).unwrap().pos.distance(w.unit(b).unwrap().pos);
            assert!(d >= Fx::from_int(28), "{a:?} and {b:?} only {d:?} apart");
        }
    }
}

#[test]
fn a_group_sent_to_one_point_all_finish() {
    let mut sim = started(&maze());
    let ids: Vec<_> = (0..9)
        .map(|k| {
            let p = Vec2::from_ints(200 + 40 * (k % 3), 200 + 40 * (k / 3));
            sim.world_mut()
                .spawn(UnitDefId(0), P0, p, Angle::ZERO)
                .unwrap()
        })
        .collect();
    let to = Vec2::from_ints(300, 900);
    let t = sim.tick().0;
    let cmds: Vec<_> = ids.iter().map(|&id| cmd(t, id.index, mv(id, to))).collect();
    sim.step(&cmds);
    run_until(&mut sim, 4000, |s| {
        ids.iter()
            .all(|&id| s.world().unit(id).unwrap().orders.is_empty())
    })
    .expect("all finish");
    let w = sim.world();
    for &id in &ids {
        let u = w.unit(id).unwrap();
        assert_eq!(u.path.outcome, Outcome::Arrived, "{id:?} at {:?}", u.pos);
        assert!(u.pos.distance(to) <= Fx::from_int(96));
    }
}

/// A busy run: 40 units on the maze, new random targets every 300 ticks,
/// some patrols. Returns the per-tick hashes and the most steps spent in a
/// tick.
fn busy_run(ticks: u32) -> (Vec<u64>, u32) {
    let sc = maze();
    let mut sim = started(&sc);
    let mut rng = Rng::new(77);
    let mut ids = vec![U0];
    while ids.len() < 40 {
        let p = Vec2::from_ints(rng.range_i32(32, 2016), rng.range_i32(32, 2016));
        if !wall_cell(p) {
            ids.push(
                sim.world_mut()
                    .spawn(UnitDefId(0), P0, p, Angle::ZERO)
                    .unwrap(),
            );
        }
    }
    let mut log = CommandLog::new();
    let mut t = 1;
    while t < ticks {
        for &id in &ids {
            let to = Vec2::from_ints(rng.range_i32(0, 2048), rng.range_i32(0, 2048));
            let command = if rng.chance(1, 5) {
                SimCommand::Patrol {
                    units: UnitSet::single(id),
                    to,
                    queue: QueueMode::Replace,
                }
            } else {
                mv(id, to)
            };
            log.record(Tick(t), P0, command);
        }
        t += 300;
    }
    let mut hashes = Vec::new();
    let mut max_steps = 0;
    while sim.tick().0 < ticks {
        let tick = sim.tick();
        hashes.push(sim.step(log.for_tick(tick)));
        max_steps = max_steps.max(sim.world().pathing.stats.steps_last_tick);
    }
    (hashes, max_steps)
}

#[test]
fn busy_runs_respect_the_budget_and_repeat_in_process() {
    let (a, max_steps) = busy_run(2000);
    assert!(max_steps <= STEPS_PER_TICK);
    assert_eq!(
        max_steps, STEPS_PER_TICK,
        "the budget was actually the limit"
    );
    let (b, _) = busy_run(2000);
    assert_eq!(a, b);
}

/// Runs [`busy_run`] in a child process (this test binary, re-executed)
/// and compares its hashes with this process's.
#[test]
fn pathing_hashes_match_across_processes() {
    const TICKS: u32 = 3000;
    if std::env::var_os("TAK_PATH_CHILD").is_some() {
        let (h, _) = busy_run(TICKS);
        let line: Vec<String> = h.iter().map(|v| format!("{v:016x}")).collect();
        println!("HASHES {}", line.join(","));
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            "pathing_hashes_match_across_processes",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("TAK_PATH_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let line = stdout
        .lines()
        .find_map(|l| l.split_once("HASHES ").map(|(_, h)| h))
        .expect("child printed its hashes");
    let theirs: Vec<u64> = line
        .split(',')
        .map(|v| u64::from_str_radix(v, 16).unwrap())
        .collect();
    let (ours, _) = busy_run(TICKS);
    assert_eq!(theirs.len(), TICKS as usize - 1);
    assert_eq!(ours, theirs);
}
