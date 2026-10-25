//! Transactional checkpoint scene replacement without recreating Winit.
//! All fallible content preparation happens before any live entity is removed.
use super::*;

#[derive(bevy::ecs::schedule::ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct RebuildScene;

#[derive(Clone, Copy)]
enum ReplacementKind {
    Checkpoint,
    Skirmish(crate::skirmish::SetupConfig),
}

#[derive(Resource, Default)]
struct RequestFrame(bool);

pub(super) fn apply_prepared_load(world: &mut World) {
    // Render the preparing notice once before synchronous validation. A close
    // or cancel in the next input pass can still clear the request without I/O.
    world.init_resource::<RequestFrame>();
    let has_request = world
        .get_resource::<skirmish_ui::PendingRequest>()
        .is_some_and(|pending| pending.request.is_some());
    if has_request && !world.resource::<RequestFrame>().0 {
        world.resource_mut::<RequestFrame>().0 = true;
        return;
    }
    world.resource_mut::<RequestFrame>().0 = false;
    let request = world
        .get_resource_mut::<skirmish_ui::PendingRequest>()
        .and_then(|mut pending| pending.request.take());
    if let Some(request) = request {
        let config = match request {
            skirmish_ui::Request::Launch(config) | skirmish_ui::Request::Rematch(config) => config,
        };
        let mut args = world.resource::<save_browser::BrowserArgs>().0.clone();
        if !args.allows_match_replacement() {
            world
                .resource_mut::<skirmish_ui::SkirmishState>()
                .report_error(
                "New matches are unavailable during authored playback, recording or verification"
                    .into(),
            );
            return;
        }
        args.skirmish = false;
        args.load_checkpoint = None;
        args.controller_config = None;
        args.map = config.map.path().into();
        args.players = config.players().into();
        args.world_seed = config.seed;
        let session = match crate::skirmish::prepare_session_with_budget(
            &args.data,
            &config,
            crate::saves::replay_budget(&args),
        ) {
            Ok(session) => session,
            Err(error) => {
                world
                    .resource_mut::<skirmish_ui::SkirmishState>()
                    .report_error(error.to_string());
                return;
            }
        };
        replace_session(world, args, session, ReplacementKind::Skirmish(config));
        return;
    }
    let Some(load) = world
        .non_send_mut::<save_browser::PendingLoad>()
        .prepared
        .take()
    else {
        return;
    };
    let mut args = world.resource::<save_browser::BrowserArgs>().0.clone();
    args.skirmish = false;
    args.load_checkpoint = Some(load.path);
    replace_session(world, args, load.session, ReplacementKind::Checkpoint);
}

