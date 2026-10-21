//! WP-P open items on synthetic definitions: frame health, corpses and
//! the feature layer (blocking, decay), assisting through Guard, lodestones
//! on sacred sites, and a cross-process determinism run with every system
//! on.

mod construction_support;

use std::cell::RefCell;
use std::rc::Rc;

use construction_support::{Log, Recorder, at, build, cmd};
use tak_commands::{
    CommandLog, LoggedCommand, PlayerId, QueueMode, SimCommand, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Angle, Fx};
use tak_sim::combat::{self, frame_health};
use tak_sim::construction::{BuildDef, placement};
use tak_sim::economy::{EconomySetup, SacredSite};
use tak_sim::features::{FeatureDef, FeatureSetup};
use tak_sim::map::{MapInfo, StartPos, TerrainGrid};
use tak_sim::testing::monarch_def;
use tak_sim::{NoScripts, PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario, Sim, UnitDef};

const KING: UnitDefId = UnitDefId(0);
const WALL: UnitDefId = UnitDefId(1);
const KEEP: UnitDefId = UnitDefId(2);
const SWORD: UnitDefId = UnitDefId(3);
const MASON: UnitDefId = UnitDefId(4);
const LODE: UnitDefId = UnitDefId(5);
const TANK: UnitDefId = UnitDefId(6);

const KING0: UnitId = UnitId {
    index: 0,
    generation: 0,
};

fn secs(s: i64) -> i64 {
    s << 16
}

fn structure(name: &str, fx: u8, fz: u8, hp: i32, build: BuildDef) -> UnitDef {
    let mut d = monarch_def(name);
    d.maxvelocity = Fx::ZERO;
    d.acceleration = Fx::from_raw(1);
    d.brakerate = Fx::from_raw(1);
    d.turnrate = 0;
    d.footprintx = fx;
    d.footprintz = fz;
    d.build = build;
    d.combat.max_damage = hp;
    d.combat.radius = i32::from(fx.max(fz)) * 8;
    d
}

fn walker(name: &str, hp: i32, buildtime: i64) -> UnitDef {
    let mut d = monarch_def(name);
    d.footprintx = 1;
    d.footprintz = 1;
    d.combat.max_damage = hp;
    d.combat.radius = 8;
    d.build = BuildDef {
        buildcost: 30,
        buildtime: secs(buildtime),
        mobile: true,
        ..BuildDef::default()
    };
    d
}

