//! Order routes are a read-only presentation of the local player's view.
//! Synthetic SimView/FogGrid fixtures, not original-game parity evidence.

use tak_client::{
    fog::{FogCell, FogGrid},
    hud,
    view::SimView,
};
use tak_commands::{PlayerId, Target, UnitDefId, UnitId};
use tak_input::WorldView;
use tak_math::{Angle, Vec2};
use tak_sim::{
    Order, World,
    combat::{CombatOrder, Dying},
    hash::StateHasher,
};

fn world() -> World {
    let mut scenario = tak_sim::testing::example_scenario(1, 3);
    scenario.victory.teams = vec![Some(1), Some(2), Some(1)];
    World::new(&scenario).unwrap()
}

fn spawn(w: &mut World, owner: u8, at: Vec2) -> UnitId {
    w.spawn(UnitDefId(0), PlayerId(owner), at, Angle::ZERO)
        .unwrap()
}

fn view<'a>(w: &'a World, fog: Option<&'a FogGrid>) -> SimView<'a> {
    SimView {
        world: w,
        local: PlayerId(0),
        defs: None,
        fog,
    }
}

fn hash(w: &World) -> u64 {
    let mut h = StateHasher::new();
    w.hash_into(&mut h);
    h.finish()
}

fn routes(w: &World, fog: Option<&FogGrid>, actor: UnitId) -> Vec<(Order, Vec2)> {
    let before = hash(w);
    let v = view(w, fog);
    let result = hud::visible_order_goals(&v, w.unit(actor).unwrap())
        .map(|(order, goal)| (*order, goal))
        .collect();
    assert_eq!(hash(w), before, "route display must not mutate the sim");
    result
}

fn unit_orders(target: UnitId) -> [Order; 3] {
    [
        Order::Attack {
            target: Target::Unit(target),
        },
        Order::Guard { target },
        Order::Heal { target },
    ]
}

#[test]
fn queued_enemy_goals_follow_only_current_sight_not_explored_or_unexplored_positions() {
    let mut w = world();
    let actor = spawn(&mut w, 0, Vec2::from_ints(40, 40));
    let target = spawn(&mut w, 1, Vec2::from_ints(168, 168));
    let order = unit_orders(target)[0];
    w.unit_mut(actor).unwrap().orders.push_back(order);
    let mut fog = FogGrid::new(128, 128);

    assert_eq!(fog.at(168.0, 168.0), FogCell::Unexplored);
    assert!(routes(&w, Some(&fog), actor).is_empty());
    fog.update(&[(168.0, 168.0, 40.0)]);
    assert_eq!(fog.at(168.0, 168.0), FogCell::Seen);
    assert_eq!(
        routes(&w, Some(&fog), actor),
        vec![(order, Vec2::from_ints(168, 168))]
    );

    w.unit_mut(target).unwrap().pos = Vec2::from_ints(184, 168);
    assert_eq!(fog.at(184.0, 168.0), FogCell::Seen);
    assert_eq!(
        routes(&w, Some(&fog), actor),
        vec![(order, Vec2::from_ints(184, 168))]
    );

    fog.update(&[]);
    assert_eq!(fog.at(184.0, 168.0), FogCell::Explored);
    assert!(routes(&w, Some(&fog), actor).is_empty());
    w.unit_mut(target).unwrap().pos = Vec2::from_ints(536, 536);
    assert_eq!(fog.at(536.0, 536.0), FogCell::Unexplored);
    assert!(routes(&w, Some(&fog), actor).is_empty());
    assert_eq!(
        hud::order_goal(&w, &order),
        Some(Vec2::from_ints(536, 536)),
        "the raw sim still pursues the target; rendering alone filters it"
    );

    fog.update(&[(536.0, 536.0, 40.0)]);
    assert_eq!(
        routes(&w, Some(&fog), actor),
        vec![(order, Vec2::from_ints(536, 536))]
    );
}

