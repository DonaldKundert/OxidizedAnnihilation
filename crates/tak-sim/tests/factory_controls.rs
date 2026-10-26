//! Separate factory standing orders and production edits, entirely synthetic.
mod factory_controls_support;
use factory_controls_support::*;
use tak_commands::{FactoryRallyOrder, PlayerId, QueueMode, SimCommand, UnitId, UnitSet};
use tak_math::Angle;
use tak_sim::world::MAX_ORDERS;
use tak_sim::{Order, Sim, construction, systems};

fn product(sim: &Sim) -> UnitId {
    sim.world()
        .unit(FACTORY)
        .unwrap()
        .build
        .factory
        .product
        .unwrap()
}
fn recipe(sim: &Sim) -> Vec<FactoryRallyOrder> {
    sim.world()
        .unit(FACTORY)
        .unwrap()
        .build
        .factory_rally
        .clone()
        .unwrap()
}
fn queue(sim: &Sim) -> Vec<Order> {
    sim.world()
        .unit(FACTORY)
        .unwrap()
        .orders
        .iter()
        .copied()
        .collect()
}
fn finish(sim: &mut Sim, id: UnitId) {
    for _ in 0..100 {
        if sim.world().unit(id).unwrap().build.is_complete() {
            return;
        }
        sim.step(&[]);
    }
    panic!("product did not finish");
}

#[test]
fn editing_rally_preserves_paid_product_and_inherits_move_move_patrol_at_completion() {
    use FactoryRallyOrder::{Move, Patrol};
    let mut s = sim();
    issue(&mut s, [build(SWORD), build(SWORD)]);
    let p = product(&s);
    let paid = s.world().unit(p).unwrap().build.frame.unwrap().paid;
    let production = queue(&s);
    issue(
        &mut s,
        [rally(Some(Move { to: at(350, 300) }), QueueMode::Replace)],
    );
    issue(
        &mut s,
        [rally(Some(Move { to: at(450, 400) }), QueueMode::Replace)],
    );
    issue(
        &mut s,
        [rally(Some(Move { to: at(650, 400) }), QueueMode::Append)],
    );
    issue(
        &mut s,
        [rally(Some(Patrol { to: at(650, 650) }), QueueMode::Append)],
    );
    assert_eq!(product(&s), p);
    assert_eq!(queue(&s), production);
    assert!(s.world().unit(p).unwrap().build.frame.unwrap().paid > paid);
    assert!(s.world_mut().construction.take_refunds().is_empty());
    finish(&mut s, p);
    assert_eq!(
        s.world()
            .unit(p)
            .unwrap()
            .orders
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![
            Order::Move { to: at(450, 400) },
            Order::Move { to: at(650, 400) },
            Order::Patrol { to: at(650, 650) },
            Order::Patrol { to: at(650, 400) }
        ]
    );
    assert_eq!(
        recipe(&s),
        vec![
            Move { to: at(450, 400) },
            Move { to: at(650, 400) },
            Patrol { to: at(650, 650) }
        ]
    );
    let inherited = s.world().unit(p).unwrap().orders.clone();
    issue(
        &mut s,
        [rally(Some(Move { to: at(700, 700) }), QueueMode::Replace)],
    );
    assert_eq!(
        s.world().unit(p).unwrap().orders,
        inherited,
        "finished units own a snapshot"
    );
}

#[test]
fn clearing_rally_does_not_cancel_product_and_overrides_legacy_moves() {
    let mut s = sim();
    issue(
        &mut s,
        [
            build(SWORD),
            SimCommand::Move {
                units: UnitSet::single(FACTORY),
                to: at(900, 900),
                queue: QueueMode::Append,
            },
        ],
    );
    let p = product(&s);
    issue(
        &mut s,
        [rally(
            Some(FactoryRallyOrder::Move { to: at(400, 400) }),
            QueueMode::Replace,
        )],
    );
    issue(&mut s, [rally(None, QueueMode::Append)]);
    assert_eq!(product(&s), p);
    assert!(recipe(&s).is_empty());
    finish(&mut s, p);
    let u = s.world().unit(p).unwrap();
    assert_eq!(u.orders.len(), 1);
    assert!(!u.orders.contains(&Order::Move { to: at(900, 900) }));
    assert!(!u.orders.contains(&Order::Move { to: at(400, 400) }));
}

