//! Structure footprints, yard transitions and lifecycle routing, without retail data.
use tak_commands::{CommandLog, PlayerId, QueueMode, SimCommand, UnitDefId, UnitSet};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::map::TerrainGrid;
use tak_sim::pathfinding::Outcome;
use tak_sim::{Scenario, Sim, UnitId};

const WALKER: UnitDefId = UnitDefId(0);
const WALL: UnitDefId = UnitDefId(2);
const FLYER: UnitDefId = UnitDefId(3);
const P0: PlayerId = PlayerId(0);

fn setup() -> Scenario {
    let mut sc = tak_sim::testing::example_scenario(19, 1);
    sc.map.start_positions[0] = tak_sim::StartPos { x: 500, y: 800 };
    sc.map.terrain = Some(TerrainGrid {
        cols: 128,
        rows: 128,
        cell_size: 16,
        flags: vec![0; 128 * 128],
    });
    let d = &mut sc.unit_defs[0];
    d.footprintx = 1;
    d.footprintz = 1;
    d.maxvelocity = Fx::from_int(2);
    d.acceleration = Fx::ONE;
    d.brakerate = Fx::ONE;
    d.turnrate = 8192;
    let mut wall = d.clone();
    wall.name = "wall".into();
    wall.maxvelocity = Fx::ZERO;
    wall.footprintx = 2;
    wall.footprintz = 16;
    let mut flyer = d.clone();
    flyer.name = "flyer".into();
    flyer.motion.canfly = true;
    sc.unit_defs.push(wall);
    sc.unit_defs.push(flyer);
    sc
}

fn wall(s: &mut Sim) -> UnitId {
    s.spawn_unit(WALL, P0, Vec2::from_ints(800, 800), Angle::ZERO)
        .unwrap()
}
fn start() -> (Sim, UnitId) {
    let mut s = Sim::new(&setup()).unwrap();
    s.step(&[]);
    let w = wall(&mut s);
    s.step(&[]);
    (s, w)
}
fn line_clear(s: &Sim) -> bool {
    s.world()
        .pathing
        .grid_for(WALKER)
        .unwrap()
        .line_of_sight(Vec2::from_ints(500, 800), Vec2::from_ints(1100, 800))
        .0
}

#[test]
fn ground_routes_around_a_living_wall_instead_of_stalling_on_its_body() {
    let (mut s, _) = start();
    assert!(!line_clear(&s), "structure omitted from movement layer");
    let walker = UnitId::new(0, 0);
    let to = Vec2::from_ints(1100, 800);
    let mut log = CommandLog::new();
    log.record(
        s.tick(),
        P0,
        SimCommand::Move {
            units: UnitSet::single(walker),
            to,
            queue: QueueMode::Replace,
        },
    );
    s.step(log.for_tick(s.tick()));
    for _ in 0..2000 {
        let u = s.world().unit(walker).unwrap();
        let x = u.pos.x.to_int_floor();
        let z = u.pos.y.to_int_floor();
        assert!(
            !(784..816).contains(&x) || !(672..928).contains(&z),
            "walker entered wall at {:?}",
            u.pos
        );
        if u.orders.is_empty() {
            break;
        }
        s.step(&[]);
    }
    let u = s.world().unit(walker).unwrap();
    assert_eq!(u.path.outcome, Outcome::Arrived);
    assert!(u.pos.distance(to) <= tak_sim::systems::ARRIVAL_TOLERANCE);
}

#[test]
fn flyers_cross_walls_without_a_ground_layer() {
    let (mut s, _) = start();
    let flyer = s
        .spawn_unit(FLYER, P0, Vec2::from_ints(500, 800), Angle::ZERO)
        .unwrap();
    let mut log = CommandLog::new();
    log.record(
        s.tick(),
        P0,
        SimCommand::Move {
            units: UnitSet::single(flyer),
            to: Vec2::from_ints(1100, 800),
            queue: QueueMode::Replace,
        },
    );
    let mut crossed = false;
    for _ in 0..400 {
        s.step(log.for_tick(s.tick()));
        let u = s.world().unit(flyer).unwrap();
        crossed |= (784..816).contains(&u.pos.x.to_int_floor())
            && (672..928).contains(&u.pos.y.to_int_floor());
    }
    assert!(crossed);
    assert!(s.world().pathing.grid_for(FLYER).is_none());
    assert_eq!(
        s.world().unit(flyer).unwrap().path.outcome,
        Outcome::Arrived
    );
}