fn replace_session(world: &mut World, args: Args, session: Session, kind: ReplacementKind) {
    let prepared = (|| -> Result<_, Box<dyn std::error::Error>> {
        crate::saves::protect_checkpoint_outputs(&args)?;
        let vfs = crate::game::open_vfs(&args.data)?;
        let ground = terrain::load(&vfs, &session.map_name, session.map.palette.as_deref())?;
        let tex = TextureIndex::load(&vfs);
        Ok((vfs, ground, tex))
    })();
    let (vfs, ground, tex) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            match kind {
                ReplacementKind::Checkpoint => world
                    .resource_mut::<save_browser::BrowserState>()
                    .report_error(format!("Load failed; current match unchanged: {error}")),
                ReplacementKind::Skirmish(_) => world
                    .resource_mut::<skirmish_ui::SkirmishState>()
                    .report_error(error.to_string()),
            }
            return;
        }
    };
    let training = if session.controllers().controllers.is_empty() {
        ""
    } else {
        " - simple-skirmish-v1 (omniscient training)"
    };
    let title = format!("Oxidized Annihilation - {}{training}", session.map_name);
    let setup = crate::skirmish::from_session(&session);
    let snap = session.snapshot();
    let cells = ground.cells;
    let mut fog = crate::fog::FogGrid::new(cells.0, cells.1);
    let w = session.sim.world();
    let eyes: Vec<_> = w
        .units()
        .filter(|(_, u)| u.owner == session.local)
        .map(|(_, u)| {
            let sight = w.defs().get(u.def).map_or(0, |d| d.combat.sight);
            (f(u.pos.x), f(u.pos.y), (sight as f32).max(FOG_MIN_SIGHT))
        })
        .collect();
    fog.update(&eyes);
    let target = focus_point(&session, Focus::Monarch(0)).unwrap_or([0.0, 0.0]);
    let kingdom = session.map.kingdom.clone();
    let fog_on = world.non_send::<SimRes>().fog_on;
    let profile = world.resource::<InputRes>().mapper.profile().clone();
    let old_feel = world.resource::<Feel>();
    let mut feel = Feel::new(&args, old_feel.profile_mtime);
    feel.profile_path = old_feel.profile_path.clone();
    feel.overlay_open = false;
    feel.overlay_captured_input = true;
    feel.toast = Some(format!(
        "Loaded paused at tick {}. Close the checkpoint browser to resume. Camera, selection and explored fog reset.",
        session.tick().0
    ));
    if let ReplacementKind::Skirmish(_) = kind {
        feel.toast = session
            .training_report()
            .or_else(|| Some("Practice match started. The opposing monarch is passive.".into()));
    }
    // An input recording/ghost cannot describe a new arbitrary checkpoint.
    // The browser rejects these modes before preparation; don't carry stale
    // local presentation state or pre-load gestures across the boundary.
    clear_scene_entities(world);
    world.insert_non_send(Content {
        vfs,
        tex,
        sprites: SpriteStore::default(),
        kingdom,
    });
    world.insert_non_send(SimRes {
        session,
        prev: snap.clone(),
        curr: snap,
        acc: 0.0,
        speed: 1.0,
        paused: matches!(kind, ReplacementKind::Checkpoint),
        fog,
        fog_changed: true,
        alerts: Default::default(),
        new_alerts: Vec::new(),
        fog_on,
    });
    world.insert_resource(Ground(Arc::new(ground)));
    world.insert_resource(InputRes {
        mapper: Mapper::new(profile),
        last_axes: BTreeMap::new(),
        last_pointer: None,
    });
    world.insert_resource(feel);
    world.resource_mut::<CamState>().geom.target = target;
    world.resource_mut::<CamState>().geom.zoom = args.zoom;
    world.insert_resource(ModelCache::default());
    world.insert_resource(UnitEntities::default());
    world.insert_resource(FeatureEntities::default());
    world.insert_resource(SpriteCache::default());
    world.insert_resource(UnitTops::default());
    world.insert_resource(FxState::default());
    world.insert_resource(MinimapState::default());
    world.insert_resource(FogRes {
        enabled: fog_on,
        ..default()
    });
    world.insert_resource(controls::GlyphArt::default());
    world.insert_resource(controls::Portraits::default());
    world.insert_resource(hud_view::HudMode::default());
    world.insert_resource(hud_view::HudHover::default());
    world.insert_resource(hud_view::BottomMsg::default());
    world.remove_resource::<Showcase>();
    world.resource_mut::<save_browser::BrowserArgs>().0 = args;
    match kind {
        ReplacementKind::Checkpoint => {
            world.resource_mut::<save_browser::BrowserState>().loaded();
            if let Some(mut state) = world.get_resource_mut::<skirmish_ui::SkirmishState>() {
                state.checkpoint_loaded(setup);
            }
        }
        ReplacementKind::Skirmish(config) => {
            world
                .resource_mut::<save_browser::BrowserState>()
                .cancel_for_transition();
            world.non_send_mut::<save_browser::PendingLoad>().prepared = None;
            world
                .resource_mut::<skirmish_ui::SkirmishState>()
                .launched(config);
        }
    }
    for mut win in world
        .query_filtered::<&mut Window, With<PrimaryWindow>>()
        .iter_mut(world)
    {
        win.title = title.clone();
    }
    world.run_schedule(RebuildScene);
}