#[test]
fn pending_removal_is_tail_first_and_preserves_paid_work_and_standing_orders() {
    let mut s = sim();
    issue(
        &mut s,
        [
            build(SWORD),
            build(MASON),
            build(SWORD),
            build(MASON),
            build(SWORD),
            rally(
                Some(FactoryRallyOrder::Move { to: at(500, 500) }),
                QueueMode::Replace,
            ),
        ],
    );
    let p = product(&s);
    let standing = recipe(&s);
    issue(&mut s, [cancel(SWORD, 1)]);
    assert_eq!(product(&s), p);
    assert_eq!(
        queue(&s),
        vec![
            Order::Build {
                def: SWORD,
                at: None
            },
            Order::Build {
                def: MASON,
                at: None
            },
            Order::Build {
                def: SWORD,
                at: None
            },
            Order::Build {
                def: MASON,
                at: None
            }
        ]
    );
    issue(&mut s, [cancel(MASON, 5)]);
    assert_eq!(product(&s), p);
    assert_eq!(
        queue(&s),
        vec![
            Order::Build {
                def: SWORD,
                at: None
            };
            2
        ]
    );
    assert_eq!(recipe(&s), standing);
    assert!(s.world_mut().construction.take_refunds().is_empty());
}

#[test]
fn active_cancel_refunds_exact_paid_amount_and_keeps_different_pending_production() {
    let mut s = sim();
    issue(&mut s, [build(SWORD), build(MASON)]);
    for _ in 0..5 {
        s.step(&[]);
    }
    let p = product(&s);
    let paid = s.world().unit(p).unwrap().build.frame.unwrap().paid;
    let mana = tak_sim::economy::mana(s.world(), PlayerId(0)).0;
    issue(&mut s, [cancel(SWORD, 1), build(SWORD)]);
    assert!(s.world().unit(p).is_none());
    assert_eq!(
        tak_sim::economy::mana(s.world(), PlayerId(0)).0,
        mana + paid
    );
    assert_eq!(
        queue(&s),
        vec![
            Order::Build {
                def: MASON,
                at: None
            },
            Order::Build {
                def: SWORD,
                at: None
            }
        ]
    );
    assert!(
        !s.world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory
            .cancel_product
    );
    s.step(&[]);
    assert_eq!(s.world().unit(product(&s)).unwrap().def, MASON);
}

#[test]
fn repeated_edits_in_one_tick_never_assign_cancelled_product_to_next_entry() {
    let mut s = sim();
    issue(&mut s, [build(SWORD), build(MASON), build(MASON)]);
    let p = product(&s);
    issue(&mut s, [cancel(SWORD, 1), cancel(MASON, 1), build(SWORD)]);
    assert!(s.world().unit(p).is_none());
    assert_eq!(
        queue(&s),
        vec![
            Order::Build {
                def: MASON,
                at: None
            },
            Order::Build {
                def: SWORD,
                at: None
            }
        ]
    );
    s.step(&[]);
    assert_eq!(s.world().unit(product(&s)).unwrap().def, MASON);
}

#[test]
fn capability_ownership_generation_and_zero_count_are_validated_without_mutation() {
    let mut s = sim();
    let mobile = s
        .world_mut()
        .spawn(KING, PlayerId(0), at(100, 100), Angle::ZERO)
        .unwrap();
    let frame = s
        .world_mut()
        .spawn(KEEP, PlayerId(0), at(400, 100), Angle::ZERO)
        .unwrap();
    s.world_mut().unit_mut(frame).unwrap().build.frame = Some(Default::default());
    let stale = UnitId::new(FACTORY.index, FACTORY.generation + 1);
    issue(&mut s, [build(SWORD)]);
    let before = s.world().clone();
    for command in [
        SimCommand::SetFactoryRally {
            units: vec![ENEMY_FACTORY, mobile, frame, stale].into(),
            order: Some(FactoryRallyOrder::Move { to: at(500, 500) }),
            queue: QueueMode::Replace,
        },
        SimCommand::CancelProduction {
            units: vec![ENEMY_FACTORY, mobile, frame, stale].into(),
            def: SWORD,
            count: u32::MAX,
        },
        cancel(SWORD, 0),
        cancel(KEEP, u32::MAX),
    ] {
        construction::apply_factory_command(s.world_mut(), PlayerId(0), &command);
    }
    assert_eq!(s.world(), &before);
    assert!(construction::can_factory_control(s.world(), FACTORY));
    assert!(!construction::can_factory_control(s.world(), mobile));
    assert!(!construction::can_factory_control(s.world(), frame));
}

