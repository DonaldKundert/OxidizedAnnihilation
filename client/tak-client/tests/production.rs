//! Opt-in owned-data coverage for explicit factory controls. Only ordinary
//! commands change the world; no retail bytes, balance overrides or injected
//! health/resources are used. Exact cancellation/refund semantics are Chosen.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use tak_client::cli::Args;
use tak_client::game::{Session, open_vfs};
use tak_client::saves;
use tak_client::skirmish::{self, Opponent, SetupConfig};
use tak_commands::{
    CommandLog, FactoryRallyOrder, PlayerId, QueueMode, SimCommand, UnitDefId, UnitId, UnitSet,
};
use tak_headless::bot::{
    ControllerConfig, Recipe, SIMPLE_SKIRMISH_V1, SkirmishController, Targeting,
};
use tak_headless::checkpoint::ReplaySession;
use tak_math::{Fx, Vec2};
use tak_sim::construction::{self, Frame};
use tak_sim::{Order, World, economy};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_TICKS: u32 = 18_000;
const OWNER: PlayerId = PlayerId(0);

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn construction_driver(world: &World) -> ControllerConfig {
    let root = world.defs().get(world.players()[0].monarch_def).unwrap();
    let recipe = root
        .build
        .canbuild
        .iter()
        .filter_map(|id| world.defs().get(*id))
        .filter(|def| {
            def.build.builder
                && !def.build.mobile
                && def.build.workertime > 0
                && def.maxvelocity == Fx::ZERO
                && def.build.buildcost > 0
        })
        .find_map(|factory| {
            factory
                .build
                .canbuild
                .iter()
                .filter_map(|id| world.defs().get(*id))
                .find(|troop| {
                    !troop.build.builder
                        && troop.build.mobile
                        && troop.maxvelocity > Fx::ZERO
                        && troop.build.buildcost > 0
                        && troop
                            .combat
                            .weapons
                            .iter()
                            .any(|weapon| !weapon.is_spell() && weapon.damage.default > 0)
                })
                .map(|troop| {
                    // The monarch's ordinary initial stock and income suffice;
                    // the planner still enforces the full-cost admission rule.
                    Recipe::resolve(world, &root.name, None, Some(&factory.name), &troop.name)
                        .unwrap()
                })
        })
        .expect("installed Aramon monarch has a paid factory/troop recipe");
    let config = ControllerConfig {
        human_players: vec![PlayerId(1)],
        controllers: vec![SkirmishController {
            policy: SIMPLE_SKIRMISH_V1.into(),
            player: OWNER,
            recipe,
            cadence_ticks: 30,
            command_budget: 8,
            site_budget: 64,
            army_cap: 1,
            mana_reserve: 0,
            targeting: Targeting::OmniscientTrainingV1,
        }],
    };
    config.validate(world).unwrap();
    config
}

fn submit(live: &mut Session, resumed: &mut Option<Session>, command: SimCommand) {
    if let Some(resumed) = resumed {
        resumed.submit(command.clone());
    }
    live.submit(command);
}

fn step(live: &mut Session, resumed: &mut Option<Session>) {
    assert!(
        live.tick().0 < MAX_TICKS,
        "ordinary production stalled at tick {}, mana {:?}",
        live.tick().0,
        economy::mana(live.sim.world(), OWNER)
    );
    let hash = live.try_step().unwrap();
    if let Some(resumed) = resumed {
        assert_eq!(resumed.try_step().unwrap(), hash);
    }
}

fn build(factory: UnitId, def: UnitDefId) -> SimCommand {
    SimCommand::Build {
        units: UnitSet::single(factory),
        def,
        at: None,
        queue: QueueMode::Append,
    }
}

fn cancel(factory: UnitId, def: UnitDefId, count: u32) -> SimCommand {
    SimCommand::CancelProduction {
        units: UnitSet::single(factory),
        def,
        count,
    }
}

