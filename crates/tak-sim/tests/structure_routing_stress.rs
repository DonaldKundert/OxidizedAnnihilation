//! Synthetic lifecycle/route soak. External fixture events are applied identically
//! to both sims; ordinary movement is submitted as commands. No retail data.
use tak_commands::{CommandLog, PlayerId, QueueMode, SimCommand, UnitDefId, UnitId, UnitSet};
use tak_math::{Angle, Fx, Rng, Vec2};
use tak_sim::map::TerrainGrid;
use tak_sim::pathfinding::{self, CellRect};
use tak_sim::{Scenario, Sim};

const WALL: UnitDefId = UnitDefId(2);
const P0: PlayerId = PlayerId(0);
const FEATURE: CellRect = CellRect {
    x: 60,
    z: 45,
    w: 4,
    h: 10,
};

fn scenario(seed: u64) -> Scenario {
    let mut s = tak_sim::testing::example_scenario(seed, 1);
    s.map.terrain = Some(TerrainGrid {
        cols: 128,
        rows: 128,
        cell_size: 16,
        flags: vec![0; 128 * 128],
    });
    let d = &mut s.unit_defs[0];
    d.footprintx = 1;
    d.footprintz = 1;
    d.maxvelocity = Fx::from_int(2);
    d.acceleration = Fx::ONE;
    d.brakerate = Fx::ONE;
    d.turnrate = 4096;
    let mut wall = d.clone();
    wall.name = "synthetic stress wall".into();
    wall.maxvelocity = Fx::ZERO;
    wall.footprintx = 2;
    wall.footprintz = 8;
    s.unit_defs.push(wall);
    s
}
fn inside(r: CellRect, x: i32, z: i32) -> bool {
    x >= r.x && x < r.x + r.w && z >= r.z && z < r.z + r.h
}

