//! Factory Guard recipes use the ordinary Guard queue after safe yard egress.
//! Entirely synthetic; no original executable behavior is asserted.
mod factory_controls_support;
use factory_controls_support::*;
use std::{cell::RefCell, rc::Rc};
use tak_commands::{FactoryRallyOrder, PlayerId, QueueMode, SimCommand, UnitId, UnitSet};
use tak_math::{Angle, Fx};
use tak_sim::combat::{CombatOrder, Dying, GUARD_DISTANCE};
use tak_sim::world::MAX_ORDERS;
use tak_sim::{Order, Scenario, Sim, construction, economy};

fn guard_scenario() -> Scenario {
    let mut sc = scenario();
    for d in &mut sc.unit_defs {
        d.combat.max_damage = 1000;
    }
    sc.victory.defeat = tak_sim::victory::DefeatRule::AllUnitsKilled;
    sc
}
fn started(sc: &Scenario) -> Sim {
    let mut s = Sim::new(sc).unwrap();
    s.step(&[]);
    s
}
fn guard_sim() -> Sim {
    started(&guard_scenario())
}
fn target(s: &mut Sim, x: i32, y: i32) -> UnitId {
    s.world_mut()
        .spawn(SWORD, PlayerId(0), at(x, y), Angle::ZERO)
        .unwrap()
}
fn product(s: &Sim) -> UnitId {
    s.world()
        .unit(FACTORY)
        .unwrap()
        .build
        .factory
        .product
        .unwrap()
}
fn guard(g: UnitId, queue: QueueMode) -> SimCommand {
    rally(Some(FactoryRallyOrder::Guard { target: g }), queue)
}
fn apply(s: &mut Sim, command: SimCommand) {
    construction::apply_factory_command(s.world_mut(), PlayerId(0), &command);
}
fn finish(s: &mut Sim, p: UnitId) {
    for _ in 0..100 {
        if s.world().unit(p).unwrap().build.is_complete() {
            return;
        }
        s.step(&[]);
    }
    panic!("product did not finish");
}
fn activate(s: &mut Sim, p: UnitId, g: UnitId) {
    for _ in 0..600 {
        if s.world().unit(p).unwrap().combat.order == Some(CombatOrder::Guard(g)) {
            return;
        }
        s.step(&[]);
    }
    panic!("Guard did not activate: {:?}", s.world().unit(p));
}
fn orders(s: &Sim, p: UnitId) -> Vec<Order> {
    s.world().unit(p).unwrap().orders.iter().copied().collect()
}
fn dying() -> Dying {
    Dying {
        since: 0,
        severity: 1,
        kind: 0,
        done: false,
    }
}

#[test]
fn guard_edits_preserve_paid_finite_and_repeat_work_positions_fifo_and_callbacks() {
    type Calls = Rc<RefCell<Vec<(UnitId, String)>>>;
    struct Host(Calls);
    impl tak_sim::ScriptHost for Host {
        fn start_script(&mut self, id: UnitId, name: &str, _: &[i32]) -> bool {
            self.0.borrow_mut().push((id, name.into()));
            true
        }
    }
    for repeated in [false, true] {
        let calls: Calls = Default::default();
        let mut s = Sim::with_scripts(&guard_scenario(), Box::new(Host(calls.clone()))).unwrap();
        s.step(&[]);
        let g = target(&mut s, 600, 400);
        if repeated {
            issue(
                &mut s,
                [SimCommand::SetFactoryRepeat {
                    units: UnitSet::single(FACTORY),
                    def: Some(SWORD),
                }],
            );
        } else {
            issue(&mut s, [build(SWORD)]);
        }
        issue(&mut s, [build(MASON), build(SWORD)]);
        let p = product(&s);
        let before_factory = s.world().unit(FACTORY).unwrap().clone();
        let before_product = s.world().unit(p).unwrap().clone();
        assert!(before_product.build.frame.unwrap().paid > 0);
        let mana = economy::mana(s.world(), PlayerId(0));
        let before_calls = calls.borrow().clone();
        for command in [
            guard(g, QueueMode::Replace),
            rally(
                Some(FactoryRallyOrder::Move { to: at(500, 350) }),
                QueueMode::Append,
            ),
            guard(FACTORY, QueueMode::Replace),
            guard(g, QueueMode::Replace),
        ] {
            apply(&mut s, command);
        }
        let mut expected = before_factory;
        expected.build.factory_rally = Some(vec![FactoryRallyOrder::Guard { target: g }]);
        assert_eq!(s.world().unit(FACTORY).unwrap(), &expected);
        assert_eq!(s.world().unit(p).unwrap(), &before_product);
        assert_eq!(economy::mana(s.world(), PlayerId(0)), mana);
        assert_eq!(*calls.borrow(), before_calls);
        assert!(s.world_mut().construction.take_refunds().is_empty());
        finish(&mut s, p);
        assert_eq!(
            orders(&s, p),
            [Order::Move { to: at(200, 256) }, Order::Guard { target: g }]
        );
        let queue = &s.world().unit(FACTORY).unwrap().orders;
        assert_eq!(
            queue
                .iter()
                .filter(|o| matches!(o, Order::Build { .. }))
                .count(),
            2
        );
        assert_eq!(
            construction::factory_repeat(s.world(), FACTORY),
            repeated.then_some(SWORD)
        );
    }
}

