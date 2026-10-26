//! Continuous production is a bounded recipe, not an ever-growing finite queue.
mod factory_controls_support;
use factory_controls_support::*;
use tak_commands::{
    CommandLog, FactoryRallyOrder, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId,
    UnitSet,
};
use tak_math::Angle;
use tak_sim::{Order, Sim, construction, economy, systems};

fn repeat(def: Option<UnitDefId>) -> SimCommand {
    SimCommand::SetFactoryRepeat {
        units: UnitSet::single(FACTORY),
        def,
    }
}
fn product(s: &Sim) -> Option<UnitId> {
    s.world().unit(FACTORY).unwrap().build.factory.product
}
fn queue(s: &Sim) -> Vec<Order> {
    s.world()
        .unit(FACTORY)
        .unwrap()
        .orders
        .iter()
        .copied()
        .collect()
}
fn finish(s: &mut Sim, p: UnitId) {
    for _ in 0..200 {
        if s.world().unit(p).unwrap().build.is_complete() {
            return;
        }
        s.step(&[]);
    }
    panic!("paid product did not finish");
}
fn next_product(s: &mut Sim) -> UnitId {
    for _ in 0..300 {
        if let Some(p) = product(s) {
            return p;
        }
        s.step(&[]);
    }
    panic!("factory did not produce");
}
fn count(s: &Sim, def: UnitDefId) -> u32 {
    construction::production_count(s.world(), FACTORY, def)
}

#[test]
fn continuous_products_pay_exact_cost_without_allocating_finite_orders() {
    let mut s = sim();
    issue(&mut s, [repeat(Some(SWORD))]);
    for _ in 0..300 {
        assert!(queue(&s).is_empty());
        assert_eq!(
            construction::factory_repeat(s.world(), FACTORY),
            Some(SWORD)
        );
        s.step(&[]);
    }
    let products: Vec<_> = s.world().units().filter(|(_, u)| u.def == SWORD).collect();
    let finished = products
        .iter()
        .filter(|(_, u)| u.build.is_complete())
        .count();
    assert!(
        finished >= 5,
        "continuous recipe must really finish several products"
    );
    let paid: i64 = products
        .iter()
        .map(|(_, u)| u.build.frame.map_or(30 << 16, |f| f.paid))
        .sum();
    assert_eq!(economy::mana(s.world(), PlayerId(0)).0, (1000 << 16) - paid);
    assert_eq!(count(&s, SWORD), u32::from(product(&s).is_some()));
    assert_eq!(s.world().unit(FACTORY).unwrap().build.factory_rally, None);
}

