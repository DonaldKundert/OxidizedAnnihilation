//! All fixtures are synthetic. The controller sees only an immutable World and
//! its generated ordinary commands are the sole simulation input.
use tak_commands::{
    CommandLog, LoggedCommand, PlayerId, SimCommand, Target, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_headless::bot::{ControllerConfig, Recipe, committed_count, demo};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_math::{Angle, Vec2};
use tak_sim::{MatchState, Order, Sim};

fn driven(direct: bool) -> ReplaySession {
    let scenario = demo::scenario(direct);
    let world = tak_sim::World::new(&scenario).unwrap();
    ReplaySession::new_with_controllers(
        CheckpointSource::synthetic(scenario),
        None,
        demo::config(&world, direct).unwrap(),
    )
    .unwrap()
}
fn empty() -> CommandLog {
    CommandLog::new()
}

#[test]
fn both_recipes_really_pay_build_and_defeat_without_spawns_or_income_cheats() {
    for direct in [false, true] {
        let mut session = driven(direct);
        session.advance_driven(&empty(), 1).unwrap();
        let first_mana = tak_sim::economy::mana(session.sim().world(), PlayerId(0)).0;
        assert_eq!(first_mana, 100 << 16);
        session.advance_driven(&empty(), 20).unwrap();
        let world = session.sim().world();
        assert_eq!(committed_count(world, PlayerId(0), UnitDefId(1)), 1);
        assert!(tak_sim::economy::income_per_second(world, PlayerId(0)).0 > 0);
        assert!(tak_sim::economy::mana(world, PlayerId(0)).0 < first_mana);
        assert_eq!(committed_count(world, PlayerId(0), UnitDefId(3)), 0);
        session.advance_driven(&empty(), 1179).unwrap();
        assert!(
            matches!(session.sim().world().victory().state(), MatchState::Over { winners } if winners == &[PlayerId(0)])
        );
        let history = session.executed_log();
        assert!(history.iter().any(|entry| matches!(
            entry.command,
            SimCommand::Build {
                def: UnitDefId(1),
                ..
            }
        )));
        assert_eq!(
            history.iter().any(|entry| matches!(
                entry.command,
                SimCommand::Build {
                    def: UnitDefId(2),
                    at: Some(_),
                    ..
                }
            )),
            !direct
        );
        assert!(
            history.iter().any(|entry| matches!(
                entry.command,
                SimCommand::Build {
                    def: UnitDefId(3),
                    at: Some(_),
                    ..
                }
            )) == direct
        );
        assert!(
            history.iter().any(|entry| matches!(
                entry.command,
                SimCommand::Build {
                    def: UnitDefId(3),
                    at: None,
                    ..
                }
            )) != direct
        );
        assert!(history.iter().any(|entry| matches!(
            entry.command,
            SimCommand::Attack {
                target: Target::Unit(_),
                ..
            }
        )));
        assert!(
            history
                .iter()
                .all(|entry| entry.tick.0 >= 1 && (entry.tick.0 - 1) % 30 == 0)
        );
        assert!(
            history
                .iter()
                .all(|entry| entry.seq < 8 && entry.player == PlayerId(0))
        );
        let count = history.len();
        let end = session.sim().tick();
        assert!(
            session
                .controllers()
                .plan(session.sim().world())
                .unwrap()
                .is_empty()
        );
        session.advance_driven(&empty(), 30).unwrap();
        assert_eq!(count, session.executed_log().len());
        assert_eq!(session.sim().tick(), Tick(end.0 + 30));
    }
}

#[test]
fn recorded_commands_replay_with_planner_disabled_and_shared_client_loop_agrees() {
    let mut session = driven(false);
    let expected = session.advance_driven(&empty(), 1200).unwrap();
    let source = session.checkpoint().unwrap().source().clone();
    let mut replay = ReplaySession::new(source, None).unwrap();
    assert_eq!(
        replay.advance(session.executed_log(), 1200).unwrap(),
        expected
    );
    assert!(replay.controllers().is_empty());
    let scenario = demo::scenario(false);
    let mut client = Sim::new(&scenario).unwrap();
    let config = demo::config(client.world(), false).unwrap();
    let mut realized = CommandLog::new();
    let mut actual = Vec::new();
    for _ in 0..1200 {
        let commands = config.plan(client.world()).unwrap();
        for entry in &commands {
            realized.insert(entry.clone()).unwrap();
        }
        actual.push(client.step(&commands));
    }
    assert_eq!(expected, actual);
    assert_eq!(session.executed_log(), &realized);
}

#[test]
fn resume_at_zero_decision_and_construction_boundaries_does_not_repeat_history() {
    for direct in [false, true] {
        let mut full = driven(direct);
        let hashes = full.advance_driven(&empty(), 1200).unwrap();
        for boundary in [0, 1, 2, 6, 11, 30, 31, 32, 90, 91, 92, 121, 200] {
            let mut split = driven(direct);
            split.advance_driven(&empty(), boundary).unwrap();
            let checkpoint = split.checkpoint().unwrap();
            let history = split.executed_log().clone();
            let text = checkpoint.to_json_pretty().unwrap();
            assert!(text.contains("simple-skirmish-v1"));
            assert!(text.contains("omniscient-training-v1"));
            let mut restored = Checkpoint::from_json(&text)
                .unwrap()
                .restore(None, ReplayBudget::default())
                .unwrap();
            assert_eq!(restored.controllers(), full.controllers());
            assert_eq!(restored.executed_log(), &history);
            assert_eq!(
                restored.advance_driven(&empty(), 1200 - boundary).unwrap(),
                hashes[boundary as usize..]
            );
            assert_eq!(restored.executed_log(), full.executed_log());
        }
    }
}

#[test]
fn defaults_preserve_payload_shape_checksum_and_old_hashes() {
    let scenario = tak_sim::testing::example_scenario(1, 2);
    let source = CheckpointSource::synthetic(scenario.clone());
    let mut default = ReplaySession::new(source.clone(), None).unwrap();
    let mut explicit_empty =
        ReplaySession::new_with_controllers(source, None, ControllerConfig::default()).unwrap();
    let log = tak_sim::testing::walk_log(&scenario, 90, 11, 2);
    assert_eq!(
        default.advance(&log, 90).unwrap(),
        explicit_empty.advance_driven(&log, 90).unwrap()
    );
    assert_eq!(
        default.checkpoint().unwrap().to_json_pretty().unwrap(),
        explicit_empty
            .checkpoint()
            .unwrap()
            .to_json_pretty()
            .unwrap()
    );
    let text = default.checkpoint().unwrap().to_json_pretty().unwrap();
    assert!(!text.contains("controllers"));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    #[derive(serde::Serialize)]
    struct LegacyPayload<'a> {
        source: &'a CheckpointSource,
        content_identity: &'a str,
        next_tick: u32,
        state_hash: &'a str,
        executed: &'a CommandLog,
    }
    let checkpoint = default.checkpoint().unwrap();
    let payload = LegacyPayload {
        source: checkpoint.source(),
        content_identity: checkpoint.content_identity(),
        next_tick: 90,
        state_hash: checkpoint.state_hash(),
        executed: default.executed_log(),
    };
    let checksum = format!(
        "{:016x}",
        xxhash_rust::xxh64::xxh64(&serde_json::to_vec(&payload).unwrap(), 0)
    );
    assert_eq!(value["checksum"].as_str(), Some(checksum.as_str()));
}

