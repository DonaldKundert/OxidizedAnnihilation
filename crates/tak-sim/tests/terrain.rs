//! Terrain in movement on a synthetic map (no game data): a goal behind an
//! unbroken wall is unreachable, so the unit stops at the nearest reachable
//! point and drops the order; water does not block; ground height and
//! terrain flags follow the unit into scripts and the hash.

use tak_commands::{LoggedCommand, PlayerId, QueueMode, SimCommand, Tick, UnitId, UnitSet};
use tak_math::{Fx, Vec2};
use tak_sim::map::{TerrainGrid, terrain};
use tak_sim::testing::example_scenario;
use tak_sim::{Scenario, ScriptCtx, ScriptHost, Sim};

const U0: UnitId = UnitId::new(0, 0);

/// Player 0 starts at (256, 256) on a 2048 map. A wall of steep cells runs
/// down column 30 (x in [480, 496)); the area x in [320, 400) is water.
fn walled() -> Scenario {
    let mut sc = example_scenario(1, 2);
    let (cols, rows) = (128u32, 128u32);
    let mut flags = vec![0u8; (cols * rows) as usize];
    let mut heights = vec![40u8; (cols * rows) as usize];
    for r in 0..rows {
        flags[(r * cols + 30) as usize] = terrain::STEEP;
        for c in 20..25 {
            flags[(r * cols + c) as usize] = terrain::WATER;
            heights[(r * cols + c) as usize] = 10;
        }
    }
    sc.map.heights.heights = heights;
    sc.map.terrain = Some(TerrainGrid {
        cols,
        rows,
        cell_size: 16,
        flags,
    });
    sc
}

fn mv(tick: u32, x: i32, y: i32, queue: QueueMode, seq: u32) -> LoggedCommand {
    LoggedCommand {
        tick: Tick(tick),
        player: PlayerId(0),
        seq,
        command: SimCommand::Move {
            units: UnitSet::single(U0),
            to: Vec2::from_ints(x, y),
            queue,
        },
    }
}

#[test]
fn walls_halt_units_and_drop_the_order() {
    let sc = walled();
    sc.validate().unwrap();
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    sim.step(&[mv(1, 900, 256, QueueMode::Replace, 0)]);
    let mut saw_water = false;
    for _ in 0..2000 {
        let u = sim.world().unit(U0).unwrap();
        assert!(u.pos.x < Fx::from_int(480), "crossed the wall: {:?}", u.pos);
        saw_water |= u.terrain & terrain::WATER != 0;
        if u.orders.is_empty() {
            break;
        }
        sim.step(&[]);
    }
    let u = sim.world().unit(U0).unwrap().clone();
    assert!(saw_water, "walked through the (passable) water strip");
    assert!(u.orders.is_empty(), "the blocked order was dropped");
    assert_eq!(u.path.outcome, tak_sim::pathfinding::Outcome::Unreachable);
    assert_eq!(u.speed, Fx::ZERO);
    // The 2x2 footprint (centred on the unit) stops just short of the wall
    // cell at x = 480.
    assert!(
        u.pos.x > Fx::from_int(456),
        "halted right at the wall: {:?}",
        u.pos
    );
    assert_eq!(u.ground, Fx::from_int(40));
    assert_eq!(u.terrain, 0);
    // It stays put.
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert_eq!(sim.world().unit(U0).unwrap().pos, u.pos);

    // A later order away from the wall still works.
    let t = sim.tick().0;
    sim.step(&[mv(t, 340, 300, QueueMode::Replace, 0)]);
    for _ in 0..2000 {
        if sim.world().unit(U0).unwrap().orders.is_empty() {
            break;
        }
        sim.step(&[]);
    }
    let u = sim.world().unit(U0).unwrap();
    assert!(u.pos.distance(Vec2::from_ints(340, 300)) <= tak_sim::systems::ARRIVAL_TOLERANCE);
    assert_eq!(u.ground, Fx::from_int(10), "standing in the water strip");
    assert_eq!(u.terrain, terrain::WATER);
}

#[test]
fn blocked_waypoint_only_drops_that_waypoint() {
    let mut sim = Sim::new(&walled()).unwrap();
    sim.step(&[]);
    sim.step(&[
        mv(1, 900, 256, QueueMode::Replace, 0),
        mv(1, 256, 700, QueueMode::Append, 1),
    ]);
    for _ in 0..4000 {
        if sim.world().unit(U0).unwrap().orders.is_empty() {
            break;
        }
        sim.step(&[]);
    }
    let u = sim.world().unit(U0).unwrap();
    assert!(u.pos.distance(Vec2::from_ints(256, 700)) <= tak_sim::systems::ARRIVAL_TOLERANCE);
}

/// Records what scripts see.
#[derive(Default)]
struct Seen(Vec<ScriptCtx>);

impl ScriptHost for Seen {
    fn on_tick(&mut self, ctx: &ScriptCtx) {
        if ctx.unit == U0 {
            self.0.push(*ctx);
        }
    }
}

#[test]
fn terrain_reaches_scripts_and_the_hash() {
    let flat = example_scenario(1, 2);
    let walled = walled();
    let log = tak_commands::CommandLog::new();
    let a = Sim::new(&flat).unwrap().run(&log, 3);
    let b = Sim::new(&walled).unwrap().run(&log, 3);
    assert!(a.iter().zip(&b).all(|(x, y)| x != y), "terrain is hashed");

    let mut sc = walled;
    // Start player 0 inside the water strip.
    sc.map.start_positions[0] = tak_sim::StartPos { x: 340, y: 256 };
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    let unit = sim.world().unit(U0).unwrap();
    assert_eq!(unit.terrain, terrain::WATER);
    assert_eq!(unit.ground, Fx::from_int(10));
}

#[test]
fn script_ctx_carries_ground_and_terrain() {
    let mut sc = walled();
    sc.map.start_positions[0] = tak_sim::StartPos { x: 340, y: 256 };
    // A host that records contexts; Sim owns it, so check via a shared log.
    use std::cell::RefCell;
    use std::rc::Rc;
    struct Shared(Rc<RefCell<Seen>>);
    impl ScriptHost for Shared {
        fn on_tick(&mut self, ctx: &ScriptCtx) {
            self.0.borrow_mut().on_tick(ctx);
        }
    }
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut sim = Sim::with_scripts(&sc, Box::new(Shared(Rc::clone(&seen)))).unwrap();
    sim.step(&[]);
    sim.step(&[mv(1, 256, 256, QueueMode::Replace, 0)]);
    for _ in 0..200 {
        sim.step(&[]);
    }
    let seen = seen.borrow();
    assert_eq!(seen.0[0].terrain, terrain::WATER);
    assert_eq!(seen.0[0].ground_height, Fx::from_int(10));
    assert!(seen.0.iter().any(|c| c.speed > Fx::ZERO && c.moving));
    let last = seen.0.last().unwrap();
    assert_eq!((last.terrain, last.ground_height), (0, Fx::from_int(40)));
}