#[test]
fn all_unit_goal_kinds_require_selectable_exact_generation_and_never_retarget_a_reused_slot() {
    let mut w = world();
    let actor = spawn(&mut w, 0, Vec2::from_ints(40, 40));
    let at = Vec2::from_ints(168, 168);
    let target = spawn(&mut w, 1, at);
    let orders = unit_orders(target);
    w.unit_mut(actor).unwrap().orders.extend(orders);
    let mut fog = FogGrid::new(128, 128);
    fog.update(&[(168.0, 168.0, 40.0)]);
    assert_eq!(
        routes(&w, Some(&fog), actor),
        orders.map(|order| (order, at))
    );

    w.unit_mut(target).unwrap().combat.dying = Some(Dying {
        since: 0,
        severity: 100,
        kind: 0,
        done: false,
    });
    assert!(!view(&w, Some(&fog)).unit(target).unwrap().selectable);
    assert!(routes(&w, Some(&fog), actor).is_empty());
    assert!(routes(&w, None, actor).is_empty());
    w.despawn(target).unwrap();
    assert!(routes(&w, Some(&fog), actor).is_empty());

    let recycled = spawn(&mut w, 0, Vec2::from_ints(600, 600));
    assert_eq!(target.index, recycled.index);
    assert_ne!(target.generation, recycled.generation);
    assert!(view(&w, Some(&fog)).unit(recycled).unwrap().selectable);
    assert!(routes(&w, Some(&fog), actor).is_empty());
    for order in orders {
        assert_eq!(hud::visible_order_goal(&view(&w, None), &order), None);
    }
}

#[test]
fn own_and_allied_targets_remain_visible_without_sight_and_no_fog_reveals_enemies() {
    let mut w = world();
    let actor = spawn(&mut w, 0, Vec2::from_ints(40, 40));
    let own = spawn(&mut w, 0, Vec2::from_ints(200, 200));
    let ally = spawn(&mut w, 2, Vec2::from_ints(400, 400));
    let enemy = spawn(&mut w, 1, Vec2::from_ints(600, 600));
    let mut expected = Vec::new();
    for target in [own, ally, enemy] {
        let pos = w.unit(target).unwrap().pos;
        for order in unit_orders(target) {
            w.unit_mut(actor).unwrap().orders.push_back(order);
            expected.push((order, pos));
        }
    }
    let fog = FogGrid::new(128, 128);
    assert!(w.are_allied(PlayerId(0), PlayerId(2)));
    assert_eq!(routes(&w, Some(&fog), actor), expected[..6]);
    assert_eq!(routes(&w, None, actor), expected);
}

#[test]
fn known_ground_route_keeps_its_order_and_coordinates_inside_fog() {
    let mut w = world();
    let actor = spawn(&mut w, 0, Vec2::from_ints(40, 40));
    let target = spawn(&mut w, 1, Vec2::from_ints(600, 600));
    let points = [200, 400, 600, 800, 1000].map(|n| Vec2::from_ints(n, n));
    let known = [
        Order::Move { to: points[0] },
        Order::AttackMove { to: points[1] },
        Order::Patrol { to: points[2] },
        Order::Attack {
            target: Target::Ground(points[3]),
        },
        Order::Build {
            def: UnitDefId(0),
            at: Some(points[4]),
        },
    ];
    for order in known {
        w.unit_mut(actor).unwrap().orders.push_back(order);
        w.unit_mut(actor)
            .unwrap()
            .orders
            .push_back(unit_orders(target)[0]);
    }
    w.unit_mut(actor).unwrap().orders.push_back(Order::Build {
        def: UnitDefId(0),
        at: None,
    });
    let fog = FogGrid::new(128, 128);
    assert_eq!(
        routes(&w, Some(&fog), actor),
        known.into_iter().zip(points).collect::<Vec<_>>()
    );
}