/// A 1024 x 1024 map with an open terrain grid (so movement layers exist),
/// one sacred site, and two corpse features: a 4x4 blocking wreck (the
/// TANK's) and a 2x2 body that decomposes after one second (the SWORD's).
fn scenario() -> Scenario {
    let mut king = monarch_def("KING");
    king.economy.mogriumincome = 10;
    king.economy.mogriumstorage = 5000;
    king.combat.max_damage = 3000;
    king.combat.radius = 16;
    king.build = BuildDef {
        buildcost: 100_000,
        buildtime: secs(10),
        workertime: 10,
        builddistance: 100,
        builder: true,
        mobile: true,
        canbuild: vec![WALL, KEEP, LODE],
        yardmap: String::new(),
    };
    let wall = structure(
        "WALL",
        2,
        2,
        1000,
        BuildDef {
            buildcost: 75,
            buildtime: secs(65),
            yardmap: "oooo".into(),
            ..BuildDef::default()
        },
    );
    let keep = structure(
        "KEEP",
        4,
        4,
        3000,
        BuildDef {
            buildcost: 500,
            buildtime: secs(10),
            workertime: 10,
            builddistance: 100,
            builder: true,
            mobile: false,
            canbuild: vec![SWORD, TANK],
            yardmap: "oooooooooooocccc".into(),
        },
    );
    let mut lode = structure(
        "LODE",
        2,
        2,
        800,
        BuildDef {
            buildcost: 50,
            buildtime: secs(10),
            yardmap: "S".into(),
            ..BuildDef::default()
        },
    );
    lode.economy.sacred_site = true;
    lode.economy.mogriumincome = 10;
    let mut mason = walker("MASON", 300, 6);
    mason.build.workertime = 10;
    mason.build.builddistance = 60;
    mason.build.builder = true;
    mason.build.canbuild = vec![WALL];
    let mut tank = walker("TANK", 900, 30);
    tank.footprintx = 2;
    tank.footprintz = 2;
    let cells = 64u32;
    let mut map = MapInfo::flat(
        1024,
        1024,
        vec![StartPos { x: 200, y: 200 }, StartPos { x: 800, y: 800 }],
    );
    map.terrain = Some(TerrainGrid {
        cols: cells,
        rows: cells,
        cell_size: 16,
        flags: vec![0; (cells * cells) as usize],
    });
    let economy = EconomySetup {
        sacred_sites: vec![SacredSite {
            x: 480,
            y: 160,
            w: 32,
            h: 32,
            multiplier: Fx::from_ratio(3, 2).unwrap(),
        }],
        ..EconomySetup::default()
    };
    let wreck = FeatureDef {
        name: "tank_dead".into(),
        footprint_x: 4,
        footprint_z: 4,
        blocking: true,
        damage: 500,
        reclaimable: true,
        ..FeatureDef::default()
    };
    let body = FeatureDef {
        name: "sword_dead".into(),
        footprint_x: 2,
        footprint_z: 2,
        damage: 100,
        reclaimable: true,
        decompose_time: 1,
        ..FeatureDef::default()
    };
    let features = FeatureSetup {
        defs: vec![wreck, body],
        placed: vec![],
        corpses: vec![None, None, None, Some(1), None, None, Some(0)],
    };
    Scenario {
        version: SCENARIO_FORMAT_VERSION,
        seed: 5,
        map,
        unit_defs: vec![king, wall, keep, walker("SWORD", 500, 9), mason, lode, tank],
        players: vec![
            PlayerSpec {
                start: 0,
                monarch: KING,
            },
            PlayerSpec {
                start: 1,
                monarch: KING,
            },
        ],
        economy: Box::new(economy),
        features: Box::new(features),
    }
}

fn new_sim() -> (Sim, Rc<RefCell<Log>>) {
    let rec = Recorder::default();
    let log = rec.log.clone();
    (Sim::with_scripts(&scenario(), Box::new(rec)).unwrap(), log)
}

fn step(sim: &mut Sim, log: &Rc<RefCell<Log>>, cmds: &[LoggedCommand]) {
    construction_support::step(sim, log, cmds);
}

fn find(sim: &Sim, def: UnitDefId) -> Vec<UnitId> {
    sim.world()
        .units()
        .filter(|(_, u)| u.def == def)
        .map(|(id, _)| id)
        .collect()
}

fn guard(tick: u32, player: u8, unit: UnitId, target: UnitId) -> LoggedCommand {
    cmd(
        tick,
        player,
        SimCommand::Guard {
            units: UnitSet::single(unit),
            target,
            queue: QueueMode::Replace,
        },
    )
}

fn hit(sim: &mut Sim, victim: UnitId, amount: i32) {
    let now = sim.tick().0;
    let pos = sim.world().unit(victim).unwrap().pos;
    combat::apply_damage(
        sim.world_mut(),
        &mut NoScripts,
        victim,
        amount,
        None,
        1,
        pos,
        now,
    );
}

// --- Frame health -----------------------------------------------------

#[test]
fn frame_health_formula_is_tas_difference_of_truncations() {
    let total = 1950i64 << 16;
    let step = 10i64 << 16;
    assert_eq!(frame_health(1000, total, total), 1000);
    assert_eq!(frame_health(1000, 0, total), 0);
    // After one monarch tick on a 65 s wall: trunc(1000 x 1940/1950) = 994,
    // so the frame has earned 6 hit points.
    assert_eq!(1000 - frame_health(1000, total - step, total), 6);
    // The gains of every step add up to exactly maxdamage.
    let mut sum = 0;
    let mut done = 0;
    while done < total {
        let next = (done + step).min(total);
        sum += frame_health(1000, total - done, total) - frame_health(1000, total - next, total);
        done = next;
    }
    assert_eq!(sum, 1000);
}