#[test]
fn move_guard_recipe_escorts_and_finished_units_keep_their_own_snapshot() {
    let mut s = guard_sim();
    let g = target(&mut s, 600, 350);
    issue(&mut s, [build(SWORD)]);
    let p = product(&s);
    apply(
        &mut s,
        rally(
            Some(FactoryRallyOrder::Move { to: at(400, 350) }),
            QueueMode::Replace,
        ),
    );
    apply(&mut s, guard(g, QueueMode::Append));
    finish(&mut s, p);
    assert_eq!(
        orders(&s, p),
        [Order::Move { to: at(400, 350) }, Order::Guard { target: g }]
    );
    let snapshot = s.world().unit(p).unwrap().clone();
    apply(&mut s, guard(FACTORY, QueueMode::Replace));
    assert_eq!(s.world().unit(p).unwrap(), &snapshot);
    activate(&mut s, p, g);
    issue(
        &mut s,
        [SimCommand::Move {
            units: UnitSet::single(g),
            to: at(850, 350),
            queue: QueueMode::Replace,
        }],
    );
    for _ in 0..700 {
        s.step(&[]);
    }
    let pu = s.world().unit(p).unwrap();
    assert!(pu.pos.x > Fx::from_int(700), "escort follows moving target");
    assert!(pu.pos.distance(s.world().unit(g).unwrap().pos) <= GUARD_DISTANCE + Fx::from_int(8));
    assert_eq!(pu.combat.order, Some(CombatOrder::Guard(g)));
}

#[test]
fn nearby_self_factory_and_far_targets_all_leave_the_pad_before_guard() {
    for kind in 0..3 {
        let mut s = guard_sim();
        let g = match kind {
            0 => FACTORY,
            1 => target(&mut s, 240, 224),
            _ => target(&mut s, 600, 400),
        };
        issue(&mut s, [build(SWORD), guard(g, QueueMode::Replace)]);
        let p = product(&s);
        finish(&mut s, p);
        assert_eq!(
            orders(&s, p),
            [Order::Move { to: at(200, 256) }, Order::Guard { target: g }]
        );
        assert_eq!(s.world().unit(p).unwrap().combat.order, None);
        activate(&mut s, p, g);
        assert!(construction::placement::exit_clear(
            s.world(),
            SWORD,
            at(200, 224),
            FACTORY
        ));
    }
}

#[test]
fn invalid_guard_replace_and_append_are_exact_noops_including_legacy_fallback() {
    let mut s = guard_sim();
    let live = target(&mut s, 500, 300);
    let dead = target(&mut s, 600, 300);
    s.world_mut().unit_mut(dead).unwrap().combat.dying = Some(dying());
    let stale = target(&mut s, 700, 300);
    s.world_mut().despawn(stale).unwrap();
    let reused = target(&mut s, 700, 300);
    assert_eq!(stale.index, reused.index);
    assert_ne!(stale.generation, reused.generation);
    for explicit in [false, true] {
        if explicit {
            apply(&mut s, guard(live, QueueMode::Replace));
        }
        for queue in [QueueMode::Replace, QueueMode::Append] {
            for g in [dead, stale, UnitId::new(999, 0), ENEMY_FACTORY] {
                assert!(!construction::can_factory_guard_target(
                    s.world(),
                    FACTORY,
                    g
                ));
                let before = s.world().clone();
                apply(&mut s, guard(g, queue));
                assert_eq!(s.world(), &before);
            }
        }
    }
}

