//! Construction on synthetic definitions: exact build times, walking to the
//! site, assisting, interruption, factories, cancellation, and the rules
//! that drop bad orders.

mod construction_support;

use construction_support::*;
use tak_commands::{QueueMode, SimCommand, UnitId, UnitSet};
use tak_math::Fx;
use tak_sim::construction::{self, placement};
use tak_sim::{Order, Sim, World, WorldPos};

const KING0: UnitId = UnitId {
    index: 0,
    generation: 0,
};

fn sim() -> (Sim, Recorder) {
    let rec = Recorder::default();
    let sim = Sim::with_scripts(&scenario(), Box::new(rec.clone())).unwrap();
    (sim, rec)
}

/// The live unit of `def` other than the monarchs, if any.
fn find(sim: &Sim, def: tak_commands::UnitDefId) -> Vec<UnitId> {
    sim.world()
        .units()
        .filter(|(_, u)| u.def == def)
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn monarch_builds_a_wall_in_exactly_buildtime_over_workertime() {
    let (mut sim, rec) = sim();
    let log = rec.log.clone();
    // 2x2 wall next to the king (in range at once): snapped to (256, 208).
    let cmds = vec![build(1, 0, &[KING0], WALL, Some(at(260, 200)))];
    let mut wall = None;
    for _ in 0..=194 {
        step(&mut sim, &log, &cmds);
        if wall.is_none() {
            wall = find(&sim, WALL).first().copied();
        }
    }
    let wall = wall.expect("frame placed on the command tick");
    let w = sim.world().unit(wall).unwrap();
    assert_eq!(w.pos, at(256, 208));
    assert_eq!(
        w.build.percent_left(),
        1,
        "one tick of work left after tick 194"
    );
    assert!(!w.build.is_complete());
    step(&mut sim, &log, &cmds); // tick 195
    let w = sim.world().unit(wall).unwrap();
    assert!(w.build.is_complete(), "195 ticks = 65 / 10 seconds");
    assert_eq!(w.build.percent_left(), 0);

    let l = log.borrow();
    assert_eq!(l.created.iter().find(|c| c.1 == wall).unwrap().0, 1);
    assert_eq!(l.calls_of(KING0, "StartBuilding"), vec![1]);
    assert_eq!(l.calls_of(KING0, "StopBuilding"), vec![195]);
    // Port 17 as the wall's script saw it: 100 down to 0, never rising.
    let seen = &l.port17[&wall];
    assert_eq!(seen.first().unwrap(), &(1, 100));
    assert_eq!(seen.last().unwrap(), &(195, 0));
    assert!(seen.windows(2).all(|w| w[1].1 <= w[0].1));
    assert_eq!(seen.iter().filter(|s| s.1 == 0).count(), 1);
    drop(l);
    let king = sim.world().unit(KING0).unwrap();
    assert!(king.orders.is_empty(), "build order done");
    assert!(king.build.job.is_none());
}

#[test]
fn monarch_walks_into_range_then_builds() {
    let (mut sim, rec) = sim();
    let log = rec.log.clone();
    let site = at(704, 208);
    let cmds = vec![build(1, 0, &[KING0], WALL, Some(at(700, 200)))];
    let mut placed = None;
    for _ in 0..2000 {
        step(&mut sim, &log, &cmds);
        if placed.is_none() {
            if let Some(&w) = find(&sim, WALL).first() {
                placed = Some((sim.tick().0 - 1, w, sim.world().unit(KING0).unwrap().pos));
            }
        }
        if let Some((_, w, _)) = placed {
            if sim.world().unit(w).unwrap().build.is_complete() {
                break;
            }
        }
    }
    let (t0, wall, king_at) = placed.expect("wall placed");
    assert_eq!(sim.world().unit(wall).unwrap().pos, site);
    // Range: centre distance minus both pads (22 + 22) within 100.
    let d = king_at.distance(site).to_int_floor();
    assert!(d <= 144, "in range when placing ({d})");
    assert!(
        d > 120,
        "stopped at the edge of range, not on the site ({d})"
    );
    let done = sim.tick().0 - 1;
    assert_eq!(done - t0, 194, "195 ticks of work from placement");
    let king = sim.world().unit(KING0).unwrap();
    assert_eq!(king.speed, Fx::ZERO);
    let l = log.borrow();
    assert_eq!(l.calls_of(KING0, "StartBuilding"), vec![t0]);
    assert_eq!(l.calls_of(KING0, "StopBuilding"), vec![done]);
}

fn world_with(units: &[(tak_commands::UnitDefId, WorldPos)]) -> (World, Vec<UnitId>) {
    let mut w = World::new(&scenario()).unwrap();
    let ids = units
        .iter()
        .map(|&(d, p)| {
            w.spawn(d, tak_commands::PlayerId(0), p, tak_math::Angle::ZERO)
                .unwrap()
        })
        .collect();
    (w, ids)
}

#[test]
fn two_builders_on_one_site_add_their_work() {
    let (mut w, ids) = world_with(&[(KING, at(200, 200)), (MASON, at(300, 260))]);
    let site = at(260, 200);
    let c = build(0, 0, &ids, WALL, Some(site));
    construction::apply_command(&mut w, tak_commands::PlayerId(0), &c.command);
    let mut host = tak_sim::NoScripts;
    let mut ticks = 0;
    loop {
        construction::tick(&mut w, &mut host);
        ticks += 1;
        let walls: Vec<_> = w.units().filter(|(_, u)| u.def == WALL).collect();
        assert_eq!(
            walls.len(),
            1,
            "the second builder assists, no second frame"
        );
        if walls[0].1.build.is_complete() {
            break;
        }
        assert!(ticks < 400);
    }
    assert_eq!(ticks, 98, "ceil(195 / 2)");
    for &id in &ids {
        assert!(w.unit(id).unwrap().build.job.is_none());
        assert!(w.unit(id).unwrap().orders.is_empty());
    }
}

#[test]
fn interrupted_frames_persist_and_can_be_resumed() {
    let (mut sim, rec) = sim();
    let log = rec.log.clone();
    let mv = cmd(
        51,
        0,
        SimCommand::Move {
            units: UnitSet::single(KING0),
            to: at(200, 300),
            queue: QueueMode::Replace,
        },
    );
    let cmds = vec![
        build(1, 0, &[KING0], WALL, Some(at(260, 200))),
        mv,
        build(100, 0, &[KING0], WALL, Some(at(260, 200))),
    ];
    for _ in 0..100 {
        step(&mut sim, &log, &cmds);
    }
    let wall = find(&sim, WALL)[0];
    let left = sim.world().unit(wall).unwrap().build.percent_left();
    assert!(left > 0 && left < 100, "partly built frame stays ({left})");
    assert_eq!(log.borrow().calls_of(KING0, "StopBuilding"), vec![51]);
    let mut done_at = None;
    for _ in 0..400 {
        step(&mut sim, &log, &cmds);
        if done_at.is_none() && sim.world().unit(wall).unwrap().build.is_complete() {
            done_at = Some(sim.tick().0 - 1);
        }
    }
    assert_eq!(find(&sim, WALL).len(), 1, "resumed, not rebuilt");
    // 50 ticks of work before the interruption, 145 after resuming. The
    // king walked back into range first, so it ends some ticks after 244.
    let done_at = done_at.expect("finished");
    assert!(done_at >= 100 + 144, "{done_at}");
}

#[test]
fn factory_builds_a_queue_of_two_that_walk_out() {
    let (mut sim, rec) = sim();
    let log = rec.log.clone();
    // Keep (4x4) at (336, 208); 300 ticks to build.
    let mut cmds = vec![build(1, 0, &[KING0], KEEP, Some(at(336, 208)))];
    for _ in 0..=300 {
        step(&mut sim, &log, &cmds);
    }
    let keep = find(&sim, KEEP)[0];
    assert!(sim.world().unit(keep).unwrap().build.is_complete());
    assert_eq!(sim.world().unit(keep).unwrap().pos, at(336, 208));
    let mut b = build(301, 0, &[keep], SWORD, None);
    cmds.push(b.clone());
    b.seq = 1;
    b.command = match b.command {
        SimCommand::Build { units, def, at, .. } => SimCommand::Build {
            units,
            def,
            at,
            queue: QueueMode::Append,
        },
        c => c,
    };
    cmds.push(b);
    step(&mut sim, &log, &cmds);
    let q = &sim.world().unit(keep).unwrap().orders;
    assert_eq!(q.len(), 2);
    assert!(matches!(
        q[0],
        Order::Build {
            def: SWORD,
            at: None
        }
    ));
    for _ in 0..400 {
        step(&mut sim, &log, &cmds);
    }
    let swords = find(&sim, SWORD);
    assert_eq!(swords.len(), 2, "two products");
    let l = log.borrow();
    assert_eq!(l.calls_of(keep, "Activate"), vec![301]);
    let created: Vec<u32> = swords
        .iter()
        .map(|s| l.created.iter().find(|c| c.1 == *s).unwrap().0)
        .collect();
    // First product waits one tick for the script's INBUILDSTANCE.
    assert_eq!(created[0], 302);
    // 27 ticks each; the second waits for the first to clear the exit.
    assert!(created[1] > 302 + 27, "{created:?}");
    assert_eq!(l.calls_of(keep, "StartBuilding"), created);
    let stops = l.calls_of(keep, "StopBuilding");
    assert_eq!(stops[0], 302 + 26);
    assert_eq!(stops[1], created[1] + 26);
    assert_eq!(l.calls_of(keep, "Deactivate"), vec![stops[1]]);
    drop(l);
    for s in swords {
        let u = sim.world().unit(s).unwrap();
        assert!(u.build.is_complete());
        // Spawned in the yard (south row) and walked out past the footprint.
        assert!(u.pos.y > Fx::from_int(208 + 32), "{:?}", u.pos);
        assert_eq!(u.pos.x, Fx::from_int(336));
    }
    let k = sim.world().unit(keep).unwrap();
    assert!(k.orders.is_empty());
    assert_eq!(k.build.factory, construction::FactoryState::default());
}

#[test]
fn stopping_a_factory_cancels_and_refunds_the_product() {
    let (mut w, ids) = world_with(&[(KEEP, at(336, 208))]);
    let keep = ids[0];
    let c = build(0, 0, &[keep], SWORD, None);
    construction::apply_command(&mut w, tak_commands::PlayerId(0), &c.command);
    let mut host = tak_sim::NoScripts;
    for _ in 0..10 {
        construction::tick(&mut w, &mut host);
    }
    let sword: Vec<_> = w
        .units()
        .filter(|(_, u)| u.def == SWORD)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(sword.len(), 1);
    let paid = w.unit(sword[0]).unwrap().build.frame.unwrap().paid;
    assert!(paid > 0);
    w.unit_mut(keep).unwrap().orders.clear(); // what Stop does
    construction::tick(&mut w, &mut host);
    assert!(w.unit(sword[0]).is_none(), "frame removed");
    let r = w.construction.take_refunds();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].mana, paid);
    assert_eq!(
        w.unit(keep).unwrap().build.factory,
        construction::FactoryState::default()
    );
}