fn rally(factory: UnitId, order: FactoryRallyOrder, queue: QueueMode) -> SimCommand {
    SimCommand::SetFactoryRally {
        units: UnitSet::single(factory),
        order: Some(order),
        queue,
    }
}

fn paid_product(live: &mut Session, resumed: &mut Option<Session>, factory: UnitId) -> UnitId {
    loop {
        if let Some(id) = live
            .sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product
            && let Some(frame) = live.sim.world().unit(id).unwrap().build.frame
            && frame.paid > 0
            && frame.done > 0
        {
            return id;
        }
        step(live, resumed);
    }
}

fn frame(live: &Session, id: UnitId) -> Frame {
    live.sim.world().unit(id).unwrap().build.frame.unwrap()
}

fn assert_preserved(live: &Session, factory: UnitId, product: UnitId, before: Frame) {
    assert_eq!(
        live.sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product,
        Some(product),
        "standing/queue edits must not replace the paid frame"
    );
    let after = frame(live, product);
    assert_eq!(after.total, before.total);
    assert!(after.done >= before.done && after.paid >= before.paid);
}

#[test]
fn installed_paid_factory_controls_inherit_orders_refund_and_resume_replay() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client_production") else {
        return;
    };
    let setup = SetupConfig {
        opponent: Opponent::PassivePractice,
        ..Default::default()
    };
    let mut live = skirmish::prepare_session(&data, &setup).unwrap();
    assert!(live.sim.cob().is_some());
    let driver = construction_driver(live.sim.world());
    let recipe = &driver.controllers[0].recipe;
    let factory_def = recipe.producer.as_ref().unwrap().id;
    let troop = recipe.troop.id;
    let mut resumed = None;
    let mut saw_paid_factory = false;
    let factory = loop {
        let found = live.sim.world().units().find_map(|(id, unit)| {
            (unit.owner == OWNER && unit.def == factory_def).then_some((id, unit.build.frame))
        });
        if let Some((id, construction)) = found {
            if let Some(frame) = construction {
                saw_paid_factory |= frame.paid > 0 && frame.done > 0;
            } else {
                break id;
            }
        }
        // Use the established legal-site planner only until the factory is
        // complete. All later queue edits are explicit client commands.
        for order in driver.plan(live.sim.world()).unwrap() {
            live.submit(order.command);
        }
        step(&mut live, &mut resumed);
    };
    assert!(
        saw_paid_factory,
        "factory must be built with ordinary paid work"
    );
    assert!(construction::can_factory_control(live.sim.world(), factory));
    assert_eq!(
        construction::production_count(live.sim.world(), factory, troop),
        0
    );

    let position = live.sim.world().unit(factory).unwrap().pos;
    let map = live.sim.world().map();
    let dx = if position.x.to_int_floor() < map.width / 2 {
        256
    } else {
        -256
    };
    let dy = if position.y.to_int_floor() < map.height / 2 {
        128
    } else {
        -128
    };
    let move_to = map.clamp(position + Vec2::from_ints(dx, 0));
    let patrol_to = map.clamp(position + Vec2::from_ints(dx, dy));
    assert_ne!(move_to, patrol_to);
    // Clear the builder's movement footprint from the completed factory's
    // door before testing sequential production. Leaving the monarch idle
    // here used to rely on closing the yard through its larger Motion extent.
    let monarch = live.sim.world().players()[0].monarch.unwrap();
    submit(
        &mut live,
        &mut resumed,
        SimCommand::Move {
            units: UnitSet::single(monarch),
            to: move_to,
            queue: QueueMode::Replace,
        },
    );
    submit(
        &mut live,
        &mut resumed,
        rally(
            factory,
            FactoryRallyOrder::Move { to: move_to },
            QueueMode::Replace,
        ),
    );
    submit(&mut live, &mut resumed, build(factory, troop));
    let product = paid_product(&mut live, &mut resumed, factory);
    let before = frame(&live, product);
    let production = live.sim.world().unit(factory).unwrap().orders.clone();

    // Replace a Move with Patrol while paid work is live, then author the
    // mixed Move -> Patrol route that the finished troop must inherit.
    submit(
        &mut live,
        &mut resumed,
        rally(
            factory,
            FactoryRallyOrder::Patrol { to: patrol_to },
            QueueMode::Replace,
        ),
    );
    step(&mut live, &mut resumed);
    assert_preserved(&live, factory, product, before);
    assert_eq!(live.sim.world().unit(factory).unwrap().orders, production);
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_rally,
        Some(vec![FactoryRallyOrder::Patrol { to: patrol_to }])
    );
    submit(
        &mut live,
        &mut resumed,
        rally(
            factory,
            FactoryRallyOrder::Move { to: move_to },
            QueueMode::Replace,
        ),
    );
    submit(
        &mut live,
        &mut resumed,
        rally(
            factory,
            FactoryRallyOrder::Patrol { to: patrol_to },
            QueueMode::Append,
        ),
    );
    for _ in 0..5 {
        submit(&mut live, &mut resumed, build(factory, troop));
    }
    step(&mut live, &mut resumed);
    assert_eq!(
        construction::production_count(live.sim.world(), factory, troop),
        6
    );
    submit(&mut live, &mut resumed, cancel(factory, troop, 1));
    step(&mut live, &mut resumed);
    assert_eq!(
        construction::production_count(live.sim.world(), factory, troop),
        5
    );
    submit(&mut live, &mut resumed, build(factory, troop));
    step(&mut live, &mut resumed);
    assert_eq!(
        construction::production_count(live.sim.world(), factory, troop),
        6
    );
    submit(&mut live, &mut resumed, cancel(factory, troop, 5));
    step(&mut live, &mut resumed);
    assert_eq!(
        construction::production_count(live.sim.world(), factory, troop),
        1
    );
    assert_preserved(&live, factory, product, before);
    let standing = Some(vec![
        FactoryRallyOrder::Move { to: move_to },
        FactoryRallyOrder::Patrol { to: patrol_to },
    ]);
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_rally,
        standing
    );

    let scratch = Scratch(std::env::temp_dir().join(format!(
        "tak-client-production-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let save = saves::quick_save(&live, &scratch.0).unwrap();
    let saved_bytes = std::fs::read(&save).unwrap();
    let saved_at = live.tick();
    let args = Args::try_parse_from([
        std::ffi::OsStr::new("tak-client"),
        std::ffi::OsStr::new("--data"),
        data.as_os_str(),
        std::ffi::OsStr::new("--load-checkpoint"),
        save.as_os_str(),
    ])
    .unwrap();
    let restored =
        Session::load_from_args(&open_vfs(&data).unwrap(), &args, CommandLog::new()).unwrap();
    assert_eq!(restored.sim.state_hash(), live.sim.state_hash());
    assert_eq!(frame(&restored, product), frame(&live, product));
    resumed = Some(restored);
    while !live.sim.world().unit(product).unwrap().build.is_complete() {
        step(&mut live, &mut resumed);
    }
    let produced = live.sim.world().unit(product).unwrap();
    assert_eq!(produced.owner, OWNER);
    assert_eq!(produced.def, troop);
    assert_eq!(
        produced.orders.iter().copied().collect::<Vec<_>>(),
        vec![
            Order::Move { to: move_to },
            Order::Patrol { to: patrol_to },
            Order::Patrol { to: move_to },
        ]
    );
    assert_eq!(
        construction::production_count(live.sim.world(), factory, troop),
        0
    );
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_rally,
        standing
    );

    // A new paid product is cancellable without clearing standing orders.
    // Tail-first removal and refund of exactly paid mana are Chosen engine
    // behavior, not a claim of observed original-executable timing.
    submit(&mut live, &mut resumed, build(factory, troop));
    let cancelled = paid_product(&mut live, &mut resumed, factory);
    let paid = frame(&live, cancelled).paid;
    let mana = economy::mana(live.sim.world(), OWNER).0;
    let income = economy::income_per_second(live.sim.world(), OWNER).0;
    let storage = economy::storage(live.sim.world(), OWNER).0;
    submit(&mut live, &mut resumed, cancel(factory, troop, 1));
    step(&mut live, &mut resumed);
    assert!(live.sim.world().unit(cancelled).is_none());
    assert_eq!(
        construction::production_count(live.sim.world(), factory, troop),
        0
    );
    assert_eq!(
        live.sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product,
        None
    );
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_rally,
        standing
    );
    assert_eq!(
        economy::player(live.sim.world(), OWNER)
            .unwrap()
            .last_granted,
        0
    );
    // Thirty idle ticks add exactly one second of income regardless of the
    // private fixed-point carry. This catches missing/double refunds without
    // changing resources or reaching into mutable economy state.
    for _ in 1..30 {
        step(&mut live, &mut resumed);
        assert_eq!(
            economy::income_per_second(live.sim.world(), OWNER).0,
            income
        );
        assert_eq!(economy::storage(live.sim.world(), OWNER).0, storage);
        assert_eq!(
            economy::player(live.sim.world(), OWNER)
                .unwrap()
                .last_granted,
            0
        );
    }
    assert!(
        mana + paid + income < storage,
        "refund must remain observable below the storage cap"
    );
    assert_eq!(
        economy::mana(live.sim.world(), OWNER).0,
        mana + paid + income
    );

    let resumed = resumed.unwrap();
    assert_eq!(resumed.log, live.log);
    assert_eq!(resumed.hashes_first_tick, saved_at);
    assert_eq!(resumed.hashes, live.hashes[saved_at.0 as usize..]);
    assert_eq!(std::fs::read(&save).unwrap(), saved_bytes);
    let recorded = CommandLog::from_json(&live.log.to_json()).unwrap();
    assert_eq!(recorded, live.log);
    let mut raw = ReplaySession::new(setup.checkpoint_source(), Some(&data)).unwrap();
    assert_eq!(raw.advance(&recorded, live.tick().0).unwrap(), live.hashes);
    eprintln!(
        "owned Aramon production: paid {} -> {}, +1/+5/-1/-5, live Move/Patrol edits, inheritance, paid refund, checkpoint and raw replay verified at {} ticks",
        recipe.producer.as_ref().unwrap().name,
        recipe.troop.name,
        live.tick().0
    );
}

