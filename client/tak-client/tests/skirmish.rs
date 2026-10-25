//! Asset-free contracts plus opt-in installed-content lifecycle checks.
use tak_client::skirmish::{self, Faction, Opponent, SetupConfig, SkirmishMap};
use tak_commands::{CommandLog, PlayerId, SimCommand, UnitDefId};
use tak_headless::bot::demo;
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};
use tak_sim::{MatchState, Scenario, World};

fn scenario(faction: Faction) -> Scenario {
    let mut scenario = demo::scenario(faction == Faction::Zhon);
    // The shared synthetic fixture controls zero. This client always controls
    // one, so swap only the authored player definitions, not any sim rules.
    scenario.players.swap(0, 1);
    scenario
}

#[test]
fn bounded_choices_roundtrip_and_reject_unknown_fields_and_variants() {
    let config = SetupConfig::default();
    assert_eq!(config.map, SkirmishMap::AdamantineGate);
    assert_eq!(config.faction, Faction::Aramon);
    assert_eq!(config.opponent, Opponent::Training);
    assert_eq!(config.players(), "ARA,ZON");
    assert_eq!(Faction::Zhon.opposite(), Faction::Aramon);
    assert_eq!(Opponent::Training.next().next(), Opponent::Training);
    assert!(Opponent::Training.disclosure().contains("omniscient"));
    assert!(
        Opponent::Training
            .disclosure()
            .contains("Not original TA:K AI")
    );
    let json = serde_json::to_value(config).unwrap();
    assert_eq!(
        serde_json::from_value::<SetupConfig>(json.clone()).unwrap(),
        config
    );
    for (key, value) in [
        ("map", "unavailable map"),
        ("faction", "TAR"),
        ("opponent", "retail AI"),
        ("extra", "ignored"),
    ] {
        let mut changed = json.clone();
        changed[key] = value.into();
        assert!(serde_json::from_value::<SetupConfig>(changed).is_err());
    }
}

#[test]
fn recipes_protect_player_zero_validate_roles_and_reject_unsupported_content() {
    for faction in [Faction::Aramon, Faction::Zhon] {
        let source = scenario(faction);
        let world = World::new(&source).unwrap();
        let config = skirmish::training_config(&world, faction).unwrap();
        assert_eq!(config.human_players, vec![PlayerId(0)]);
        assert!(!config.controls(PlayerId(0)));
        assert!(config.controls(PlayerId(1)));
        assert_eq!(
            config.controllers[0].recipe.producer.is_some(),
            faction == Faction::Aramon
        );
        assert!(skirmish::training_config(&world, faction.opposite()).is_err());
        let mut missing_menu = source.clone();
        missing_menu.unit_defs[0].build.canbuild.clear();
        assert!(skirmish::training_config(&World::new(&missing_menu).unwrap(), faction).is_err());
        let mut unarmed = source;
        unarmed.unit_defs[3].combat.weapons.clear();
        assert!(skirmish::training_config(&World::new(&unarmed).unwrap(), faction).is_err());
    }
    assert!(
        skirmish::training_config(
            &World::new(&tak_sim::testing::example_scenario(1, 1)).unwrap(),
            Faction::Aramon
        )
        .is_err()
    );
}