#[test]
fn overlapping_structure_and_feature_blockers_survive_independent_removal() {
    let (mut s, first) = start();
    let second = wall(&mut s);
    s.step(&[]);
    assert!(!line_clear(&s));
    s.world_mut().despawn(first).unwrap();
    s.step(&[]);
    assert!(!line_clear(&s), "one removed structure erased its overlap");
    let r = tak_sim::pathfinding::CellRect {
        x: 49,
        z: 42,
        w: 2,
        h: 16,
    };
    tak_sim::pathfinding::add_obstacle(s.world_mut(), r);
    s.world_mut().despawn(second).unwrap();
    s.step(&[]);
    assert!(!line_clear(&s), "structure removal erased a feature");
    tak_sim::pathfinding::remove_obstacle(s.world_mut(), r);
    s.step(&[]);
    assert!(line_clear(&s));
    let reused = wall(&mut s);
    assert_eq!(reused.index, first.index);
    assert_ne!(reused.generation, first.generation);
    s.step(&[]);
    assert!(
        !line_clear(&s),
        "generational slot reuse lost the new structure"
    );
}

#[test]
fn placement_retains_structure_reason_and_can_ignore_its_own_footprint() {
    let (s, wall) = start();
    let pos = s.world().unit(wall).unwrap().pos;
    use tak_sim::construction::placement::{SiteError, check_site};
    assert_eq!(
        check_site(s.world(), WALL, pos, None),
        Err(SiteError::Occupied)
    );
    assert_eq!(check_site(s.world(), WALL, pos, Some(wall)), Ok(()));
}

#[derive(Clone, Default)]
struct YardHost(std::rc::Rc<std::cell::RefCell<std::collections::BTreeMap<UnitId, i32>>>);
impl tak_sim::ScriptHost for YardHost {
    fn port_value(&self, id: UnitId, port: i32) -> Option<i32> {
        (port == 18).then(|| *self.0.borrow().get(&id).unwrap_or(&0))
    }
    fn set_port(&mut self, id: UnitId, port: i32, value: i32) {
        if port == 18 {
            self.0.borrow_mut().insert(id, value);
        }
    }
    fn hash_state(&self, h: &mut tak_sim::StateHasher) {
        for (id, value) in self.0.borrow().iter() {
            h.unit_id(*id);
            h.i32(*value);
        }
    }
}

#[test]
fn yard_door_cannot_close_on_a_ground_unit_but_flyers_do_not_hold_it_open() {
    let mut sc = setup();
    sc.unit_defs[WALL.0 as usize].build.yardmap = "c".repeat(32);
    let host = YardHost::default();
    let ports = host.0.clone();
    let mut s = Sim::with_scripts(&sc, Box::new(host)).unwrap();
    s.step(&[]);
    let gate = wall(&mut s);
    s.step(&[]);
    assert!(!line_clear(&s));
    ports.borrow_mut().insert(gate, 1);
    s.step(&[]);
    assert!(line_clear(&s));
    let walker = UnitId::new(0, 0);
    s.world_mut().unit_mut(walker).unwrap().pos = Vec2::from_ints(800, 800);
    assert_eq!(
        tak_sim::lifecycle::script_ctx(s.world(), gate)
            .unwrap()
            .yard_blocked_states
            & 1,
        1
    );
    ports.borrow_mut().insert(gate, 0);
    s.step(&[]);
    assert_eq!(
        ports.borrow()[&gate],
        1,
        "denied close must be visible to the script"
    );
    assert!(line_clear(&s));
    s.world_mut().unit_mut(walker).unwrap().pos = Vec2::from_ints(500, 800);
    s.spawn_unit(FLYER, P0, Vec2::from_ints(800, 800), Angle::ZERO)
        .unwrap();
    ports.borrow_mut().insert(gate, 0);
    s.step(&[]);
    assert_eq!(ports.borrow()[&gate], 0);
    assert!(!line_clear(&s));
}

