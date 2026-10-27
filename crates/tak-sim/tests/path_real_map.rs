//! Pathfinding on real maps (gated on `TAK_DATA_DIR`): a monarch crosses
//! Adamantine Gate from one plateau to the other through the canyon gap,
//! and 100 units with random targets all finish within a bounded number of
//! ticks under the per-tick step budget.

mod map_common;

use map_common::blocked;
use tak_commands::{
    LoggedCommand, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Angle, Fx, Rng, Vec2};
use tak_sim::content::load_match;
use tak_sim::pathfinding::{Outcome, PathStatus, STEPS_PER_TICK};
use tak_sim::systems::ARRIVAL_TOLERANCE;
use tak_sim::{Match, PlayerSetup, Sim, World, WorldPos};

const MAP: &str = "kmap/Adamantine Gate";

fn load(test: &str, players: &str) -> Option<Match> {
    let dir = tak_vfs::testing::data_dir_or_skip(test)?;
    let vfs = tak_vfs::open_kingdoms(&dir).expect("open install");
    Some(load_match(&vfs, MAP, &PlayerSetup::parse_list(players), 7).expect("load match"))
}

fn mv(tick: u32, player: u8, unit: UnitId, to: WorldPos) -> LoggedCommand {
    LoggedCommand {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: unit.index,
        command: SimCommand::Move {
            units: UnitSet::single(unit),
            to,
            queue: QueueMode::Replace,
        },
    }
}

/// Blocked cells crossed by the straight line `a`–`b` (sampled every 4 units).
fn blocked_on_line(world: &World, a: WorldPos, b: WorldPos) -> usize {
    let len = a.distance(b).to_int_floor().max(1);
    let d = (b - a).normalize().unwrap_or(Vec2::ZERO);
    (0..len / 4)
        .filter(|k| blocked(world, a + d.scale(Fx::from_int(4 * k))))
        .count()
}

#[test]
fn monarch_crosses_between_plateaus_through_the_canyon_gap() {
    let Some(m) = load(
        "monarch_crosses_between_plateaus_through_the_canyon_gap",
        "ARA,TAR",
    ) else {
        return;
    };
    let sc = m.scenario.clone();
    let mut sim = Sim::with_scripts(&sc, Box::new(m.scripts)).unwrap();
    sim.step(&[]);
    let id = UnitId::new(0, 0);
    let from = sim.world().unit(id).unwrap().pos;
    // Player 1's plateau, a little off its start marker.
    let to = Vec2::from_ints(1000, 2400);
    assert!(
        blocked_on_line(sim.world(), from, to) > 0,
        "the straight line is blocked"
    );
    sim.step(&[mv(1, 0, id, to)]);
    let mut ticks = 0;
    let mut detour = Fx::ZERO;
    let mut prev = from;
    for _ in 0..6000 {
        let u = sim.world().unit(id).unwrap();
        assert!(
            !blocked(sim.world(), u.pos),
            "entered a blocked cell at {:?}",
            u.pos
        );
        detour = detour.saturating_add(prev.distance(u.pos));
        prev = u.pos;
        if u.orders.is_empty() {
            break;
        }
        sim.step(&[]);
        ticks += 1;
    }
    let u = sim.world().unit(id).unwrap();
    assert_eq!(
        u.path.outcome,
        Outcome::Arrived,
        "at {:?} after {ticks} ticks, {:?}",
        u.pos,
        u.path
    );
    assert!(u.pos.distance(to) <= ARRIVAL_TOLERANCE);
    eprintln!(
        "plateau crossing: {ticks} ticks, walked {} units (straight line {}), stats {:?}",
        detour.to_int_floor(),
        from.distance(to).to_int_floor(),
        sim.world().pathing.stats
    );
}

/// 100 Aramon monarchs on random open cells, each ordered to a random
/// point. Every unit must finish (arrive, or stop at the reachable point
/// nearest an unreachable goal) within a bounded number of ticks, and the
/// planner must never exceed its per-tick step budget.
#[test]
fn hundred_units_with_random_targets_all_finish_under_budget() {
    let Some(m) = load(
        "hundred_units_with_random_targets_all_finish_under_budget",
        "ARA,TAR",
    ) else {
        return;
    };
    let sc = m.scenario.clone();
    let mut sim = Sim::new(&sc).unwrap();
    sim.step(&[]);
    let mut rng = Rng::new(2026);
    let (w, h) = (sc.map.width, sc.map.height);
    let mut ids = Vec::new();
    while ids.len() < 100 {
        let p = Vec2::from_ints(rng.range_i32(64, w - 64), rng.range_i32(64, h - 64));
        if blocked(sim.world(), p) {
            continue;
        }
        let a = Angle::from_raw(rng.next_u32() as u16);
        ids.push(
            sim.world_mut()
                .spawn(UnitDefId(0), PlayerId(0), p, a)
                .unwrap(),
        );
    }
    let t = sim.tick().0;
    let cmds: Vec<_> = ids
        .iter()
        .map(|&id| {
            let to = Vec2::from_ints(rng.range_i32(64, w - 64), rng.range_i32(64, h - 64));
            mv(t, 0, id, to)
        })
        .collect();
    let started = std::time::Instant::now();
    sim.step(&cmds);
    const LIMIT: u32 = 9000;
    let mut ticks = 1;
    let mut max_steps = 0;
    let mut planned_by = 0;
    while ticks < LIMIT {
        let w = sim.world();
        max_steps = max_steps.max(w.pathing.stats.steps_last_tick);
        assert!(w.pathing.stats.steps_last_tick <= STEPS_PER_TICK);
        let waiting = ids.iter().any(|&id| {
            matches!(
                w.unit(id).unwrap().path.status,
                PathStatus::Pending | PathStatus::Searching
            )
        });
        if planned_by == 0 && !waiting {
            planned_by = ticks;
        }
        if ids.iter().all(|&id| w.unit(id).unwrap().orders.is_empty()) {
            break;
        }
        sim.step(&[]);
        ticks += 1;
    }
    let elapsed = started.elapsed();
    let w = sim.world();
    let mut arrived = 0;
    let mut unreachable = 0;
    let mut gave_up = Vec::new();
    for &id in &ids {
        let u = w.unit(id).unwrap();
        assert!(
            u.orders.is_empty(),
            "{id:?} still busy at {:?}: {:?}",
            u.pos,
            u.path
        );
        match u.path.outcome {
            Outcome::Arrived => arrived += 1,
            Outcome::Unreachable => unreachable += 1,
            o => gave_up.push((id, o, u.pos)),
        }
    }
    eprintln!(
        "100 units: {arrived} arrived, {unreachable} unreachable, gave up {gave_up:?}; \
         all planned by tick {planned_by}, all done by tick {ticks}; max steps/tick {max_steps} \
         (budget {STEPS_PER_TICK}); {:.0} ticks/s; stats {:?}",
        f64::from(ticks) / elapsed.as_secs_f64(),
        w.pathing.stats
    );
    assert!(gave_up.is_empty(), "gave up: {gave_up:?}");
    assert_eq!(arrived + unreachable, 100);
    assert!(ticks < LIMIT);
    assert!(max_steps <= STEPS_PER_TICK);
}