#[test]
fn config_changes_are_checksums_and_invalid_versions_recipes_owners_fail_closed() {
    let mut session = driven(false);
    session.advance_driven(&empty(), 30).unwrap();
    let checkpoint = session.checkpoint().unwrap();
    let original = checkpoint.to_json_pretty().unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&original).unwrap();
    value["payload"]["controllers"]["controllers"][0]["army_cap"] = 4.into();
    assert!(
        Checkpoint::from_json(&value.to_string())
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    let world = session.sim().world();
    let valid = session.controllers().clone();
    for case in 0..9 {
        let mut bad = valid.clone();
        match case {
            0 => bad.controllers[0].policy = "simple-skirmish-v2".into(),
            1 => bad.controllers[0].player = PlayerId(99),
            2 => bad.human_players = vec![PlayerId(0)],
            3 => bad.controllers.push(bad.controllers[0].clone()),
            4 => bad.controllers[0].recipe.troop.name = "wrong identity".into(),
            5 => bad.controllers[0].recipe.troop.id = UnitDefId(0),
            6 => bad.controllers[0].site_budget = 257,
            7 => bad.controllers[0].cadence_ticks = 0,
            _ => bad.controllers[0].command_budget = 33,
        }
        assert!(bad.validate(world).is_err(), "case {case}");
    }
    assert!(
        Recipe::resolve(
            world,
            "training_builder",
            Some("training_income"),
            Some("training_income"),
            "training_troop"
        )
        .is_err()
    );
    let mut scenario = demo::scenario(false);
    scenario.unit_defs[2].build.canbuild.clear();
    assert!(demo::config(&tak_sim::World::new(&scenario).unwrap(), false).is_err());
    let campaign = CheckpointSource::campaign(
        "mmap/example",
        &[
            tak_sim::campaign::CampaignPlayer {
                ota_slot: 1,
                side: "ARA".into(),
            },
            tak_sim::campaign::CampaignPlayer {
                ota_slot: 2,
                side: "ZON".into(),
            },
        ],
        1,
        false,
        false,
    );
    let error = match ReplaySession::new_with_controllers(campaign, None, valid) {
        Ok(_) => panic!("campaign accepted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("campaign"));
}

#[test]
fn competing_input_changed_history_and_command_budgets_reject_atomically() {
    let mut session = driven(false);
    session.advance_driven(&empty(), 32).unwrap();
    let before = session.checkpoint().unwrap().to_json_pretty().unwrap();
    let mut competing = CommandLog::new();
    competing.record(
        Tick(100),
        PlayerId(0),
        SimCommand::Stop {
            units: UnitSet::single(UnitId::new(0, 0)),
        },
    );
    assert!(session.advance_driven(&competing, 1).is_err()); // even later supplied competing control
    let mut changed = session.executed_log().clone();
    let first = changed.entries()[0].clone();
    let mut changed_entry = first;
    changed_entry.command = SimCommand::Stop {
        units: UnitSet::new(),
    };
    let mut bad_past = CommandLog::new();
    bad_past.insert(changed_entry).unwrap();
    assert!(session.advance_driven(&bad_past, 1).is_err());
    assert!(
        session
            .advance_driven(&empty(), tak_headless::MAX_TICKS)
            .is_err()
    );
    let mut greedy = session.controllers().clone();
    greedy.controllers[0].cadence_ticks = 1;
    greedy.controllers[0].command_budget = 32;
    let source = session.checkpoint().unwrap().source().clone();
    let mut greedy = ReplaySession::new_with_controllers(source, None, greedy).unwrap();
    assert!(greedy.advance_driven(&empty(), 100_000).is_err());
    assert_eq!(greedy.sim().tick(), Tick(0));
    assert_eq!(
        session.checkpoint().unwrap().to_json_pretty().unwrap(),
        before
    );
    // Simulate the public record() saturation pitfall and reject before any tick.
    changed = empty();
    changed
        .insert(LoggedCommand {
            tick: Tick(40),
            player: PlayerId(1),
            seq: u32::MAX,
            command: SimCommand::Stop {
                units: UnitSet::new(),
            },
        })
        .unwrap();
    changed.record(
        Tick(40),
        PlayerId(1),
        SimCommand::Stop {
            units: UnitSet::new(),
        },
    );
    assert!(session.advance_driven(&changed, 20).is_err());
    assert_eq!(session.sim().tick(), Tick(32));
}

#[test]
fn cap_counts_frames_active_jobs_and_unmaterialized_queued_orders_once() {
    let mut sim = Sim::new(&demo::scenario(false)).unwrap();
    sim.step(&[]);
    let world = sim.world_mut();
    let owner = PlayerId(0);
    let root = UnitId::new(0, 0);
    let frame = world
        .spawn(UnitDefId(3), owner, Vec2::from_ints(200, 160), Angle::ZERO)
        .unwrap();
    world.unit_mut(frame).unwrap().build.frame = Some(tak_sim::construction::Frame::new(100));
    let unit = world.unit_mut(root).unwrap();
    unit.build.job = Some(tak_sim::construction::BuilderJob {
        def: UnitDefId(3),
        site: Vec2::from_ints(200, 160),
        target: Some(frame),
        walking: false,
        building: true,
    });
    unit.orders.push_back(Order::Build {
        def: UnitDefId(3),
        at: Some(Vec2::from_ints(200, 160)),
    });
    unit.orders.push_back(Order::Build {
        def: UnitDefId(3),
        at: Some(Vec2::from_ints(220, 160)),
    });
    assert_eq!(committed_count(world, owner, UnitDefId(3)), 2);
    let factory = world
        .spawn(UnitDefId(2), owner, Vec2::from_ints(260, 160), Angle::ZERO)
        .unwrap();
    let product = world
        .spawn(UnitDefId(3), owner, Vec2::from_ints(260, 160), Angle::ZERO)
        .unwrap();
    world.unit_mut(product).unwrap().build.frame = Some(tak_sim::construction::Frame::new(100));
    let factory = world.unit_mut(factory).unwrap();
    factory.build.factory.product = Some(product);
    factory.orders.push_back(Order::Build {
        def: UnitDefId(3),
        at: None,
    });
    factory.orders.push_back(Order::Build {
        def: UnitDefId(3),
        at: None,
    });
    assert_eq!(committed_count(world, owner, UnitDefId(3)), 4);
}

#[test]
fn unaffordable_blocked_occupied_sacred_and_busy_cases_are_bounded_and_keep_orders() {
    for direct in [false, true] {
        let mut scenario = demo::scenario(direct);
        scenario.economy.rules.start_mana = 0;
        let world = tak_sim::World::new(&scenario).unwrap();
        let config = demo::config(&world, direct).unwrap();
        let mut session = ReplaySession::new_with_controllers(
            CheckpointSource::synthetic(scenario),
            None,
            config,
        )
        .unwrap();
        session.advance_driven(&empty(), 600).unwrap();
        assert!(session.executed_log().is_empty());
        assert_eq!(session.sim().world().unit_count(), 2);
    }
    let mut scenario = demo::scenario(false);
    scenario.unit_defs[1].economy.sacred_site = true;
    let mut sim = Sim::new(&scenario).unwrap();
    sim.step(&[]);
    let config = demo::config(sim.world(), false).unwrap();
    assert!(config.plan(sim.world()).unwrap().is_empty());
    // One nearest candidate is illegal at the map's upper edge. No unbounded
    // widening or substitute placement is attempted when the budget is one.
    let mut sim = Sim::new(&demo::scenario(false)).unwrap();
    sim.step(&[]);
    sim.world_mut().unit_mut(UnitId::new(0, 0)).unwrap().pos = Vec2::from_ints(32, 32);
    let mut config = demo::config(sim.world(), false).unwrap();
    config.controllers[0].site_budget = 1;
    assert!(config.plan(sim.world()).unwrap().is_empty());
    // A busy root's human-authored order is not replaced by a build command.
    sim.world_mut()
        .unit_mut(UnitId::new(0, 0))
        .unwrap()
        .orders
        .push_back(Order::Move {
            to: Vec2::from_ints(200, 200),
        });
    let before = sim.world().clone();
    assert!(config.plan(sim.world()).unwrap().is_empty());
    assert_eq!(&before, sim.world());
}

#[test]
fn targets_use_stable_distance_hostility_and_live_generational_handles() {
    let mut scenario = demo::scenario(true);
    scenario.players.push(tak_sim::PlayerSpec {
        start: 1,
        monarch: UnitDefId(4),
    });
    scenario.victory.teams = vec![Some(1), Some(1), Some(2)];
    let mut sim = Sim::new(&scenario).unwrap();
    sim.step(&[]);
    let mut config = demo::config(sim.world(), true).unwrap();
    config.controllers[0].recipe.income = None;
    let world = sim.world_mut();
    let troop = world
        .spawn(
            UnitDefId(3),
            PlayerId(0),
            Vec2::from_ints(200, 160),
            Angle::ZERO,
        )
        .unwrap();
    let near = world
        .spawn(
            UnitDefId(4),
            PlayerId(2),
            Vec2::from_ints(300, 160),
            Angle::ZERO,
        )
        .unwrap();
    let equal = world
        .spawn(
            UnitDefId(4),
            PlayerId(2),
            Vec2::from_ints(100, 160),
            Angle::ZERO,
        )
        .unwrap();
    let allied = world
        .spawn(
            UnitDefId(4),
            PlayerId(1),
            Vec2::from_ints(210, 160),
            Angle::ZERO,
        )
        .unwrap();
    // Ordinary tick initializes synthetic units' combat state.
    sim.step(&[]);
    for _ in 0..29 {
        sim.step(&[]);
    }
    sim.world_mut().unit_mut(troop).unwrap().pos = Vec2::from_ints(200, 160);
    let commands = config.plan(sim.world()).unwrap();
    let target = commands
        .iter()
        .find_map(|entry| match &entry.command {
            SimCommand::Attack {
                units,
                target: Target::Unit(target),
                ..
            } if units.iter().any(|id| *id == troop) => Some(*target),
            _ => None,
        })
        .unwrap();
    assert_eq!(target, near.min(equal));
    assert_ne!(target, allied);
    let world = sim.world_mut();
    world.despawn(near);
    let reused = world
        .spawn(
            UnitDefId(4),
            PlayerId(1),
            Vec2::from_ints(300, 160),
            Angle::ZERO,
        )
        .unwrap();
    assert_eq!(near.index, reused.index);
    assert_ne!(near.generation, reused.generation);
    let commands = config.plan(world).unwrap();
    assert!(commands.iter().all(|entry| !matches!(entry.command, SimCommand::Attack { target: Target::Unit(target), .. } if target == near || target == reused || target == allied)));
}

#[test]
fn blocked_terrain_and_occupied_sacred_sites_never_issue_illegal_builds() {
    let mut scenario = demo::scenario(false);
    scenario.map.terrain = Some(tak_sim::TerrainGrid {
        cols: 40,
        rows: 20,
        cell_size: 16,
        flags: vec![tak_sim::map::terrain::STEEP; 800],
    });
    let mut sim = Sim::new(&scenario).unwrap();
    sim.step(&[]);
    let config = demo::config(sim.world(), false).unwrap();
    for _ in 0..300 {
        let commands = config.plan(sim.world()).unwrap();
        assert!(commands.is_empty());
        sim.step(&commands);
    }
    let mut scenario = demo::scenario(false);
    scenario.unit_defs[1].economy.sacred_site = true;
    scenario
        .economy
        .sacred_sites
        .push(tak_sim::economy::SacredSite {
            x: 160,
            y: 128,
            w: 32,
            h: 32,
            multiplier: tak_math::Fx::ONE,
        });
    let mut sim = Sim::new(&scenario).unwrap();
    sim.step(&[]);
    let config = demo::config(sim.world(), false).unwrap();
    let commands = config.plan(sim.world()).unwrap();
    let site = commands
        .iter()
        .find_map(|entry| match entry.command {
            SimCommand::Build { at: Some(site), .. } => Some(site),
            _ => None,
        })
        .unwrap();
    assert!(tak_sim::economy::free_site_at(sim.world(), site, None).is_some());
    assert!(
        tak_sim::construction::placement::check_site(sim.world(), UnitDefId(1), site, None).is_ok()
    );
    sim.world_mut()
        .spawn(UnitDefId(1), PlayerId(1), site, Angle::ZERO)
        .unwrap();
    assert!(config.plan(sim.world()).unwrap().is_empty());
}

#[test]
fn blocked_factory_exit_retains_single_pending_order_without_reissuing_or_overrunning_cap() {
    let mut sim = Sim::new(&demo::scenario(false)).unwrap();
    sim.step(&[]);
    let mut config = demo::config(sim.world(), false).unwrap();
    config.controllers[0].recipe.income = None;
    config.controllers[0].army_cap = 1;
    // Factory and stationary blocker are part of the synthetic test setup,
    // never actions available to the planner/driver.
    let factory = sim
        .world_mut()
        .spawn(
            UnitDefId(2),
            PlayerId(0),
            Vec2::from_ints(256, 160),
            Angle::ZERO,
        )
        .unwrap();
    sim.world_mut()
        .spawn(
            UnitDefId(4),
            PlayerId(1),
            Vec2::from_ints(256, 160),
            Angle::ZERO,
        )
        .unwrap();
    let commands = config.plan(sim.world()).unwrap();
    assert!(commands.iter().any(|entry| matches!(
        entry.command,
        SimCommand::Build {
            def: UnitDefId(3),
            at: None,
            ..
        }
    )));
    sim.step(&commands);
    for _ in 0..300 {
        let commands = config.plan(sim.world()).unwrap();
        assert!(
            !commands
                .iter()
                .any(|entry| matches!(entry.command, SimCommand::Build { .. }))
        );
        sim.step(&commands);
    }
    assert_eq!(committed_count(sim.world(), PlayerId(0), UnitDefId(3)), 1);
    let unit = sim.world().unit(factory).unwrap();
    assert!(unit.build.factory.product.is_none());
    assert!(unit.build.factory.retry_at.is_some());
    assert_eq!(unit.orders.len(), 1);
}

#[test]
fn defeated_owner_and_dead_builder_never_generate_more_commands() {
    let mut scenario = demo::scenario(false);
    scenario.players.push(tak_sim::PlayerSpec {
        start: 1,
        monarch: UnitDefId(4),
    });
    let mut sim = Sim::new(&scenario).unwrap();
    sim.step(&[]);
    let config = demo::config(sim.world(), false).unwrap();
    sim.world_mut().despawn(UnitId::new(0, 0));
    sim.step(&[]);
    assert!(sim.world().is_defeated(PlayerId(0)));
    assert!(matches!(sim.world().victory().state(), MatchState::Running));
    for _ in 0..100 {
        assert!(config.plan(sim.world()).unwrap().is_empty());
        sim.step(&[]);
    }
}

#[test]
fn installed_build_trees_validate_factory_and_direct_builder_recipes() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("skirmish_bot_recipes") else {
        return;
    };
    let game = tak_headless::load_map(&data, "kmap/Adamantine Gate", "ARA,ZON", 1).unwrap();
    let world = tak_sim::World::new(&game.scenario).unwrap();
    let ordinary = |def: &tak_sim::UnitDef| {
        !def.build.builder
            && def.build.mobile
            && def.maxvelocity > tak_math::Fx::ZERO
            && def.combat.max_damage > 0
            && def
                .combat
                .weapons
                .iter()
                .any(|weapon| !weapon.is_spell() && weapon.damage.default > 0)
    };
    let mut factory_recipes = 0;
    let mut direct_recipes = 0;
    for player in world.players() {
        let root = world.defs().get(player.monarch_def).unwrap();
        let income = root
            .build
            .canbuild
            .iter()
            .filter_map(|id| world.defs().get(*id))
            .find(|def| def.economy.mogriumincome > 0 && def.maxvelocity == tak_math::Fx::ZERO);
        let direct = root
            .build
            .canbuild
            .iter()
            .filter_map(|id| world.defs().get(*id))
            .find(|def| ordinary(def));
        if let Some(troop) = direct {
            Recipe::resolve(
                &world,
                &root.name,
                income.map(|def| def.name.as_str()),
                None,
                &troop.name,
            )
            .unwrap();
            direct_recipes += 1;
        }
        for producer in root
            .build
            .canbuild
            .iter()
            .filter_map(|id| world.defs().get(*id))
            .filter(|def| {
                def.build.builder
                    && !def.build.mobile
                    && def.build.workertime > 0
                    && def.maxvelocity == tak_math::Fx::ZERO
            })
        {
            if let Some(troop) = producer
                .build
                .canbuild
                .iter()
                .filter_map(|id| world.defs().get(*id))
                .find(|def| ordinary(def))
            {
                Recipe::resolve(
                    &world,
                    &root.name,
                    income.map(|def| def.name.as_str()),
                    Some(&producer.name),
                    &troop.name,
                )
                .unwrap();
                factory_recipes += 1;
            }
        }
    }
    assert!(factory_recipes > 0);
    assert!(direct_recipes > 0);
}

#[test]
fn existing_attack_is_retained_and_frames_dying_units_and_builders_are_not_attack_cohort() {
    let mut sim = Sim::new(&demo::scenario(true)).unwrap();
    sim.step(&[]);
    let mut config = demo::config(sim.world(), true).unwrap();
    config.controllers[0].recipe.income = None;
    config.controllers[0].army_cap = 1;
    let troop = sim
        .world_mut()
        .spawn(
            UnitDefId(3),
            PlayerId(0),
            Vec2::from_ints(200, 200),
            Angle::ZERO,
        )
        .unwrap();
    sim.step(&[]);
    for _ in 0..29 {
        sim.step(&[]);
    }
    let attack = Order::Attack {
        target: Target::Unit(UnitId::new(1, 0)),
    };
    sim.world_mut()
        .unit_mut(troop)
        .unwrap()
        .orders
        .push_back(attack);
    let before = sim.world().clone();
    assert!(config.plan(sim.world()).unwrap().is_empty());
    assert_eq!(&before, sim.world());
    sim.world_mut().unit_mut(troop).unwrap().orders.clear();
    sim.world_mut().unit_mut(troop).unwrap().build.frame =
        Some(tak_sim::construction::Frame::new(100));
    assert!(config.plan(sim.world()).unwrap().is_empty());
    let unit = sim.world_mut().unit_mut(troop).unwrap();
    unit.build.frame = None;
    unit.combat.dying = Some(tak_sim::combat::Dying {
        since: 31,
        severity: 100,
        kind: 1,
        done: false,
    });
    assert!(config.plan(sim.world()).unwrap().is_empty());
    // Recipe validation itself rejects putting the root builder (or monarch
    // definitions) into the ordinary attack cohort.
    let root = config.controllers[0].recipe.builder.clone();
    config.controllers[0].recipe.troop = root;
    assert!(config.plan(sim.world()).is_err());
}
