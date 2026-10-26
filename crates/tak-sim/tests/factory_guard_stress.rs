//! Bounded, synthetic interaction stress for factory Guard recipes.
//!
//! Initial units, income sites and the target-loss weapon are hand-authored
//! fixtures, not claims about retail gameplay. After Scenario construction,
//! every change enters through a recorded command: no mutable world access,
//! injected damage, progress edits, or discarded products. The one destroyed
//! unit is a non-production target, so production conservation has zero losses.
mod factory_controls_support;

use std::collections::{BTreeMap, BTreeSet};

use factory_controls_support::{KEEP, KING, MASON, SWORD, at, scenario};
use tak_commands::{
    CommandLog, FactoryRallyOrder, FireState, PlayerId, QueueMode, SimCommand, Target, Tick,
    UnitDefId, UnitId, UnitSet,
};
use tak_math::{Angle, Fx, Rng};
use tak_sim::campaign::InitialUnit;
use tak_sim::combat::{CombatOrder, DamageStats, WeaponKind, WeaponStats};
use tak_sim::world::MAX_ORDERS;
use tak_sim::{Order, Scenario, Sim, construction, economy};

const SEEDS: [u64; 4] = [1, 0x5eed, 0xdecafbad, u64::MAX];
const STOP: u32 = 1300;
const TICKS: u32 = STOP + 20;
const LOST_TARGET: UnitId = UnitId::new(4, 0);
const SHOOTER: UnitId = UnitId::new(8, 0);

fn id(index: u32) -> UnitId {
    UnitId::new(index, 0)
}

fn initial(def: UnitDefId, owner: u8, x: i32, y: i32) -> InitialUnit {
    InitialUnit {
        def,
        owner: PlayerId(owner),
        pos: at(x, y),
        heading: Angle::ZERO,
        health_percentage: 100,
        mana_percentage: 100,
        kills: 0,
    }
}