#[test]
fn frames_start_at_zero_health_and_gain_it_as_they_are_built() {
    let (mut sim, log) = new_sim();
    let cmds = vec![build(1, 0, &[KING0], WALL, Some(at(260, 200)))];
    let mut seen = Vec::new();
    let mut wall = None;
    for _ in 0..=195 {
        step(&mut sim, &log, &cmds);
        if wall.is_none() {
            wall = find(&sim, WALL).first().copied();
        }
        if let Some(w) = wall.and_then(|w| sim.world().unit(w)) {
            seen.push((w.combat.health, w.build.percent_left()));
        }
    }
    assert_eq!(seen[0].0, 6, "one tick of work on the placement tick");
    assert!(seen.windows(2).all(|w| w[1].0 >= w[0].0), "never drops");
    let mid = seen.iter().find(|s| s.1 <= 50).unwrap();
    assert!(
        (495..=510).contains(&mid.0),
        "about half health at half built: {mid:?}"
    );
    assert_eq!(
        seen.last().unwrap(),
        &(1000, 0),
        "full health exactly on completion"
    );
    let w = sim.world().unit(wall.unwrap()).unwrap();
    assert_eq!(w.combat.health_percent(), 100);
}

#[test]
fn damage_to_a_frame_is_health_loss_not_progress_loss() {
    let (mut sim, log) = new_sim();
    let cmds = vec![build(1, 0, &[KING0], WALL, Some(at(260, 200)))];
    for _ in 0..100 {
        step(&mut sim, &log, &cmds);
    }
    let wall = find(&sim, WALL)[0];
    let before = sim.world().unit(wall).unwrap().clone();
    hit(&mut sim, wall, 300);
    let after = sim.world().unit(wall).unwrap();
    assert_eq!(after.combat.health, before.combat.health - 300);
    assert_eq!(after.build.frame, before.build.frame, "progress untouched");
    for _ in 100..=195 {
        step(&mut sim, &log, &cmds);
    }
    let w = sim.world().unit(wall).unwrap();
    assert!(w.build.is_complete(), "still completes on tick 195");
    assert_eq!(w.combat.health, 700, "completes 300 short of full");
    // Completed units regenerate only with healtime (0 here).
    step(&mut sim, &log, &cmds);
    assert_eq!(sim.world().unit(wall).unwrap().combat.health, 700);
}

#[test]
fn a_fresh_frame_dies_to_a_small_hit() {
    let (mut sim, log) = new_sim();
    let cmds = vec![build(1, 0, &[KING0], WALL, Some(at(260, 200)))];
    for _ in 0..5 {
        step(&mut sim, &log, &cmds);
    }
    let wall = find(&sim, WALL)[0];
    assert!(sim.world().unit(wall).unwrap().combat.health < 40);
    hit(&mut sim, wall, 50);
    for _ in 0..3 {
        step(&mut sim, &log, &cmds);
    }
    assert!(
        sim.world().unit(wall).is_none(),
        "the frame died and was removed"
    );
    let king = sim.world().unit(KING0).unwrap();
    assert!(
        king.build.job.is_none() && king.orders.is_empty(),
        "job dropped"
    );
}

// --- Corpses and the feature layer ------------------------------------