#[test]
fn rally_bound_reserves_patrol_return_and_clamps_coordinates() {
    let mut s = sim();
    let point = at(500, 500);
    for _ in 0..MAX_ORDERS - 2 {
        construction::apply_factory_command(
            s.world_mut(),
            PlayerId(0),
            &rally(
                Some(FactoryRallyOrder::Move { to: point }),
                QueueMode::Append,
            ),
        );
    }
    let raw = at(-100, 5000);
    let clamped = s.world().map().clamp(raw);
    construction::apply_factory_command(
        s.world_mut(),
        PlayerId(0),
        &rally(
            Some(FactoryRallyOrder::Patrol { to: raw }),
            QueueMode::Append,
        ),
    );
    let before = s.world().clone();
    construction::apply_factory_command(
        s.world_mut(),
        PlayerId(0),
        &rally(
            Some(FactoryRallyOrder::Move { to: point }),
            QueueMode::Append,
        ),
    );
    assert_eq!(s.world(), &before);
    assert_eq!(recipe(&s).len(), MAX_ORDERS - 1);
    assert_eq!(
        recipe(&s).last(),
        Some(&FactoryRallyOrder::Patrol { to: clamped })
    );
    issue(&mut s, [build(SWORD)]);
    let p = product(&s);
    finish(&mut s, p);
    let orders = &s.world().unit(p).unwrap().orders;
    assert_eq!(orders.len(), MAX_ORDERS);
    assert_eq!(orders.back(), Some(&Order::Patrol { to: point }));
}

