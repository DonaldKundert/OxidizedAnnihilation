//! Entirely hand-authored fixture, including the display-only map descriptor.
use super::*;
use tak_commands::{PlayerId, SimCommand, Tick, UnitSet};
use tak_headless::bot::{ControllerConfig, demo};
use tak_sim::MatchState;

fn session_from(replay: ReplaySession) -> Session {
    let checkpoint = replay.checkpoint().unwrap();
    let executed = replay.executed_log().clone();
    let controllers = replay.controllers().clone();
    let tick = replay.sim().tick();
    let map = tak_content::MapDef {
        name: "synthetic-training".into(),
        ota: Default::default(),
        width: 40,
        height: 20,
        sea_level: 0,
        heights: vec![0; 800],
        kingdom: None,
        palette: None,
        features: Vec::new(),
        paved_cells: 0,
        marker_cells: 0,
        start_positions: Vec::new(),
        player_slots: 2,
        crt: None,
        passability: tak_content::PassGrid {
            width: 40,
            height: 20,
            rule: Default::default(),
            cells: vec![0; 800],
        },
        issues: Vec::new(),
        dangling: Vec::new(),
        coverage: Default::default(),
    };
    let mut session = Session::from_sim(
        &tak_vfs::Vfs::new(),
        "synthetic-training",
        map,
        replay.into_sim(),
        executed.clone(),
        None,
    )
    .unwrap();
    session.executed_log = executed;
    session.hashes_first_tick = tick;
    session.controllers = controllers;
    session.save_origin = Some(SaveOrigin {
        checkpoint,
        data: PathBuf::new(),
        budget: ReplayBudget::default(),
    });
    session
}

fn training(direct: bool) -> ReplaySession {
    let mut scenario = demo::scenario(direct);
    let world = tak_sim::World::new(&scenario).unwrap();
    let mut config = demo::config(&world, direct).unwrap();
    // The playable client always owns human zero. Move the bot's explicit
    // starting builder to owner one, without mutating a running world.
    let root = scenario.players[0].monarch;
    scenario.players[0].monarch = scenario.players[1].monarch;
    scenario.players[1].monarch = root;
    config.human_players = vec![PlayerId(0)];
    config.controllers[0].player = PlayerId(1);
    ReplaySession::new_with_controllers(CheckpointSource::synthetic(scenario), None, config)
        .unwrap()
}

#[test]
fn actual_client_factory_and_direct_loops_match_headless_every_tick_and_defeat() {
    for direct in [false, true] {
        let mut expected = training(direct);
        let mut client = session_from(training(direct));
        let hashes = expected.advance_driven(&CommandLog::new(), 1200).unwrap();
        let mut target = None;
        let initial_health = client
            .sim
            .world()
            .defs()
            .get(client.sim.world().players()[0].monarch_def)
            .unwrap()
            .combat
            .max_damage;
        let mut damaged = false;
        for hash in &hashes {
            assert_eq!(client.try_step().unwrap(), *hash);
            target = target.or(client.sim.world().players()[0].monarch);
            if let Some(target) = target {
                damaged |= client
                    .sim
                    .world()
                    .unit(target)
                    .is_none_or(|unit| unit.combat.health < initial_health);
            }
        }
        assert!(damaged, "real target damage, not just planned orders");
        assert!(
            matches!(client.sim.world().victory().state(), MatchState::Over { winners } if winners == &[PlayerId(1)])
        );
        assert_eq!(&client.log, expected.executed_log());
        assert_eq!(client.executed_log, client.log);
        let checkpoint = capture(&client).unwrap();
        assert_eq!(checkpoint.controllers(), expected.controllers());
        let mut raw = ReplaySession::new(checkpoint.source().clone(), None).unwrap();
        assert!(raw.controllers().is_empty());
        assert_eq!(raw.advance(&client.log, 1200).unwrap(), hashes);
        assert!(
            client
                .training_report()
                .unwrap()
                .contains("full-map/omniscient")
        );
    }
}

