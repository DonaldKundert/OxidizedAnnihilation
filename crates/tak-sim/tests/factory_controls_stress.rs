//! CI-bounded production soak using synthetic content and command-only replay.
//! The oracle models the documented finite FIFO/recipe edits, not work timing.
//! No resource, progress, queue, or outcome fields are edited by the soak.
mod factory_controls_support;

use std::collections::BTreeSet;

use factory_controls_support::{KEEP, KING, MASON, SWORD, at, scenario};
use tak_commands::{
    CommandLog, FactoryRallyOrder, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId,
    UnitSet,
};
use tak_math::{Angle, Fx, Rng};
use tak_sim::campaign::InitialUnit;
use tak_sim::world::MAX_ORDERS;
use tak_sim::{Order, Scenario, Sim, construction, economy};

const FACTORIES: u32 = 4;
const STOP_TICK: u32 = 1600;
const TICKS: u32 = STOP_TICK + 20;
const SEEDS: [u64; 4] = [1, 0x5eed, 0xdecafbad, u64::MAX];

fn factory(index: u32) -> UnitId {
    UnitId::new(index, 0)
}

fn owner(index: u32) -> PlayerId {
    PlayerId((index / 2) as u8)
}

fn initial(def: UnitDefId, owner: PlayerId, x: i32, y: i32) -> InitialUnit {
    InitialUnit {
        def,
        owner,
        pos: at(x, y),
        heading: Angle::ZERO,
        health_percentage: 100,
        mana_percentage: 100,
        kills: 0,
    }
}

fn soak_scenario(seed: u64) -> Scenario {
    let mut sc = scenario();
    sc.seed = seed;
    sc.economy.rules.start_mana = 0;
    sc.economy.rules.site_slack = 0;
    sc.economy.rules.off_site_multiplier = Fx::ZERO;
    // Exact two mana/tick while on a site, and zero off it. Ordinary Move
    // commands turn income off/on; two factories share each owner's supply.
    let generator = &mut sc.unit_defs[KING.0 as usize];
    generator.economy.mogriumincome = 60;
    generator.economy.sacred_site = true;
    sc.economy.sacred_sites = vec![
        economy::SacredSite {
            x: 48,
            y: 360,
            w: 32,
            h: 32,
            multiplier: Fx::ONE,
        },
        economy::SacredSite {
            x: 944,
            y: 760,
            w: 32,
            h: 32,
            multiplier: Fx::ONE,
        },
    ];
    let blocker = UnitDefId(sc.unit_defs.len() as u32);
    let mut blocker_def = sc.unit_defs[SWORD.0 as usize].clone();
    blocker_def.name = "SYNTHETIC_EXIT_BLOCKER".into();
    // Keep these parked over the pad until commanded away, rather than
    // allowing ground-unit separation to clear the obstruction by itself.
    blocker_def.motion.canfly = true;
    sc.unit_defs.push(blocker_def);
    sc.initial_units = Some(vec![
        initial(KEEP, PlayerId(0), 200, 200),
        initial(KEEP, PlayerId(0), 600, 200),
        initial(KEEP, PlayerId(1), 200, 700),
        initial(KEEP, PlayerId(1), 600, 700),
        initial(KING, PlayerId(0), 64, 300),
        initial(KING, PlayerId(1), 960, 700),
        initial(blocker, PlayerId(0), 600, 224),
        initial(blocker, PlayerId(1), 600, 724),
    ]);
    sc
}

fn build(units: UnitSet, def: UnitDefId) -> SimCommand {
    SimCommand::Build {
        units,
        def,
        at: None,
        queue: QueueMode::Append,
    }
}

fn repeat(units: UnitSet, def: Option<UnitDefId>) -> SimCommand {
    SimCommand::SetFactoryRepeat { units, def }
}

fn owned_factories(player: PlayerId) -> UnitSet {
    (0..FACTORIES)
        .filter(|&i| owner(i) == player)
        .map(factory)
        .collect()
}