#[test]
fn unseen_active_attack_hides_only_its_front_steering_not_a_later_identical_move() {
    let mut w = world();
    let actor = spawn(&mut w, 0, Vec2::from_ints(40, 40));
    let goal = Vec2::from_ints(168, 168);
    let target = spawn(&mut w, 1, goal);
    let walk = Order::Move { to: goal };
    let attack = unit_orders(target)[0];
    let patrol = Order::Patrol {
        to: Vec2::from_ints(800, 800),
    };
    let unit = w.unit_mut(actor).unwrap();
    unit.combat.order = Some(CombatOrder::Attack(Target::Unit(target)));
    unit.combat.goal = Some(goal);
    unit.orders.extend([walk, attack, walk, patrol]);
    let queue_before = unit.orders.clone();
    let text_before = hud::order_text(&w, w.unit(actor).unwrap(), |_| "unit".into());
    let mut fog = FogGrid::new(128, 128);
    fog.update(&[(168.0, 168.0, 40.0)]);
    assert_eq!(routes(&w, Some(&fog), actor).len(), 4);

    fog.update(&[]);
    assert_eq!(fog.at(168.0, 168.0), FogCell::Explored);
    assert_eq!(
        routes(&w, Some(&fog), actor),
        vec![(walk, goal), (patrol, Vec2::from_ints(800, 800))]
    );
    assert_eq!(w.unit(actor).unwrap().orders, queue_before);
    assert_eq!(
        hud::orders_for_display(w.unit(actor).unwrap())
            .copied()
            .collect::<Vec<_>>(),
        queue_before.into_iter().collect::<Vec<_>>()
    );
    assert_eq!(
        hud::order_text(&w, w.unit(actor).unwrap(), |_| "unit".into()),
        text_before,
        "route privacy must not change status text or queue counts"
    );

    let moved = Vec2::from_ints(536, 536);
    w.unit_mut(target).unwrap().pos = moved;
    let unit = w.unit_mut(actor).unwrap();
    unit.combat.goal = Some(moved);
    unit.orders[0] = Order::Move { to: moved };
    assert_eq!(fog.at(536.0, 536.0), FogCell::Unexplored);
    assert_eq!(
        routes(&w, Some(&fog), actor),
        vec![(walk, goal), (patrol, Vec2::from_ints(800, 800))],
        "the moving hidden target and its new steering goal must stay hidden"
    );
}

#[test]
fn active_attack_steering_is_retained_for_visible_own_and_allied_targets() {
    for owner in [0, 1, 2] {
        let mut w = world();
        let actor = spawn(&mut w, 0, Vec2::from_ints(40, 40));
        let goal = Vec2::from_ints(168, 168);
        let target = spawn(&mut w, owner, goal);
        let walk = Order::Move { to: goal };
        let unit = w.unit_mut(actor).unwrap();
        unit.combat.order = Some(CombatOrder::Attack(Target::Unit(target)));
        unit.combat.goal = Some(goal);
        unit.orders.push_back(walk);
        let mut fog = FogGrid::new(128, 128);
        if owner == 1 {
            fog.update(&[(168.0, 168.0, 40.0)]);
        }
        assert_eq!(routes(&w, Some(&fog), actor), vec![(walk, goal)]);
    }
}

#[test]
fn invalid_active_target_does_not_expose_old_steering_or_a_recycled_units_position() {
    let mut w = world();
    let actor = spawn(&mut w, 0, Vec2::from_ints(40, 40));
    let goal = Vec2::from_ints(168, 168);
    let target = spawn(&mut w, 1, goal);
    let unit = w.unit_mut(actor).unwrap();
    unit.combat.order = Some(CombatOrder::Attack(Target::Unit(target)));
    unit.combat.goal = Some(goal);
    unit.orders.push_back(Order::Move { to: goal });
    assert_eq!(routes(&w, None, actor).len(), 1);
    w.unit_mut(target).unwrap().combat.dying = Some(Dying {
        since: 0,
        severity: 100,
        kind: 0,
        done: true,
    });
    assert!(routes(&w, None, actor).is_empty());
    w.despawn(target).unwrap();
    assert!(routes(&w, None, actor).is_empty());
    let recycled = spawn(&mut w, 0, Vec2::from_ints(600, 600));
    assert_eq!(target.index, recycled.index);
    assert_ne!(target.generation, recycled.generation);
    assert!(routes(&w, None, actor).is_empty());
}