#[test]
fn built_in_factory_and_direct_presets_build_attack_finish_and_replay_without_cheats() {
    for faction in [Faction::Aramon, Faction::Zhon] {
        let scenario = scenario(faction);
        let world = World::new(&scenario).unwrap();
        let config = skirmish::training_config(&world, faction).unwrap();
        let source = CheckpointSource::synthetic(scenario);
        let mut live =
            ReplaySession::new_with_controllers(source.clone(), None, config.clone()).unwrap();
        let mut hashes = live.advance_driven(&CommandLog::new(), 32).unwrap();
        let checkpoint =
            Checkpoint::from_json(&live.checkpoint().unwrap().to_json_pretty().unwrap()).unwrap();
        let mut resumed = checkpoint.restore(None, ReplayBudget::default()).unwrap();
        let suffix = live.advance_driven(&CommandLog::new(), 1168).unwrap();
        assert_eq!(
            resumed.advance_driven(&CommandLog::new(), 1168).unwrap(),
            suffix
        );
        hashes.extend(suffix);
        assert_eq!(resumed.executed_log(), live.executed_log());
        assert_eq!(resumed.controllers(), &config);
        assert!(
            matches!(live.sim().world().victory().state(), MatchState::Over { winners } if winners == &[PlayerId(1)])
        );
        assert!(
            live.executed_log()
                .iter()
                .all(|entry| entry.player == PlayerId(1))
        );
        assert!(live.executed_log().iter().any(|entry| matches!(
            entry.command,
            SimCommand::Build {
                def: UnitDefId(1),
                ..
            }
        )));
        assert_eq!(
            live.executed_log().iter().any(|entry| matches!(
                entry.command,
                SimCommand::Build {
                    def: UnitDefId(2),
                    ..
                }
            )),
            faction == Faction::Aramon
        );
        assert!(live.executed_log().iter().any(|entry| matches!(
            entry.command,
            SimCommand::Build {
                def: UnitDefId(3),
                ..
            }
        )));
        assert!(
            live.executed_log()
                .iter()
                .any(|entry| matches!(entry.command, SimCommand::Attack { .. }))
        );
        let mut raw = ReplaySession::new(source.clone(), None).unwrap();
        assert_eq!(raw.advance(live.executed_log(), 1200).unwrap(), hashes);
        let mut rematch = ReplaySession::new_with_controllers(source, None, config).unwrap();
        assert_eq!(
            rematch.advance_driven(&CommandLog::new(), 1200).unwrap(),
            hashes
        );
    }
}

#[test]
fn missing_install_is_a_recoverable_setup_error() {
    let path = std::env::temp_dir().join(format!(
        "tak-missing-skirmish-install-{}",
        std::process::id()
    ));
    assert!(skirmish::prepare_session(&path, &SetupConfig::default()).is_err());
}

#[test]
fn installed_both_factions_build_train_attack_save_replay_and_rematch() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client_skirmish") else {
        return;
    };
    for faction in [Faction::Aramon, Faction::Zhon] {
        let setup = SetupConfig {
            faction,
            ..Default::default()
        };
        let mut live = skirmish::prepare_session(&data, &setup).unwrap();
        assert!(live.sim.cob().is_some());
        assert_eq!(live.local, PlayerId(0));
        assert_eq!(skirmish::from_session(&live), Some(setup));
        let recipe = live.controllers().controllers[0].recipe.clone();
        for _ in 0..32 {
            live.try_step().unwrap();
        }
        let checkpoint = tak_client::saves::capture(&live).unwrap();
        let mut resumed = checkpoint
            .restore(Some(&data), ReplayBudget::default())
            .unwrap();
        let mut saw_attack = false;
        // Normal economy/construction and travel may take several game minutes.
        // A hard simulation-tick bound keeps broken content from hanging CI.
        for _ in 32..18_000 {
            assert_eq!(
                live.try_step().unwrap(),
                resumed.advance_driven(&CommandLog::new(), 1).unwrap()[0]
            );
            saw_attack = live
                .log
                .iter()
                .any(|entry| matches!(entry.command, SimCommand::Attack { .. }));
            if saw_attack {
                break;
            }
        }
        if !saw_attack {
            eprintln!(
                "stalled preset {:?}; mana {:?}",
                recipe,
                tak_sim::economy::mana(live.sim.world(), PlayerId(1))
            );
            for (id, unit) in live.sim.world().units() {
                eprintln!(
                    "unit {id:?} owner {:?}, def {}, built {:?}, job {:?}, orders {:?}",
                    unit.owner,
                    live.sim.world().defs().get(unit.def).unwrap().name,
                    unit.build.built_fraction(),
                    unit.build.job,
                    unit.orders
                );
            }
        }
        assert!(
            saw_attack,
            "{} opponent never attacked by tick {}",
            faction.opposite().label(),
            live.tick().0
        );
        assert!(live.log.iter().any(
            |entry| matches!(entry.command, SimCommand::Build { def, .. } if def == recipe.troop.id)
        ));
        assert!(
            live.sim
                .world()
                .units()
                .any(|(_, unit)| unit.owner == PlayerId(1)
                    && unit.def == recipe.troop.id
                    && unit.build.is_complete())
        );
        if let Some(income) = recipe.income {
            assert!(live.log.iter().any(
                |entry| matches!(entry.command, SimCommand::Build { def, .. } if def == income.id)
            ));
        }
        if let Some(producer) = recipe.producer {
            assert!(live.log.iter().any(
                |entry| matches!(entry.command, SimCommand::Build { def, .. } if def == producer.id)
            ));
        }
        assert_eq!(live.log, *resumed.executed_log());
        let final_save = tak_client::saves::capture(&live).unwrap();
        assert_eq!(final_save.controllers(), live.controllers());
        // Exercise the client checkpoint loader too, not just headless replay.
        let saved_path = std::env::temp_dir().join(format!(
            "tak-skirmish-client-resume-{}-{}.json",
            std::process::id(),
            faction.code()
        ));
        std::fs::write(&saved_path, final_save.to_json_pretty().unwrap()).unwrap();
        use clap::Parser;
        let args = tak_client::cli::Args::try_parse_from([
            std::ffi::OsStr::new("tak-client"),
            std::ffi::OsStr::new("--data"),
            data.as_os_str(),
            std::ffi::OsStr::new("--load-checkpoint"),
            saved_path.as_os_str(),
        ])
        .unwrap();
        let vfs = tak_client::game::open_vfs(&data).unwrap();
        let loaded = tak_client::game::Session::load_from_args(&vfs, &args, CommandLog::new());
        std::fs::remove_file(&saved_path).unwrap();
        let mut loaded = loaded.unwrap();
        assert_eq!(skirmish::from_session(&loaded), Some(setup));
        assert_eq!(loaded.controllers(), live.controllers());
        assert_eq!(loaded.sim.state_hash(), live.sim.state_hash());
        let next = resumed.advance_driven(&CommandLog::new(), 1).unwrap()[0];
        assert_eq!(loaded.try_step().unwrap(), next);
        let mut raw = ReplaySession::new(setup.checkpoint_source(), Some(&data)).unwrap();
        assert_eq!(raw.advance(&live.log, live.tick().0).unwrap(), live.hashes);
        let mut rematch =
            skirmish::prepare_session(&data, &skirmish::from_session(&live).unwrap()).unwrap();
        for expected in &live.hashes {
            assert_eq!(rematch.try_step().unwrap(), *expected);
        }
        eprintln!(
            "{} built-in opponent: scripted production, attack, save/resume/replay/rematch at {} ticks",
            faction.opposite().label(),
            live.tick().0
        );
    }
}