fn setup(seed: u64) -> Scenario {
    let mut sc = scenario();
    sc.seed = seed;
    sc.victory.defeat = tak_sim::DefeatRule::AllUnitsKilled;
    sc.economy.rules.start_mana = 0;
    sc.economy.rules.site_slack = 0;
    sc.economy.rules.off_site_multiplier = Fx::ZERO;
    for def in &mut sc.unit_defs {
        def.combat.max_damage = 1000;
    }
    sc.unit_defs[KING.0 as usize].economy.mogriumincome = 60;
    sc.unit_defs[KING.0 as usize].economy.sacred_site = true;
    sc.economy.sacred_sites = [300, 700]
        .into_iter()
        .map(|y| economy::SacredSite {
            x: 48,
            y: y + 60,
            w: 32,
            h: 32,
            multiplier: Fx::ONE,
        })
        .collect();
    // Separate definitions keep initial target death outside production cost
    // accounting. Flying blockers remain on the pad until commanded away.
    let target = UnitDefId(sc.unit_defs.len() as u32);
    let mut d = sc.unit_defs[SWORD.0 as usize].clone();
    d.name = "SYNTHETIC_GUARD_TARGET".into();
    sc.unit_defs.push(d.clone());
    let blocker = UnitDefId(sc.unit_defs.len() as u32);
    d.name = "SYNTHETIC_PAD_BLOCKER".into();
    d.motion.canfly = true;
    sc.unit_defs.push(d.clone());
    let shooter = UnitDefId(sc.unit_defs.len() as u32);
    d.name = "SYNTHETIC_TARGET_LOSS_TRIGGER".into();
    d.maxvelocity = Fx::ZERO;
    d.combat.weapons = vec![WeaponStats {
        name: "SYNTHETIC_INSTANT_TARGET_LOSS".into(),
        kind: WeaponKind::RemoteEffect,
        range: 2000,
        reload_ticks: 30,
        aim_tolerance: Some(u16::MAX),
        damage: DamageStats {
            default: 10_000,
            multipliers: vec![],
        },
        ..WeaponStats::default()
    }];
    sc.unit_defs.push(d);
    sc.initial_units = Some(vec![
        initial(KEEP, 0, 200, 200),
        initial(KEEP, 1, 600, 700),
        initial(KING, 0, 64, 300),
        initial(KING, 1, 64, 700),
        initial(target, 0, 500, 380),
        initial(target, 1, 500, 780),
        initial(blocker, 0, 200, 224),
        initial(blocker, 1, 600, 724),
        initial(shooter, 1, 950, 950),
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

fn rally(units: UnitSet, order: Option<FactoryRallyOrder>, queue: QueueMode) -> SimCommand {
    SimCommand::SetFactoryRally {
        units,
        order,
        queue,
    }
}

fn schedule(seed: u64) -> CommandLog {
    let mut log = CommandLog::new();
    log.record(
        Tick(0),
        PlayerId(1),
        SimCommand::SetFireState {
            units: UnitSet::single(SHOOTER),
            state: FireState::HoldFire,
        },
    );
    for p in 0..2 {
        let player = PlayerId(p);
        let units = UnitSet::single(id(u32::from(p)));
        let guard = FactoryRallyOrder::Guard {
            target: id(4 + u32::from(p)),
        };
        for _ in 0..MAX_ORDERS + 3 {
            log.record(Tick(1), player, build(units.clone(), SWORD));
        }
        log.record(
            Tick(2),
            player,
            rally(units.clone(), Some(guard), QueueMode::Replace),
        );
        for n in 0..MAX_ORDERS + 3 {
            let order = if n % 3 == 0 {
                FactoryRallyOrder::Patrol { to: at(750, 480) }
            } else {
                guard
            };
            log.record(
                Tick(3),
                player,
                rally(units.clone(), Some(order), QueueMode::Append),
            );
        }
        log.record(
            Tick(50),
            player,
            SimCommand::Move {
                units: UnitSet::single(id(6 + u32::from(p))),
                to: at(900, 300 + 400 * i32::from(p)),
                queue: QueueMode::Replace,
            },
        );
        // Keep the capped recipe through the first completions; later edits
        // must not rewrite those released units' generational Guard snapshots.
        for (tick, on_site) in [
            (80, true),
            (480, false),
            (670, true),
            (900, false),
            (1060, true),
        ] {
            log.record(
                Tick(tick),
                player,
                SimCommand::Move {
                    units: UnitSet::single(id(2 + u32::from(p))),
                    to: at(64, 300 + 400 * i32::from(p) + if on_site { 76 } else { 0 }),
                    queue: QueueMode::Replace,
                },
            );
        }
        log.record(
            Tick(190),
            player,
            SimCommand::Stop {
                units: units.clone(),
            },
        );
        log.record(
            Tick(191),
            player,
            SimCommand::SetFactoryRepeat {
                units: units.clone(),
                def: Some(SWORD),
            },
        );
        log.record(
            Tick(191),
            player,
            rally(units.clone(), Some(guard), QueueMode::Replace),
        );
        for def in [MASON, SWORD, MASON] {
            log.record(Tick(220), player, build(units.clone(), def));
        }
        // Duplicate cancellations deliberately ask for the same refund twice.
        for _ in 0..2 {
            log.record(
                Tick(370),
                player,
                SimCommand::CancelProduction {
                    units: units.clone(),
                    def: SWORD,
                    count: u32::MAX,
                },
            );
        }
        log.record(
            Tick(371),
            player,
            SimCommand::SetFactoryRepeat {
                units: units.clone(),
                def: Some(MASON),
            },
        );
        for (tick, x) in [(150, 700), (210, 450), (330, 800)] {
            log.record(
                Tick(tick),
                player,
                SimCommand::Move {
                    units: UnitSet::single(id(4 + u32::from(p))),
                    to: at(x, 380 + 400 * i32::from(p)),
                    queue: QueueMode::Replace,
                },
            );
        }
        log.record(Tick(STOP), player, SimCommand::Stop { units });
    }
    log.record(
        Tick(280),
        PlayerId(1),
        SimCommand::Attack {
            units: UnitSet::single(SHOOTER),
            target: Target::Unit(LOST_TARGET),
            queue: QueueMode::Replace,
        },
    );
    log.record(
        Tick(282),
        PlayerId(1),
        SimCommand::Stop {
            units: UnitSet::single(SHOOTER),
        },
    );
    let mut rng = Rng::with_stream(seed, 0x6a4d);
    for tick in (400..STOP).step_by(4) {
        let player = PlayerId(rng.below(2) as u8);
        // Every command mixes the owned factory, a foreign factory and stale
        // generations. Per-recipient filtering is part of the oracle.
        let units: UnitSet = [id(0), id(1), UnitId::new(0, 1), UnitId::new(1, 1)]
            .into_iter()
            .collect();
        let def = if rng.chance(1, 2) { SWORD } else { MASON };
        let queue = if rng.chance(1, 2) {
            QueueMode::Replace
        } else {
            QueueMode::Append
        };
        let target = match rng.below(5) {
            0 => id(2 + u32::from(player.0)),
            1 => id(4 + u32::from(player.0)),
            2 => id(5 - u32::from(player.0)),
            3 => UnitId::new(5, 1),
            _ => UnitId::new(4, u32::MAX),
        };
        let to = at(rng.range_i32(350, 900), rng.range_i32(350, 600));
        let command = match rng.below(12) {
            0..=2 => build(units, def),
            3 => SimCommand::CancelProduction {
                units,
                def,
                count: 1 + rng.below(5),
            },
            4 => SimCommand::CancelProduction {
                units,
                def,
                count: 0,
            },
            5 => SimCommand::SetFactoryRepeat {
                units,
                def: Some(def),
            },
            6 => SimCommand::SetFactoryRepeat { units, def: None },
            7 => SimCommand::Stop { units },
            8..=9 => rally(units, Some(FactoryRallyOrder::Guard { target }), queue),
            10 => rally(units, Some(FactoryRallyOrder::Move { to }), queue),
            _ => rally(units, Some(FactoryRallyOrder::Patrol { to }), queue),
        };
        log.record(Tick(tick), player, command);
    }
    log
}

#[derive(Clone, Copy, Debug)]
struct Active {
    id: UnitId,
    def: UnitDefId,
    repeated: bool,
    paid: i64,
}

fn active(sim: &Sim, factory: UnitId) -> Option<Active> {
    let state = sim.world().unit(factory)?.build.factory;
    let id = state.product?;
    let unit = sim
        .world()
        .unit(id)
        .expect("factory references a live frame");
    Some(Active {
        id,
        def: unit.def,
        repeated: state.product_from_repeat,
        paid: unit
            .build
            .frame
            .expect("attached product is unfinished")
            .paid,
    })
}

fn valid_target(sim: &Sim, player: PlayerId, target: UnitId) -> bool {
    sim.world().unit(target).is_some_and(|unit| {
        !unit.combat.is_dying()
            && (unit.combat.max_health == 0 || unit.combat.health > 0)
            && !sim.world().are_hostile(player, unit.owner)
    })
}

fn route_cost(route: &[FactoryRallyOrder]) -> usize {
    route.len()
        + usize::from(
            route
                .iter()
                .any(|o| matches!(o, FactoryRallyOrder::Guard { .. })),
        )
        + usize::from(
            route
                .iter()
                .any(|o| matches!(o, FactoryRallyOrder::Patrol { .. })),
        )
}

#[derive(Default)]
struct Model {
    finite: Vec<UnitDefId>,
    repeat: Option<UnitDefId>,
    route: Option<Vec<FactoryRallyOrder>>,
}

impl Model {
    fn edit(
        &mut self,
        sim: &Sim,
        player: PlayerId,
        command: &SimCommand,
        active: Option<Active>,
        cancelled: &mut bool,
    ) {
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
                for i in (0..self.finite.len()).rev() {
                    if left > 0 && self.finite[i] == *def {
                        self.finite.remove(i);
                        left -= 1;
                        *cancelled |= i == 0 && active.is_some_and(|a| !a.repeated);
                    }
                }
                *cancelled |= left > 0 && active.is_some_and(|a| a.repeated && a.def == *def);
            }
            SimCommand::Stop { .. } => {
                self.finite.clear();
                self.repeat = None;
                if let Some(route) = &mut self.route {
                    route.clear();
                }
                *cancelled |= active.is_some();
            }
            SimCommand::SetFactoryRally { order, queue, .. } => {
                if let Some(FactoryRallyOrder::Guard { target }) = order {
                    if !valid_target(sim, player, *target) {
                        return;
                    }
                }
                let route = self.route.get_or_insert_with(Vec::new);
                if *queue == QueueMode::Replace || order.is_none() {
                    route.clear();
                }
                if let Some(order) = order {
                    let mut next = route.clone();
                    next.push(*order);
                    if route_cost(&next) <= MAX_ORDERS {
                        *route = next;
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Default, Debug)]
struct Coverage {
    finite: usize,
    repeated: usize,
    paid_cancel: usize,
    blocked: usize,
    starved: usize,
    resumed: usize,
    capped_queue: usize,
    capped_route: usize,
    guard_dispatch: usize,
    finite_guard_dispatch: usize,
    repeat_guard_dispatch: usize,
    filtered_dispatch: usize,
    active_guard: usize,
    moving_escort: usize,
    guarded_before_loss: bool,
    reused_target_slot: bool,
}

fn run(sc: &Scenario, log: &CommandLog) -> Vec<u64> {
    let mut sim = Sim::new(sc).unwrap();
    let mut models = [Model::default(), Model::default()];
    let mut supplied = [0i64; 2];
    let mut snapshots: BTreeMap<UnitId, Vec<UnitId>> = BTreeMap::new();
    let mut cancelled_ids = BTreeSet::new();
    let mut stalled = BTreeSet::new();
    let mut coverage = Coverage::default();
    let mut hashes = Vec::new();
    for tick in 0..TICKS {
        let context = format!("seed {}, tick {tick}", sc.seed);
        let before = [active(&sim, id(0)), active(&sim, id(1))];
        let escorts: BTreeMap<_, _> = sim
            .world()
            .units()
            .filter_map(|(uid, unit)| match unit.combat.order {
                Some(CombatOrder::Guard(target)) if [id(4), id(5)].contains(&target) => sim
                    .world()
                    .unit(target)
                    .map(|guarded| (uid, (target, unit.pos, guarded.pos))),
                _ => None,
            })
            .collect();
        let mut cancelled = [false; 2];
        let commands = log.for_tick(Tick(tick));
        for command in commands {
            for p in 0..2 {
                if command.player == PlayerId(p as u8)
                    && command.command.units().contains(id(p as u32))
                {
                    models[p].edit(
                        &sim,
                        command.player,
                        &command.command,
                        before[p],
                        &mut cancelled[p],
                    );
                }
            }
        }
        // Construction dispatch precedes combat: a target killed later in this
        // same tick was still legal when its Guard snapshot was released.
        let expected_guards: Vec<Vec<_>> = models
            .iter()
            .enumerate()
            .map(|(p, model)| {
                model
                    .route
                    .iter()
                    .flatten()
                    .filter_map(|order| match *order {
                        FactoryRallyOrder::Guard { target }
                            if valid_target(&sim, PlayerId(p as u8), target) =>
                        {
                            Some(target)
                        }
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        hashes.push(sim.step(commands));
        let mut referenced = BTreeSet::new();
        for p in 0..2 {
            let factory = id(p as u32);
            let model = &mut models[p];
            let unit = sim.world().unit(factory).unwrap();
            let state = unit.build.factory;
            if let Some(previous) = before[p] {
                if cancelled[p] {
                    assert!(cancelled_ids.insert(previous.id), "refund once: {context}");
                    assert!(
                        sim.world().unit(previous.id).is_none(),
                        "cancel removes frame: {context}"
                    );
                    assert!(
                        state.product.is_none(),
                        "no replacement in refund tick: {context}"
                    );
                    coverage.paid_cancel += usize::from(previous.paid > 0);
                } else if sim.world().unit(previous.id).unwrap().build.is_complete() {
                    if previous.repeated {
                        coverage.repeated += 1;
                    } else {
                        assert_eq!(model.finite.remove(0), previous.def, "FIFO: {context}");
                        coverage.finite += 1;
                    }
                    let product = sim.world().unit(previous.id).unwrap();
                    let actual: Vec<_> = product
                        .orders
                        .iter()
                        .filter_map(|order| match *order {
                            Order::Guard { target } => Some(target),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        actual, expected_guards[p],
                        "exact legal dispatch: {context}"
                    );
                    assert_eq!(
                        product.combat.order, None,
                        "egress precedes Guard activation: {context}"
                    );
                    coverage.guard_dispatch += usize::from(!actual.is_empty());
                    coverage.finite_guard_dispatch +=
                        usize::from(!actual.is_empty() && !previous.repeated);
                    coverage.repeat_guard_dispatch +=
                        usize::from(!actual.is_empty() && previous.repeated);
                    let requested = model
                        .route
                        .iter()
                        .flatten()
                        .filter(|o| matches!(o, FactoryRallyOrder::Guard { .. }))
                        .count();
                    coverage.filtered_dispatch += usize::from(requested > actual.len());
                    assert!(snapshots.insert(previous.id, actual).is_none());
                }
            }
            let actual: Vec<_> = unit
                .orders
                .iter()
                .map(|order| match *order {
                    Order::Build { def, at: None } => def,
                    other => panic!("unexpected factory order {other:?}: {context}"),
                })
                .collect();
            assert_eq!(actual, model.finite, "finite oracle: {context}");
            assert_eq!(
                unit.build.factory_repeat, model.repeat,
                "repeat oracle: {context}"
            );
            assert_eq!(
                unit.build.factory_rally, model.route,
                "route oracle: {context}"
            );
            assert!(!state.cancel_product, "cancellation resolved: {context}");
            assert!(
                !state.product_from_repeat || state.product.is_some(),
                "no orphan repeat flag: {context}"
            );
            assert!(actual.len() <= MAX_ORDERS);
            coverage.capped_queue += usize::from(actual.len() == MAX_ORDERS);
            if let Some(route) = &model.route {
                assert!(
                    route_cost(route) <= MAX_ORDERS,
                    "reserved route cap: {context}"
                );
                coverage.capped_route += usize::from(route_cost(route) == MAX_ORDERS);
            }
            coverage.blocked += usize::from(state.retry_at.is_some());
            let current = active(&sim, factory);
            if let Some(product) = current {
                assert!(
                    referenced.insert(product.id),
                    "one owner per paid frame: {context}"
                );
                let frame = sim.world().unit(product.id).unwrap();
                assert_eq!(frame.owner, PlayerId(p as u8));
                assert!(
                    frame.orders.is_empty() && frame.combat.order.is_none(),
                    "no dispatch before completion: {context}"
                );
                if !product.repeated {
                    assert_eq!(actual.first(), Some(&product.def));
                }
                if before[p].is_none() {
                    assert_eq!(
                        Some(product.def),
                        model.finite.first().copied().or(model.repeat)
                    );
                    assert_eq!(product.repeated, model.finite.is_empty());
                }
                if let Some(previous) = before[p].filter(|a| a.id == product.id) {
                    if previous.paid == product.paid
                        && economy::income_per_second(sim.world(), PlayerId(p as u8)).0 == 0
                    {
                        coverage.starved += 1;
                        stalled.insert(product.id);
                    } else if product.paid > previous.paid && stalled.remove(&product.id) {
                        coverage.resumed += 1;
                    }
                }
            }
            for def in [SWORD, MASON] {
                let expected = actual.iter().filter(|&&d| d == def).count()
                    + usize::from(current.is_some_and(|a| a.repeated && a.def == def));
                assert_eq!(
                    construction::production_count(sim.world(), factory, def) as usize,
                    expected,
                    "active-inclusive count: {context}"
                );
            }
        }
        let mut committed = [0i64; 2];
        let mut frames = BTreeSet::new();
        for (uid, unit) in sim.world().units() {
            assert!(
                !cancelled_ids.contains(&uid),
                "cancelled generation cannot return: {context}"
            );
            assert!(
                unit.orders.len() <= MAX_ORDERS,
                "released order cap: {context}"
            );
            if !unit.build.is_complete() {
                frames.insert(uid);
            }
            coverage.reused_target_slot |=
                uid.index == LOST_TARGET.index && uid.generation != LOST_TARGET.generation;
            if ![SWORD, MASON].contains(&unit.def) {
                continue;
            }
            assert!(
                !unit.combat.is_dying(),
                "no production destruction is expected: {context}"
            );
            let cost = sim.world().defs().get(unit.def).unwrap().build.buildcost << 16;
            let paid = unit.build.frame.map_or(cost, |frame| {
                assert!((0..=cost).contains(&frame.paid), "paid range: {context}");
                assert!(
                    (0..=frame.total).contains(&frame.done),
                    "progress range: {context}"
                );
                frame.paid
            });
            committed[unit.owner.0 as usize] += paid;
            let allowed = snapshots.get(&uid);
            for target in unit
                .orders
                .iter()
                .filter_map(|o| match *o {
                    Order::Guard { target } => Some(target),
                    _ => None,
                })
                .chain(match unit.combat.order {
                    Some(CombatOrder::Guard(target)) => Some(target),
                    _ => None,
                })
            {
                assert!(
                    allowed.is_some_and(|targets| targets.contains(&target)),
                    "Guard retains exact released generation: {context}"
                );
            }
            if let Some(CombatOrder::Guard(target)) = unit.combat.order {
                coverage.active_guard += 1;
                coverage.guarded_before_loss |= target == LOST_TARGET && tick < 280;
                if let Some(&(old_target, old_pos, old_target_pos)) = escorts.get(&uid) {
                    coverage.moving_escort += usize::from(
                        old_target == target
                            && old_pos != unit.pos
                            && sim.world().unit(target).is_some_and(|guarded| {
                                guarded.pos != old_target_pos
                                    && unit.pos.distance(guarded.pos)
                                        < old_pos.distance(guarded.pos)
                            }),
                    );
                }
                if tick > 281 {
                    assert_ne!(target, LOST_TARGET, "lost active Guard clears: {context}");
                }
            }
        }
        assert_eq!(frames, referenced, "no orphan paid frames: {context}");
        assert!(
            sim.world().construction.refunds.is_empty(),
            "refunds credited this tick: {context}"
        );
        for p in 0..2 {
            let player = PlayerId(p as u8);
            let income = economy::income_per_second(sim.world(), player).0;
            assert_eq!(income % i64::from(tak_sim::TICKS_PER_SECOND), 0);
            supplied[p] += income / i64::from(tak_sim::TICKS_PER_SECOND);
            let mana = economy::mana(sim.world(), player).0;
            assert!(mana >= 0);
            // No product is destroyed or discarded; refunds cancel committed
            // paid cost exactly once, with neither a loss nor a creation term.
            assert_eq!(
                mana + committed[p],
                supplied[p],
                "production conservation, player {p}: {context}"
            );
        }
        if tick == 49 {
            assert_eq!(referenced.len(), 0, "blocked pads: {context}");
        }
        if tick > 281 {
            assert!(
                sim.world().unit(LOST_TARGET).is_none(),
                "command-driven target loss: {context}"
            );
        }
    }
    assert_eq!(
        sim.world().combat.deaths,
        1,
        "only synthetic Guard target dies"
    );
    assert!(
        coverage.finite >= 2 && coverage.repeated >= 2,
        "production origins: {coverage:?}"
    );
    assert!(
        coverage.paid_cancel >= 2,
        "paid cancellations: {coverage:?}"
    );
    assert!(
        coverage.blocked >= 50 && coverage.starved >= 50 && coverage.resumed >= 2,
        "blocked/starved/resumed: {coverage:?}"
    );
    assert!(
        coverage.capped_queue > 0 && coverage.capped_route > 0,
        "caps exercised: {coverage:?}"
    );
    assert!(
        coverage.guard_dispatch >= 2 && coverage.filtered_dispatch > 0 && coverage.active_guard > 0,
        "Guard lifecycle: {coverage:?}"
    );
    assert!(
        coverage.finite_guard_dispatch > 0 && coverage.repeat_guard_dispatch > 0,
        "both production origins dispatch Guard: {coverage:?}"
    );
    assert!(
        coverage.guarded_before_loss,
        "active Guard target is lost: {coverage:?}"
    );
    assert!(
        coverage.reused_target_slot,
        "target index reused with a new generation: {coverage:?}"
    );
    assert!(
        coverage.moving_escort > 0,
        "escort pursues a moving target: {coverage:?}"
    );
    assert!(
        models
            .iter()
            .all(|m| m.finite.is_empty() && m.repeat.is_none())
    );
    assert!((0..2).all(|p| active(&sim, id(p)).is_none()));
    hashes
}

#[test]
fn four_seed_guard_production_interactions_preserve_invariants_and_replay() {
    for seed in SEEDS {
        let sc = setup(seed);
        let log = schedule(seed);
        let hashes = run(&sc, &log);
        assert_eq!(hashes, run(&sc, &log), "raw replay, seed {seed}");
        let json_sc = Scenario::from_json(&sc.to_json_pretty()).unwrap();
        let json_log = CommandLog::from_json(&log.to_json()).unwrap();
        assert_eq!(
            hashes,
            run(&json_sc, &json_log),
            "scenario and command JSON replay, seed {seed}"
        );
    }
}