#[test]
fn a_blocking_wreck_blocks_paths_and_building_until_removed() {
    let (mut sim, log) = new_sim();
    step(&mut sim, &log, &[]);
    let tank = sim
        .spawn_unit(TANK, PlayerId(1), at(520, 520), Angle::ZERO)
        .unwrap();
    step(&mut sim, &log, &[]);
    hit(&mut sim, tank, 10_000);
    for _ in 0..3 {
        step(&mut sim, &log, &[]);
    }
    assert!(sim.world().unit(tank).is_none());
    let w = sim.world();
    let corpses: Vec<_> = w.features.corpses().collect();
    assert_eq!(corpses.len(), 1);
    let (_, f) = corpses[0];
    assert_eq!(w.features.def(f).unwrap().name, "tank_dead");
    // 4x4, centred on the tank: anchor nearest to (520 - 32, 520 - 32).
    assert_eq!((f.rect.x, f.rect.z, f.rect.w, f.rect.h), (31, 31, 4, 4));
    assert_eq!(f.health, 500);
    for z in 31..35 {
        for x in 31..35 {
            assert!(tak_sim::pathfinding::is_obstacle(w, x, z));
        }
    }
    assert!(!tak_sim::pathfinding::is_obstacle(w, 30, 31));
    let grid = w.pathing.grid_for(SWORD).expect("layers built");
    assert_eq!(grid.tier((32, 32)), 0, "blocked in the movement layer");
    assert_eq!(
        placement::check_site(w, WALL, at(528, 528), None),
        Err(placement::SiteError::Feature)
    );

    // A swordsman walks straight across: it routes around the wreck.
    let sword = sim
        .spawn_unit(SWORD, PlayerId(0), at(440, 536), Angle::ZERO)
        .unwrap();
    let to = at(620, 536);
    let t0 = sim.tick().0;
    let go = vec![cmd(
        t0,
        0,
        SimCommand::Move {
            units: UnitSet::single(sword),
            to,
            queue: QueueMode::Replace,
        },
    )];
    let mut arrived = false;
    for _ in 0..600 {
        step(&mut sim, &log, &go);
        let p = sim.world().unit(sword).unwrap().pos;
        let (cx, cz) = (p.x.to_int_floor() / 16, p.y.to_int_floor() / 16);
        assert!(
            !(31..35).contains(&cx) || !(31..35).contains(&cz),
            "walked into the wreck at {p:?}"
        );
        if p.distance(to) <= Fx::from_int(2) {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "reached the far side");

    // Removing the wreck clears the obstacle.
    let id = sim.world().features.corpses().next().unwrap().0;
    tak_sim::features::remove(sim.world_mut(), id).unwrap();
    let w = sim.world();
    assert!(!tak_sim::pathfinding::is_obstacle(w, 32, 32));
    assert_ne!(w.pathing.grid_for(SWORD).unwrap().tier((32, 32)), 0);
}

#[test]
fn bodies_do_not_block_and_decompose() {
    let (mut sim, log) = new_sim();
    step(&mut sim, &log, &[]);
    let sword = sim
        .spawn_unit(SWORD, PlayerId(1), at(600, 300), Angle::ZERO)
        .unwrap();
    step(&mut sim, &log, &[]);
    hit(&mut sim, sword, 10_000);
    step(&mut sim, &log, &[]);
    step(&mut sim, &log, &[]);
    let born = {
        let w = sim.world();
        let (_, f) = w.features.corpses().next().expect("body spawned");
        assert_eq!(w.features.def(f).unwrap().name, "sword_dead");
        assert!(!f.obstacle);
        assert!(!tak_sim::pathfinding::is_obstacle(w, f.rect.x, f.rect.z));
        f.born
    };
    while sim.tick().0 < born + 30 {
        step(&mut sim, &log, &[]);
    }
    assert_eq!(sim.world().features.corpses().count(), 1);
    step(&mut sim, &log, &[]);
    assert_eq!(
        sim.world().features.corpses().count(),
        0,
        "gone after 30 ticks"
    );
    assert_eq!(sim.world().features.corpses_spawned, 1);
}

#[test]
fn units_without_a_corpse_leave_nothing() {
    let (mut sim, log) = new_sim();
    step(&mut sim, &log, &[]);
    let mason = sim
        .spawn_unit(MASON, PlayerId(1), at(600, 300), Angle::ZERO)
        .unwrap();
    step(&mut sim, &log, &[]);
    hit(&mut sim, mason, 10_000);
    for _ in 0..3 {
        step(&mut sim, &log, &[]);
    }
    assert!(sim.world().unit(mason).is_none());
    assert_eq!(sim.world().features.len(), 0);
    assert_eq!(sim.world().combat.corpses.len(), 1, "death log kept");
}

// --- Assist via Guard -------------------------------------------------

fn completion_tick(
    sim: &mut Sim,
    log: &Rc<RefCell<Log>>,
    cmds: &[LoggedCommand],
    def: UnitDefId,
) -> u32 {
    let mut frame = None;
    for _ in 0..600 {
        step(sim, log, cmds);
        if frame.is_none() {
            frame = find(sim, def).first().copied();
        }
        if let Some(f) = frame {
            if sim.world().unit(f).is_some_and(|u| u.build.is_complete()) {
                return sim.tick().0 - 1;
            }
        }
    }
    panic!("never completed");
}

#[test]
fn a_builder_guarding_a_frame_helps_build_it() {
    let (mut sim, log) = new_sim();
    step(&mut sim, &log, &[]);
    let mason = sim
        .spawn_unit(MASON, PlayerId(0), at(250, 250), Angle::ZERO)
        .unwrap();
    // The wall frame is placed on tick 2 (slot 2, after the two kings and
    // the mason in slot 2 -> the frame takes slot 3).
    let wall = UnitId::new(3, 0);
    let cmds = vec![
        build(2, 0, &[KING0], WALL, Some(at(260, 200))),
        guard(3, 0, mason, wall),
    ];
    let done = completion_tick(&mut sim, &log, &cmds, WALL);
    assert_eq!(find(&sim, WALL), vec![wall]);
    // Alone: placed on tick 2, done on tick 196. Two builders from tick 3
    // or 4: about half the time.
    assert!((95..=105).contains(&done), "completed on tick {done}");
    let f = sim.world().unit(wall).unwrap();
    assert_eq!(f.combat.health, 1000);
    let l = log.borrow();
    assert_eq!(l.calls_of(mason, "StartBuilding").len(), 1);
    assert_eq!(l.calls_of(mason, "StopBuilding"), vec![done]);
    drop(l);
    let m = sim.world().unit(mason).unwrap();
    assert!(m.build.assist.is_none());
    assert!(matches!(
        m.combat.order,
        Some(combat::CombatOrder::Guard(_))
    ));
}

#[test]
fn a_builder_guarding_a_builder_or_a_factory_assists_its_work() {
    // Guarding the king while it builds.
    let (mut sim, log) = new_sim();
    step(&mut sim, &log, &[]);
    let mason = sim
        .spawn_unit(MASON, PlayerId(0), at(250, 250), Angle::ZERO)
        .unwrap();
    let cmds = vec![
        guard(1, 0, mason, KING0),
        build(2, 0, &[KING0], WALL, Some(at(260, 200))),
    ];
    let done = completion_tick(&mut sim, &log, &cmds, WALL);
    assert!((95..=110).contains(&done), "completed on tick {done}");

    // Guarding a factory: its product (TANK, buildtime 30 = 90 ticks
    // alone) builds about twice as fast.
    let (mut sim, log) = new_sim();
    step(&mut sim, &log, &[]);
    let keep = sim
        .spawn_unit(KEEP, PlayerId(0), at(400, 400), Angle::QUARTER)
        .unwrap();
    let alone = {
        let (mut s2, l2) = sim_with_keep();
        let k = UnitId::new(2, 0);
        completion_tick(&mut s2, &l2, &[build(2, 0, &[k], TANK, None)], TANK)
    };
    let mason = sim
        .spawn_unit(MASON, PlayerId(0), at(400, 470), Angle::ZERO)
        .unwrap();
    let cmds = vec![build(2, 0, &[keep], TANK, None), guard(2, 0, mason, keep)];
    let helped = completion_tick(&mut sim, &log, &cmds, TANK);
    assert!(
        helped + 30 < alone,
        "assisted product done on tick {helped}, alone on tick {alone}"
    );
}

fn sim_with_keep() -> (Sim, Rc<RefCell<Log>>) {
    let (mut sim, log) = new_sim();
    step(&mut sim, &log, &[]);
    sim.spawn_unit(KEEP, PlayerId(0), at(400, 400), Angle::QUARTER)
        .unwrap();
    (sim, log)
}

// --- Lodestones on sacred sites ---------------------------------------

#[test]
fn lodestones_must_stand_on_a_free_sacred_site() {
    let (mut sim, log) = new_sim();
    // Off the site: refused when the king gets there, with the reason.
    let cmds = vec![build(1, 0, &[KING0], LODE, Some(at(300, 300)))];
    for _ in 0..60 {
        step(&mut sim, &log, &cmds);
    }
    assert!(find(&sim, LODE).is_empty());
    let r = sim.world().construction.rejections.last().copied().unwrap();
    assert_eq!(r.reason, placement::SiteError::NotSacredSite);
    assert_eq!((r.builder, r.def, r.player), (KING0, LODE, PlayerId(0)));
    assert_eq!(r.reason.reason(), "must be built on a free sacred site");

    // On the site (centre 496, 176): built, and it claims the site.
    let t = sim.tick().0;
    let cmds = vec![build(t, 0, &[KING0], LODE, Some(at(496, 176)))];
    for _ in 0..600 {
        step(&mut sim, &log, &cmds);
    }
    let lode = find(&sim, LODE);
    assert_eq!(lode.len(), 1);
    let l = sim.world().unit(lode[0]).unwrap();
    assert!(l.build.is_complete());
    assert_eq!(l.pos, at(496, 176));
    assert_eq!(l.economy.site(), Some(0));

    // A second lodestone on the same site is refused.
    assert_eq!(
        placement::check_site(sim.world(), LODE, at(500, 180), None),
        Err(placement::SiteError::NotSacredSite)
    );
    // Other structures do not care about sites.
    assert_eq!(
        placement::check_site(sim.world(), WALL, at(300, 300), None),
        Ok(())
    );
}

// --- Determinism with every system on ---------------------------------

const TICKS: u32 = 1500;
const CHILD_ENV: &str = "TAK_OPEN_ITEMS_DETERMINISM_CHILD";

fn busy_log() -> CommandLog {
    let mut log = CommandLog::new();
    let mut add = |c: LoggedCommand| log.record(c.tick, c.player, c.command);
    add(build(1, 0, &[KING0], WALL, Some(at(260, 200))));
    add(build(1, 1, &[UnitId::new(1, 0)], LODE, Some(at(496, 176))));
    // Mason (slot 4 once spawned on tick 1) guards the king.
    add(guard(5, 0, UnitId::new(4, 0), KING0));
    add(build(300, 0, &[KING0], KEEP, Some(at(400, 400))));
    add(cmd(
        600,
        1,
        SimCommand::Move {
            units: UnitSet::single(UnitId::new(1, 0)),
            to: at(560, 560),
            queue: QueueMode::Replace,
        },
    ));
    log
}

/// The run: commands plus scripted spawns and hits at fixed ticks.
fn run_busy(sc: &Scenario) -> Vec<u64> {
    let rec = Recorder::default();
    let clock = rec.log.clone();
    let mut sim = Sim::with_scripts(sc, Box::new(rec)).unwrap();
    let log = busy_log();
    let mut hashes = Vec::new();
    let mut tank = None;
    let mut sword = None;
    for _ in 0..TICKS {
        let t = sim.tick();
        clock.borrow_mut().tick = t.0;
        match t.0 {
            1 => {
                tank = sim.spawn_unit(TANK, PlayerId(1), at(520, 520), Angle::ZERO);
                sword = sim.spawn_unit(SWORD, PlayerId(1), at(560, 300), Angle::ZERO);
                sim.spawn_unit(MASON, PlayerId(0), at(250, 250), Angle::ZERO);
            }
            200 => hit(&mut sim, tank.unwrap(), 10_000),
            250 => hit(&mut sim, sword.unwrap(), 10_000),
            _ => {}
        }
        hashes.push(sim.step(log.for_tick(t)));
    }
    let w = sim.world();
    assert_eq!(w.features.corpses_spawned, 2);
    assert_eq!(
        w.features.corpses().count(),
        1,
        "the wreck stays, the body rots"
    );
    hashes
}

fn digest(h: &[u64]) -> String {
    format!(
        "{:016x} {:016x} {}",
        h[h.len() / 2],
        h[h.len() - 1],
        h.len()
    )
}

#[test]
fn open_items_are_deterministic_across_runs_and_processes() {
    let sc = scenario();
    let a = run_busy(&sc);
    if std::env::var_os(CHILD_ENV).is_some() {
        println!("DIGEST {}", digest(&a));
        return;
    }
    assert_eq!(a, run_busy(&sc));
    let sc2 = Scenario::from_json(&sc.to_json_pretty()).unwrap();
    assert_eq!(sc, sc2, "feature data survives JSON");
    assert_eq!(a, run_busy(&sc2));
    let exe = std::env::current_exe().unwrap();
    let mut digests = Vec::new();
    for _ in 0..2 {
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "open_items_are_deterministic_across_runs_and_processes",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .unwrap();
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let line = text
            .split("DIGEST ")
            .nth(1)
            .and_then(|rest| rest.lines().next())
            .unwrap_or_else(|| panic!("child printed no digest: {text}"))
            .to_string();
        digests.push(line);
    }
    assert_eq!(digests[0], digests[1]);
    assert_eq!(digests[0], digest(&a));
}