#[test]
fn capture_restore_human_order_continue_and_resave_keep_absolute_decisions() {
    for direct in [false, true] {
        for boundary in [0, 1, 2, 6, 11, 30, 31, 32, 91, 92, 121, 200] {
            let mut live = session_from(training(direct));
            for _ in 0..boundary {
                live.try_step().unwrap();
            }
            let checkpoint = capture(&live).unwrap();
            let text = checkpoint.to_json_pretty().unwrap();
            let loaded = Checkpoint::from_json(&text).unwrap();
            let mut resumed = session_from(loaded.restore(None, ReplayBudget::default()).unwrap());
            assert_eq!(resumed.tick(), Tick(boundary));
            assert_eq!(resumed.controllers(), live.controllers());
            assert_eq!(resumed.executed_log, live.executed_log);
            let human = live.sim.world().players()[0]
                .monarch
                .unwrap_or(tak_commands::UnitId::new(0, 0));
            let order = SimCommand::Stop {
                units: UnitSet::single(human),
            };
            assert_eq!(live.submit(order.clone()), Tick(boundary));
            assert_eq!(resumed.submit(order), Tick(boundary));
            for _ in boundary..240 {
                assert_eq!(resumed.try_step().unwrap(), live.try_step().unwrap());
            }
            assert_eq!(resumed.log, live.log);
            assert_eq!(
                capture(&resumed).unwrap().to_json_pretty().unwrap(),
                capture(&live).unwrap().to_json_pretty().unwrap()
            );
        }
    }
}

#[test]
fn competing_future_owner_and_saturated_keys_fail_before_any_mutation() {
    let mut client = session_from(training(false));
    client.try_step().unwrap();
    let human = client.sim.world().players()[0].monarch.unwrap();
    client.log.record(
        Tick(300),
        PlayerId(1),
        SimCommand::Stop {
            units: UnitSet::single(human),
        },
    );
    let before = (
        client.tick(),
        client.sim.state_hash(),
        client.log.clone(),
        client.executed_log.clone(),
        client.hashes.clone(),
    );
    assert!(
        client
            .try_step()
            .unwrap_err()
            .to_string()
            .contains("competes")
    );
    assert_eq!(
        before,
        (
            client.tick(),
            client.sim.state_hash(),
            client.log.clone(),
            client.executed_log.clone(),
            client.hashes.clone()
        )
    );
    assert!(client.last_step_error().unwrap().contains("competes"));
    client.log = client.executed_log.clone();
    let order = SimCommand::Stop {
        units: UnitSet::single(human),
    };
    client
        .log
        .insert(tak_commands::LoggedCommand {
            tick: client.tick(),
            player: PlayerId(0),
            seq: u32::MAX,
            command: order.clone(),
        })
        .unwrap();
    client.log.record(client.tick(), PlayerId(0), order);
    let before = (
        client.sim.state_hash(),
        client.log.clone(),
        client.executed_log.clone(),
        client.hashes.clone(),
    );
    assert!(
        client
            .try_step()
            .unwrap_err()
            .to_string()
            .contains("unique")
    );
    assert_eq!(
        before,
        (
            client.sim.state_hash(),
            client.log.clone(),
            client.executed_log.clone(),
            client.hashes.clone()
        )
    );
}

#[test]
fn local_human_is_required_and_legacy_empty_configuration_stays_valid() {
    assert!(crate::training::validate_local(&ControllerConfig::default()).is_ok());
    let mut config = training(false).controllers().clone();
    config.human_players.clear();
    assert!(crate::training::validate_local(&config).is_err());
    config.controllers[0].player = PlayerId(0);
    config.human_players = vec![PlayerId(1)];
    assert!(crate::training::validate_local(&config).is_err());
}

#[test]
fn edited_public_past_never_changes_actual_history_and_capture_still_refuses_it() {
    let mut client = session_from(training(false));
    let mut expected = session_from(training(false));
    for _ in 0..2 {
        assert_eq!(client.try_step().unwrap(), expected.try_step().unwrap());
    }
    assert!(!client.executed_log.is_empty());
    let actual = client.executed_log.clone();
    client.log = CommandLog::new();
    assert_eq!(client.try_step().unwrap(), expected.try_step().unwrap());
    assert_eq!(client.executed_log, actual);
    assert!(
        capture(&client)
            .unwrap_err()
            .to_string()
            .contains("executed command history was edited")
    );
}