#[test]
fn frames_hold_still_and_ignore_orders() {
    let (mut sim, rec) = sim();
    let log = rec.log.clone();
    let mut cmds = vec![build(1, 0, &[KING0], MASON, Some(at(260, 200)))];
    for _ in 0..5 {
        step(&mut sim, &log, &cmds);
    }
    let mason = find(&sim, MASON)[0];
    let start = sim.world().unit(mason).unwrap().pos;
    cmds.push(cmd(
        5,
        0,
        SimCommand::Move {
            units: UnitSet::single(mason),
            to: at(900, 900),
            queue: QueueMode::Replace,
        },
    ));
    cmds.push(build(6, 0, &[mason], WALL, Some(at(500, 500))));
    // The mason (buildtime 6, 18 ticks) is a frame until tick 18.
    for _ in 5..18 {
        step(&mut sim, &log, &cmds);
        let u = sim.world().unit(mason).unwrap();
        assert!(!u.build.is_complete());
        assert!(u.orders.is_empty());
        assert_eq!(u.pos, start);
    }
}

#[test]
fn bad_orders_are_dropped() {
    let (mut w, ids) = world_with(&[
        (KING, at(200, 200)),
        (KEEP, at(336, 400)),
        (WALL, at(600, 600)),
    ]);
    let (king, keep) = (ids[0], ids[1]);
    let p0 = tak_commands::PlayerId(0);
    let apply = |w: &mut World, c: tak_commands::LoggedCommand| {
        construction::apply_command(w, c.player, &c.command)
    };
    // Not in the king's menu; a factory product with a site; a structure
    // for a factory; someone else's unit; a non-builder.
    apply(&mut w, build(0, 0, &[king], SWORD, Some(at(300, 300))));
    apply(&mut w, build(0, 0, &[king], WALL, None));
    apply(&mut w, build(0, 0, &[keep], SWORD, Some(at(300, 300))));
    apply(&mut w, build(0, 1, &[king], WALL, Some(at(300, 300))));
    apply(&mut w, build(0, 0, &[ids[2]], WALL, Some(at(300, 300))));
    assert!(w.unit(king).unwrap().orders.is_empty());
    assert!(w.unit(keep).unwrap().orders.is_empty());
    // A site overlapping a structure is refused when the builder gets there.
    apply(&mut w, build(0, 0, &[king], WALL, Some(at(330, 390))));
    assert_eq!(w.unit(king).unwrap().orders.len(), 1);
    assert_eq!(
        placement::check_site(&w, WALL, at(336, 400), None),
        Err(placement::SiteError::Occupied)
    );
    assert_eq!(
        placement::check_site(&w, WALL, at(8, 8), None),
        Err(placement::SiteError::Bounds)
    );
    assert_eq!(placement::check_site(&w, WALL, at(200, 200), None), Ok(()));
    let _ = p0;
}