#[test]
fn guard_validates_factory_eligibility_and_commanding_owner() {
    let mut s = guard_sim();
    let mobile = s
        .world_mut()
        .spawn(KING, PlayerId(0), at(100, 100), Angle::ZERO)
        .unwrap();
    let frame = s
        .world_mut()
        .spawn(KEEP, PlayerId(0), at(400, 100), Angle::ZERO)
        .unwrap();
    s.world_mut().unit_mut(frame).unwrap().build.frame = Some(Default::default());
    let dead = s
        .world_mut()
        .spawn(KEEP, PlayerId(0), at(600, 100), Angle::ZERO)
        .unwrap();
    s.world_mut().unit_mut(dead).unwrap().combat.dying = Some(dying());
    let stale = UnitId::new(FACTORY.index, FACTORY.generation + 1);
    let before = s.world().clone();
    construction::apply_factory_command(
        s.world_mut(),
        PlayerId(0),
        &SimCommand::SetFactoryRally {
            units: vec![ENEMY_FACTORY, mobile, frame, dead, stale].into(),
            order: Some(FactoryRallyOrder::Guard { target: FACTORY }),
            queue: QueueMode::Replace,
        },
    );
    assert_eq!(s.world(), &before);
    for factory in [mobile, frame, dead, stale] {
        assert!(!construction::can_factory_guard_target(
            s.world(),
            factory,
            FACTORY
        ));
    }
    assert!(construction::can_factory_guard_target(
        s.world(),
        FACTORY,
        FACTORY
    ));
    assert!(construction::can_factory_guard_target(
        s.world(),
        FACTORY,
        mobile
    ));
}

#[test]
fn allied_guard_is_allowed_without_giving_control_of_the_allied_factory() {
    let mut sc = guard_scenario();
    sc.victory.teams = vec![Some(1), Some(1)];
    let mut s = started(&sc);
    assert!(construction::can_factory_guard_target(
        s.world(),
        FACTORY,
        ENEMY_FACTORY
    ));
    let before = s.world().unit(ENEMY_FACTORY).unwrap().clone();
    apply(
        &mut s,
        SimCommand::SetFactoryRally {
            units: vec![FACTORY, ENEMY_FACTORY].into(),
            order: Some(FactoryRallyOrder::Guard {
                target: ENEMY_FACTORY,
            }),
            queue: QueueMode::Replace,
        },
    );
    assert_eq!(s.world().unit(ENEMY_FACTORY).unwrap(), &before);
    assert_eq!(
        s.world().unit(FACTORY).unwrap().build.factory_rally,
        Some(vec![FactoryRallyOrder::Guard {
            target: ENEMY_FACTORY
        }])
    );
}

#[test]
fn completion_revalidates_death_generation_capture_and_product_ownership() {
    for change in 0..6 {
        let mut s = guard_sim();
        let g = target(&mut s, 600, 400);
        issue(&mut s, [build(SWORD), guard(g, QueueMode::Replace)]);
        let p = product(&s);
        match change {
            0 => {
                s.world_mut().despawn(g).unwrap();
            }
            1 => {
                s.world_mut().despawn(g).unwrap();
                let replacement = target(&mut s, 600, 400);
                assert_eq!(g.index, replacement.index);
                assert_ne!(g, replacement);
            }
            2 => s.world_mut().unit_mut(g).unwrap().combat.dying = Some(dying()),
            3 => s.world_mut().unit_mut(g).unwrap().owner = PlayerId(1),
            4 => s.world_mut().unit_mut(p).unwrap().owner = PlayerId(1),
            _ => s.world_mut().unit_mut(g).unwrap().combat.health = 0,
        }
        finish(&mut s, p);
        assert_eq!(orders(&s, p), [Order::Move { to: at(200, 256) }]);
        assert_eq!(s.world().unit(p).unwrap().combat.order, None);
        assert_eq!(
            s.world().unit(FACTORY).unwrap().build.factory_rally,
            Some(vec![FactoryRallyOrder::Guard { target: g }]),
            "recipe remains a snapshot source"
        );
    }
    // Capturing the factory itself invalidates Guard(factory) for an already
    // owned product; factory ownership cannot substitute for product ownership.
    let mut s = guard_sim();
    issue(&mut s, [build(SWORD), guard(FACTORY, QueueMode::Replace)]);
    let p = product(&s);
    s.world_mut().unit_mut(FACTORY).unwrap().owner = PlayerId(1);
    finish(&mut s, p);
    assert_eq!(orders(&s, p), [Order::Move { to: at(200, 256) }]);
}

