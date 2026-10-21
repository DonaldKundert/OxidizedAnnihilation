//! Real data (gated on `TAK_DATA_DIR`): Adamantine Gate with an Aramon and
//! a Taros monarch, original COB scripts running. Monarchs spawn on the
//! map's start markers, walk under their FBI stats, route around impassable
//! terrain, and their scripts see "moving" (port 29) while walking.

use tak_commands::{LoggedCommand, PlayerId, QueueMode, SimCommand, Tick, UnitId, UnitSet};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::content::{cell_center, load_match};
use tak_sim::map::{MapInfo, terrain};
use tak_sim::systems::ARRIVAL_TOLERANCE;
use tak_sim::{Match, PlayerSetup, ScriptHost, Sim, WorldPos};

const MAP: &str = "kmap/Adamantine Gate";

fn load(test: &str) -> Option<Match> {
    let dir = tak_vfs::testing::data_dir_or_skip(test)?;
    let vfs = tak_vfs::open_kingdoms(&dir).expect("open install");
    Some(load_match(&vfs, MAP, &PlayerSetup::parse_list("ARA,TAR"), 7).expect("load match"))
}

fn mv(tick: u32, player: u8, unit: UnitId, to: WorldPos) -> LoggedCommand {
    LoggedCommand {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: 0,
        command: SimCommand::Move {
            units: UnitSet::single(unit),
            to,
            queue: QueueMode::Replace,
        },
    }
}

fn blocked(map: &MapInfo, p: WorldPos) -> bool {
    map.terrain_at(p) & terrain::BLOCKS != 0
}

/// Points every 4 units from `a` toward angle `dir`, up to `len`.
fn ray(a: WorldPos, dir: Angle, len: i32) -> impl Iterator<Item = WorldPos> {
    (1..=len / 4).map(move |k| a + Vec2::from_angle(dir).scale(Fx::from_int(4 * k)))
}

fn inside(map: &MapInfo, p: WorldPos) -> bool {
    p.x > Fx::from_int(16)
        && p.y > Fx::from_int(16)
        && p.x < Fx::from_int(map.width - 16)
        && p.y < Fx::from_int(map.height - 16)
}

/// A direction (of 32) whose straight line of `len` units from `from` is
/// open ground inside the map, with a margin of open cells around it.
fn open_line(map: &MapInfo, from: WorldPos, len: i32) -> Option<WorldPos> {
    (0..32u16).map(|i| Angle::from_raw(i * 2048)).find_map(|d| {
        let ok = ray(from, d, len + 24).all(|p| inside(map, p) && !blocked(map, p))
            && [-0x300i32, 0x300].iter().all(|off| {
                let side = Angle::from_raw((i32::from(d.raw()) + off) as u16);
                ray(from, side, len).all(|p| inside(map, p) && !blocked(map, p))
            });
        ok.then(|| from + Vec2::from_angle(d).scale(Fx::from_int(len)))
    })
}

/// A direction whose line from `from` runs over open ground and then hits
/// a blocking cell between 48 and 400 units out. Returns a target 64 units
/// past the first blocked point.
fn line_into_blocker(map: &MapInfo, from: WorldPos) -> Option<WorldPos> {
    (0..64u16).map(|i| Angle::from_raw(i * 1024)).find_map(|d| {
        let pts: Vec<_> = ray(from, d, 400).collect();
        let first = pts.iter().position(|&p| blocked(map, p))?;
        let ok = first >= 12 && pts[..first].iter().all(|&p| inside(map, p));
        // The turn happens in place, so the walk follows the line closely;
        // require open ground on both sides of the line too.
        let sides_ok = [-0x200i32, 0x200].iter().all(|off| {
            let side = Angle::from_raw((i32::from(d.raw()) + off) as u16);
            ray(from, side, 4 * first as i32 - 24).all(|p| !blocked(map, p))
        });
        (ok && sides_ok).then(|| pts[first] + Vec2::from_angle(d).scale(Fx::from_int(64)))
    })
}