#[test]
fn inverse_yard_cells_obey_open_state_and_occupied_transition_gate() {
    let mut sc = setup();
    sc.unit_defs[WALL.0 as usize].build.yardmap = "O".repeat(32);
    let host = YardHost::default();
    let ports = host.0.clone();
    let mut s = Sim::with_scripts(&sc, Box::new(host)).unwrap();
    s.step(&[]);
    let gate = wall(&mut s);
    s.step(&[]);
    assert!(line_clear(&s));
    let walker = UnitId::new(0, 0);
    s.world_mut().unit_mut(walker).unwrap().pos = Vec2::from_ints(800, 800);
    ports.borrow_mut().insert(gate, 1);
    s.step(&[]);
    assert_eq!(ports.borrow()[&gate], 0);
    assert!(line_clear(&s));
    s.world_mut().unit_mut(walker).unwrap().pos = Vec2::from_ints(500, 800);
    ports.borrow_mut().insert(gate, 1);
    s.step(&[]);
    assert!(!line_clear(&s));
    ports.borrow_mut().insert(gate, 0);
    s.step(&[]);
    assert!(line_clear(&s));
}

#[path = "construction_support/mod.rs"]
mod construction;

#[test]
fn paid_factory_products_leave_an_open_yard_and_the_yard_closes_afterwards() {
    let mut sc = construction::scenario();
    sc.map.terrain = Some(TerrainGrid {
        cols: 64,
        rows: 64,
        cell_size: 16,
        flags: vec![0; 64 * 64],
    });
    sc.players[0].monarch = construction::KEEP;
    sc.unit_defs[construction::KEEP.0 as usize]
        .economy
        .mogriumstorage = 5000;
    let mut s = Sim::new(&sc).unwrap();
    s.step(&[]);
    let factory = UnitId::new(0, 0);
    let mut log = CommandLog::new();
    for _ in 0..3 {
        log.record(
            s.tick(),
            P0,
            SimCommand::Build {
                units: UnitSet::single(factory),
                def: construction::SWORD,
                at: None,
                queue: QueueMode::Append,
            },
        );
    }
    s.run(&log, 450);
    let products: Vec<_> = s
        .world()
        .units()
        .filter(|(_, u)| u.def == construction::SWORD)
        .collect();
    assert_eq!(
        products.len(),
        3,
        "paid queue must not jam on its own structure blocker"
    );
    assert!(products.iter().all(|(_, u)| u.build.is_complete()));
    let pos = s.world().unit(factory).unwrap().pos;
    assert!(
        products
            .iter()
            .all(|(_, u)| u.pos.distance(pos) > Fx::from_int(40))
    );
    assert!(
        s.world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product
            .is_none()
    );
    assert_eq!(
        tak_sim::economy::player(s.world(), P0).unwrap().mana,
        910 << 16
    );
}

#[test]
fn paid_structure_frame_blocks_until_death_then_its_wreck_blocks_until_removed() {
    let mut sc = construction::scenario();
    sc.map.terrain = Some(TerrainGrid {
        cols: 64,
        rows: 64,
        cell_size: 16,
        flags: vec![0; 64 * 64],
    });
    sc.unit_defs[construction::WALL.0 as usize]
        .combat
        .max_damage = 100;
    sc.features.defs.push(tak_sim::features::FeatureDef {
        name: "synthetic-wall-wreck".into(),
        footprint_x: 2,
        footprint_z: 2,
        blocking: true,
        ..Default::default()
    });
    sc.features.corpses = vec![None, Some(0), None, None, None];
    let mut s = Sim::new(&sc).unwrap();
    s.step(&[]);
    let king = UnitId::new(0, 0);
    let at = Vec2::from_ints(300, 200);
    let mut log = CommandLog::new();
    log.record(
        s.tick(),
        P0,
        SimCommand::Build {
            units: UnitSet::single(king),
            def: construction::WALL,
            at: Some(at),
            queue: QueueMode::Replace,
        },
    );
    s.step(log.for_tick(s.tick()));
    let frame = s
        .world()
        .units()
        .find(|(_, u)| u.def == construction::WALL)
        .unwrap()
        .0;
    assert!(!s.world().unit(frame).unwrap().build.is_complete());
    let clear = |s: &Sim| {
        s.world()
            .pathing
            .grid_for(construction::KING)
            .unwrap()
            .line_of_sight(Vec2::from_ints(200, 200), Vec2::from_ints(400, 200))
            .0
    };
    assert!(!clear(&s));
    s.run(&CommandLog::new(), 220);
    assert!(s.world().unit(frame).unwrap().build.is_complete());
    assert!(!clear(&s));
    let now = s.tick().0;
    let pos = s.world().unit(frame).unwrap().pos;
    tak_sim::combat::apply_damage(
        s.world_mut(),
        &mut tak_sim::NoScripts,
        frame,
        1000,
        None,
        tak_sim::combat::kind::NORMAL,
        pos,
        now,
    );
    s.run(&CommandLog::new(), 2);
    assert!(s.world().unit(frame).is_none());
    assert!(!clear(&s), "wreck must keep blocking");
    let wreck = s.world().features.iter().next().unwrap().0;
    tak_sim::features::remove(s.world_mut(), wreck).unwrap();
    s.step(&[]);
    assert!(
        clear(&s),
        "wreck removal must not leave stale structure occupancy"
    );
}