#[test]
fn vanished_guards_are_skipped_without_erasing_remaining_moves_or_patrols() {
    let mut s = guard_sim();
    let g = target(&mut s, 600, 400);
    issue(&mut s, [build(SWORD), guard(g, QueueMode::Replace)]);
    let p = product(&s);
    apply(
        &mut s,
        rally(
            Some(FactoryRallyOrder::Move { to: at(400, 350) }),
            QueueMode::Append,
        ),
    );
    apply(&mut s, guard(g, QueueMode::Append));
    apply(
        &mut s,
        rally(
            Some(FactoryRallyOrder::Patrol { to: at(500, 350) }),
            QueueMode::Append,
        ),
    );
    s.world_mut().despawn(g).unwrap();
    finish(&mut s, p);
    assert_eq!(
        orders(&s, p),
        [
            Order::Move { to: at(400, 350) },
            Order::Patrol { to: at(500, 350) },
            Order::Patrol { to: at(400, 350) },
        ]
    );
}

#[test]
fn target_loss_after_release_keeps_exit_and_after_activation_uses_ordinary_guard_rules() {
    for before_activation in [true, false] {
        let mut s = guard_sim();
        let g = target(&mut s, 600, 400);
        issue(&mut s, [build(SWORD), guard(g, QueueMode::Replace)]);
        let p = product(&s);
        finish(&mut s, p);
        if !before_activation {
            activate(&mut s, p, g);
            // Direct Guard has no new lifelong allegiance policy. Capture
            // after activation therefore keeps the existing Guard contract.
            s.world_mut().unit_mut(g).unwrap().owner = PlayerId(1);
            s.step(&[]);
            assert_eq!(
                s.world().unit(p).unwrap().combat.order,
                Some(CombatOrder::Guard(g))
            );
        }
        s.world_mut().despawn(g).unwrap();
        let replacement = target(&mut s, 650, 400);
        assert_eq!(g.index, replacement.index);
        for _ in 0..120 {
            s.step(&[]);
        }
        assert_eq!(s.world().unit(p).unwrap().combat.order, None);
        assert!(!orders(&s, p).contains(&Order::Guard {
            target: replacement
        }));
        assert!(construction::placement::exit_clear(
            s.world(),
            SWORD,
            at(200, 224),
            FACTORY
        ));
    }
}

#[test]
fn guard_route_limit_reserves_exit_and_patrol_return_without_partial_append() {
    for patrol in [false, true] {
        let mut s = guard_sim();
        apply(&mut s, guard(FACTORY, QueueMode::Replace));
        if patrol {
            apply(
                &mut s,
                rally(
                    Some(FactoryRallyOrder::Patrol { to: at(400, 400) }),
                    QueueMode::Append,
                ),
            );
        }
        let recipe_limit = MAX_ORDERS - 1 - usize::from(patrol);
        while s
            .world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory_rally
            .as_ref()
            .unwrap()
            .len()
            < recipe_limit
        {
            apply(&mut s, guard(FACTORY, QueueMode::Append));
        }
        let before = s.world().clone();
        for extra in [
            guard(FACTORY, QueueMode::Append),
            rally(
                Some(FactoryRallyOrder::Move { to: at(500, 500) }),
                QueueMode::Append,
            ),
            rally(
                Some(FactoryRallyOrder::Patrol { to: at(500, 500) }),
                QueueMode::Append,
            ),
        ] {
            apply(&mut s, extra);
            assert_eq!(s.world(), &before);
        }
        issue(&mut s, [build(SWORD)]);
        let p = product(&s);
        finish(&mut s, p);
        assert_eq!(orders(&s, p).len(), MAX_ORDERS);
        assert_eq!(
            orders(&s, p).first(),
            Some(&Order::Move { to: at(200, 256) })
        );
        if patrol {
            assert!(orders(&s, p).contains(&Order::Patrol { to: at(200, 256) }));
        }
    }
    // A previously full Move-only recipe stays intact when adding Guard
    // would newly require the extra reservation.
    let mut s = guard_sim();
    for _ in 0..MAX_ORDERS {
        apply(
            &mut s,
            rally(
                Some(FactoryRallyOrder::Move { to: at(500, 500) }),
                QueueMode::Append,
            ),
        );
    }
    let before = s.world().clone();
    apply(&mut s, guard(FACTORY, QueueMode::Append));
    assert_eq!(s.world(), &before);
}