fn repeat(factory: UnitId, def: Option<UnitDefId>) -> SimCommand {
    SimCommand::SetFactoryRepeat {
        units: UnitSet::single(factory),
        def,
    }
}

fn finite_queue(live: &Session, factory: UnitId) -> Vec<UnitDefId> {
    live.sim
        .world()
        .unit(factory)
        .unwrap()
        .orders
        .iter()
        .filter_map(|order| match order {
            Order::Build { def, at: None } => Some(*def),
            _ => None,
        })
        .collect()
}

fn finish_product(live: &mut Session, resumed: &mut Option<Session>, product: UnitId) {
    while !live
        .sim
        .world()
        .unit(product)
        .expect("ordinary production must not silently discard its active unit")
        .build
        .is_complete()
    {
        step(live, resumed);
    }
}

#[test]
fn installed_paid_repeat_production_yields_to_finite_orders_refunds_and_replays() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client_repeat_production") else {
        return;
    };
    let setup = SetupConfig {
        opponent: Opponent::PassivePractice,
        ..Default::default()
    };
    let mut live = skirmish::prepare_session(&data, &setup).unwrap();
    assert!(live.sim.cob().is_some());
    let driver = construction_driver(live.sim.world());
    let recipe = &driver.controllers[0].recipe;
    let factory_def = recipe.producer.as_ref().unwrap().id;
    let troop = recipe.troop.id;
    let mut resumed = None;
    let mut saw_paid_factory = false;
    let factory = loop {
        let found = live.sim.world().units().find_map(|(id, unit)| {
            (unit.owner == OWNER && unit.def == factory_def).then_some((id, unit.build.frame))
        });
        if let Some((id, construction)) = found {
            if let Some(frame) = construction {
                saw_paid_factory |= frame.paid > 0 && frame.done > 0;
            } else {
                break id;
            }
        }
        for order in driver.plan(live.sim.world()).unwrap() {
            live.submit(order.command);
        }
        step(&mut live, &mut resumed);
    };
    assert!(
        saw_paid_factory,
        "the factory must be constructed with ordinary paid work"
    );
    assert!(construction::can_factory_control(live.sim.world(), factory));
    assert!(finite_queue(&live, factory).is_empty());

    // Send released troops away from the pad through an ordinary standing
    // order, not by relocating units or bypassing collision/exit behavior.
    let position = live.sim.world().unit(factory).unwrap().pos;
    let map = live.sim.world().map();
    let dx = if position.x.to_int_floor() < map.width / 2 {
        256
    } else {
        -256
    };
    let dy = if position.y.to_int_floor() < map.height / 2 {
        128
    } else {
        -128
    };
    let rally_to = map.clamp(position + Vec2::from_ints(dx, dy));
    submit(
        &mut live,
        &mut resumed,
        rally(
            factory,
            FactoryRallyOrder::Move { to: rally_to },
            QueueMode::Replace,
        ),
    );
    submit(&mut live, &mut resumed, repeat(factory, Some(troop)));
    let first = paid_product(&mut live, &mut resumed, factory);
    assert!(
        live.sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    assert!(finite_queue(&live, factory).is_empty());
    let before = frame(&live, first);
    submit(&mut live, &mut resumed, cancel(factory, troop, 0));
    step(&mut live, &mut resumed);
    assert_preserved(&live, factory, first, before);
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_repeat,
        Some(troop)
    );
    finish_product(&mut live, &mut resumed, first);
    let second = paid_product(&mut live, &mut resumed, factory);
    assert_ne!(first, second);
    assert!(
        live.sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    finish_product(&mut live, &mut resumed, second);
    let third = paid_product(&mut live, &mut resumed, factory);
    assert_ne!(second, third);
    for product in [first, second] {
        let unit = live.sim.world().unit(product).unwrap();
        assert_eq!(unit.owner, OWNER);
        assert_eq!(unit.def, troop);
        assert!(unit.build.is_complete());
    }

    let before = frame(&live, third);
    submit(&mut live, &mut resumed, build(factory, troop));
    step(&mut live, &mut resumed);
    assert_preserved(&live, factory, third, before);
    assert!(
        live.sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    assert_eq!(finite_queue(&live, factory), vec![troop]);

    // Save an active paid repeat with a finite order waiting. Restoring must
    // retain the origin of the pad product, or finishing it loses that order.
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "tak-client-repeat-production-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let save = saves::quick_save(&live, &scratch.0).unwrap();
    let saved_bytes = std::fs::read(&save).unwrap();
    let saved_at = live.tick();
    let args = Args::try_parse_from([
        std::ffi::OsStr::new("tak-client"),
        std::ffi::OsStr::new("--data"),
        data.as_os_str(),
        std::ffi::OsStr::new("--load-checkpoint"),
        save.as_os_str(),
    ])
    .unwrap();
    let restored =
        Session::load_from_args(&open_vfs(&data).unwrap(), &args, CommandLog::new()).unwrap();
    assert_eq!(restored.sim.state_hash(), live.sim.state_hash());
    assert_eq!(restored.sim.world(), live.sim.world());
    assert_eq!(frame(&restored, third), frame(&live, third));
    resumed = Some(restored);

    let before = frame(&live, third);
    submit(&mut live, &mut resumed, repeat(factory, None));
    step(&mut live, &mut resumed);
    assert_preserved(&live, factory, third, before);
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_repeat,
        None
    );
    finish_product(&mut live, &mut resumed, third);
    assert_eq!(finite_queue(&live, factory), vec![troop]);
    let finite = paid_product(&mut live, &mut resumed, factory);
    assert_ne!(third, finite);
    assert!(
        !live
            .sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    let before = frame(&live, finite);
    submit(&mut live, &mut resumed, repeat(factory, Some(troop)));
    step(&mut live, &mut resumed);
    assert_preserved(&live, factory, finite, before);
    assert!(
        !live
            .sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    finish_product(&mut live, &mut resumed, finite);
    assert!(finite_queue(&live, factory).is_empty());

    let cancelled = paid_product(&mut live, &mut resumed, factory);
    assert!(
        live.sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    submit(&mut live, &mut resumed, build(factory, troop));
    step(&mut live, &mut resumed);
    let before = frame(&live, cancelled);
    submit(&mut live, &mut resumed, cancel(factory, troop, 1));
    step(&mut live, &mut resumed);
    assert_preserved(&live, factory, cancelled, before);
    assert!(finite_queue(&live, factory).is_empty());
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_repeat,
        None
    );

    // A second positive decrement reaches the paid repeat after pending
    // finite work is exhausted. Verify its exact refund through ordinary
    // idle income rather than mutating mana or the frame to make it pass.
    let paid = frame(&live, cancelled).paid;
    let mana = economy::mana(live.sim.world(), OWNER).0;
    let income = economy::income_per_second(live.sim.world(), OWNER).0;
    let storage = economy::storage(live.sim.world(), OWNER).0;
    submit(&mut live, &mut resumed, cancel(factory, troop, 1));
    step(&mut live, &mut resumed);
    assert!(live.sim.world().unit(cancelled).is_none());
    for idle_tick in 0..30 {
        if idle_tick > 0 {
            step(&mut live, &mut resumed);
        }
        let world = live.sim.world();
        let build = &world.unit(factory).unwrap().build;
        assert_eq!(build.factory_repeat, None);
        assert_eq!(build.factory.product, None);
        assert!(!build.factory.product_from_repeat);
        assert!(finite_queue(&live, factory).is_empty());
        assert_eq!(economy::income_per_second(world, OWNER).0, income);
        assert_eq!(economy::storage(world, OWNER).0, storage);
        assert_eq!(economy::player(world, OWNER).unwrap().last_granted, 0);
    }
    assert!(
        mana + paid + income < storage,
        "refund must be observable below the storage cap"
    );
    assert_eq!(
        economy::mana(live.sim.world(), OWNER).0,
        mana + paid + income
    );

    let resumed = resumed.unwrap();
    assert_eq!(resumed.log, live.log);
    assert_eq!(resumed.hashes_first_tick, saved_at);
    assert_eq!(resumed.hashes, live.hashes[saved_at.0 as usize..]);
    assert_eq!(std::fs::read(&save).unwrap(), saved_bytes);
    let recorded = CommandLog::from_json(&live.log.to_json()).unwrap();
    assert_eq!(recorded, live.log);
    let mut raw = ReplaySession::new(setup.checkpoint_source(), Some(&data)).unwrap();
    assert_eq!(raw.advance(&recorded, live.tick().0).unwrap(), live.hashes);
    eprintln!(
        "owned Aramon repeat production: ordinary paid {} -> {}, multiple repeats, finite interruption, disable, pending-first cancellation, exact paid refund, checkpoint and raw replay verified at {} ticks",
        recipe.producer.as_ref().unwrap().name,
        recipe.troop.name,
        live.tick().0
    );
}

#[test]
fn installed_paid_factory_guard_tracks_moving_monarch_preserves_work_and_replays() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client_factory_guard") else {
        return;
    };
    let setup = SetupConfig {
        opponent: Opponent::PassivePractice,
        ..Default::default()
    };
    let mut live = skirmish::prepare_session(&data, &setup).unwrap();
    assert!(live.sim.cob().is_some());
    let driver = construction_driver(live.sim.world());
    let recipe = &driver.controllers[0].recipe;
    let factory_def = recipe.producer.as_ref().unwrap().id;
    let troop = recipe.troop.id;
    let mut resumed = None;
    let mut saw_paid_factory = false;
    let factory = loop {
        if let Some((id, construction)) = live.sim.world().units().find_map(|(id, unit)| {
            (unit.owner == OWNER && unit.def == factory_def).then_some((id, unit.build.frame))
        }) {
            if let Some(frame) = construction {
                saw_paid_factory |= frame.paid > 0 && frame.done > 0;
            } else {
                break id;
            }
        }
        for order in driver.plan(live.sim.world()).unwrap() {
            live.submit(order.command);
        }
        step(&mut live, &mut resumed);
    };
    assert!(
        saw_paid_factory,
        "factory must use ordinary paid construction"
    );
    let monarch = live.sim.world().players()[0].monarch.unwrap();
    assert_eq!(live.sim.world().unit(monarch).unwrap().owner, OWNER);
    let origin = live.sim.world().unit(monarch).unwrap().pos;
    let map = live.sim.world().map();
    let dx = if origin.x.to_int_floor() < map.width / 2 {
        384
    } else {
        -384
    };
    let dy = if origin.y.to_int_floor() < map.height / 2 {
        256
    } else {
        -256
    };
    let patrol_to = map.clamp(origin + Vec2::from_ints(dx, dy));
    assert_ne!(origin, patrol_to);
    // Keep the target moving with an ordinary durable patrol, not a test
    // relocation, so inherited Guard retains identity rather than a waypoint.
    submit(
        &mut live,
        &mut resumed,
        SimCommand::Patrol {
            units: UnitSet::single(monarch),
            to: patrol_to,
            queue: QueueMode::Replace,
        },
    );
    submit(&mut live, &mut resumed, repeat(factory, Some(troop)));
    let repeated = paid_product(&mut live, &mut resumed, factory);
    assert!(
        live.sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    submit(&mut live, &mut resumed, build(factory, troop));
    step(&mut live, &mut resumed);
    assert_eq!(finite_queue(&live, factory), vec![troop]);
    let before = frame(&live, repeated);
    let production = live.sim.world().unit(factory).unwrap().orders.clone();
    let standing = Some(vec![FactoryRallyOrder::Guard { target: monarch }]);
    submit(
        &mut live,
        &mut resumed,
        rally(
            factory,
            FactoryRallyOrder::Guard { target: monarch },
            QueueMode::Replace,
        ),
    );
    step(&mut live, &mut resumed);
    assert_preserved(&live, factory, repeated, before);
    assert_eq!(live.sim.world().unit(factory).unwrap().orders, production);
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_repeat,
        Some(troop)
    );
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_rally,
        standing
    );

    let scratch = Scratch(std::env::temp_dir().join(format!(
        "tak-client-factory-guard-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let save = saves::quick_save(&live, &scratch.0).unwrap();
    let saved_bytes = std::fs::read(&save).unwrap();
    let saved_at = live.tick();
    let args = Args::try_parse_from([
        std::ffi::OsStr::new("tak-client"),
        std::ffi::OsStr::new("--data"),
        data.as_os_str(),
        std::ffi::OsStr::new("--load-checkpoint"),
        save.as_os_str(),
    ])
    .unwrap();
    let restored =
        Session::load_from_args(&open_vfs(&data).unwrap(), &args, CommandLog::new()).unwrap();
    assert_eq!(restored.sim.world(), live.sim.world());
    assert_eq!(restored.sim.state_hash(), live.sim.state_hash());
    resumed = Some(restored);

    finish_product(&mut live, &mut resumed, repeated);
    let released = live.sim.world().unit(repeated).unwrap();
    assert_eq!(released.owner, OWNER);
    assert_eq!(released.def, troop);
    let release_pos = released.pos;
    let inherited: Vec<_> = released.orders.iter().copied().collect();
    assert!(
        matches!(inherited.as_slice(), [Order::Move { .. }, Order::Guard { target }] if *target == monarch),
        "a leading Guard must inherit ordinary pad egress before its live target: {inherited:?}"
    );
    assert_ne!(
        live.sim.world().unit(monarch).unwrap().pos,
        origin,
        "the owned commander must actually move while production runs"
    );
    assert_eq!(
        finite_queue(&live, factory),
        vec![troop],
        "repeat completion lost finite work"
    );
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_repeat,
        Some(troop)
    );

    let finite = paid_product(&mut live, &mut resumed, factory);
    assert_ne!(finite, repeated);
    assert!(
        !live
            .sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    let before = frame(&live, finite);
    let production = live.sim.world().unit(factory).unwrap().orders.clone();
    submit(
        &mut live,
        &mut resumed,
        rally(
            factory,
            FactoryRallyOrder::Guard { target: monarch },
            QueueMode::Replace,
        ),
    );
    step(&mut live, &mut resumed);
    assert_preserved(&live, factory, finite, before);
    assert_eq!(live.sim.world().unit(factory).unwrap().orders, production);
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_repeat,
        Some(troop)
    );
    finish_product(&mut live, &mut resumed, finite);
    assert!(
        live.sim
            .world()
            .unit(finite)
            .unwrap()
            .orders
            .iter()
            .any(|order| *order == Order::Guard { target: monarch })
    );
    assert!(finite_queue(&live, factory).is_empty());
    let next_repeat = paid_product(&mut live, &mut resumed, factory);
    assert_ne!(next_repeat, finite);
    assert!(
        live.sim
            .world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    assert_eq!(
        live.sim.world().unit(factory).unwrap().build.factory_rally,
        standing
    );

    // Observe real Guard execution after leaving the yard while the target
    // continues moving. Guard remains a durable target order, not a snapshot
    // of the commander's position at production start or at release.
    let mut saw_active_guard = false;
    let mut saw_moving_target = false;
    let mut saw_guard_motion = false;
    let mut last_target_pos = live.sim.world().unit(monarch).unwrap().pos;
    let mut last_product_pos = live.sim.world().unit(repeated).unwrap().pos;
    for _ in 0..300 {
        step(&mut live, &mut resumed);
        let world = live.sim.world();
        let product = world.unit(repeated).unwrap();
        let target_pos = world.unit(monarch).unwrap().pos;
        if product.combat.order == Some(tak_sim::combat::CombatOrder::Guard(monarch)) {
            saw_active_guard = true;
            saw_moving_target |= target_pos != last_target_pos;
            saw_guard_motion |= product.pos != last_product_pos;
        }
        last_target_pos = target_pos;
        last_product_pos = product.pos;
    }
    assert!(
        saw_active_guard,
        "paid product never activated inherited Guard after egress"
    );
    assert!(
        saw_moving_target,
        "Guard must be observed while its own commander moves"
    );
    assert!(
        saw_guard_motion,
        "active Guard must move the paid escort, not only leave the yard"
    );
    assert_ne!(live.sim.world().unit(repeated).unwrap().pos, release_pos);
    let resumed = resumed.unwrap();
    assert_eq!(resumed.log, live.log);
    assert_eq!(resumed.hashes_first_tick, saved_at);
    assert_eq!(resumed.hashes, live.hashes[saved_at.0 as usize..]);
    assert_eq!(std::fs::read(&save).unwrap(), saved_bytes);
    let recorded = CommandLog::from_json(&live.log.to_json()).unwrap();
    assert_eq!(recorded, live.log);
    let mut raw = ReplaySession::new(setup.checkpoint_source(), Some(&data)).unwrap();
    assert_eq!(raw.advance(&recorded, live.tick().0).unwrap(), live.hashes);
    eprintln!(
        "owned Aramon factory Guard: paid {} -> {}, finite/repeat work preserved, ordinary egress, moving own monarch, checkpoint and raw replay verified at {} ticks",
        recipe.producer.as_ref().unwrap().name,
        recipe.troop.name,
        live.tick().0
    );
}