#[test]
fn installed_passive_practice_has_no_planner_and_preserves_replay_identity() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client_skirmish_passive") else {
        return;
    };
    let setup = SetupConfig {
        opponent: Opponent::PassivePractice,
        ..Default::default()
    };
    let mut session = skirmish::prepare_session_with_budget(
        &data,
        &setup,
        ReplayBudget {
            max_ticks: 90,
            max_commands: 100,
        },
    )
    .unwrap();
    assert!(session.controllers().controllers.is_empty());
    assert_eq!(session.controllers().human_players, vec![PlayerId(0)]);
    for _ in 0..90 {
        session.try_step().unwrap();
    }
    assert!(session.log.is_empty());
    assert_eq!(skirmish::from_session(&session), Some(setup));
    assert!(tak_client::saves::capture(&session).is_ok());
    session.try_step().unwrap();
    assert!(
        tak_client::saves::capture(&session)
            .unwrap_err()
            .to_string()
            .contains("replay ticks")
    );
}

#[test]
fn human_victory_is_replayable_and_fresh_rematch_has_no_prior_commands_or_units() {
    use tak_commands::{QueueMode, Target, Tick, UnitId, UnitSet};
    for faction in [Faction::Aramon, Faction::Zhon] {
        let mut scenario = scenario(faction);
        // A hand-authored armed human fixture, not runtime mutation or retail
        // balance changes. The human's normal attack can kill the bot monarch.
        let mut weapon = scenario.unit_defs[3].combat.weapons[0].clone();
        weapon.range = 1024;
        weapon.damage.default = 2000;
        scenario.unit_defs[4].combat.weapons = vec![weapon];
        let config = skirmish::training_config(&World::new(&scenario).unwrap(), faction).unwrap();
        let source = CheckpointSource::synthetic(scenario);
        let mut live =
            ReplaySession::new_with_controllers(source.clone(), None, config.clone()).unwrap();
        let initial_hash = live.sim().state_hash();
        let mut human = CommandLog::new();
        human.record(
            Tick(1),
            PlayerId(0),
            SimCommand::Attack {
                units: UnitSet::single(UnitId::new(0, 0)),
                target: Target::Unit(UnitId::new(1, 0)),
                queue: QueueMode::Replace,
            },
        );
        let hashes = live.advance_driven(&human, 300).unwrap();
        assert!(
            matches!(live.sim().world().victory().state(), MatchState::Over { winners } if winners == &[PlayerId(0)])
        );
        let mut raw = ReplaySession::new(source.clone(), None).unwrap();
        assert_eq!(raw.advance(live.executed_log(), 300).unwrap(), hashes);
        let mut rematch =
            ReplaySession::new_with_controllers(source, None, config.clone()).unwrap();
        assert_eq!(rematch.sim().state_hash(), initial_hash);
        assert_eq!(rematch.sim().tick(), Tick(0));
        assert!(rematch.executed_log().is_empty());
        assert_eq!(rematch.sim().world().units().count(), 0);
        assert_eq!(rematch.controllers(), &config);
        assert_eq!(rematch.advance_driven(&human, 300).unwrap(), hashes);
    }
}

