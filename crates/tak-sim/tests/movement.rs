//! Movement: arrival within tolerance, turn-rate limits, Stop, queue modes,
//! ownership validation.

use tak_commands::{
    CommandLog, FireState, LoggedCommand, PlayerId, QueueMode, SimCommand, Tick, UnitId, UnitSet,
};
use tak_math::{Fx, Vec2};
use tak_sim::systems::ARRIVAL_TOLERANCE;
use tak_sim::testing::example_scenario;
use tak_sim::{Order, Sim, Unit};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const U0: UnitId = UnitId::new(0, 0);
const U1: UnitId = UnitId::new(1, 0);

fn cmd(tick: u32, player: PlayerId, seq: u32, command: SimCommand) -> LoggedCommand {
    LoggedCommand {
        tick: Tick(tick),
        player,
        seq,
        command,
    }
}

fn mv(unit: UnitId, x: i32, y: i32, queue: QueueMode) -> SimCommand {
    SimCommand::Move {
        units: UnitSet::single(unit),
        to: Vec2::from_ints(x, y),
        queue,
    }
}

/// A two-player sim after tick 0 (monarchs spawned).
fn started() -> Sim {
    let mut sim = Sim::new(&example_scenario(1, 2)).unwrap();
    sim.step(&[]);
    sim
}

fn unit(sim: &Sim, id: UnitId) -> Unit {
    sim.world().unit(id).unwrap().clone()
}

/// Step until `done` holds or `limit` ticks pass; returns ticks taken.
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
fn monarchs_spawn_on_tick_zero() {
    let mut sim = Sim::new(&example_scenario(1, 3)).unwrap();
    assert_eq!(sim.world().unit_count(), 0);
    sim.step(&[]);
    assert_eq!(sim.world().unit_count(), 3);
    for (i, p) in sim.world().players().iter().enumerate() {
        let id = p.monarch.unwrap();
        assert_eq!(id, UnitId::new(i as u32, 0));
        let u = sim.world().unit(id).unwrap();
        assert_eq!(u.owner, p.id);
        assert_eq!(u.pos, p.start);
    }
    // Player 0 at (256, 256) faces the centre (1024, 1024): south-east.
    assert_eq!(unit(&sim, U0).heading.raw(), 0x2000);
}

#[test]
fn arrives_within_tolerance_and_stays() {
    let mut sim = started();
    let target = Vec2::from_ints(900, 400);
    sim.step(&[cmd(1, P0, 0, mv(U0, 900, 400, QueueMode::Replace))]);
    let took = run_until(&mut sim, 3000, |s| unit(s, U0).orders.is_empty()).expect("arrives");
    assert!(took > 100, "it is ~660 units away at <= 1.5/tick");
    let u = unit(&sim, U0);
    assert!(u.pos.distance(target) <= ARRIVAL_TOLERANCE, "{:?}", u.pos);
    assert_eq!(u.speed, Fx::ZERO);
    for _ in 0..60 {
        sim.step(&[]);
    }
    assert_eq!(unit(&sim, U0).pos, u.pos);
}

#[test]
fn never_exceeds_turn_rate_or_max_speed() {
    let mut sim = started();
    let def = sim.world().defs().as_slice()[0].clone();
    // Target behind the unit, forcing a large turn.
    sim.step(&[cmd(1, P0, 0, mv(U0, 100, 100, QueueMode::Replace))]);
    let mut prev = unit(&sim, U0);
    let mut max_turn = 0u32;
    for _ in 0..2000 {
        sim.step(&[]);
        let u = unit(&sim, U0);
        let turn = u32::from(prev.heading.delta_to(u.heading).unsigned_abs());
        assert!(turn <= u32::from(def.turnrate), "turned {turn}");
        assert!(u.speed <= def.maxvelocity && u.speed >= Fx::ZERO);
        // Speed changes by at most accel up or brake down (or zeroes on
        // arrival).
        if !u.orders.is_empty() {
            assert!(u.speed - prev.speed <= def.acceleration);
            assert!(prev.speed - u.speed <= def.brakerate);
        }
        max_turn = max_turn.max(turn);
        prev = u;
    }
    assert_eq!(max_turn, u32::from(def.turnrate), "turn was rate-limited");
    assert!(prev.orders.is_empty());
    assert!(prev.pos.distance(Vec2::from_ints(100, 100)) <= ARRIVAL_TOLERANCE);
}