#[test]
fn guard_target_identity_and_generation_are_hashed() {
    let mut s = guard_sim();
    let a = target(&mut s, 600, 400);
    let b = target(&mut s, 650, 400);
    apply(&mut s, guard(a, QueueMode::Replace));
    let first = s.state_hash();
    apply(&mut s, guard(b, QueueMode::Replace));
    assert_ne!(s.state_hash(), first);
    let mut world = s.world().clone();
    world.unit_mut(FACTORY).unwrap().build.factory_rally = Some(vec![FactoryRallyOrder::Guard {
        target: UnitId::new(b.index, b.generation + 1),
    }]);
    let mut lhs = tak_sim::StateHasher::new();
    let mut rhs = tak_sim::StateHasher::new();
    s.world().hash_into(&mut lhs);
    world.hash_into(&mut rhs);
    assert_ne!(lhs.finish(), rhs.finish());
}

#[test]
fn zero_health_guard_is_rejected_without_changing_legacy_direct_guard_admission() {
    let mut s = guard_sim();
    let g = target(&mut s, 600, 400);
    s.step(&[]);
    s.world_mut().unit_mut(g).unwrap().combat.health = 0;
    assert!(!s.world().unit(g).unwrap().combat.is_dying());
    assert!(!construction::can_factory_guard_target(
        s.world(),
        FACTORY,
        g
    ));
    let before = s.world().clone();
    apply(&mut s, guard(g, QueueMode::Replace));
    assert_eq!(s.world(), &before);
    let p = target(&mut s, 400, 400);
    tak_sim::combat::apply_command(
        s.world_mut(),
        PlayerId(0),
        &SimCommand::Guard {
            units: UnitSet::single(p),
            target: g,
            queue: QueueMode::Replace,
        },
    );
    assert_eq!(
        s.world().unit(p).unwrap().combat.order,
        Some(CombatOrder::Guard(g))
    );
}

#[test]
fn self_product_target_is_skipped_at_completion_and_still_exits() {
    let mut s = guard_sim();
    issue(&mut s, [build(SWORD)]);
    let p = product(&s);
    assert!(construction::can_factory_guard_target(
        s.world(),
        FACTORY,
        p
    ));
    apply(&mut s, guard(p, QueueMode::Replace));
    finish(&mut s, p);
    assert_eq!(orders(&s, p), [Order::Move { to: at(200, 256) }]);
}

#[test]
fn mixed_guard_recipe_keeps_the_same_append_order_as_direct_commands() {
    let mut s = guard_sim();
    let a = target(&mut s, 600, 300);
    let b = target(&mut s, 700, 300);
    issue(&mut s, [build(SWORD), guard(a, QueueMode::Replace)]);
    let p = product(&s);
    for command in [
        rally(
            Some(FactoryRallyOrder::Move { to: at(400, 350) }),
            QueueMode::Append,
        ),
        guard(b, QueueMode::Append),
        rally(
            Some(FactoryRallyOrder::Patrol { to: at(450, 450) }),
            QueueMode::Append,
        ),
    ] {
        apply(&mut s, command);
    }
    finish(&mut s, p);
    let direct = target(&mut s, 100, 500);
    let mut commands = tak_commands::CommandLog::new();
    for command in [
        SimCommand::Move {
            units: UnitSet::single(direct),
            to: at(200, 256),
            queue: QueueMode::Append,
        },
        SimCommand::Guard {
            units: UnitSet::single(direct),
            target: a,
            queue: QueueMode::Append,
        },
        SimCommand::Move {
            units: UnitSet::single(direct),
            to: at(400, 350),
            queue: QueueMode::Append,
        },
        SimCommand::Guard {
            units: UnitSet::single(direct),
            target: b,
            queue: QueueMode::Append,
        },
        SimCommand::Patrol {
            units: UnitSet::single(direct),
            to: at(450, 450),
            queue: QueueMode::Append,
        },
    ] {
        commands.record(s.tick(), PlayerId(0), command);
    }
    let tick = s.tick();
    tak_sim::systems::apply_commands(s.world_mut(), commands.for_tick(tick));
    assert_eq!(orders(&s, p), orders(&s, direct));
    assert_eq!(
        s.world().unit(p).unwrap().combat.order,
        s.world().unit(direct).unwrap().combat.order
    );
}

