//! Optional owned-data smoke. Fixtures/configuration are derived locally and
//! never embedded or committed. Synthetic damage/defeat tests live in saves.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use tak_client::cli::Args;
use tak_client::game::{Session, open_vfs};
use tak_client::saves;
use tak_commands::{CommandLog, PlayerId, SimCommand, UnitSet};
use tak_headless::bot::{
    ControllerConfig, Recipe, SIMPLE_SKIRMISH_V1, SkirmishController, Targeting,
};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplaySession};
use tak_math::Fx;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn recipe(world: &tak_sim::World, direct: bool) -> Recipe {
    let root = world.defs().get(world.players()[1].monarch_def).unwrap();
    let income = root
        .build
        .canbuild
        .iter()
        .filter_map(|id| world.defs().get(*id))
        .find(|def| def.economy.mogriumincome > 0 && def.maxvelocity == Fx::ZERO);
    let ordinary = |def: &&tak_sim::UnitDef| {
        !def.build.builder
            && def.build.mobile
            && def.maxvelocity > Fx::ZERO
            && def
                .combat
                .weapons
                .iter()
                .any(|weapon| !weapon.is_spell() && weapon.damage.default > 0)
    };
    if direct {
        let troop = root
            .build
            .canbuild
            .iter()
            .filter_map(|id| world.defs().get(*id))
            .find(ordinary)
            .unwrap();
        return Recipe::resolve(
            world,
            &root.name,
            income.map(|def| def.name.as_str()),
            None,
            &troop.name,
        )
        .unwrap();
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
                && def.maxvelocity == Fx::ZERO
        })
    {
        if let Some(troop) = producer
            .build
            .canbuild
            .iter()
            .filter_map(|id| world.defs().get(*id))
            .find(ordinary)
        {
            return Recipe::resolve(
                world,
                &root.name,
                income.map(|def| def.name.as_str()),
                Some(&producer.name),
                &troop.name,
            )
            .unwrap();
        }
    }
    panic!("installed starting builder has no supported factory recipe");
}

#[test]
fn installed_ara_factory_and_zhon_direct_recipes_use_scripts_save_resume_and_replay() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("client_training") else {
        return;
    };
    let root = Scratch(std::env::temp_dir().join(format!(
        "tak-client-training-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&root.0).unwrap();
    for (players, side, direct) in [("ZON,ARA", "ARA", false), ("ARA,ZON", "ZON", true)] {
        let source = CheckpointSource::map("kmap/Adamantine Gate", players, 1, true);
        let plain = ReplaySession::new(source.clone(), Some(&data)).unwrap();
        let config = ControllerConfig {
            human_players: vec![PlayerId(0)],
            controllers: vec![SkirmishController {
                policy: SIMPLE_SKIRMISH_V1.into(),
                player: PlayerId(1),
                recipe: recipe(plain.sim().world(), direct),
                cadence_ticks: 30,
                command_budget: 8,
                site_budget: 64,
                army_cap: 3,
                mana_reserve: 0,
                targeting: Targeting::OmniscientTrainingV1,
            }],
        };
        let config_path = root.0.join(format!("{side}.json"));
        std::fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        if let Some(directory) = std::env::var_os("TAK_TRAINING_RECIPE_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                PathBuf::from(directory).join(format!("client-training-{side}.json")),
                serde_json::to_vec_pretty(&config).unwrap(),
            )
            .unwrap();
        }
        let mut args = Args::try_parse_from([
            "tak-client",
            "--data",
            data.to_str().unwrap(),
            "--players",
            players,
            "--controller-config",
            config_path.to_str().unwrap(),
        ])
        .unwrap();
        let vfs = open_vfs(&data).unwrap();
        let mut client = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
        assert!(client.sim.cob().is_some());
        let mut headless =
            ReplaySession::new_with_controllers(source.clone(), Some(&data), config.clone())
                .unwrap();
        let first = headless.advance_driven(&CommandLog::new(), 32).unwrap();
        for hash in first {
            assert_eq!(client.try_step().unwrap(), hash);
        }
        assert!(client.log.iter().any(|entry| entry.player == PlayerId(1)
            && matches!(entry.command, SimCommand::Build { .. })));
        let save = saves::quick_save(&client, &root.0).unwrap();
        let saved_bytes = std::fs::read(&save).unwrap();
        std::fs::remove_file(&config_path).unwrap(); // saved identity is self-contained
        args.controller_config = None;
        args.load_checkpoint = Some(save.clone());
        let mut resumed = Session::load_from_args(&vfs, &args, CommandLog::new()).unwrap();
        assert_eq!(resumed.controllers(), &config);
        let unit = client.sim.world().players()[0].monarch.unwrap();
        let order = SimCommand::Stop {
            units: UnitSet::single(unit),
        };
        let mut human = CommandLog::new();
        human.record(client.tick(), PlayerId(0), order.clone());
        client.submit(order.clone());
        resumed.submit(order);
        let suffix = headless.advance_driven(&human, 568).unwrap();
        for hash in suffix {
            assert_eq!(client.try_step().unwrap(), hash);
            assert_eq!(resumed.try_step().unwrap(), hash);
        }
        assert_eq!(client.log, resumed.log);
        assert_eq!(&client.log, headless.executed_log());
        let resave = saves::quick_save(&resumed, &root.0).unwrap();
        assert_eq!(std::fs::read(&save).unwrap(), saved_bytes);
        assert_eq!(Checkpoint::read(&resave).unwrap().controllers(), &config);
        let mut raw = ReplaySession::new(source, Some(&data)).unwrap();
        assert_eq!(raw.advance(&client.log, 600).unwrap(), client.hashes);
        eprintln!(
            "owned-data {side} {}: scripts, 600 shared ticks, {} canonical commands, save/resume/human order/resave/raw replay verified; recipe {:?}",
            if direct { "direct builder" } else { "factory" },
            client.log.len(),
            config.controllers[0].recipe
        );
    }
}