#[test]
fn active_steering_filter_requires_a_unit_attack_and_matching_front_goal() {
    let mut w = world();
    let actor = spawn(&mut w, 0, Vec2::from_ints(40, 40));
    let goal = Vec2::from_ints(168, 168);
    let other = Vec2::from_ints(500, 500);
    let target = spawn(&mut w, 1, goal);
    let attack = Some(CombatOrder::Attack(Target::Unit(target)));
    let walk = Order::Move { to: goal };
    let fog = FogGrid::new(128, 128);
    for (order, combat_goal) in [
        (None, Some(goal)),
        (Some(CombatOrder::Attack(Target::Ground(goal))), Some(goal)),
        (attack, None),
        (attack, Some(other)),
    ] {
        let unit = w.unit_mut(actor).unwrap();
        unit.combat.order = order;
        unit.combat.goal = combat_goal;
        unit.orders.clear();
        unit.orders.push_back(walk);
        assert_eq!(routes(&w, Some(&fog), actor), vec![(walk, goal)]);
    }
    let unit = w.unit_mut(actor).unwrap();
    unit.combat.order = attack;
    unit.combat.goal = Some(goal);
    unit.orders.clear();
    unit.orders.extend([Order::Move { to: other }, walk]);
    assert_eq!(
        routes(&w, Some(&fog), actor),
        vec![(Order::Move { to: other }, other), (walk, goal)],
        "a matching non-front Move is a player order, not active steering"
    );
}

#[test]
fn direct_view_lookup_matches_enumeration_for_fog_death_teams_metadata_and_generations() {
    let mut scenario = tak_sim::testing::example_scenario(1, 3);
    scenario.victory.teams = vec![Some(1), Some(2), Some(1)];
    scenario.unit_defs[0].build.builder = true;
    scenario.unit_defs[0].build.mobile = true;
    scenario.unit_defs[0].footprintx = 3;
    scenario.unit_defs[1].build.builder = true;
    scenario.unit_defs[1].build.mobile = false;
    let mut w = World::new(&scenario).unwrap();
    let own = spawn(&mut w, 0, Vec2::from_ints(40, 40));
    let ally = w
        .spawn(
            UnitDefId(1),
            PlayerId(2),
            Vec2::from_ints(600, 600),
            Angle::ZERO,
        )
        .unwrap();
    let seen_enemy = spawn(&mut w, 1, Vec2::from_ints(168, 168));
    let unseen_enemy = spawn(&mut w, 1, Vec2::from_ints(800, 800));
    let dying_enemy = spawn(&mut w, 1, Vec2::from_ints(184, 168));
    w.unit_mut(dying_enemy).unwrap().combat.dying = Some(Dying {
        since: 0,
        severity: 100,
        kind: 0,
        done: false,
    });
    w.unit_mut(ally).unwrap().build.frame = Some(tak_sim::construction::progress::Frame::new(10));
    let removed = spawn(&mut w, 1, Vec2::from_ints(1000, 1000));
    w.despawn(removed).unwrap();
    let recycled = spawn(&mut w, 0, Vec2::from_ints(1200, 1200));
    assert_eq!(removed.index, recycled.index);
    assert_ne!(removed.generation, recycled.generation);
    let mut fog = FogGrid::new(128, 128);
    fog.update(&[(168.0, 168.0, 40.0)]);
    let mut explored = fog.clone();
    explored.update(&[]);
    let before = hash(&w);

    for grid in [Some(&fog), Some(&explored), None] {
        let v = view(&w, grid);
        let mut enumerated = std::collections::BTreeMap::new();
        v.for_each_unit(&mut |u| {
            enumerated.insert(u.id, *u);
        });
        for id in [
            own,
            ally,
            seen_enemy,
            unseen_enemy,
            dying_enemy,
            removed,
            recycled,
            UnitId::new(u32::MAX, 0),
        ] {
            assert_eq!(v.unit(id), enumerated.get(&id).copied(), "{id:?}");
        }
        assert_eq!(
            v.unit(own).unwrap().builder,
            Some(tak_input::BuilderKind::Mobile)
        );
        assert_eq!(
            v.unit(ally).unwrap().builder,
            Some(tak_input::BuilderKind::Factory)
        );
        assert!(v.unit(ally).unwrap().under_construction);
        assert!(v.unit(removed).is_none());
    }
    assert!(view(&w, Some(&fog)).unit(seen_enemy).is_some());
    assert!(view(&w, Some(&explored)).unit(seen_enemy).is_none());
    assert!(view(&w, Some(&fog)).unit(unseen_enemy).is_none());
    assert!(view(&w, None).unit(unseen_enemy).is_some());
    assert!(!view(&w, Some(&fog)).unit(dying_enemy).unwrap().selectable);
    assert_eq!(hash(&w), before);
}
