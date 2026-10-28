//! Synthetic Clear capability, HUD, fog and exact feature lifetime checks.
use tak_client::{feel, fog::FogGrid, hud, view::SimView};
use tak_commands::{FeatureId, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId, UnitSet};
use tak_input::{
    Action, Camera, CursorKind, Mapper, Mode, Output, Profile, WorldView, clearers_for,
};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::{
    Order, World,
    construction::progress::Frame,
    features::{self, FeatureDef, FeatureSetup, PlacedFeature},
};

fn world() -> World {
    let mut sc = tak_sim::testing::example_scenario(1, 2);
    let mut builder = sc.unit_defs[0].clone();
    builder.name = "builder".into();
    builder.build.builder = true;
    builder.build.canreclaim = true;
    builder.build.mobile = true;
    builder.build.workertime = 10;
    builder.build.builddistance = 32;
    builder.build.canbuild.clear();
    builder.combat.max_damage = 100;
    builder.combat.weapons.clear();
    let mut no_flag = builder.clone();
    no_flag.build.canreclaim = false;
    let mut nonbuilder = builder.clone();
    nonbuilder.build.builder = false;
    let mut zero = builder.clone();
    zero.build.workertime = 0;
    let mut stationary = builder.clone();
    stationary.build.mobile = false;
    stationary.maxvelocity = Fx::ZERO;
    sc.unit_defs = vec![builder, no_flag, nonbuilder, zero, stationary];
    let tree = FeatureDef {
        name: "synthetic_tree".into(),
        footprint_x: 1,
        footprint_z: 1,
        reclaimable: true,
        blocking: true,
        damage: 20,
        energy: 50,
        feature_dead: Some(1),
        ..Default::default()
    };
    let stump = FeatureDef {
        name: "synthetic_stump".into(),
        feature_dead: None,
        ..tree.clone()
    };
    let decoration = FeatureDef {
        name: "synthetic_decoration".into(),
        reclaimable: false,
        ..tree.clone()
    };
    *sc.features = FeatureSetup {
        defs: vec![tree, stump, decoration],
        placed: vec![
            PlacedFeature {
                def: 0,
                x: 50,
                z: 12,
            },
            PlacedFeature {
                def: 1,
                x: 50,
                z: 12,
            },
            PlacedFeature {
                def: 2,
                x: 55,
                z: 12,
            },
        ],
        ..Default::default()
    };
    World::new(&sc).unwrap()
}
fn spawn(w: &mut World, def: u32, owner: u8, x: i32) -> UnitId {
    w.spawn(
        UnitDefId(def),
        PlayerId(owner),
        Vec2::from_ints(x, 200),
        Angle::ZERO,
    )
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
fn selected(w: &impl WorldView, id: UnitId) -> Mapper {
    let mut profile = Profile::default_combined();
    profile.tuning.reticle = false;
    let mut m = Mapper::new(profile);
    m.set_camera(Camera {
        origin: Vec2::ZERO,
        world_per_px: Fx::ONE,
        width_px: 2048,
        height_px: 2048,
    });
    m.warp_cursor(w.unit(id).unwrap().pos);
    m.perform(Action::Select, w, Tick(0));
    assert_eq!(m.selection(), &UnitSet::single(id));
    m
}
fn commands(out: Vec<Output>) -> Vec<SimCommand> {
    out.into_iter()
        .filter_map(|o| match o {
            Output::Sim { command, .. } => Some(command),
            _ => None,
        })
        .collect()
}

#[test]
fn actual_capability_requires_authored_flag_worker_rate_and_live_complete_owned_unit() {
    let mut w = world();
    let builder = spawn(&mut w, 0, 0, 100);
    let no_flag = spawn(&mut w, 1, 0, 200);
    let nonbuilder = spawn(&mut w, 2, 0, 300);
    let zero = spawn(&mut w, 3, 0, 400);
    let stationary = spawn(&mut w, 4, 0, 500);
    let enemy = spawn(&mut w, 0, 1, 600);
    let frame = spawn(&mut w, 0, 0, 700);
    w.unit_mut(frame).unwrap().build.frame = Some(Frame::new(10));
    let dying = spawn(&mut w, 0, 0, 900);
    w.unit_mut(dying).unwrap().combat.dying = Some(tak_sim::combat::Dying {
        since: 0,
        severity: 100,
        kind: 0,
        done: false,
    });
    let v = view(&w, None);
    let all = UnitSet::from(vec![
        builder,
        no_flag,
        nonbuilder,
        zero,
        stationary,
        enemy,
        frame,
        dying,
        UnitId::new(0, 99),
    ]);
    assert_eq!(
        clearers_for(&v, &all),
        UnitSet::from(vec![builder, nonbuilder, stationary])
    );
    assert!(
        v.build_options(builder).is_empty(),
        "empty menus still allow authored Clear"
    );
}

#[test]
fn feature_picking_is_half_open_reclaimable_visible_and_exact_generation() {
    let mut w = world();
    let point = Vec2::from_ints(808, 200);
    let old = FeatureId::new(0, 0);
    assert_eq!(
        view(&w, None).clear_target_at(point),
        Some(old),
        "overlap tie uses low id"
    );
    assert_eq!(
        view(&w, None).clear_target_at(Vec2::from_ints(816, 200)),
        None,
        "exclusive right edge"
    );
    assert_eq!(
        view(&w, None).clear_target_at(Vec2::from_ints(888, 200)),
        None,
        "not reclaimable"
    );
    let mut fog = FogGrid::new(128, 128);
    assert_eq!(view(&w, Some(&fog)).clear_target_at(point), None);
    fog.update(&[(808.0, 200.0, 40.0)]);
    assert_eq!(view(&w, Some(&fog)).clear_target_at(point), Some(old));
    fog.update(&[]);
    assert_eq!(
        view(&w, Some(&fog)).clear_target_at(point),
        None,
        "remembered terrain is not live feature knowledge"
    );
    features::apply_damage(&mut w, old, 20);
    let new = view(&w, None).clear_target_at(point).unwrap();
    assert_eq!(new.index, old.index);
    assert_ne!(new.generation, old.generation);
    assert_eq!(view(&w, None).clear_target_pos(old), None);
    assert_eq!(view(&w, None).clear_target_pos(new), Some(point));
}

#[test]
fn panel_mode_prompts_cursor_and_command_use_clear_not_weapon_attack() {
    let mut w = world();
    let builder = spawn(&mut w, 0, 0, 200);
    // An overlapping unit cannot hijack explicit feature targeting.
    spawn(&mut w, 2, 1, 808);
    let v = view(&w, None);
    let mut m = selected(&v, builder);
    assert_eq!(hud::PANEL_ACTIONS[5], Action::ClearMode);
    assert_eq!(hud::PANEL_BUTTONS[5], "ClearButton");
    m.perform(hud::PANEL_ACTIONS[5], &v, Tick(0));
    assert_eq!(m.mode(), Mode::Clear);
    m.warp_cursor(Vec2::from_ints(808, 200));
    assert_eq!(m.cursor_kind(&v), CursorKind::Clear);
    assert_eq!(hud::cursor_seq(CursorKind::Clear), "Cursorreclamate");
    assert!(hud::CURSOR_KINDS.contains(&CursorKind::Clear));
    assert_eq!(feel::action_label(&Action::ClearMode), "Clear");
    for hints in [
        feel::hints(&m, Default::default()),
        feel::hints_classic(&m, Default::default()),
    ] {
        assert!(hints.iter().any(|h| h.text == "clear feature"));
        assert!(hints.iter().any(|h| h.text == "cancel"));
    }
    let command = SimCommand::Clear {
        units: UnitSet::single(builder),
        target: FeatureId::new(0, 0),
        queue: QueueMode::Append,
    };
    assert_eq!(
        commands(m.perform(Action::Context { append: true }, &v, Tick(1))),
        vec![command.clone()]
    );
    assert_eq!(m.mode(), Mode::Clear);
    assert_eq!(hud::order_sound(&command), Some("MOVE"));
    m.warp_cursor(Vec2::from_ints(888, 200));
    assert_eq!(m.cursor_kind(&v), CursorKind::Invalid);
    assert!(commands(m.perform(Action::Context { append: false }, &v, Tick(1))).is_empty());
    assert_eq!(m.mode(), Mode::Clear);
    m.perform(Action::Cancel, &v, Tick(1));
    assert_eq!(m.mode(), Mode::None);
}

#[test]
fn hints_keep_clear_heal_and_squad_help_without_granting_capability() {
    let m = Mapper::new(Profile::default_combined());
    let hints = feel::hints_classic(
        &m,
        feel::HintCx {
            has_units: true,
            has_clearer: true,
            has_healer: true,
            ..Default::default()
        },
    );
    assert_eq!(hints.len(), 10);
    assert!(
        hints
            .iter()
            .any(|h| h.text == "clear feature" && h.key == feel::HintKey::Text("C".into()))
    );
    assert!(hints.iter().any(|h| h.text == "heal / repair"));
    assert!(hints.iter().any(|h| h.text == "squad"));
    assert!(
        !feel::hints_classic(
            &m,
            feel::HintCx {
                has_units: true,
                ..Default::default()
            }
        )
        .iter()
        .any(|h| h.text == "clear feature")
    );
}

#[test]
fn hud_hides_owned_clear_walk_preserves_player_tail_and_filters_fog_routes() {
    let mut w = world();
    let builder = spawn(&mut w, 0, 0, 200);
    let target = FeatureId::new(0, 0);
    let goal = Vec2::from_ints(808, 200);
    tak_sim::construction::apply_clear_command(
        &mut w,
        PlayerId(0),
        &SimCommand::Clear {
            units: UnitSet::single(builder),
            target,
            queue: QueueMode::Replace,
        },
    );
    tak_sim::construction::tick(&mut w, &mut tak_sim::NoScripts);
    let u = w.unit_mut(builder).unwrap();
    assert!(u.build.clear.is_some_and(|j| j.walking));
    u.orders.push_back(Order::Move { to: goal });
    let u = w.unit(builder).unwrap();
    assert_eq!(
        hud::orders_for_display(u).copied().collect::<Vec<_>>(),
        vec![Order::Clear { target }, Order::Move { to: goal }]
    );
    assert_eq!(
        hud::order_text(&w, u, |_| "builder".into()),
        "Clearing synthetic_tree (+1)"
    );
    assert_ne!(
        hud::order_rgba(&Order::Clear { target }),
        hud::order_rgba(&Order::Move { to: goal })
    );
    let fog = FogGrid::new(128, 128);
    let v = view(&w, Some(&fog));
    assert_eq!(
        hud::visible_order_goals(&v, u)
            .map(|(o, p)| (*o, p))
            .collect::<Vec<_>>(),
        vec![(Order::Move { to: goal }, goal)],
        "only the explicit player ground tail remains visible"
    );
    assert_eq!(hud::order_goal(&w, &Order::Clear { target }), Some(goal));
    assert!(features::remove(&mut w, target).is_some());
    assert_eq!(hud::order_goal(&w, &Order::Clear { target }), None);
    assert_eq!(
        hud::order_text(&w, w.unit(builder).unwrap(), |_| "builder".into()),
        "Clearing ? (+1)"
    );
}

#[test]
fn installed_clear_cursor_and_button_sequences_decode_in_memory() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("Clear cursor/button art") else {
        return;
    };
    let vfs = tak_client::game::open_vfs(&dir).unwrap();
    for (path, name) in [
        ("anims/cursors.gaf", hud::cursor_seq(CursorKind::Clear)),
        ("anims/igcommonbuttons.gaf", hud::PANEL_BUTTONS[5]),
    ] {
        let gaf = tak_formats::gaf::Gaf::parse(&vfs.read(path).unwrap()).unwrap();
        let entry = gaf
            .entry(name)
            .unwrap_or_else(|| panic!("missing {name} in {path}"));
        assert!(!entry.frames.is_empty());
        assert!(
            entry
                .frames
                .iter()
                .all(|f| f.frame.width > 0 && f.frame.height > 0)
        );
    }
}