#[test]
fn monarchs_spawn_walk_and_their_scripts_see_moving() {
    let Some(m) = load("monarchs_spawn_walk_and_their_scripts_see_moving") else {
        return;
    };
    let sc = m.scenario.clone();
    assert_eq!(sc.map.width, 192 * 16);
    assert_eq!(sc.unit_defs.len(), 2);
    assert_eq!(sc.unit_defs[0].name, "ARAKING");
    assert_eq!(sc.unit_defs[0].maxvelocity, Fx::from_ratio(17, 10).unwrap());
    assert_eq!(sc.unit_defs[0].turnrate, 1500);
    assert_eq!(sc.unit_defs[1].name, "TARNECRO");
    assert!(m.scripts.stats().missing.is_empty());
    assert_eq!(
        m.scripts.stats().unmapped_pieces,
        0,
        "COB pieces map by name"
    );

    let mut sim = Sim::with_scripts(&sc, Box::new(m.scripts)).unwrap();
    sim.step(&[]);
    for (i, s) in m.map.start_positions.iter().take(2).enumerate() {
        let u = sim.world().unit(UnitId::new(i as u32, 0)).unwrap();
        assert_eq!(u.pos, cell_center(s.x, s.z).to_world(), "player {i}");
        assert_eq!(
            u.ground,
            Fx::from_int(i32::from(m.map.height_at(s.x as u32, s.z as u32).unwrap()))
        );
    }
    // Idle: nobody is moving, so port 29 never says so.
    for _ in 0..60 {
        sim.step(&[]);
    }
    let idle = sim.script_stats();
    assert_eq!(idle.units, 2);
    assert!(
        idle.reads.get(&29).copied().unwrap_or(0) > 0,
        "MoveWatcher polls port 29"
    );
    assert_eq!(idle.speed_reads_moving, 0);
    assert!(idle.errors.is_empty(), "{:?}", idle.errors);

    // Walk each monarch 160 units over open ground.
    let mut targets = Vec::new();
    for i in 0..2u32 {
        let id = UnitId::new(i, 0);
        let from = sim.world().unit(id).unwrap().pos;
        let to = open_line(&sc.map, from, 160).expect("an open line near the start");
        targets.push((id, to));
    }
    let t = sim.tick().0;
    sim.step(&[
        mv(t, 0, targets[0].0, targets[0].1),
        mv(t, 1, targets[1].0, targets[1].1),
    ]);
    let mut top = [Fx::ZERO; 2];
    for _ in 0..1500 {
        for (i, (id, _)) in targets.iter().enumerate() {
            let u = sim.world().unit(*id).unwrap();
            top[i] = top[i].max(u.speed);
            assert!(!blocked(&sc.map, u.pos));
        }
        if targets
            .iter()
            .all(|(id, _)| sim.world().unit(*id).unwrap().orders.is_empty())
        {
            break;
        }
        sim.step(&[]);
    }
    for (i, (id, to)) in targets.iter().enumerate() {
        let u = sim.world().unit(*id).unwrap();
        assert!(
            u.pos.distance(*to) <= ARRIVAL_TOLERANCE,
            "player {i} at {:?}",
            u.pos
        );
        assert_eq!(top[i], sc.unit_defs[i].maxvelocity, "reached FBI top speed");
    }
    let walked = sim.script_stats();
    assert!(walked.speed_reads_moving > 0, "scripts saw moving");
    assert!(walked.pose_ticks_moving > 0, "walk animation ran");
    assert!(walked.errors.is_empty(), "{:?}", walked.errors);
}