fn schedule(seed: u64) -> CommandLog {
    let mut log = CommandLog::new();
    for player in [PlayerId(0), PlayerId(1)] {
        let units = owned_factories(player);
        log.record(Tick(1), player, repeat(units.clone(), Some(SWORD)));
        // Exercise admission limits while products cannot yet spend mana.
        for n in 0..MAX_ORDERS + 5 {
            log.record(
                Tick(3),
                player,
                build(units.clone(), if n % 2 == 0 { MASON } else { SWORD }),
            );
            log.record(
                Tick(5),
                player,
                SimCommand::SetFactoryRally {
                    units: units.clone(),
                    order: Some(FactoryRallyOrder::Patrol { to: at(400, 480) }),
                    queue: QueueMode::Append,
                },
            );
        }
        log.record(
            Tick(60),
            player,
            SimCommand::Stop {
                units: units.clone(),
            },
        );
        log.record(Tick(61), player, repeat(units.clone(), Some(SWORD)));
        log.record(
            Tick(61),
            player,
            SimCommand::SetFactoryRally {
                units: units.clone(),
                order: Some(FactoryRallyOrder::Move { to: at(400, 460) }),
                queue: QueueMode::Replace,
            },
        );
        log.record(
            Tick(62),
            player,
            SimCommand::SetFactoryRally {
                units: units.clone(),
                order: Some(FactoryRallyOrder::Patrol { to: at(750, 460) }),
                queue: QueueMode::Append,
            },
        );
        log.record(
            Tick(90),
            player,
            SimCommand::Move {
                units: UnitSet::single(UnitId::new(6 + u32::from(player.0), 0)),
                to: at(820, 400 + 500 * i32::from(player.0)),
                queue: QueueMode::Replace,
            },
        );
        for (tick, on_site) in [
            (120, true),
            (600, false),
            (800, true),
            (1100, false),
            (1250, true),
        ] {
            log.record(
                Tick(tick),
                player,
                SimCommand::Move {
                    units: UnitSet::single(UnitId::new(4 + u32::from(player.0), 0)),
                    to: at(
                        if player.0 == 0 { 64 } else { 960 },
                        300 + 400 * i32::from(player.0) + if on_site { 76 } else { 0 },
                    ),
                    queue: QueueMode::Replace,
                },
            );
        }
        // A quiet production interval makes successful finite/repeat completion
        // mandatory, rather than letting random edits cancel everything.
        for def in [MASON, SWORD, MASON] {
            log.record(Tick(240), player, build(units.clone(), def));
        }
        log.record(Tick(STOP_TICK), player, SimCommand::Stop { units });
    }
    let mut rng = Rng::with_stream(seed, 0xfac70);
    for tick in (340..STOP_TICK).step_by(5) {
        for _ in 0..1 + rng.below(3) {
            let player = PlayerId(rng.below(2) as u8);
            let mut units = if rng.chance(1, 3) {
                // Include foreign factories: ownership filtering must apply to
                // each recipient, not to the entire mixed command.
                (0..FACTORIES).map(factory).collect()
            } else {
                UnitSet::single(factory(2 * u32::from(player.0) + rng.below(2)))
            };
            units.insert(UnitId::new(rng.below(FACTORIES), 1));
            let def = if rng.chance(1, 2) { SWORD } else { MASON };
            let count = if rng.chance(1, 2) { 1 } else { 5 };
            let to = at(rng.range_i32(100, 925), rng.range_i32(320, 620));
            let command = match rng.below(18) {
                0..=4 => {
                    for _ in 1..count {
                        log.record(Tick(tick), player, build(units.clone(), def));
                    }
                    build(units, def)
                }
                5..=7 => SimCommand::CancelProduction { units, def, count },
                8..=10 => repeat(units, Some(def)),
                11 => repeat(units, None),
                12 => SimCommand::Stop { units },
                13 => SimCommand::SetFactoryRally {
                    units,
                    order: Some(FactoryRallyOrder::Move { to }),
                    queue: QueueMode::Replace,
                },
                14 => SimCommand::SetFactoryRally {
                    units,
                    order: Some(FactoryRallyOrder::Patrol { to }),
                    queue: QueueMode::Append,
                },
                15 => SimCommand::SetFactoryRally {
                    units,
                    order: Some(FactoryRallyOrder::Move { to }),
                    queue: QueueMode::Append,
                },
                16 => SimCommand::SetFactoryRally {
                    units,
                    order: None,
                    queue: QueueMode::Replace,
                },
                _ => SimCommand::CancelProduction {
                    units,
                    def,
                    count: 0,
                },
            };
            let stop = matches!(command, SimCommand::Stop { .. });
            log.record(Tick(tick), player, command);
            // Legacy finite Stop+Build in one tick intentionally retains its
            // historical replay behavior. Keep Stop last; explicit cancels
            // still interleave freely with later same-tick production edits.
            if stop {
                break;
            }
        }
    }
    log
}