#[test]
fn installed_custom_controller_is_not_silently_replaced_on_rematch() {
    use clap::Parser;
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client_skirmish_custom") else {
        return;
    };
    let session = skirmish::prepare_session(&data, &SetupConfig::default()).unwrap();
    let mut custom = session.controllers().clone();
    custom.controllers[0].army_cap += 1;
    let config_path =
        std::env::temp_dir().join(format!("tak-skirmish-custom-{}.json", std::process::id()));
    std::fs::write(&config_path, serde_json::to_vec(&custom).unwrap()).unwrap();
    let args = tak_client::cli::Args::try_parse_from([
        std::ffi::OsStr::new("tak-client"),
        std::ffi::OsStr::new("--data"),
        data.as_os_str(),
        std::ffi::OsStr::new("--controller-config"),
        config_path.as_os_str(),
    ])
    .unwrap();
    let vfs = tak_client::game::open_vfs(&data).unwrap();
    let loaded = tak_client::game::Session::load_from_args(&vfs, &args, CommandLog::new());
    std::fs::remove_file(config_path).unwrap();
    let loaded = loaded.unwrap();
    assert_eq!(loaded.controllers(), &custom);
    assert_eq!(skirmish::from_session(&loaded), None);
}

/// Longer real-content acceptance exercise, explicitly selected rather than
/// adding potentially minutes of replay to the ordinary asset-free test suite.
#[test]
#[ignore = "long owned-data result lifecycle; run explicitly with TAK_DATA_DIR"]
fn installed_ordinary_production_reaches_actual_results_without_world_edits() {
    use tak_headless::bot::Recipe;
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client_skirmish_results") else {
        return;
    };
    let mut failures = Vec::new();
    for faction in [Faction::Aramon, Faction::Zhon] {
        for scripted_human in [false, true] {
            let setup = SetupConfig {
                faction,
                opponent: if scripted_human {
                    Opponent::PassivePractice
                } else {
                    Opponent::Training
                },
                ..Default::default()
            };
            let mut session = skirmish::prepare_session(&data, &setup).unwrap();
            let human_driver = if scripted_human {
                // Resolve by existing definition names, because the source's
                // player/build-tree order may assign different numeric IDs.
                let other = skirmish::prepare_session(
                    &data,
                    &SetupConfig {
                        faction: faction.opposite(),
                        ..Default::default()
                    },
                )
                .unwrap();
                let mut config = other.controllers().clone();
                let recipe = &config.controllers[0].recipe;
                let resolved = Recipe::resolve(
                    session.sim.world(),
                    &recipe.builder.name,
                    recipe.income.as_ref().map(|def| def.name.as_str()),
                    recipe.producer.as_ref().map(|def| def.name.as_str()),
                    &recipe.troop.name,
                )
                .unwrap();
                config.human_players = vec![PlayerId(1)];
                config.controllers[0].player = PlayerId(0);
                config.controllers[0].recipe = resolved;
                config.validate(session.sim.world()).unwrap();
                Some(config)
            } else {
                None
            };
            let began = std::time::Instant::now();
            for _ in 0..54_000 {
                if let Some(config) = &human_driver {
                    for order in config.plan(session.sim.world()).unwrap() {
                        session.submit(order.command);
                    }
                }
                session.try_step().unwrap();
                if !matches!(session.sim.world().victory().state(), MatchState::Running) {
                    break;
                }
                if session.tick().0.is_multiple_of(3000) {
                    eprintln!(
                        "owned result {} human_scripted={scripted_human}: tick {}, units {}, commands {}, wall {:?}",
                        faction.label(),
                        session.tick().0,
                        session.sim.world().units().count(),
                        session.log.len(),
                        began.elapsed()
                    );
                }
            }
            eprintln!(
                "owned result {} human_scripted={scripted_human}: {:?} at tick {}, wall {:?}",
                faction.label(),
                session.sim.world().victory().state(),
                session.tick().0,
                began.elapsed()
            );
            let expected = if scripted_human {
                PlayerId(0)
            } else {
                PlayerId(1)
            };
            if !matches!(session.sim.world().victory().state(), MatchState::Over { winners } if winners == &[expected])
            {
                failures.push(format!(
                    "{} human_scripted={scripted_human}: {:?}",
                    faction.label(),
                    session.sim.world().victory().state()
                ));
            }
            // End-state save and independent raw-command replay must agree,
            // including an unfinished/bounded diagnostic run.
            let saved = tak_client::saves::capture(&session).unwrap();
            let restored = saved.restore(Some(&data), ReplayBudget::default()).unwrap();
            assert_eq!(restored.sim().state_hash(), session.sim.state_hash());
            if let Some(directory) = std::env::var_os("TAK_SKIRMISH_RESULT_DIR") {
                // Optional private QA artifacts: only descriptors and ordinary
                // commands, never copied install assets. The extra replay is
                // historical; it does not invoke or alter opponent planning.
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                let stem = format!(
                    "{}-{}",
                    faction.code(),
                    if scripted_human {
                        "scripted-human-victory"
                    } else {
                        "training-defeat"
                    }
                );
                let near_tick = session.tick().0.saturating_sub(60);
                let mut near = ReplaySession::new_with_controllers(
                    setup.checkpoint_source(),
                    Some(&data),
                    session.controllers().clone(),
                )
                .unwrap();
                near.advance(&session.log, near_tick).unwrap();
                near.checkpoint()
                    .unwrap()
                    .write_atomic(&directory.join(format!("{stem}.save.json")))
                    .unwrap();
                std::fs::write(
                    directory.join(format!("{stem}.commands.json")),
                    session.log.to_json_pretty(),
                )
                .unwrap();
                std::fs::write(
                    directory.join(format!("{stem}.setup.json")),
                    serde_json::to_vec_pretty(&setup).unwrap(),
                )
                .unwrap();
                std::fs::write(directory.join(format!("{stem}.txt")), format!("Generated by ordinary scripted gameplay through the shared command-only engine, not manual GUI gameplay.\nSource: {:?}\nCheckpoint tick: {near_tick}\nExpected result tick: {}\nExpected result: {:?}\nFor scripted-human victory, supply the matching commands log to retain any remaining human orders. Training defeat continues from its saved controller without a log.\n", setup, session.tick().0, session.sim.world().victory().state())).unwrap();
            }
        }
    }
    assert!(
        failures.is_empty(),
        "owned result limit: {}",
        failures.join("; ")
    );
}