#[test]
fn blocked_product_pad_keeps_retry_and_production_until_blocker_moves() {
    let mut sc = guard_scenario();
    let blocker_def = tak_commands::UnitDefId(sc.unit_defs.len() as u32);
    let mut flyer = sc.unit_defs[SWORD.0 as usize].clone();
    flyer.name = "SYNTHETIC_PAD_BLOCKER".into();
    flyer.motion.canfly = true;
    sc.unit_defs.push(flyer);
    let mut s = started(&sc);
    let blocker = s
        .world_mut()
        .spawn(blocker_def, PlayerId(0), at(200, 224), Angle::ZERO)
        .unwrap();
    issue(&mut s, [build(SWORD), guard(blocker, QueueMode::Replace)]);
    assert_eq!(s.world().unit(FACTORY).unwrap().build.factory.product, None);
    assert!(
        s.world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory
            .retry_at
            .is_some()
    );
    let before = s.world().unit(FACTORY).unwrap().build.factory;
    apply(&mut s, guard(FACTORY, QueueMode::Replace));
    assert_eq!(s.world().unit(FACTORY).unwrap().build.factory, before);
    for _ in 0..35 {
        s.step(&[]);
        assert_eq!(s.world().unit(FACTORY).unwrap().build.factory.product, None);
    }
    assert_eq!(economy::mana(s.world(), PlayerId(0)).0, 1000 << 16);
    issue(
        &mut s,
        [SimCommand::Move {
            units: UnitSet::single(blocker),
            to: at(500, 224),
            queue: QueueMode::Replace,
        }],
    );
    let mut p = None;
    for _ in 0..120 {
        p = s.world().unit(FACTORY).unwrap().build.factory.product;
        if p.is_some() {
            break;
        }
        s.step(&[]);
    }
    let p = p.expect("ordinary retry notices cleared pad");
    finish(&mut s, p);
    activate(&mut s, p, FACTORY);
    assert!(construction::placement::exit_clear(
        s.world(),
        SWORD,
        at(200, 224),
        FACTORY
    ));
    assert!(s.world_mut().construction.take_refunds().is_empty());
}

#[test]
fn blocked_yard_exit_uses_ordinary_bounded_pathfinding_without_teleporting() {
    use tak_sim::map::{TerrainGrid, terrain};
    let mut sc = guard_scenario();
    let mut flags = vec![0; 64 * 64];
    // Block the south exit itself while leaving the product's pad and the
    // surrounding region reachable. Ordinary pathfinding chooses the nearest
    // reachable point, rather than bypassing the obstruction for Guard.
    for x in 0..64 {
        flags[16 * 64 + x] = terrain::FEATURE;
    }
    sc.map.terrain = Some(TerrainGrid {
        cols: 64,
        rows: 64,
        cell_size: 16,
        flags,
    });
    let mut guarded = started(&sc);
    let mut ordinary = started(&sc);
    issue(
        &mut guarded,
        [build(SWORD), guard(FACTORY, QueueMode::Replace)],
    );
    issue(&mut ordinary, [build(SWORD)]);
    let p = product(&guarded);
    assert_eq!(product(&ordinary), p);
    finish(&mut guarded, p);
    finish(&mut ordinary, p);
    // Compare with an ordinary factory exit plus a directly appended Guard.
    // A trailing order intentionally changes last-leg braking versus idle.
    tak_sim::combat::apply_command(
        ordinary.world_mut(),
        PlayerId(0),
        &SimCommand::Guard {
            units: UnitSet::single(p),
            target: FACTORY,
            queue: QueueMode::Append,
        },
    );
    for _ in 0..160 {
        assert_eq!(
            guarded.world().unit(p).unwrap().pos,
            ordinary.world().unit(p).unwrap().pos
        );
        // Neither route can cross the wall or teleport through it.
        assert!(guarded.world().unit(p).unwrap().pos.y < Fx::from_int(256));
        assert!(guarded.world().unit(p).unwrap().orders.len() <= MAX_ORDERS);
        guarded.step(&[]);
        ordinary.step(&[]);
    }
    assert!(guarded.world().pathing.stats.unreachable > 0);
    assert_eq!(
        guarded.world().unit(p).unwrap().combat.order,
        Some(CombatOrder::Guard(FACTORY))
    );
    assert!(construction::placement::exit_clear(
        guarded.world(),
        SWORD,
        at(200, 224),
        FACTORY
    ));
}