#[derive(Clone, Copy)]
struct Active {
    id: UnitId,
    def: UnitDefId,
    repeated: bool,
    paid: i64,
}

#[derive(Default)]
struct Model {
    finite: Vec<UnitDefId>,
    repeat: Option<UnitDefId>,
    rally: Option<Vec<FactoryRallyOrder>>,
}

impl Model {
    fn edit(&mut self, command: &SimCommand, active: Option<Active>, cancelled: &mut bool) {
        match command {
            SimCommand::Build { def, .. } if self.finite.len() < MAX_ORDERS => {
                self.finite.push(*def)
            }
            SimCommand::SetFactoryRepeat { def, .. } => self.repeat = *def,
            SimCommand::CancelProduction { def, count, .. } if *count > 0 => {
                if self.repeat == Some(*def) {
                    self.repeat = None;
                }
                let mut left = *count;
                for index in (0..self.finite.len()).rev() {
                    if left > 0 && self.finite[index] == *def {
                        self.finite.remove(index);
                        left -= 1;
                        if index == 0 && active.is_some_and(|a| !a.repeated) && !*cancelled {
                            *cancelled = true;
                        }
                    }
                }
                if left > 0 && active.is_some_and(|a| a.repeated && a.def == *def) {
                    *cancelled = true;
                }
            }
            SimCommand::Stop { .. } => {
                self.finite.clear();
                self.repeat = None;
                if let Some(route) = &mut self.rally {
                    route.clear();
                }
                *cancelled |= active.is_some();
            }
            SimCommand::SetFactoryRally { order, queue, .. } => {
                let route = self.rally.get_or_insert_with(Vec::new);
                if *queue == QueueMode::Replace || order.is_none() {
                    route.clear();
                }
                if let Some(order) = order {
                    let patrol = matches!(order, FactoryRallyOrder::Patrol { .. })
                        || route
                            .iter()
                            .any(|o| matches!(o, FactoryRallyOrder::Patrol { .. }));
                    if route.len() + 1 + usize::from(patrol) <= MAX_ORDERS {
                        route.push(*order);
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Default, Debug)]
struct Coverage {
    finite_completed: usize,
    repeat_completed: usize,
    paid_cancellations: usize,
    blocked_ticks: usize,
    starved_ticks: usize,
    resumed_frames: usize,
    full_queues: usize,
    full_routes: usize,
}

fn active(sim: &Sim, id: UnitId) -> Option<Active> {
    let state = sim.world().unit(id)?.build.factory;
    let id = state.product?;
    let product = sim
        .world()
        .unit(id)
        .expect("factory references a live product");
    Some(Active {
        id,
        def: product.def,
        repeated: state.product_from_repeat,
        paid: product
            .build
            .frame
            .expect("on-pad product must be unfinished")
            .paid,
    })
}

fn check_accounting(sim: &Sim, supplied: &[i64; 2], lost: &[i64; 2], context: &str) {
    let mut committed = [0i64; 2];
    for (_, unit) in sim
        .world()
        .units()
        .filter(|(_, u)| [SWORD, MASON].contains(&u.def))
    {
        let cost = sim.world().defs().get(unit.def).unwrap().build.buildcost << 16;
        let paid = unit.build.frame.map_or(cost, |frame| {
            assert!(
                (0..=cost).contains(&frame.paid),
                "invalid paid cost: {context}"
            );
            assert!(
                (0..=frame.total).contains(&frame.done),
                "invalid work: {context}"
            );
            frame.paid
        });
        committed[unit.owner.0 as usize] += paid;
    }
    for index in 0..2 {
        let mana = economy::mana(sim.world(), PlayerId(index as u8)).0;
        assert!(mana >= 0, "negative mana: {context}");
        assert_eq!(
            mana + committed[index] + lost[index],
            supplied[index],
            "mana + live paid/completed cost + losses must equal supply: player {index}, {context}"
        );
    }
}

fn run(sc: &Scenario, log: &CommandLog) -> Vec<u64> {
    let mut sim = Sim::new(sc).unwrap();
    let mut models: Vec<Model> = (0..FACTORIES).map(|_| Model::default()).collect();
    let mut supplied = [0i64; 2];
    let mut coverage = Coverage::default();
    let mut stalled = BTreeSet::new();
    let mut hashes = Vec::new();
    for tick in 0..TICKS {
        let context = format!("seed {}, tick {tick}", sc.seed);
        let before: Vec<_> = (0..FACTORIES).map(|i| active(&sim, factory(i))).collect();
        let mut cancelled = [false; FACTORIES as usize];
        let commands = log.for_tick(Tick(tick));
        for command in commands {
            for index in 0..FACTORIES {
                if command.player == owner(index)
                    && command.command.units().contains(factory(index))
                {
                    models[index as usize].edit(
                        &command.command,
                        before[index as usize],
                        &mut cancelled[index as usize],
                    );
                }
            }
        }
        hashes.push(sim.step(commands));
        if matches!(tick, 2 | 89) {
            for index in [1, 3] {
                assert!(
                    active(&sim, factory(index)).is_none(),
                    "blocked prelude: {context}"
                );
                assert!(
                    sim.world()
                        .unit(factory(index))
                        .unwrap()
                        .build
                        .factory
                        .retry_at
                        .is_some()
                );
            }
        } else if tick == 115 {
            // Both blockers have moved off their pads, but the income units
            // have not received their first movement command yet.
            for index in 0..FACTORIES {
                assert_eq!(
                    active(&sim, factory(index)).unwrap().paid,
                    0,
                    "unblocked, starved prelude: {context}"
                );
            }
        }
        let mut referenced = BTreeSet::new();
        for index in 0..FACTORIES {
            let id = factory(index);
            let model = &mut models[index as usize];
            let factory_unit = sim.world().unit(id).unwrap();
            let state = factory_unit.build.factory;
            if let Some(previous) = before[index as usize] {
                if cancelled[index as usize] {
                    assert!(
                        sim.world().unit(previous.id).is_none(),
                        "cancel must remove frame: factory {index}, {context}, commands {commands:?}"
                    );
                    assert_eq!(
                        state.product, None,
                        "cancel tick cannot start replacement: {context}"
                    );
                    coverage.paid_cancellations += usize::from(previous.paid > 0);
                } else if sim.world().unit(previous.id).unwrap().build.is_complete() {
                    if previous.repeated {
                        coverage.repeat_completed += 1;
                    } else {
                        assert_eq!(
                            model.finite.remove(0),
                            previous.def,
                            "FIFO completion: {context}"
                        );
                        coverage.finite_completed += 1;
                    }
                }
            }
            let actual: Vec<_> = factory_unit
                .orders
                .iter()
                .map(|order| match order {
                    Order::Build { def, at: None } => *def,
                    other => panic!("unexpected production order {other:?}: {context}"),
                })
                .collect();
            assert_eq!(
                actual, model.finite,
                "finite queue oracle: factory {index}, {context}"
            );
            assert_eq!(
                factory_unit.build.factory_repeat, model.repeat,
                "repeat oracle: {context}"
            );
            assert_eq!(
                factory_unit.build.factory_rally, model.rally,
                "rally oracle: {context}"
            );
            assert!(!state.cancel_product, "unresolved cancellation: {context}");
            assert!(
                !state.product_from_repeat || state.product.is_some(),
                "orphan origin: {context}"
            );
            assert!(actual.len() <= MAX_ORDERS, "finite bound: {context}");
            coverage.full_queues += usize::from(actual.len() == MAX_ORDERS);
            if let Some(route) = &model.rally {
                let length = route.len()
                    + usize::from(
                        route
                            .iter()
                            .any(|o| matches!(o, FactoryRallyOrder::Patrol { .. })),
                    );
                assert!(length <= MAX_ORDERS, "released route bound: {context}");
                coverage.full_routes += usize::from(length == MAX_ORDERS);
            }
            coverage.blocked_ticks += usize::from(state.retry_at.is_some());
            let current = active(&sim, id);
            if let Some(product) = current {
                assert!(
                    referenced.insert(product.id),
                    "shared paid frame: {context}"
                );
                let unit = sim.world().unit(product.id).unwrap();
                assert_eq!(unit.owner, owner(index), "wrong product owner: {context}");
                if !product.repeated {
                    assert_eq!(
                        actual.first(),
                        Some(&product.def),
                        "active finite FIFO: {context}"
                    );
                }
                if before[index as usize].is_none() {
                    let expected = model.finite.first().copied().or(model.repeat);
                    assert_eq!(
                        Some(product.def),
                        expected,
                        "next product selection: {context}"
                    );
                    assert_eq!(
                        product.repeated,
                        model.finite.is_empty(),
                        "next product origin: {context}"
                    );
                }
                if let Some(previous) = before[index as usize].filter(|a| a.id == product.id) {
                    if previous.paid == product.paid
                        && economy::income_per_second(sim.world(), owner(index)).0 == 0
                    {
                        coverage.starved_ticks += 1;
                        stalled.insert(product.id);
                    } else if product.paid > previous.paid && stalled.remove(&product.id) {
                        coverage.resumed_frames += 1;
                    }
                }
            }
            for def in [SWORD, MASON] {
                let count = actual.iter().filter(|&&d| d == def).count()
                    + usize::from(current.is_some_and(|a| a.repeated && a.def == def));
                assert_eq!(
                    construction::production_count(sim.world(), id, def) as usize,
                    count,
                    "active-inclusive count: {context}"
                );
            }
        }
        let frames: BTreeSet<_> = sim
            .world()
            .units()
            .filter(|(_, u)| !u.build.is_complete())
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            frames, referenced,
            "each paid frame belongs to exactly one factory: {context}"
        );
        for (_, unit) in sim.world().units() {
            assert!(
                unit.orders.len() <= MAX_ORDERS,
                "unit order bound: {context}"
            );
        }
        for player in [PlayerId(0), PlayerId(1)] {
            let income = economy::income_per_second(sim.world(), player).0;
            assert_eq!(income % i64::from(tak_sim::TICKS_PER_SECOND), 0);
            supplied[player.0 as usize] += income / i64::from(tak_sim::TICKS_PER_SECOND);
        }
        assert!(
            sim.world().construction.refunds.is_empty(),
            "uncredited refund: {context}"
        );
        check_accounting(&sim, &supplied, &[0, 0], &context);
    }
    assert!(
        coverage.finite_completed >= 4,
        "finite completions: {coverage:?}"
    );
    assert!(
        coverage.repeat_completed >= 4,
        "repeat completions: {coverage:?}"
    );
    assert!(
        coverage.paid_cancellations >= 4,
        "paid cancellations: {coverage:?}"
    );
    assert!(coverage.blocked_ticks >= 30, "blocked exits: {coverage:?}");
    assert!(coverage.starved_ticks >= 100, "starvation: {coverage:?}");
    assert!(coverage.resumed_frames >= 4, "resumption: {coverage:?}");
    assert!(
        coverage.full_queues > 0 && coverage.full_routes > 0,
        "admission bounds: {coverage:?}"
    );
    assert!(
        models
            .iter()
            .all(|m| m.finite.is_empty() && m.repeat.is_none())
    );
    assert!((0..FACTORIES).all(|i| active(&sim, factory(i)).is_none()));
    hashes
}

#[test]
fn seeded_multi_factory_controls_preserve_queue_and_resource_invariants_and_replay() {
    for seed in SEEDS {
        let sc = soak_scenario(seed);
        let log = schedule(seed);
        let hashes = run(&sc, &log);
        let replay = CommandLog::from_json(&log.to_json()).unwrap();
        assert_eq!(
            hashes,
            run(&sc, &replay),
            "per-tick JSON replay: seed {seed}"
        );
    }
}

fn lethal(sim: &mut Sim, id: UnitId) {
    let tick = sim.tick().0;
    let pos = sim.world().unit(id).unwrap().pos;
    tak_sim::combat::apply_damage(
        sim.world_mut(),
        &mut tak_sim::NoScripts,
        id,
        100_000,
        None,
        tak_sim::combat::kind::NORMAL,
        pos,
        tick,
    );
    assert!(sim.world().unit(id).unwrap().combat.is_dying());
}

fn death_run(seed: u64) -> Vec<u64> {
    let mut sc = soak_scenario(seed);
    sc.economy.rules.start_mana = 500;
    sc.initial_units.as_mut().unwrap().truncate(6); // No exit blockers here.
    sc.victory.defeat = tak_sim::DefeatRule::AllUnitsKilled;
    for def in [KEEP, SWORD, MASON] {
        sc.unit_defs[def.0 as usize].combat.max_damage = 100;
    }
    let mut sim = Sim::new(&sc).unwrap();
    let mut hashes = vec![sim.step(&[])];
    let mut rng = Rng::with_stream(seed, 0xdead);
    let dead_factory = factory(if rng.chance(1, 2) { 1 } else { 3 });
    let bereaved_factory = factory(if dead_factory.index == 1 { 2 } else { 0 });
    let mut commands = CommandLog::new();
    for player in [PlayerId(0), PlayerId(1)] {
        commands.record(
            sim.tick(),
            player,
            repeat(owned_factories(player), Some(SWORD)),
        );
    }
    hashes.push(sim.step(commands.for_tick(sim.tick())));
    for _ in 0..rng.range(3, 12) {
        hashes.push(sim.step(&[]));
    }
    let refunded = active(&sim, dead_factory).unwrap();
    let destroyed = active(&sim, bereaved_factory).unwrap();
    assert!(refunded.paid > 0 && destroyed.paid > 0);
    commands.record(
        sim.tick(),
        owner(dead_factory.index),
        repeat(UnitSet::single(dead_factory), None),
    );
    // The repeated origin must survive disabling its recipe and inserting
    // finite work, so death still cleans up the right paid interval.
    commands.record(
        sim.tick(),
        owner(dead_factory.index),
        build(UnitSet::single(dead_factory), MASON),
    );
    hashes.push(sim.step(commands.for_tick(sim.tick())));
    let destroyed = active(&sim, bereaved_factory).unwrap();
    let mut lost = [0i64; 2];
    lost[owner(bereaved_factory.index).0 as usize] = destroyed.paid;
    lethal(&mut sim, destroyed.id);
    lethal(&mut sim, dead_factory);
    lethal(&mut sim, dead_factory); // Repeated lethal notification cannot refund twice.
    assert!(sim.world().unit(refunded.id).is_none());
    for player in [PlayerId(0), PlayerId(1)] {
        commands.record(
            sim.tick(),
            player,
            SimCommand::Stop {
                units: owned_factories(player),
            },
        );
    }
    hashes.push(sim.step(commands.for_tick(sim.tick())));
    assert!(sim.world().unit(dead_factory).is_none());
    assert!(sim.world().unit(destroyed.id).is_none());
    assert_eq!(
        sim.world()
            .unit(bereaved_factory)
            .unwrap()
            .build
            .factory
            .product,
        None
    );
    assert_eq!(
        sim.world()
            .unit(bereaved_factory)
            .unwrap()
            .build
            .factory_repeat,
        None
    );
    check_accounting(
        &sim,
        &[500 << 16; 2],
        &lost,
        "after factory and product death",
    );
    // Restart all IDs, including the stale factory ID. Later products reuse
    // freed slots, but must not inherit that old factory's queued commands.
    for player in [PlayerId(0), PlayerId(1)] {
        commands.record(
            sim.tick(),
            player,
            repeat(owned_factories(player), Some(MASON)),
        );
    }
    for _ in 0..100 {
        hashes.push(sim.step(commands.for_tick(sim.tick())));
        let mut referenced = BTreeSet::new();
        for (_, unit) in sim.world().units().filter(|(_, unit)| unit.def == KEEP) {
            if let Some(product) = unit.build.factory.product {
                assert!(
                    referenced.insert(product),
                    "duplicate product after death: seed {seed}"
                );
                assert!(sim.world().unit(product).unwrap().build.frame.is_some());
            }
        }
        let frames: BTreeSet<_> = sim
            .world()
            .units()
            .filter(|(_, unit)| unit.build.frame.is_some())
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            frames, referenced,
            "orphan paid frame after death: seed {seed}"
        );
        check_accounting(&sim, &[500 << 16; 2], &lost, "after death and restart");
    }
    assert!(
        sim.world()
            .units()
            .any(|(id, unit)| id.generation > 0 && unit.def == MASON && unit.build.is_complete())
    );
    hashes
}

#[test]
fn seeded_factory_and_product_deaths_refund_only_surviving_paid_work_once() {
    for seed in SEEDS {
        assert_eq!(
            death_run(seed),
            death_run(seed),
            "death replay: seed {seed}"
        );
    }
}

#[test]
fn legacy_finite_stop_then_same_tick_build_retains_old_frame_unlike_explicit_cancel() {
    // Characterization, not the desired semantics of a new cancellation API:
    // finite Stop cleanup historically infers cancellation from the queue at
    // construction time. Re-populating it in the same tick retains the old
    // frame and consumes the new head when the old product finishes. Explicit
    // CancelProduction owns a cancellation marker and avoids this ambiguity.
    // Even an explicit modern rally route does not opt finite Stop out of the
    // legacy behavior. Preserve this evidence while replay policy is reviewed.
    for explicit_cancel in [false, true] {
        let mut sim = factory_controls_support::sim();
        let id = factory_controls_support::FACTORY;
        let units = UnitSet::single(id);
        factory_controls_support::issue(
            &mut sim,
            [
                build(units.clone(), SWORD),
                SimCommand::SetFactoryRally {
                    units: units.clone(),
                    order: Some(FactoryRallyOrder::Move { to: at(500, 500) }),
                    queue: QueueMode::Replace,
                },
            ],
        );
        for _ in 0..4 {
            sim.step(&[]);
        }
        let previous = active(&sim, id).unwrap();
        assert!(previous.paid > 0 && !previous.repeated);
        let command = if explicit_cancel {
            SimCommand::CancelProduction {
                units: units.clone(),
                def: SWORD,
                count: 1,
            }
        } else {
            SimCommand::Stop {
                units: units.clone(),
            }
        };
        factory_controls_support::issue(&mut sim, [command, build(units, MASON)]);
        assert_eq!(
            sim.world()
                .unit(id)
                .unwrap()
                .orders
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [Order::Build {
                def: MASON,
                at: None
            }]
        );
        if explicit_cancel {
            assert!(sim.world().unit(previous.id).is_none());
            assert!(active(&sim, id).is_none());
            assert_eq!(economy::mana(sim.world(), PlayerId(0)).0, 1000 << 16);
        } else {
            let retained = active(&sim, id).unwrap();
            assert_eq!(retained.id, previous.id);
            assert_eq!(retained.def, SWORD);
            assert!(retained.paid > previous.paid);
            assert_eq!(
                economy::mana(sim.world(), PlayerId(0)).0 + retained.paid,
                1000 << 16
            );
        }
        for _ in 0..100 {
            sim.step(&[]);
        }
        let products: Vec<_> = sim
            .world()
            .units()
            .filter(|(_, unit)| [SWORD, MASON].contains(&unit.def))
            .collect();
        assert_eq!(products.len(), 1);
        assert!(products[0].1.build.is_complete());
        assert_eq!(
            products[0].1.def,
            if explicit_cancel { MASON } else { SWORD }
        );
        assert!(sim.world().unit(id).unwrap().orders.is_empty());
        assert_eq!(
            economy::mana(sim.world(), PlayerId(0)).0,
            if explicit_cancel {
                960 << 16
            } else {
                970 << 16
            }
        );
        check_accounting(
            &sim,
            &[1000 << 16; 2],
            &[0, 0],
            "legacy Stop versus explicit cancel",
        );
    }
}

#[test]
#[ignore = "bounded 64-additional-seed production/replay/death soak; run explicitly"]
fn extended_seeded_factory_controls_soak() {
    // Run explicitly from the workspace root:
    // cargo test -p tak-sim --test factory_controls_stress \
    //   extended_seeded_factory_controls_soak -- --ignored --exact --nocapture
    // Fixed additional seeds 0x5354524553530000..=0x535452455353003f.
    // Each seed runs 1,620 production ticks twice (original + JSON replay),
    // plus the bounded death/restart scenario twice. No wall-clock stop,
    // environment-selected seeds, random entropy, or installed data is used.
    let mut production_ticks = 0usize;
    let mut death_ticks = 0usize;
    for offset in 0..64u64 {
        let seed = 0x5354_5245_5353_0000 + offset;
        let sc = soak_scenario(seed);
        let log = schedule(seed);
        let hashes = run(&sc, &log);
        let replay = CommandLog::from_json(&log.to_json()).unwrap();
        let replay_hashes = run(&sc, &replay);
        production_ticks += hashes.len() + replay_hashes.len();
        assert_eq!(hashes, replay_hashes, "extended JSON replay: seed {seed}");
        let deaths = death_run(seed);
        let death_replay = death_run(seed);
        death_ticks += deaths.len() + death_replay.len();
        assert_eq!(deaths, death_replay, "extended death replay: seed {seed}");
    }
    println!(
        "EXTENDED_SOAK seeds=64 production_schedules=64 production_runs=128 \
         production_ticks={production_ticks} death_schedules=64 death_runs=128 \
         death_ticks={death_ticks} total_runs=256 total_ticks={}",
        production_ticks + death_ticks
    );
}