/// Only presentation roots belong to this scene. Keep windows, connected
/// devices, and unrelated entities intact. Despawning roots recursively also
/// removes model pieces and UI descendants through Bevy's linked hierarchy.
fn clear_scene_entities(world: &mut World) {
    let roots: Vec<_> = world
        .query_filtered::<Entity, (
            Without<ChildOf>,
            Without<Window>,
            Without<Gamepad>,
            Or<(With<Transform>, With<Node>, With<controls::SoundTtl>)>,
        )>()
        .iter(world)
        .collect();
    for entity in roots {
        world.despawn(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use clap::Parser;

    #[test]
    fn queued_skirmish_can_be_cancelled_before_preparation() {
        let mut app = super::super::overlay_input_tests::app();
        let config = crate::skirmish::SetupConfig::default();
        app.insert_resource(skirmish_ui::SkirmishState::new(config, false))
            .insert_resource(skirmish_ui::PendingRequest {
                request: Some(skirmish_ui::Request::Launch(config)),
            });
        let before = app.world().non_send::<SimRes>().session.sim.state_hash();
        app.world_mut()
            .run_system_once(apply_prepared_load)
            .unwrap();
        assert!(
            app.world()
                .resource::<skirmish_ui::PendingRequest>()
                .request
                .is_some()
        );
        app.world_mut()
            .resource_mut::<skirmish_ui::PendingRequest>()
            .request = None;
        app.world_mut()
            .run_system_once(apply_prepared_load)
            .unwrap();
        assert_eq!(
            app.world().non_send::<SimRes>().session.sim.state_hash(),
            before
        );
        assert!(!app.world().resource::<RequestFrame>().0);
    }

    #[test]
    fn failed_skirmish_preparation_preserves_live_match_and_pending_orders() {
        for protected in [false, true] {
            let mut app = super::super::overlay_input_tests::app();
            let config = crate::skirmish::SetupConfig::default();
            app.insert_resource(skirmish_ui::SkirmishState::new(config, false))
                .insert_resource(skirmish_ui::PendingRequest {
                    request: Some(skirmish_ui::Request::Launch(config)),
                });
            let mut args = Args::parse_from(["test", "--data", "/synthetic-not-read"]);
            if protected {
                args.record = Some("/synthetic-never-written".into());
            }
            app.insert_resource(save_browser::BrowserArgs(args));
            let unit = app
                .world()
                .non_send::<SimRes>()
                .session
                .sim
                .world()
                .units()
                .next()
                .unwrap()
                .0;
            app.world_mut().non_send_mut::<SimRes>().session.submit(
                tak_commands::SimCommand::Stop {
                    units: tak_commands::UnitSet::single(unit),
                },
            );
            let before_log = app.world().non_send::<SimRes>().session.recorded.clone();
            let before_profile = app.world().resource::<InputRes>().mapper.profile().clone();
            let before = app.world().non_send::<SimRes>().session.sim.state_hash();
            let before_tick = app.world().non_send::<SimRes>().session.tick();
            let marker = app.world_mut().spawn(Transform::default()).id();
            app.world_mut()
                .run_system_once(apply_prepared_load)
                .unwrap();
            assert!(
                app.world()
                    .resource::<skirmish_ui::PendingRequest>()
                    .request
                    .is_some()
            );
            app.world_mut()
                .run_system_once(apply_prepared_load)
                .unwrap();
            assert_eq!(
                app.world().non_send::<SimRes>().session.recorded,
                before_log
            );
            assert_eq!(
                app.world().resource::<InputRes>().mapper.profile(),
                &before_profile
            );
            assert_eq!(
                app.world().non_send::<SimRes>().session.sim.state_hash(),
                before
            );
            assert_eq!(app.world().non_send::<SimRes>().session.tick(), before_tick);
            assert!(app.world().get_entity(marker).is_ok());
            assert!(
                app.world()
                    .resource::<skirmish_ui::PendingRequest>()
                    .request
                    .is_none()
            );
        }
    }

    #[test]
    fn failed_render_preparation_preserves_live_match_scene_and_profile() {
        let mut app = super::super::overlay_input_tests::app();
        let mut replacement = super::super::overlay_input_tests::app()
            .world_mut()
            .remove_non_send::<SimRes>()
            .unwrap();
        replacement.session.try_step().unwrap();
        let absent_data = std::env::temp_dir().join(format!(
            "tak-reload-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(!absent_data.exists());
        let mut args = Args::parse_from(["test", "--data", "/synthetic-not-read"]);
        args.data = absent_data.clone();
        let mut browser = save_browser::BrowserState::default();
        browser.loaded();
        app.insert_resource(save_browser::BrowserArgs(args))
            .insert_resource(browser)
            .insert_non_send(save_browser::PendingLoad {
                prepared: Some(save_browser::PreparedLoad {
                    session: replacement.session,
                    path: absent_data.join("selected.save.json"),
                }),
            });
        let world = app.world_mut();
        world.non_send_mut::<SimRes>().paused = true;
        let before = world.non_send::<SimRes>();
        let before_tick = before.session.tick();
        let before_hash = before.session.sim.state_hash();
        let profile = world.resource::<InputRes>().mapper.profile().clone();
        let ground = world.resource::<Ground>().0.clone();
        let scene_root = world.spawn(Transform::from_xyz(3.0, 4.0, 5.0)).id();
        let scene_child = world
            .spawn((Transform::default(), ChildOf(scene_root)))
            .id();
        let window = world
            .spawn((
                Window {
                    title: "Original live match".into(),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        let browser_root = world.spawn((save_browser::Root, Visibility::Hidden)).id();
        let status = world
            .spawn((save_browser::Label::Status, Text::new("")))
            .id();

        apply_prepared_load(world);

        let current = world.non_send::<SimRes>();
        assert_eq!(current.session.tick(), before_tick);
        assert_eq!(current.session.sim.state_hash(), before_hash);
        assert!(current.paused);
        assert_eq!(world.resource::<InputRes>().mapper.profile(), &profile);
        assert!(Arc::ptr_eq(&world.resource::<Ground>().0, &ground));
        assert!(world.get_entity(scene_root).is_ok());
        assert!(world.get_entity(scene_child).is_ok());
        assert_eq!(
            world.get::<Window>(window).unwrap().title,
            "Original live match"
        );
        assert!(
            world
                .resource::<save_browser::BrowserArgs>()
                .0
                .load_checkpoint
                .is_none()
        );
        assert!(
            world
                .non_send::<save_browser::PendingLoad>()
                .prepared
                .is_none()
        );
        assert!(
            world
                .resource::<save_browser::BrowserState>()
                .captures_input()
        );
        world.run_system_once(save_browser::draw).unwrap();
        assert_eq!(
            world.get::<Visibility>(browser_root),
            Some(&Visibility::Inherited)
        );
        assert!(
            world
                .get::<Text>(status)
                .unwrap()
                .0
                .starts_with("Not completed: Load failed; current match unchanged:")
        );
    }

    #[test]
    fn scene_cleanup_preserves_window_pad_and_unrelated_entities() {
        let mut world = World::new();
        let window = world.spawn(Window::default()).id();
        let pad = world.spawn(Gamepad::default()).id();
        let unrelated = world.spawn_empty().id();
        let root = world.spawn(Transform::default()).id();
        let child = world.spawn((Transform::default(), ChildOf(root))).id();
        let ui = world.spawn(Node::default()).id();
        clear_scene_entities(&mut world);
        for entity in [window, pad, unrelated] {
            assert!(world.get_entity(entity).is_ok());
        }
        for entity in [root, child, ui] {
            assert!(world.get_entity(entity).is_err());
        }
    }
}
