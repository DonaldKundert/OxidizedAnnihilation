//! Synthetic training-controller integration. Factory and optional blocker
//! placement are explicit fixture setup; repeat/queue changes and all work
//! afterward use ordinary commands without resource/frame/health mutation.
use tak_commands::{LoggedCommand, PlayerId, QueueMode, SimCommand, UnitDefId, UnitId, UnitSet};
use tak_headless::bot::{ControllerConfig, committed_count, demo};
use tak_math::{Angle, Vec2};
use tak_sim::Sim;

const OWNER: PlayerId = PlayerId(0);
const TROOP: UnitDefId = UnitDefId(3);

fn fixture(block_exit: bool) -> (Sim, ControllerConfig, UnitId) {
    let mut sim = Sim::new(&demo::scenario(false)).unwrap();
    // Preplaced synthetic units isolate the planner's idle/count contract.
    // Their construction is covered separately by the ordinary training loop.
    let factory = sim
        .world_mut()
        .spawn(UnitDefId(2), OWNER, Vec2::from_ints(256, 160), Angle::ZERO)
        .unwrap();
    if block_exit {
        sim.world_mut()
            .spawn(
                UnitDefId(4),
                PlayerId(1),
                Vec2::from_ints(256, 160),
                Angle::ZERO,
            )
            .unwrap();
    }
    sim.step(&[]);
    let mut config = demo::config(sim.world(), false).unwrap();
    config.controllers[0].recipe.income = None;
    config.controllers[0].cadence_ticks = 1;
    config.validate(sim.world()).unwrap();
    (sim, config, factory)
}

fn issue(sim: &mut Sim, commands: impl IntoIterator<Item = SimCommand>) {
    let commands: Vec<_> = commands
        .into_iter()
        .enumerate()
        .map(|(seq, command)| LoggedCommand {
            tick: sim.tick(),
            player: OWNER,
            seq: seq as u32,
            command,
        })
        .collect();
    sim.step(&commands);
}

fn repeat(factory: UnitId, def: Option<UnitDefId>) -> SimCommand {
    SimCommand::SetFactoryRepeat {
        units: UnitSet::single(factory),
        def,
    }
}

#[test]
fn armed_repeat_with_blocked_empty_pad_is_not_idle_for_training_controller() {
    let (mut sim, config, factory) = fixture(true);
    assert!(config.plan(sim.world()).unwrap().iter().any(|entry| {
        matches!(&entry.command, SimCommand::Build { units, def, .. }
            if *def == TROOP && units.iter().any(|&id| id == factory))
    }));
    issue(&mut sim, [repeat(factory, Some(TROOP))]);
    for _ in 0..120 {
        let unit = sim.world().unit(factory).unwrap();
        assert_eq!(unit.build.factory_repeat, Some(TROOP));
        assert!(unit.build.factory.product.is_none());
        assert!(unit.build.factory.retry_at.is_some());
        assert!(unit.orders.is_empty());
        let before = sim.state_hash();
        let planned = config.plan(sim.world()).unwrap();
        assert_eq!(sim.state_hash(), before, "planning must remain read-only");
        assert!(
            planned
                .iter()
                .all(|entry| !matches!(entry.command, SimCommand::Build { .. })),
            "an armed repeat must not gain a competing finite build while its pad is blocked"
        );
        sim.step(&planned);
    }
    assert_eq!(committed_count(sim.world(), OWNER, TROOP), 0);
    issue(&mut sim, [repeat(factory, None)]);
    assert!(
        config.plan(sim.world()).unwrap().iter().any(|entry| {
            matches!(&entry.command, SimCommand::Build { units, def, .. }
            if *def == TROOP && units.iter().any(|&id| id == factory))
        }),
        "disabling repeat restores the ordinary idle-factory policy"
    );
}

#[test]
fn repeated_product_does_not_back_an_independent_finite_commitment() {
    let (mut sim, _, factory) = fixture(false);
    issue(&mut sim, [repeat(factory, Some(TROOP))]);
    let product = loop {
        if let Some(product) = sim.world().unit(factory).unwrap().build.factory.product {
            let frame = sim.world().unit(product).unwrap().build.frame.unwrap();
            if frame.paid > 0 && frame.done > 0 {
                break product;
            }
        }
        assert!(
            sim.tick().0 < 30,
            "synthetic repeat did not receive paid work"
        );
        sim.step(&[]);
    };
    assert!(
        sim.world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    assert_eq!(committed_count(sim.world(), OWNER, TROOP), 1);
    issue(
        &mut sim,
        [0, 1].map(|_| SimCommand::Build {
            units: UnitSet::single(factory),
            def: TROOP,
            at: None,
            queue: QueueMode::Append,
        }),
    );
    assert_eq!(sim.world().unit(factory).unwrap().orders.len(), 2);
    assert_eq!(
        committed_count(sim.world(), OWNER, TROOP),
        3,
        "one paid repeated product plus two independent finite orders are three commitments"
    );
    issue(&mut sim, [repeat(factory, None)]);
    while !sim.world().unit(product).unwrap().build.is_complete() {
        assert!(sim.tick().0 < 100, "synthetic paid repeat did not finish");
        sim.step(&[]);
    }
    assert_eq!(sim.world().unit(factory).unwrap().orders.len(), 2);
    assert_eq!(committed_count(sim.world(), OWNER, TROOP), 3);
    let finite = loop {
        assert!(
            sim.tick().0 < 100,
            "finite work did not follow the repeated product"
        );
        sim.step(&[]);
        if let Some(product) = sim.world().unit(factory).unwrap().build.factory.product {
            break product;
        }
    };
    assert_ne!(product, finite);
    assert!(
        !sim.world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    assert_eq!(sim.world().unit(factory).unwrap().orders.len(), 2);
    assert_eq!(
        committed_count(sim.world(), OWNER, TROOP),
        3,
        "ordinary finite products still back exactly one finite entry without double-counting"
    );
}