#[test]
fn finite_fifo_precedes_repeat_and_active_repeat_never_consumes_new_finite_entries() {
    let mut s = sim();
    issue(&mut s, [repeat(Some(SWORD)), build(MASON), build(SWORD)]);
    let first = product(&s).unwrap();
    assert_eq!(s.world().unit(first).unwrap().def, MASON);
    assert!(
        !s.world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    finish(&mut s, first);
    let second = next_product(&mut s);
    assert_eq!(s.world().unit(second).unwrap().def, SWORD);
    assert!(
        !s.world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    finish(&mut s, second);
    let repeated = next_product(&mut s);
    assert!(
        s.world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
    issue(&mut s, [build(MASON), build(SWORD)]);
    assert_eq!(product(&s), Some(repeated));
    let pending = queue(&s);
    assert_eq!(count(&s, SWORD), 2, "active repeat is counted separately");
    finish(&mut s, repeated);
    assert_eq!(
        queue(&s),
        pending,
        "repeat completion must not pop finite work"
    );
    assert_eq!(count(&s, SWORD), 1);
    let finite = next_product(&mut s);
    assert_eq!(s.world().unit(finite).unwrap().def, MASON);
    assert!(
        !s.world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
}

#[test]
fn changing_or_disabling_repeat_preserves_active_paid_frame_and_release_recipe() {
    for recipe in [None, Some(MASON)] {
        let mut s = sim();
        issue(&mut s, [repeat(Some(SWORD))]);
        let p = product(&s).unwrap();
        let paid = s.world().unit(p).unwrap().build.frame.unwrap().paid;
        issue(
            &mut s,
            [
                repeat(recipe),
                rally(
                    Some(FactoryRallyOrder::Move { to: at(700, 500) }),
                    QueueMode::Replace,
                ),
            ],
        );
        assert_eq!(product(&s), Some(p));
        assert!(s.world().unit(p).unwrap().build.frame.unwrap().paid > paid);
        assert_eq!(construction::factory_repeat(s.world(), FACTORY), recipe);
        assert_eq!(
            count(&s, SWORD),
            1,
            "disabled recipe does not hide active work"
        );
        finish(&mut s, p);
        assert_eq!(
            s.world().unit(p).unwrap().orders.front(),
            Some(&Order::Move { to: at(700, 500) })
        );
        assert!(
            !s.world()
                .unit(FACTORY)
                .unwrap()
                .build
                .factory
                .product_from_repeat
        );
        if recipe.is_some() {
            let next = next_product(&mut s);
            assert_eq!(s.world().unit(next).unwrap().def, MASON);
            assert!(
                s.world()
                    .unit(FACTORY)
                    .unwrap()
                    .build
                    .factory
                    .product_from_repeat
            );
        } else {
            for _ in 0..50 {
                s.step(&[]);
            }
            assert_eq!(product(&s), None);
            assert!(!s.world().unit(FACTORY).unwrap().build.factory.active);
            assert_eq!(economy::mana(s.world(), PlayerId(0)).0, 970 << 16);
        }
    }
}

#[test]
fn cancellation_clears_matching_recipe_for_free_then_finite_tail_before_active_repeat() {
    let mut s = sim();
    issue(&mut s, [repeat(Some(SWORD))]);
    let p = product(&s).unwrap();
    issue(&mut s, [build(SWORD), build(MASON), build(SWORD)]);
    let before = s.world().clone();
    construction::apply_factory_command(s.world_mut(), PlayerId(0), &cancel(SWORD, 0));
    assert_eq!(s.world(), &before, "zero count cannot even clear a recipe");
    issue(&mut s, [cancel(SWORD, 1)]);
    assert_eq!(construction::factory_repeat(s.world(), FACTORY), None);
    assert_eq!(product(&s), Some(p));
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
            }
        ]
    );
    assert_eq!(count(&s, SWORD), 2);
    // A different future recipe must survive cancelling the old active frame.
    construction::apply_factory_command(s.world_mut(), PlayerId(0), &repeat(Some(MASON)));
    let paid = s.world().unit(p).unwrap().build.frame.unwrap().paid;
    let mana = economy::mana(s.world(), PlayerId(0)).0;
    issue(&mut s, [cancel(SWORD, 2), cancel(SWORD, 1), build(SWORD)]);
    assert!(s.world().unit(p).is_none());
    assert_eq!(economy::mana(s.world(), PlayerId(0)).0, mana + paid);
    assert_eq!(
        construction::factory_repeat(s.world(), FACTORY),
        Some(MASON)
    );
    assert_eq!(
        product(&s),
        None,
        "replacement starts next tick, never refund tick"
    );
    assert_eq!(count(&s, SWORD), 1);
    let next = next_product(&mut s);
    assert_eq!(s.world().unit(next).unwrap().def, MASON);
    assert!(
        !s.world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
}

#[test]
fn removing_active_finite_work_preserves_different_repeat_and_restarts_next_tick() {
    let mut s = sim();
    issue(&mut s, [build(SWORD), repeat(Some(MASON))]);
    let p = product(&s).unwrap();
    let paid = s.world().unit(p).unwrap().build.frame.unwrap().paid;
    let mana = economy::mana(s.world(), PlayerId(0)).0;
    issue(&mut s, [cancel(SWORD, 1)]);
    assert_eq!(product(&s), None);
    assert!(s.world().unit(p).is_none());
    assert_eq!(economy::mana(s.world(), PlayerId(0)).0, mana + paid);
    assert_eq!(
        construction::factory_repeat(s.world(), FACTORY),
        Some(MASON)
    );
    let next = next_product(&mut s);
    assert_eq!(s.world().unit(next).unwrap().def, MASON);
    assert!(
        s.world()
            .unit(FACTORY)
            .unwrap()
            .build
            .factory
            .product_from_repeat
    );
}

#[test]
fn stop_and_generic_replace_cancel_active_repeat_even_after_recipe_disabled() {
    for enabled in [false, true] {
        for command in [
            SimCommand::Stop {
                units: UnitSet::single(FACTORY),
            },
            SimCommand::Move {
                units: UnitSet::single(FACTORY),
                to: at(500, 500),
                queue: QueueMode::Replace,
            },
            SimCommand::Patrol {
                units: UnitSet::single(FACTORY),
                to: at(500, 500),
                queue: QueueMode::Replace,
            },
        ] {
            let mut s = sim();
            issue(&mut s, [repeat(Some(SWORD))]);
            let p = product(&s).unwrap();
            if !enabled {
                issue(&mut s, [repeat(None)]);
            }
            let paid = s.world().unit(p).unwrap().build.frame.unwrap().paid;
            let mana = economy::mana(s.world(), PlayerId(0)).0;
            issue(&mut s, [command, build(MASON)]);
            assert_eq!(construction::factory_repeat(s.world(), FACTORY), None);
            assert_eq!(product(&s), None);
            assert!(s.world().unit(p).is_none());
            assert_eq!(economy::mana(s.world(), PlayerId(0)).0, mana + paid);
            let next = next_product(&mut s);
            assert_eq!(s.world().unit(next).unwrap().def, MASON);
        }
    }
}

#[test]
fn destroyed_on_pad_product_clears_finite_work_and_repeat_without_refund() {
    let mut s = sim();
    issue(&mut s, [repeat(Some(SWORD))]);
    let p = product(&s).unwrap();
    issue(&mut s, [repeat(Some(MASON)), build(SWORD)]);
    let mana = economy::mana(s.world(), PlayerId(0)).0;
    s.world_mut().despawn(p).unwrap();
    s.step(&[]);
    assert_eq!(product(&s), None);
    assert_eq!(construction::factory_repeat(s.world(), FACTORY), None);
    assert!(queue(&s).is_empty());
    assert_eq!(economy::mana(s.world(), PlayerId(0)).0, mana);
    assert!(!s.world().unit(FACTORY).unwrap().build.factory.active);
}

#[test]
fn repeat_validation_and_sparse_hashes_preserve_default_state() {
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
    let dying = s
        .world_mut()
        .spawn(KEEP, PlayerId(0), at(600, 100), Angle::ZERO)
        .unwrap();
    s.world_mut().unit_mut(dying).unwrap().combat.dying = Some(tak_sim::combat::Dying {
        since: 0,
        severity: 1,
        kind: 0,
        done: false,
    });
    let stale = UnitId::new(FACTORY.index, FACTORY.generation + 1);
    let invalid = vec![ENEMY_FACTORY, mobile, frame, dying, stale].into();
    let before = s.world().clone();
    let hash = s.state_hash();
    for command in [
        SimCommand::SetFactoryRepeat {
            units: invalid,
            def: Some(SWORD),
        },
        repeat(Some(UnitDefId(u32::MAX))),
        repeat(Some(KEEP)),
        repeat(None),
    ] {
        construction::apply_factory_command(s.world_mut(), PlayerId(0), &command);
    }
    assert_eq!(s.world(), &before);
    assert_eq!(s.state_hash(), hash);
    construction::apply_factory_command(s.world_mut(), PlayerId(0), &repeat(Some(SWORD)));
    let sword_hash = s.state_hash();
    assert_ne!(sword_hash, hash);
    for bad in [UnitDefId(u32::MAX), KEEP] {
        construction::apply_factory_command(s.world_mut(), PlayerId(0), &repeat(Some(bad)));
        assert_eq!(
            s.state_hash(),
            sword_hash,
            "invalid recipe must not clear existing recipe"
        );
    }
    construction::apply_factory_command(s.world_mut(), PlayerId(0), &repeat(Some(MASON)));
    assert_ne!(s.state_hash(), sword_hash);
    construction::apply_factory_command(s.world_mut(), PlayerId(0), &repeat(None));
    assert_eq!(
        s.state_hash(),
        hash,
        "no sparse suffix remains after a no-work disable"
    );
    // Eligibility queries never reveal state on a stale or incapable unit.
    for id in [mobile, frame, dying, stale] {
        assert_eq!(construction::factory_repeat(s.world(), id), None);
    }
    issue(&mut s, [repeat(Some(SWORD)), repeat(None)]);
    assert_eq!(
        product(&s),
        None,
        "same-tick disable does not create a frame"
    );
    issue(&mut s, [repeat(Some(SWORD))]);
    construction::apply_factory_command(s.world_mut(), PlayerId(0), &repeat(None));
    let active_hash = s.state_hash();
    s.world_mut()
        .unit_mut(FACTORY)
        .unwrap()
        .build
        .factory
        .product_from_repeat = false;
    assert_ne!(
        s.state_hash(),
        active_hash,
        "active origin is independently hashed"
    );
}

#[test]
fn repeat_dispatch_rejects_wrong_tick_and_unknown_player() {
    let mut s = sim();
    let before = s.world().clone();
    let mut c = tak_commands::LoggedCommand {
        tick: Tick(100),
        player: PlayerId(0),
        seq: 0,
        command: repeat(Some(SWORD)),
    };
    systems::apply_commands(s.world_mut(), &[c.clone()]);
    c.tick = s.tick();
    c.player = PlayerId(42);
    systems::apply_commands(s.world_mut(), &[c]);
    assert_eq!(s.world(), &before);
}

#[test]
fn repeat_preserves_door_handshake_exit_retry_and_one_continuous_activation() {
    let rec = Recorder::default();
    let log = rec.log.clone();
    let mut s = Sim::with_scripts(&scenario(), Box::new(rec)).unwrap();
    step(&mut s, &log, &[]);
    let blocker = s
        .world_mut()
        .spawn(SWORD, PlayerId(0), at(200, 224), Angle::ZERO)
        .unwrap();
    let mut commands = CommandLog::new();
    commands.record(Tick(1), PlayerId(0), repeat(Some(SWORD)));
    step(&mut s, &log, commands.entries());
    assert_eq!(product(&s), None, "Activate must wait for INBUILDSTANCE");
    step(&mut s, &log, &[]);
    let retry = s
        .world()
        .unit(FACTORY)
        .unwrap()
        .build
        .factory
        .retry_at
        .unwrap();
    assert_eq!(retry, Tick(2 + construction::EXIT_RETRY_TICKS));
    s.world_mut().despawn(blocker).unwrap();
    while s.tick() < retry {
        step(&mut s, &log, &[]);
        assert_eq!(
            product(&s),
            None,
            "exit is not rechecked before retry deadline"
        );
    }
    step(&mut s, &log, &[]);
    assert!(product(&s).is_some());
    for _ in 0..120 {
        step(&mut s, &log, &[]);
    }
    let l = log.borrow();
    assert_eq!(l.calls_of(FACTORY, "Activate").len(), 1);
    assert!(l.calls_of(FACTORY, "StartBuilding").len() >= 2);
    assert!(l.calls_of(FACTORY, "StopBuilding").len() >= 2);
    assert!(l.calls_of(FACTORY, "Deactivate").is_empty());
}

#[test]
fn empty_mana_stalls_repeat_without_changing_origin_and_later_income_resumes() {
    let mut sc = scenario();
    sc.economy.rules.start_mana = 0;
    let mut s = Sim::new(&sc).unwrap();
    s.step(&[]);
    issue(&mut s, [repeat(Some(SWORD))]);
    let p = product(&s).unwrap();
    let frame = s.world().unit(p).unwrap().build.frame.unwrap();
    assert_eq!(frame.paid, 0);
    for _ in 0..50 {
        s.step(&[]);
    }
    assert_eq!(s.world().unit(p).unwrap().build.frame.unwrap(), frame);
    assert_eq!(count(&s, SWORD), 1);
    // A newly completed mana-income unit provides ordinary economy income.
    s.world_mut()
        .spawn(KING, PlayerId(0), at(100, 100), Angle::ZERO)
        .unwrap();
    finish(&mut s, p);
    assert!(s.world().unit(p).unwrap().build.is_complete());
    assert_eq!(
        construction::factory_repeat(s.world(), FACTORY),
        Some(SWORD)
    );
}

#[test]
fn repeat_cancel_and_replacing_orders_precede_lower_slot_guard_and_heal_finish() {
    use tak_sim::campaign::InitialUnit;
    for heal in [false, true] {
        for stop in [false, true] {
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
            sc.unit_defs[KING.0 as usize].economy.mogriumincome = 0;
            sc.unit_defs[SWORD.0 as usize].combat.max_damage = 100;
            let mut s = Sim::new(&sc).unwrap();
            s.step(&[]);
            issue(
                &mut s,
                [SimCommand::SetFactoryRepeat {
                    units: UnitSet::single(factory),
                    def: Some(SWORD),
                }],
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
            assert_eq!(
                if heal {
                    build.heal.unwrap().work.target
                } else {
                    build.assist.unwrap().target
                },
                p
            );
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
            let mana = economy::mana(s.world(), PlayerId(0)).0;
            let cancel = if stop {
                // A disabled recipe still retains an active repeated frame.
                construction::apply_factory_command(
                    s.world_mut(),
                    PlayerId(0),
                    &SimCommand::SetFactoryRepeat {
                        units: UnitSet::single(factory),
                        def: None,
                    },
                );
                SimCommand::Stop {
                    units: UnitSet::single(factory),
                }
            } else {
                SimCommand::CancelProduction {
                    units: UnitSet::single(factory),
                    def: SWORD,
                    count: 1,
                }
            };
            issue(
                &mut s,
                [
                    cancel,
                    SimCommand::SetFactoryRepeat {
                        units: UnitSet::single(factory),
                        def: Some(MASON),
                    },
                ],
            );
            assert!(
                s.world().unit(p).is_none(),
                "lower-slot helper cannot complete cancelled frame"
            );
            assert_eq!(economy::mana(s.world(), PlayerId(0)).0, mana + frame.paid);
            assert_eq!(s.world().unit(factory).unwrap().build.factory.product, None);
            assert_eq!(
                construction::factory_repeat(s.world(), factory),
                Some(MASON)
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
    }
}

fn repeat_log() -> CommandLog {
    let mut log = CommandLog::new();
    for (tick, command) in [
        (1, repeat(Some(SWORD))),
        (4, build(MASON)),
        (5, build(SWORD)),
        (6, repeat(Some(MASON))),
        (8, cancel(SWORD, 1)),
        (12, repeat(None)),
        (18, repeat(Some(SWORD))),
        (20, cancel(SWORD, 1)),
        (20, build(MASON)),
        (21, repeat(Some(MASON))),
        (
            35,
            rally(
                Some(FactoryRallyOrder::Patrol { to: at(600, 400) }),
                QueueMode::Replace,
            ),
        ),
        (65, repeat(Some(SWORD))),
        (100, build(MASON)),
        (130, repeat(None)),
        (170, repeat(Some(SWORD))),
        (
            190,
            SimCommand::Stop {
                units: UnitSet::single(FACTORY),
            },
        ),
        (191, repeat(Some(MASON))),
        (240, repeat(None)),
    ] {
        log.record(Tick(tick), PlayerId(0), command);
    }
    log
}
fn repeat_hashes(log: &CommandLog) -> Vec<u64> {
    let mut s = Sim::new(&scenario()).unwrap();
    let hashes = (0..300).map(|_| s.step(log.for_tick(s.tick()))).collect();
    assert!(
        s.world()
            .units()
            .any(|(_, u)| u.def == MASON && u.build.is_complete())
    );
    assert_eq!(construction::factory_repeat(s.world(), FACTORY), None);
    hashes
}
fn digest(hashes: &[u64]) -> String {
    let mut hash = tak_sim::StateHasher::new();
    for &h in hashes {
        hash.u64(h);
    }
    format!("{:016x}", hash.finish())
}

#[test]
fn repeat_command_json_and_per_tick_hashes_match_across_runs_and_processes() {
    const CHILD: &str = "TAK_FACTORY_REPEAT_DETERMINISM_CHILD";
    let log = repeat_log();
    let hashes = repeat_hashes(&log);
    let d = digest(&hashes);
    if std::env::var_os(CHILD).is_some() {
        println!("REPEAT_DIGEST {d}");
        return;
    }
    assert_eq!(hashes, repeat_hashes(&log));
    let json = log.to_json();
    assert!(json.contains("\"type\":\"set_factory_repeat\""));
    assert_eq!(
        hashes,
        repeat_hashes(&CommandLog::from_json(&json).unwrap())
    );
    assert_eq!(repeat(Some(SWORD)).units(), &UnitSet::single(FACTORY));
    assert_eq!(repeat(Some(SWORD)).queue(), None);
    assert_eq!(repeat(None).queue(), None);
    for _ in 0..2 {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "repeat_command_json_and_per_tick_hashes_match_across_runs_and_processes",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout);
        let child = text
            .split("REPEAT_DIGEST ")
            .nth(1)
            .and_then(|s| s.lines().next())
            .unwrap();
        assert_eq!(child, d);
    }
}

#[derive(Clone, Default)]
struct DelayedDeath(std::rc::Rc<std::cell::Cell<bool>>);
impl tak_sim::ScriptHost for DelayedDeath {
    fn has_function(&self, _: UnitId, name: &str) -> bool {
        name == "Dying"
    }
    fn port(&self, _: UnitId, port: i32) -> Option<i32> {
        (port == tak_sim::combat::port::DEATH_DONE).then_some(i32::from(self.0.get()))
    }
}
fn lethal(s: &mut Sim, scripts: &mut dyn tak_sim::ScriptHost, id: UnitId) {
    let now = s.tick().0;
    let pos = s.world().unit(id).unwrap().pos;
    tak_sim::combat::apply_damage(
        s.world_mut(),
        scripts,
        id,
        100_000,
        None,
        tak_sim::combat::kind::NORMAL,
        pos,
        now,
    );
    assert!(s.world().unit(id).unwrap().combat.is_dying());
}

#[test]
fn lethal_factory_damage_cancels_paid_repeat_before_long_death_animation() {
    for enabled in [false, true] {
        let mut sc = scenario();
        sc.unit_defs[KEEP.0 as usize].combat.max_damage = 100;
        let mut host = DelayedDeath::default();
        let mut s = Sim::with_scripts(&sc, Box::new(host.clone())).unwrap();
        s.step(&[]);
        issue(&mut s, [repeat(Some(SWORD))]);
        let p = product(&s).unwrap();
        if !enabled {
            issue(&mut s, [repeat(None)]);
        }
        let mana = economy::mana(s.world(), PlayerId(0)).0;
        let paid = s.world().unit(p).unwrap().build.frame.unwrap().paid;
        lethal(&mut s, &mut host, FACTORY);
        assert_eq!(s.world().unit(FACTORY).unwrap().build.factory_repeat, None);
        assert_eq!(product(&s), None);
        assert!(
            s.world().unit(p).is_none(),
            "dead factory cannot orphan its paid repeat"
        );
        for _ in 0..50 {
            s.step(&[]);
        }
        assert!(s.world().unit(FACTORY).unwrap().combat.is_dying());
        assert_eq!(product(&s), None);
        assert_eq!(economy::mana(s.world(), PlayerId(0)).0, mana + paid);
        assert!(!s.world().units().any(|(_, u)| u.def == SWORD));
        host.0.set(true);
        s.step(&[]);
        assert!(s.world().unit(FACTORY).is_none());
    }
}

#[test]
fn dying_product_one_work_step_from_finish_clears_repeat_and_finite_work_without_refund() {
    for (repeated, enabled) in [(false, true), (true, false), (true, true)] {
        let mut sc = scenario();
        sc.unit_defs[SWORD.0 as usize].combat.max_damage = 100;
        let mut host = DelayedDeath::default();
        let mut s = Sim::with_scripts(&sc, Box::new(host.clone())).unwrap();
        s.step(&[]);
        if repeated {
            issue(&mut s, [repeat(Some(SWORD))]);
        } else {
            issue(&mut s, [build(SWORD), repeat(Some(MASON))]);
        }
        let p = product(&s).unwrap();
        while {
            let f = s.world().unit(p).unwrap().build.frame.unwrap();
            f.total - f.done > construction::progress::worker_step(10)
        } {
            s.step(&[]);
        }
        let frame = s.world().unit(p).unwrap().build.frame.unwrap();
        assert_eq!(
            frame.total - frame.done,
            construction::progress::worker_step(10)
        );
        if !enabled {
            construction::apply_factory_command(s.world_mut(), PlayerId(0), &repeat(None));
        }
        // Queue without advancing construction onto its final step.
        construction::apply_command(s.world_mut(), PlayerId(0), &build(MASON));
        let mana = economy::mana(s.world(), PlayerId(0)).0;
        lethal(&mut s, &mut host, p);
        s.step(&[]);
        assert_eq!(product(&s), None);
        assert_eq!(s.world().unit(FACTORY).unwrap().build.factory_repeat, None);
        assert!(queue(&s).is_empty());
        assert_eq!(s.world().unit(p).unwrap().build.frame, Some(frame));
        assert_eq!(economy::mana(s.world(), PlayerId(0)).0, mana);
        for _ in 0..35 {
            s.step(&[]);
        }
        assert_eq!(product(&s), None);
        assert_eq!(s.world().unit(p).unwrap().build.frame, Some(frame));
        host.0.set(true);
        s.step(&[]);
        assert!(s.world().unit(p).is_none());
        assert_eq!(economy::mana(s.world(), PlayerId(0)).0, mana);
    }
}

#[test]
fn chain_death_removes_higher_slot_factory_without_orphaning_active_repeat() {
    let mut sc = scenario();
    // Enemy factory (slot 1) is killed by slot 0's death explosion and then
    // removed later in that very same progress_death loop (no Dying script).
    sc.unit_defs[KEEP.0 as usize].combat.max_damage = 100;
    sc.unit_defs[KING.0 as usize].economy.mogriumincome = 0;
    sc.victory.defeated_units = tak_sim::DefeatedUnits::Remain;
    sc.unit_defs[KEEP.0 as usize].combat.explode_as = Some(tak_sim::combat::WeaponStats {
        name: "synthetic chain death".into(),
        kind: tak_sim::combat::WeaponKind::Melee,
        range: 2000,
        min_range: 0,
        reload_ticks: 1,
        velocity: 0,
        area_of_effect: 2000,
        edge_effectiveness: tak_math::Fx::ONE,
        aim_tolerance: None,
        mana_per_shot: 0,
        damage_kind: tak_sim::combat::kind::NORMAL,
        damage: tak_sim::combat::DamageStats {
            default: 1000,
            multipliers: vec![],
        },
    });
    let mut s = Sim::new(&sc).unwrap();
    s.step(&[]);
    // Preserve storage after both factories die, without adding income.
    s.world_mut()
        .spawn(KING, PlayerId(1), at(100, 900), Angle::ZERO)
        .unwrap();
    let command = tak_commands::LoggedCommand {
        tick: s.tick(),
        player: PlayerId(1),
        seq: 0,
        command: SimCommand::SetFactoryRepeat {
            units: UnitSet::single(ENEMY_FACTORY),
            def: Some(SWORD),
        },
    };
    s.step(&[command]);
    let p = s
        .world()
        .unit(ENEMY_FACTORY)
        .unwrap()
        .build
        .factory
        .product
        .unwrap();
    let paid = s.world().unit(p).unwrap().build.frame.unwrap().paid;
    let mana = economy::mana(s.world(), PlayerId(1)).0;
    lethal(&mut s, &mut tak_sim::NoScripts, FACTORY);
    s.step(&[]);
    assert!(s.world().unit(FACTORY).is_none());
    assert!(s.world().unit(ENEMY_FACTORY).is_none());
    assert!(
        s.world().unit(p).is_none(),
        "same-pass chain death must not orphan repeat frame"
    );
    // Combat happened after economy; the exact refund is credited next tick.
    s.step(&[]);
    assert_eq!(economy::mana(s.world(), PlayerId(1)).0, mana + paid);
}