/// Every mobile unit's script, driven by real movement (port 29 from the
/// sim, no combat callbacks): walk 160 units and idle again. Counts runaway
/// loops (instruction cap) and other VM errors across the corpus.
#[test]
fn every_mobile_unit_script_walks_without_runaways() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("every_mobile_unit_walks") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    let rs = tak_content::Ruleset::load(&vfs, tak_content::RulesetKind::Base);
    let map = tak_content::MapDef::load(&vfs, &rs, MAP).unwrap();
    let base = tak_sim::Scenario::from_content(&map, &rs, &PlayerSetup::parse_list("ARA")).unwrap();
    let mut units = 0;
    let mut walked = 0;
    let mut runaway = std::collections::BTreeMap::new();
    let mut errors = std::collections::BTreeMap::new();
    let mut unknown = std::collections::BTreeMap::new();
    for fbi in rs.units.values() {
        if fbi.motion.maxvelocity.unwrap_or(Fx::ZERO) <= Fx::ZERO {
            continue;
        }
        let mut sc = base.clone();
        sc.unit_defs[0] = tak_sim::content::unit_def_from_fbi(fbi);
        let scripts = tak_sim::content::load_scripts(&vfs, &rs, &sc);
        assert!(scripts.stats().missing.is_empty(), "{}", fbi.unitname);
        let mut sim = Sim::with_scripts(&sc, Box::new(scripts)).unwrap();
        let id = UnitId::new(0, 0);
        sim.step(&[]);
        let from = sim.world().unit(id).unwrap().pos;
        let to = open_line(&sc.map, from, 160).unwrap();
        sim.step(&[mv(1, 0, id, to)]);
        let mut moved = false;
        for _ in 0..600 {
            moved |= sim.world().unit(id).unwrap().speed > Fx::ZERO;
            sim.step(&[]);
        }
        units += 1;
        walked += usize::from(moved);
        let st = sim.script_stats();
        for (k, v) in &st.unknown_reads {
            *unknown.entry(*k).or_insert(0u64) += v;
        }
        runaway.extend(st.runaway);
        errors.extend(st.errors);
    }
    eprintln!(
        "{units} mobile units, {walked} walked; unknown port reads {unknown:?}; \
         runaway {runaway:?}; errors {errors:?}"
    );
    assert!(units > 100);
    assert_eq!(walked, units);
    assert!(runaway.is_empty(), "runaway loops: {runaway:?}");
    assert!(errors.is_empty(), "VM errors: {errors:?}");
}

#[test]
fn monarch_routes_around_impassable_terrain() {
    let Some(m) = load("monarch_routes_around_impassable_terrain") else {
        return;
    };
    let sc = m.scenario.clone();
    let mut sim = Sim::with_scripts(&sc, Box::new(m.scripts)).unwrap();
    sim.step(&[]);
    let id = UnitId::new(0, 0);
    let start = sim.world().unit(id).unwrap().pos;
    // Walk somewhere with a blocker in the way; try a few spots near the
    // start until one has a clean line into a blocker.
    let (from, to) = (0..16)
        .find_map(|k| {
            let a = Angle::from_raw(k * 4096);
            let p = start + Vec2::from_angle(a).scale(Fx::from_int(32 * i32::from(k % 4)));
            (!blocked(&sc.map, p) && inside(&sc.map, p))
                .then(|| line_into_blocker(&sc.map, p).map(|t| (p, t)))
                .flatten()
        })
        .expect("a blocker near the start");
    let mut tick = sim.tick().0;
    if from != start {
        sim.step(&[mv(tick, 0, id, from)]);
        for _ in 0..2000 {
            if sim.world().unit(id).unwrap().orders.is_empty() {
                break;
            }
            sim.step(&[]);
        }
        tick = sim.tick().0;
    }
    sim.step(&[mv(tick, 0, id, to)]);
    let mut moved = false;
    for _ in 0..3000 {
        let u = sim.world().unit(id).unwrap();
        assert!(
            !blocked(&sc.map, u.pos),
            "entered a blocked cell at {:?}",
            u.pos
        );
        moved |= u.speed > Fx::ZERO;
        if u.orders.is_empty() {
            break;
        }
        sim.step(&[]);
    }
    let u = sim.world().unit(id).unwrap();
    assert!(moved);
    assert!(u.orders.is_empty(), "the order finished");
    assert_eq!(u.speed, Fx::ZERO);
    // Since WP-L the unit routes around the blocker instead of halting in
    // front of it: it either reaches the target or, when the target lies
    // inside the obstruction, the reachable point nearest to it.
    use tak_sim::pathfinding::Outcome;
    match u.path.outcome {
        Outcome::Arrived => assert!(u.pos.distance(to) <= ARRIVAL_TOLERANCE),
        Outcome::Unreachable => assert!(u.pos.distance(to) < Fx::from_int(160)),
        o => panic!("unexpected outcome {o:?} at {:?}", u.pos),
    }
}