#[test]
fn different_standing_recipes_and_explicit_clear_are_hashed() {
    let mut a = sim();
    let mut b = sim();
    assert_eq!(a.state_hash(), b.state_hash());
    construction::apply_factory_command(
        a.world_mut(),
        PlayerId(0),
        &rally(None, QueueMode::Replace),
    );
    assert_ne!(
        a.state_hash(),
        b.state_hash(),
        "explicit clear differs from legacy fallback"
    );
    for (s, order) in [
        (&mut a, FactoryRallyOrder::Move { to: at(500, 500) }),
        (&mut b, FactoryRallyOrder::Patrol { to: at(500, 500) }),
    ] {
        construction::apply_factory_command(
            s.world_mut(),
            PlayerId(0),
            &rally(Some(order), QueueMode::Replace),
        );
    }
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn legacy_move_replace_still_cancels_production_for_existing_logs() {
    let mut s = sim();
    issue(&mut s, [build(SWORD)]);
    let p = product(&s);
    issue(
        &mut s,
        [SimCommand::Move {
            units: UnitSet::single(FACTORY),
            to: at(500, 500),
            queue: QueueMode::Replace,
        }],
    );
    assert!(s.world().unit(p).is_none());
    assert_eq!(s.world().unit(FACTORY).unwrap().build.factory_rally, None);
}

#[test]
fn command_dispatch_rejects_wrong_tick_and_unknown_player() {
    let mut s = sim();
    let command = rally(
        Some(FactoryRallyOrder::Move { to: at(500, 500) }),
        QueueMode::Replace,
    );
    let before = s.world().clone();
    let mut c = tak_commands::LoggedCommand {
        tick: tak_commands::Tick(100),
        player: PlayerId(0),
        seq: 0,
        command,
    };
    systems::apply_commands(s.world_mut(), &[c.clone()]);
    c.tick = s.tick();
    c.player = PlayerId(42);
    systems::apply_commands(s.world_mut(), &[c]);
    assert_eq!(s.world(), &before);
}

#[test]
fn queue_counts_include_active_work_and_zero_for_ineligible_factories() {
    let mut s = sim();
    issue(&mut s, [build(SWORD), build(SWORD), build(MASON)]);
    assert_eq!(construction::production_count(s.world(), FACTORY, SWORD), 2);
    assert_eq!(construction::production_count(s.world(), FACTORY, MASON), 1);
    issue(&mut s, [cancel(SWORD, 1)]);
    assert_eq!(construction::production_count(s.world(), FACTORY, SWORD), 1);
    let p = product(&s);
    finish(&mut s, p);
    assert_eq!(construction::production_count(s.world(), FACTORY, SWORD), 0);
    assert_eq!(construction::production_count(s.world(), FACTORY, MASON), 1);
    s.world_mut().unit_mut(FACTORY).unwrap().combat.dying = Some(tak_sim::combat::Dying {
        since: 0,
        severity: 1,
        kind: 0,
        done: false,
    });
    assert_eq!(construction::production_count(s.world(), FACTORY, MASON), 0);
    assert!(!construction::can_factory_control(s.world(), FACTORY));
    let before = s.world().clone();
    for c in [
        cancel(MASON, 1),
        rally(
            Some(FactoryRallyOrder::Move { to: at(400, 400) }),
            QueueMode::Replace,
        ),
    ] {
        construction::apply_factory_command(s.world_mut(), PlayerId(0), &c);
    }
    assert_eq!(s.world(), &before);
    assert_eq!(
        construction::production_count(s.world(), UnitId::new(99, 99), MASON),
        0
    );
}

#[test]
fn active_cancel_stops_factory_callbacks_once_then_restarts_remaining_product() {
    use std::{cell::RefCell, rc::Rc};
    type Calls = Rc<RefCell<Vec<(UnitId, String)>>>;
    struct Host(Calls);
    impl tak_sim::ScriptHost for Host {
        fn start_script(&mut self, id: UnitId, name: &str, _args: &[i32]) -> bool {
            self.0.borrow_mut().push((id, name.to_owned()));
            true
        }
    }
    let calls: Calls = Default::default();
    let mut s = Sim::with_scripts(&scenario(), Box::new(Host(calls.clone()))).unwrap();
    s.step(&[]);
    issue(&mut s, [build(SWORD), build(MASON)]);
    let events = || {
        calls
            .borrow()
            .iter()
            .filter(|(id, _)| *id == FACTORY)
            .map(|(_, name)| name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(events(), ["Activate", "StartBuilding"]);
    issue(
        &mut s,
        [rally(
            Some(FactoryRallyOrder::Move { to: at(500, 500) }),
            QueueMode::Replace,
        )],
    );
    assert_eq!(
        events(),
        ["Activate", "StartBuilding"],
        "rally edit has no construction callbacks"
    );
    issue(&mut s, [cancel(SWORD, 1)]);
    assert_eq!(
        events(),
        ["Activate", "StartBuilding", "StopBuilding", "Deactivate"]
    );
    s.step(&[]);
    assert_eq!(
        events(),
        [
            "Activate",
            "StartBuilding",
            "StopBuilding",
            "Deactivate",
            "Activate",
            "StartBuilding"
        ]
    );
}

#[test]
fn patrol_without_moves_returns_to_product_release_position_not_factory_center() {
    let mut s = sim();
    issue(
        &mut s,
        [
            build(SWORD),
            rally(
                Some(FactoryRallyOrder::Patrol { to: at(600, 600) }),
                QueueMode::Replace,
            ),
        ],
    );
    let p = product(&s);
    let mut origin = s.world().unit(p).unwrap().pos;
    for _ in 0..100 {
        if s.world().unit(p).unwrap().build.is_complete() {
            break;
        }
        origin = s.world().unit(p).unwrap().pos;
        s.step(&[]);
    }
    assert!(s.world().unit(p).unwrap().build.is_complete());
    assert_ne!(origin, s.world().unit(FACTORY).unwrap().pos);
    assert_eq!(
        s.world()
            .unit(p)
            .unwrap()
            .orders
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![
            Order::Patrol { to: at(600, 600) },
            Order::Patrol { to: origin }
        ]
    );
}

#[test]
fn stop_clears_explicit_rally_and_production_while_legacy_state_stays_absent() {
    let mut s = sim();
    issue(
        &mut s,
        [
            build(SWORD),
            rally(
                Some(FactoryRallyOrder::Move { to: at(500, 500) }),
                QueueMode::Replace,
            ),
        ],
    );
    let p = product(&s);
    issue(
        &mut s,
        [SimCommand::Stop {
            units: UnitSet::single(FACTORY),
        }],
    );
    assert!(s.world().unit(p).is_none());
    assert!(queue(&s).is_empty());
    assert_eq!(
        s.world().unit(FACTORY).unwrap().build.factory_rally,
        Some(vec![])
    );
    issue(
        &mut s,
        [SimCommand::Stop {
            units: UnitSet::single(ENEMY_FACTORY),
        }],
    );
    assert_eq!(
        s.world().unit(ENEMY_FACTORY).unwrap().build.factory_rally,
        None
    );
    let mut legacy = sim();
    issue(
        &mut legacy,
        [SimCommand::Stop {
            units: UnitSet::single(FACTORY),
        }],
    );
    assert_eq!(
        legacy.world().unit(FACTORY).unwrap().build.factory_rally,
        None
    );
}

#[test]
fn explicit_cancel_precedes_lower_slot_guard_or_heal_that_would_finish_the_frame() {
    for heal in [false, true] {
        cancellation_before_helper(heal);
    }
}

fn cancellation_before_helper(heal: bool) {
    use tak_sim::campaign::InitialUnit;
    let mut sc = scenario();
    let factory = UnitId::new(1, 0);
    let helper = UnitId::new(0, 0);
    sc.initial_units = Some(vec![
        InitialUnit {
            def: KING,
            owner: PlayerId(0),
            pos: at(140, 200),
            heading: Angle::ZERO,
            health_percentage: 100,
            mana_percentage: 100,
            kills: 0,
        },
        InitialUnit {
            def: KEEP,
            owner: PlayerId(0),
            pos: at(200, 200),
            heading: Angle::ZERO,
            health_percentage: 100,
            mana_percentage: 100,
            kills: 0,
        },
        InitialUnit {
            def: KEEP,
            owner: PlayerId(1),
            pos: at(800, 800),
            heading: Angle::ZERO,
            health_percentage: 100,
            mana_percentage: 100,
            kills: 0,
        },
    ]);
    // Keep refund assertions independent of ordinary income. Heal requires
    // a health-bearing product, unlike the legacy movement-only fixture.
    sc.unit_defs[KING.0 as usize].economy.mogriumincome = 0;
    sc.unit_defs[SWORD.0 as usize].combat.max_damage = 100;
    let mut s = Sim::new(&sc).unwrap();
    s.step(&[]);
    issue(
        &mut s,
        [
            SimCommand::Build {
                units: UnitSet::single(factory),
                def: SWORD,
                at: None,
                queue: QueueMode::Append,
            },
            SimCommand::Build {
                units: UnitSet::single(factory),
                def: MASON,
                at: None,
                queue: QueueMode::Append,
            },
        ],
    );
    let p = s
        .world()
        .unit(factory)
        .unwrap()
        .build
        .factory
        .product
        .unwrap();
    s.step(&[]);
    let help = if heal {
        SimCommand::Heal {
            units: UnitSet::single(helper),
            target: p,
            queue: QueueMode::Replace,
        }
    } else {
        SimCommand::Guard {
            units: UnitSet::single(helper),
            target: factory,
            queue: QueueMode::Replace,
        }
    };
    issue(&mut s, [help]);
    let build = &s.world().unit(helper).unwrap().build;
    let target = if heal {
        build.heal.unwrap().work.target
    } else {
        build.assist.unwrap().target
    };
    assert_eq!(target, p);
    // Two factory-only work steps, then two workers per tick, reach 26 of
    // 27 steps. On the next tick the lower-slot helper alone would finish.
    for _ in 0..30 {
        let frame = s.world().unit(p).unwrap().build.frame.unwrap();
        if frame.total - frame.done <= construction::progress::worker_step(10) {
            break;
        }
        s.step(&[]);
    }
    let frame = s.world().unit(p).unwrap().build.frame.unwrap();
    assert_eq!(
        frame.total - frame.done,
        construction::progress::worker_step(10)
    );
    let paid = frame.paid;
    let mana = tak_sim::economy::mana(s.world(), PlayerId(0)).0;
    issue(
        &mut s,
        [SimCommand::CancelProduction {
            units: UnitSet::single(factory),
            def: SWORD,
            count: 1,
        }],
    );
    assert!(
        s.world().unit(p).is_none(),
        "a helper must not finish the cancelled product"
    );
    assert_eq!(
        tak_sim::economy::mana(s.world(), PlayerId(0)).0,
        mana + paid
    );
    assert_eq!(construction::production_count(s.world(), factory, MASON), 1);
    assert!(
        s.world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product
            .is_none()
    );
    s.step(&[]);
    let next = s
        .world()
        .unit(factory)
        .unwrap()
        .build
        .factory
        .product
        .unwrap();
    assert_eq!(s.world().unit(next).unwrap().def, MASON);
}

#[test]
fn move_patrol_only_literal_hashes_stay_compatible() {
    let mut s = sim();
    let initial = s.state_hash();
    construction::apply_factory_command(
        s.world_mut(),
        PlayerId(0),
        &rally(
            Some(FactoryRallyOrder::Move { to: at(500, 500) }),
            QueueMode::Replace,
        ),
    );
    let moved = s.state_hash();
    construction::apply_factory_command(
        s.world_mut(),
        PlayerId(0),
        &rally(
            Some(FactoryRallyOrder::Patrol { to: at(600, 500) }),
            QueueMode::Append,
        ),
    );
    let patrolled = s.state_hash();
    let mut replay = Sim::new(&scenario()).unwrap();
    let mut digest = tak_sim::StateHasher::new();
    for hash in replay.run(&log(), 200) {
        digest.u64(hash);
    }
    // Captured on the pre-Guard implementation (68aebc3).
    assert_eq!(initial, 0x092f3d90a03b3553);
    assert_eq!(moved, 0x152c58a539d21039);
    assert_eq!(patrolled, 0x2de61e887bea4b5d);
    assert_eq!(digest.finish(), 0xa6a4be02cdc8c25a);
}