#[test]
fn direct_preset_does_not_pick_costly_endgame_troop_just_because_it_is_first() {
    let mut source = scenario(Faction::Zhon);
    let mut expensive = source.unit_defs[3].clone();
    expensive.name = "training_expensive".into();
    expensive.build.buildcost = 100_000;
    source.unit_defs.push(expensive);
    source.unit_defs[0].build.canbuild.insert(0, UnitDefId(5));
    let world = World::new(&source).unwrap();
    let config = skirmish::training_config(&world, Faction::Zhon).unwrap();
    assert_eq!(config.controllers[0].recipe.troop.id, UnitDefId(3));
}

#[test]
fn mobile_intermediary_is_selected_from_actual_menus_instead_of_expensive_direct_troop() {
    let mut source = scenario(Faction::Aramon);
    source.unit_defs[2].build.mobile = true;
    source.unit_defs[2].maxvelocity = tak_math::Fx::from_int(1);
    source.unit_defs[2].build.builddistance = 160;
    let mut expensive = source.unit_defs[3].clone();
    expensive.name = "training_expensive".into();
    expensive.build.buildcost = 100_000;
    source.unit_defs.push(expensive);
    source.unit_defs[0].build.canbuild.insert(0, UnitDefId(5));
    let world = World::new(&source).unwrap();
    let config = skirmish::training_config(&world, Faction::Zhon).unwrap();
    assert_eq!(
        config.controllers[0].recipe.producer.as_ref().unwrap().id,
        UnitDefId(2)
    );
    assert_eq!(config.controllers[0].recipe.troop.id, UnitDefId(3));
}