#[test]
fn training_defensive_checkpoint_continuation_matches_raw_replay() {
    use tak_commands::FireState;

    for direct in [false, true] {
        for boundary in [2, 31, 92, 200] {
            let mut live = session_from(training(direct));
            // Initial spawns are realized by the first simulation tick.
            live.try_step().unwrap();
            let human = live.sim.world().players()[0].monarch.unwrap();
            let stance = |state| SimCommand::SetFireState {
                units: UnitSet::single(human),
                state,
            };
            live.submit(stance(FireState::Defensive));
            for _ in 1..boundary {
                live.try_step().unwrap();
            }
            assert_eq!(
                live.sim.world().unit(human).unwrap().fire_state,
                FireState::Defensive
            );
            let saved = capture(&live).unwrap();
            let encoded = saved.to_json_pretty().unwrap();
            let decoded = Checkpoint::from_json(&encoded).unwrap();
            let mut resumed = session_from(decoded.restore(None, ReplayBudget::default()).unwrap());
            assert_eq!(resumed.controllers(), live.controllers());
            assert_eq!(resumed.executed_log, live.executed_log);
            assert_eq!(resumed.sim.state_hash(), live.sim.state_hash());
            assert_eq!(
                resumed.sim.world().unit(human).unwrap().fire_state,
                FireState::Defensive
            );

            // Exercise a real post-restore transition, not merely setting the
            // already-active state again. Human orders share ticks with the
            // ordinary generated opponent command stream.
            live.submit(stance(FireState::HoldFire));
            resumed.submit(stance(FireState::HoldFire));
            assert_eq!(resumed.try_step().unwrap(), live.try_step().unwrap());
            live.submit(stance(FireState::Defensive));
            resumed.submit(stance(FireState::Defensive));
            for _ in boundary + 1..240 {
                assert_eq!(resumed.try_step().unwrap(), live.try_step().unwrap());
            }
            assert_eq!(resumed.log, live.log);
            assert_eq!(
                capture(&resumed).unwrap().to_json_pretty().unwrap(),
                capture(&live).unwrap().to_json_pretty().unwrap()
            );

            // JSON command replay with no controller metadata must reproduce
            // all historical ticks, including Defensive, without AI planning.
            let log = CommandLog::from_json(&live.log.to_json_pretty()).unwrap();
            let mut raw = ReplaySession::new(saved.source().clone(), None).unwrap();
            assert!(raw.controllers().is_empty());
            assert_eq!(raw.advance(&log, 240).unwrap(), live.hashes);
            assert_eq!(raw.sim().state_hash(), live.sim.state_hash());
        }
    }
}

#[test]
fn later_executed_key_collision_rejects_entire_tick_before_inserting_first_command() {
    let mut client = session_from(training(false));
    client.try_step().unwrap();
    let human = client.sim.world().players()[0].monarch.unwrap();
    for state in [
        tak_commands::FireState::HoldFire,
        tak_commands::FireState::Defensive,
    ] {
        client.submit(SimCommand::SetFireState {
            units: UnitSet::single(human),
            state,
        });
    }
    // Deliberately corrupt private history to exercise the insertion safety
    // guard. The collision is the SECOND command, after a valid first key.
    client
        .executed_log
        .insert(client.log.for_tick(client.tick())[1].clone())
        .unwrap();
    let before = (
        client.tick(),
        client.sim.state_hash(),
        client.log.clone(),
        client.executed_log.clone(),
        client.hashes.clone(),
    );
    assert!(
        client
            .try_step()
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    assert_eq!(
        before,
        (
            client.tick(),
            client.sim.state_hash(),
            client.log.clone(),
            client.executed_log.clone(),
            client.hashes.clone(),
        )
    );
}