fn run(seed: u64, ticks: u32) -> (u64, u64) {
    let sc = scenario(seed);
    let mut a = Sim::new(&sc).unwrap();
    let mut b = Sim::new(&Scenario::from_json(&sc.to_json_pretty()).unwrap()).unwrap();
    a.step(&[]);
    b.step(&[]);
    let mut movers = vec![UnitId::new(0, 0)];
    for i in 0..99 {
        let p = Vec2::from_ints(80 + i % 10 * 32, 80 + i / 10 * 32);
        let id = a.spawn_unit(UnitDefId(0), P0, p, Angle::ZERO).unwrap();
        assert_eq!(b.spawn_unit(UnitDefId(0), P0, p, Angle::ZERO), Some(id));
        movers.push(id);
    }
    let mut walls = Vec::new();
    for z in 0..6 {
        for x in 0..5 {
            let r = CellRect {
                x: 20 + x * 20,
                z: 20 + z * 16,
                w: 2,
                h: 8,
            };
            let p = Vec2::from_ints(r.x * 16 + 16, r.z * 16 + 64);
            let id = a.spawn_unit(WALL, P0, p, Angle::ZERO).unwrap();
            assert_eq!(b.spawn_unit(WALL, P0, p, Angle::ZERO), Some(id));
            walls.push((id, r));
        }
    }
    let mut rng = Rng::new(seed ^ 0x5a17);
    let mut feature = false;
    let mut checks = 0;
    for t in 0..ticks {
        if t % 75 == 0 {
            let k = rng.below(walls.len() as u32) as usize;
            let (old, _) = walls.remove(k);
            assert!(a.world_mut().despawn(old).is_some());
            assert!(b.world_mut().despawn(old).is_some());
            let r = CellRect {
                x: 5 + rng.below(115) as i32,
                z: 5 + rng.below(110) as i32,
                w: 2,
                h: 8,
            };
            let p = Vec2::from_ints(r.x * 16 + 16, r.z * 16 + 64);
            let id = a.spawn_unit(WALL, P0, p, Angle::ZERO).unwrap();
            assert_eq!(b.spawn_unit(WALL, P0, p, Angle::ZERO), Some(id));
            assert!(a.world_mut().despawn(old).is_none());
            assert!(b.world_mut().despawn(old).is_none());
            walls.push((id, r));
        }
        if t % 101 == 0 {
            for sim in [&mut a, &mut b] {
                if feature {
                    pathfinding::remove_obstacle(sim.world_mut(), FEATURE);
                } else {
                    pathfinding::add_obstacle(sim.world_mut(), FEATURE);
                }
            }
            feature = !feature;
        }
        let mut log = CommandLog::new();
        if t % 120 == 0 {
            for &id in &movers {
                let to = Vec2::from_ints(32 + rng.below(1984) as i32, 32 + rng.below(1984) as i32);
                log.record(
                    a.tick(),
                    P0,
                    SimCommand::Move {
                        units: UnitSet::single(id),
                        to,
                        queue: QueueMode::Replace,
                    },
                );
            }
        }
        assert_eq!(
            a.step(log.for_tick(a.tick())),
            b.step(log.for_tick(b.tick())),
            "seed {seed} tick {t}"
        );
        assert!(a.world().pathing.stats.steps_last_tick <= pathfinding::STEPS_PER_TICK);
        if t % 25 == 0 {
            let grid = a.world().pathing.grid_for(UnitDefId(0)).unwrap();
            for _ in 0..128 {
                let x = rng.below(128) as i32;
                let z = rng.below(128) as i32;
                // Independent point-in-rectangle oracle, using fixture events,
                // without reading the path state's cached stamps or counters.
                let blocked = walls.iter().any(|(_, r)| inside(*r, x, z))
                    || (feature && inside(FEATURE, x, z));
                assert_eq!(
                    grid.cell_tier_at(Vec2::from_ints(x * 16 + 8, z * 16 + 8)) == 0,
                    blocked,
                    "seed {seed} tick {t} cell {x},{z}"
                );
                checks += 1;
            }
        }
    }
    for sim in [&mut a, &mut b] {
        for &(id, _) in &walls {
            sim.world_mut().despawn(id).unwrap();
        }
        if feature {
            pathfinding::remove_obstacle(sim.world_mut(), FEATURE);
        }
    }
    assert_eq!(a.step(&[]), b.step(&[]));
    let grid = a.world().pathing.grid_for(UnitDefId(0)).unwrap();
    for z in 0..128 {
        for x in 0..128 {
            assert_ne!(
                grid.cell_tier_at(Vec2::from_ints(x * 16 + 8, z * 16 + 8)),
                0,
                "ghost blocker after teardown"
            );
            checks += 1;
        }
    }
    (u64::from(ticks + 2) * 2, checks)
}

#[test]
fn bounded_structure_lifecycle_routes_and_occupancy_oracle() {
    for seed in [1, u64::MAX] {
        run(seed, 250);
    }
}

#[test]
#[ignore = "extended 100-mover/30-structure lifecycle and occupancy oracle; run locally"]
fn extended_structure_lifecycle_soak() {
    let seeds = std::env::var("OA_STRUCTURE_SOAK_SEEDS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(64)
        .clamp(1, 256);
    let ticks = std::env::var("OA_STRUCTURE_SOAK_TICKS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(2500)
        .clamp(250, 10000);
    let began = std::time::Instant::now();
    let mut simulated = 0;
    let mut checks = 0;
    let max_seconds = std::env::var("OA_STRUCTURE_SOAK_MAX_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    for seed in 0..seeds {
        if seed > 0 && max_seconds.is_some_and(|limit| began.elapsed().as_secs() >= limit) {
            eprintln!(
                "time-bounded soak stopped after {seed} complete seeds; {simulated} simulation ticks, {checks} occupancy checks"
            );
            break;
        }
        let (n, c) = run(u64::from(seed) ^ 0xc17e, ticks);
        simulated += n;
        checks += c;
        eprintln!(
            "structure soak {}/{seeds}: {simulated} simulation ticks, {checks} occupancy checks, elapsed {:?}",
            seed + 1,
            began.elapsed()
        );
    }
}