#[test]
fn stop_halts_the_unit() {
    let mut sim = started();
    sim.step(&[cmd(1, P0, 0, mv(U0, 1800, 300, QueueMode::Replace))]);
    for _ in 0..200 {
        sim.step(&[]);
    }
    let before = unit(&sim, U0);
    assert!(before.speed > Fx::ZERO);
    let t = sim.tick().0;
    sim.step(&[cmd(
        t,
        P0,
        0,
        SimCommand::Stop {
            units: UnitSet::single(U0),
        },
    )]);
    assert!(unit(&sim, U0).orders.is_empty());
    // Brakes at 0.25/tick from at most 1.5: halted within 6 ticks.
    let took = run_until(&mut sim, 10, |s| unit(s, U0).speed == Fx::ZERO).unwrap();
    assert!(took <= 6);
    let halted = unit(&sim, U0).pos;
    for _ in 0..30 {
        sim.step(&[]);
    }
    assert_eq!(unit(&sim, U0).pos, halted);
}

#[test]
fn append_queues_and_replace_clears() {
    let mut sim = started();
    sim.step(&[
        cmd(1, P0, 0, mv(U0, 400, 256, QueueMode::Replace)),
        cmd(1, P0, 1, mv(U0, 400, 400, QueueMode::Append)),
        cmd(1, P0, 2, mv(U0, 256, 400, QueueMode::Append)),
    ]);
    assert_eq!(unit(&sim, U0).orders.len(), 3);
    // Visits the waypoints in order.
    let mut seen = Vec::new();
    for _ in 0..3000 {
        let n = unit(&sim, U0).orders.len();
        sim.step(&[]);
        if unit(&sim, U0).orders.len() < n {
            seen.push(unit(&sim, U0).pos);
        }
    }
    assert_eq!(seen.len(), 3);
    assert!(seen[0].distance(Vec2::from_ints(400, 256)) <= ARRIVAL_TOLERANCE * Fx::from_int(2));
    assert!(seen[2].distance(Vec2::from_ints(256, 400)) <= ARRIVAL_TOLERANCE);

    let t = sim.tick().0;
    sim.step(&[
        cmd(t, P0, 0, mv(U0, 900, 900, QueueMode::Append)),
        cmd(t, P0, 1, mv(U0, 1000, 1000, QueueMode::Append)),
        cmd(t, P0, 2, mv(U0, 300, 300, QueueMode::Replace)),
    ]);
    let u = unit(&sim, U0);
    assert_eq!(
        u.orders.iter().copied().collect::<Vec<_>>(),
        vec![Order::Move {
            to: Vec2::from_ints(300, 300)
        }]
    );
}

#[test]
fn foreign_and_stale_ids_are_ignored() {
    let mut sim = started();
    let before = sim.state_hash();
    let mut probe = started();
    probe.step(&[
        // Player 1 tries to move player 0's unit.
        cmd(1, P1, 0, mv(U0, 900, 900, QueueMode::Replace)),
        // Stale generation and out-of-range slot.
        cmd(
            1,
            P0,
            0,
            mv(UnitId::new(0, 1), 900, 900, QueueMode::Replace),
        ),
        cmd(
            1,
            P0,
            1,
            mv(UnitId::new(77, 0), 900, 900, QueueMode::Replace),
        ),
        // Unknown player.
        cmd(1, PlayerId(9), 0, mv(U0, 900, 900, QueueMode::Replace)),
        // Wrong tick stamp.
        cmd(5, P0, 0, mv(U0, 900, 900, QueueMode::Replace)),
    ]);
    sim.step(&[]);
    assert_ne!(before, sim.state_hash(), "the tick counter advanced");
    assert_eq!(probe.state_hash(), sim.state_hash());
    assert!(unit(&probe, U0).orders.is_empty());
    assert!(unit(&probe, U1).orders.is_empty());
}