fn structure_hashes() -> Vec<u64> {
    let (mut s, first) = start();
    let mut log = CommandLog::new();
    log.record(
        s.tick(),
        P0,
        SimCommand::Move {
            units: UnitSet::single(UnitId::new(0, 0)),
            to: Vec2::from_ints(1100, 800),
            queue: QueueMode::Replace,
        },
    );
    let r = tak_sim::pathfinding::CellRect {
        x: 60,
        z: 40,
        w: 2,
        h: 20,
    };
    let mut hashes = Vec::new();
    for t in 0..700 {
        match t {
            100 => {
                s.world_mut().despawn(first).unwrap();
            }
            200 => {
                wall(&mut s);
            }
            300 => tak_sim::pathfinding::add_obstacle(s.world_mut(), r),
            400 => tak_sim::pathfinding::remove_obstacle(s.world_mut(), r),
            _ => {}
        }
        hashes.push(s.step(log.for_tick(s.tick())));
    }
    hashes
}

#[test]
fn structure_routing_child() {
    let Ok(out) = std::env::var("OA_STRUCTURE_CHILD") else {
        return;
    };
    std::fs::write(out, format!("{:?}", structure_hashes())).unwrap();
}

#[test]
fn structure_lifecycle_and_routes_agree_across_processes_and_json() {
    let out = std::env::temp_dir().join(format!("oa-structure-{}.txt", std::process::id()));
    assert!(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "structure_routing_child"])
            .env("OA_STRUCTURE_CHILD", &out)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        format!("{:?}", structure_hashes())
    );
    std::fs::remove_file(out).unwrap();
    let sc = setup();
    let json = sc.to_json_pretty();
    let parsed = Scenario::from_json(&json).unwrap();
    let mut a = Sim::new(&sc).unwrap();
    let mut b = Sim::new(&parsed).unwrap();
    wall(&mut a);
    wall(&mut b);
    assert_eq!(a.run(&CommandLog::new(), 50), b.run(&CommandLog::new(), 50));
}

#[test]
fn changing_occupancy_restarts_a_suspended_search_under_the_same_budget() {
    let mut sc = setup();
    sc.map.start_positions[0] = tak_sim::StartPos { x: 40, y: 40 };
    let t = sc.map.terrain.as_mut().unwrap();
    for (i, x) in [20usize, 40, 60, 80, 100].into_iter().enumerate() {
        for z in 0..128 {
            let gap = if i % 2 == 0 { 116..124 } else { 4..12 };
            if !gap.contains(&z) {
                t.flags[z * 128 + x] = tak_sim::map::terrain::FEATURE;
            }
        }
    }
    let mut s = Sim::new(&sc).unwrap();
    s.step(&[]);
    let walker = UnitId::new(0, 0);
    let mut log = CommandLog::new();
    log.record(
        s.tick(),
        P0,
        SimCommand::Move {
            units: UnitSet::single(walker),
            to: Vec2::from_ints(1900, 1900),
            queue: QueueMode::Replace,
        },
    );
    s.step(log.for_tick(s.tick()));
    assert_eq!(
        s.world().unit(walker).unwrap().path.status,
        tak_sim::pathfinding::PathStatus::Searching,
        "fixture needs a suspended search"
    );
    let requests = s.world().pathing.stats.requests;
    s.spawn_unit(WALL, P0, Vec2::from_ints(1800, 800), Angle::ZERO)
        .unwrap();
    s.step(&[]);
    assert!(
        s.world().pathing.stats.requests > requests,
        "search reused stale occupancy inputs"
    );
    assert!(s.world().pathing.stats.steps_last_tick <= tak_sim::pathfinding::STEPS_PER_TICK);
}