#[test]
fn mixed_unit_set_applies_only_to_owned_units() {
    let mut sim = started();
    let units: UnitSet = [U0, U1].into_iter().collect();
    sim.step(&[cmd(
        1,
        P1,
        0,
        SimCommand::SetFireState {
            units,
            state: FireState::HoldFire,
        },
    )]);
    assert_eq!(unit(&sim, U0).fire_state, FireState::FireAtWill);
    assert_eq!(unit(&sim, U1).fire_state, FireState::HoldFire);
}

#[test]
fn off_map_targets_are_clamped() {
    let mut sim = started();
    sim.step(&[cmd(1, P0, 0, mv(U0, -500, 256, QueueMode::Replace))]);
    assert_eq!(
        unit(&sim, U0).orders.front(),
        Some(&Order::Move {
            to: Vec2::from_ints(0, 256)
        })
    );
    run_until(&mut sim, 3000, |s| unit(s, U0).orders.is_empty()).expect("arrives at the edge");
    let p = unit(&sim, U0).pos;
    assert!(p.x >= Fx::ZERO && p.distance(Vec2::from_ints(0, 256)) <= ARRIVAL_TOLERANCE);
}

#[test]
fn order_to_own_position_completes_immediately() {
    let mut sim = started();
    sim.step(&[cmd(1, P0, 0, mv(U0, 256, 256, QueueMode::Replace))]);
    assert!(unit(&sim, U0).orders.is_empty());
    assert_eq!(unit(&sim, U0).pos, Vec2::from_ints(256, 256));
}

#[test]
fn command_log_drives_the_same_result_as_direct_steps() {
    let mut log = CommandLog::new();
    log.record(Tick(1), P0, mv(U0, 700, 300, QueueMode::Replace));
    log.record(Tick(40), P1, mv(U1, 1500, 1200, QueueMode::Replace));
    let mut a = Sim::new(&example_scenario(1, 2)).unwrap();
    let ha = a.run(&log, 400);
    let mut b = started();
    let mut hb = vec![b.state_hash()];
    for t in 1..400 {
        hb.push(b.step(log.for_tick(Tick(t))));
    }
    assert_eq!(ha, hb);
}

#[test]
fn random_targets_always_arrive() {
    // Both definitions (fast/agile and slow/stiff), many headings and
    // distances, including targets inside the turning circle.
    let mut rng = tak_math::Rng::new(4242);
    let mut sim = started();
    for i in 0..150u32 {
        let id = if i % 2 == 0 { U0 } else { U1 };
        let player = if i % 2 == 0 { P0 } else { P1 };
        let here = unit(&sim, id).pos;
        let r = rng.range_i32(1, 400);
        let a = tak_math::Angle::from_raw(rng.next_u32() as u16);
        let to = sim
            .world()
            .map()
            .clamp(here + Vec2::from_angle(a).scale(Fx::from_int(r)));
        let t = sim.tick().0;
        sim.step(&[cmd(
            t,
            player,
            0,
            SimCommand::Move {
                units: UnitSet::single(id),
                to,
                queue: QueueMode::Replace,
            },
        )]);
        run_until(&mut sim, 2000, |s| unit(s, id).orders.is_empty())
            .unwrap_or_else(|| panic!("move {i} to {to:?} never arrived"));
        assert!(unit(&sim, id).pos.distance(to) <= ARRIVAL_TOLERANCE);
    }
}
