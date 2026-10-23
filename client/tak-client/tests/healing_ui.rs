//! Display/input adapters for explicit Heal. All game state is synthetic;
//! optional installed-data checks decode art in memory without storing it.

use tak_client::{feel, hud, view::SimView};
use tak_commands::{PlayerId, QueueMode, Scope, SimCommand, Tick, UnitDefId, UnitId, UnitSet};
use tak_input::{
    Action, BLANK, Binding, BuilderKind, Camera, CursorKind, EventKind, Gesture, Input, InputEvent,
    Key, Mapper, Mode, Profile, WorldView, healers_for, resolve_context,
};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::{Order, World, construction::progress::Frame};

fn world() -> World {
    let mut scenario = tak_sim::testing::example_scenario(1, 2);
    let mut healer = scenario.unit_defs[0].clone();
    healer.name = "healer".into();
    healer.combat.max_damage = 100;
    healer.combat.weapons.clear();
    healer.build.builder = true;
    healer.build.mobile = true;
    healer.build.workertime = 10;
    healer.build.canbuild.clear();
    let mut builder = healer.clone();
    builder.name = "builder".into();
    builder.build.canbuild = vec![UnitDefId(5)];
    let mut nonbuilder = healer.clone();
    nonbuilder.name = "nonbuilder".into();
    nonbuilder.build.builder = false;
    let mut zero = healer.clone();
    zero.name = "zero".into();
    zero.build.workertime = 0;
    let mut factory = builder.clone();
    factory.name = "factory".into();
    factory.build.mobile = false;
    let mut structure = nonbuilder.clone();
    structure.name = "structure".into();
    structure.build.mobile = false;
    scenario.unit_defs = vec![healer, builder, nonbuilder, zero, factory, structure];
    World::new(&scenario).unwrap()
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

fn view(w: &World) -> SimView<'_> {
    SimView {
        world: w,
        local: PlayerId(0),
        defs: None,
        fog: None,
    }
}

fn selected(w: &impl WorldView, actor: UnitId) -> Mapper {
    let mut profile = Profile::default_combined();
    profile.tuning.reticle = false;
    profile.fixed_slots = true;
    profile.tuning.wheel_context = false;
    profile.bindings.retain(
        |b| !matches!(&b.input, Input::Key(k) if k == &Key::new("f6") || k == &Key::new("f7")),
    );
    profile.command_wheel = vec![Action::HealMode, Action::Stop];
    for (key, action) in [("f6", Action::BuildWheel), ("f7", Action::CommandWheel)] {
        profile.bindings.push(Binding {
            input: Input::Key(Key::new(key)),
            with: None,
            gesture: Gesture::Press,
            action,
        });
    }
    let mut m = Mapper::new(profile);
    m.set_camera(Camera {
        origin: Vec2::ZERO,
        world_per_px: Fx::ONE,
        width_px: 2048,
        height_px: 2048,
    });
    m.warp_cursor(w.unit(actor).unwrap().pos);
    m.perform(Action::Select, w, Tick(0));
    assert_eq!(m.selection(), &UnitSet::single(actor));
    m.take_feedback();
    m
}

#[test]
fn capability_uses_real_builder_flag_and_work_rate_not_weapons_or_build_menu() {
    let mut w = world();
    let healer = spawn(&mut w, 0, 0, 200);
    let builder = spawn(&mut w, 1, 0, 300);
    let nonbuilder = spawn(&mut w, 2, 0, 400);
    let zero = spawn(&mut w, 3, 0, 500);
    let factory = spawn(&mut w, 4, 0, 600);
    let enemy = spawn(&mut w, 0, 1, 700);
    let frame = spawn(&mut w, 0, 0, 800);
    w.unit_mut(frame).unwrap().build.frame = Some(Frame::new(10));
    let dying = spawn(&mut w, 0, 0, 900);
    w.unit_mut(dying).unwrap().combat.dying = Some(tak_sim::combat::Dying {
        since: 0,
        severity: 100,
        kind: 0,
        done: false,
    });
    let v = view(&w);
    assert_eq!(v.unit(healer).unwrap().builder, Some(BuilderKind::Mobile));
    assert!(v.in_category(healer, tak_input::Category::Builder));
    assert!(v.build_options(healer).is_empty());
    assert_eq!(v.build_options(builder), vec![UnitDefId(5)]);
    let all = UnitSet::from(vec![
        healer, builder, nonbuilder, zero, factory, enemy, frame, dying,
    ]);
    assert_eq!(healers_for(&v, &all), UnitSet::from(vec![healer, builder]));
    for id in [
        nonbuilder,
        zero,
        factory,
        enemy,
        frame,
        dying,
        UnitId::new(0, 99),
    ] {
        assert!(!v.can_heal(id), "{id:?}");
    }
    let mut m = selected(&v, healer);
    m.handle(
        &InputEvent::new(
            0,
            EventKind::KeyDown {
                key: Key::new("f6"),
            },
        ),
        &v,
        Tick(0),
    );
    m.handle(
        &InputEvent::new(
            1,
            EventKind::KeyUp {
                key: Key::new("f6"),
            },
        ),
        &v,
        Tick(0),
    );
    assert!(m.wheel().is_none(), "Heal does not invent a build menu");
    m.handle(
        &InputEvent::new(
            2,
            EventKind::KeyDown {
                key: Key::new("f7"),
            },
        ),
        &v,
        Tick(0),
    );
    assert_eq!(m.wheel().unwrap().slots[0], Action::HealMode);
    let mut m = selected(&v, zero);
    m.handle(
        &InputEvent::new(
            2,
            EventKind::KeyDown {
                key: Key::new("f7"),
            },
        ),
        &v,
        Tick(0),
    );
    assert_eq!(
        m.wheel().unwrap().slots[0],
        BLANK,
        "fixed slots do not shift"
    );
}

#[test]
fn mixed_empty_menu_healer_and_builder_keep_both_build_and_heal_controls() {
    let mut w = world();
    let healer = spawn(&mut w, 0, 0, 200);
    let builder = spawn(&mut w, 1, 0, 400);
    let v = view(&w);
    let mut m = selected(&v, healer);
    m.perform(Action::SelectAll { scope: Scope::Map }, &v, Tick(0));
    m.handle(
        &InputEvent::new(
            1,
            EventKind::KeyDown {
                key: Key::new("f6"),
            },
        ),
        &v,
        Tick(0),
    );
    assert_eq!(
        m.wheel().unwrap().slots[0],
        Action::BuildOption { index: 0 }
    );
    m.perform(Action::Cancel, &v, Tick(0));
    m.perform(Action::BuildOption { index: 0 }, &v, Tick(0));
    assert_eq!(m.mode(), Mode::Build(UnitDefId(5)));
    m.perform(Action::HealMode, &v, Tick(0));
    assert_eq!(m.mode(), Mode::Heal);
    assert_eq!(
        healers_for(&v, m.selection()),
        UnitSet::from(vec![healer, builder])
    );
}

#[test]
fn panel_keyboard_and_hint_paths_enter_distinct_heal_mode_and_reject_invalid_targets() {
    let mut w = world();
    let healer = spawn(&mut w, 0, 0, 200);
    let target = spawn(&mut w, 2, 0, 400);
    let enemy = spawn(&mut w, 0, 1, 600);
    let v = view(&w);
    let mut m = selected(&v, healer);
    m.perform(hud::PANEL_ACTIONS[6], &v, Tick(0));
    assert_eq!(m.mode(), Mode::Heal);
    assert_eq!(feel::action_label(&Action::HealMode), "Heal / repair");
    assert!(
        feel::hints(&m, feel::HintCx::default())
            .iter()
            .any(|h| h.text == "heal / repair")
    );
    m.warp_cursor(v.unit(target).unwrap().pos);
    assert_eq!(m.cursor_kind(&v), CursorKind::Heal);
    assert_ne!(
        hud::cursor_seq(CursorKind::Heal),
        hud::cursor_seq(CursorKind::Guard)
    );
    assert_ne!(
        hud::cursor_seq(CursorKind::Heal),
        hud::cursor_seq(CursorKind::Cast)
    );
    for id in [healer, enemy] {
        let hovered = v.unit(id).unwrap();
        m.warp_cursor(hovered.pos);
        assert_eq!(m.cursor_kind(&v), CursorKind::Invalid);
        assert_eq!(
            resolve_context(
                &v,
                m.selection(),
                hovered.pos,
                Some(&hovered),
                Mode::Heal,
                QueueMode::Replace
            ),
            None
        );
    }
    let hovered = v.unit(target).unwrap();
    for queue in [QueueMode::Replace, QueueMode::Append] {
        assert_eq!(
            resolve_context(
                &v,
                m.selection(),
                hovered.pos,
                Some(&hovered),
                Mode::Heal,
                queue
            ),
            Some(SimCommand::Heal {
                units: UnitSet::single(healer),
                target,
                queue
            })
        );
    }
    m.perform(Action::Cancel, &v, Tick(0));
    m.handle(
        &InputEvent::new(1, EventKind::KeyDown { key: Key::new("h") }),
        &v,
        Tick(0),
    );
    assert_eq!(m.mode(), Mode::Heal);
    let hints = feel::hints_classic(
        &Mapper::new(Profile::default_combined()),
        feel::HintCx {
            has_units: true,
            has_healer: true,
            ..Default::default()
        },
    );
    assert_eq!(hints.len(), 9, "the Heal hint must not displace squad help");
    assert!(
        hints
            .iter()
            .any(|h| h.text == "heal / repair" && h.key == feel::HintKey::Text("H".into()))
    );
    let hints = feel::hints_classic(
        &Mapper::new(Profile::default_combined()),
        feel::HintCx {
            has_units: true,
            ..Default::default()
        },
    );
    assert!(!hints.iter().any(|h| h.text == "heal / repair"));
    // Selection operations remain available and do not grant enemy actors.
    m.perform(Action::SelectAll { scope: Scope::Map }, &v, Tick(0));
    assert!(!m.selection().contains(enemy));
}

#[test]
fn heal_keeps_original_art_slot_and_acknowledges_through_existing_sound() {
    assert_eq!(hud::PANEL_BUTTONS[6], "RepairButton");
    assert_eq!(hud::PANEL_ACTIONS[6], Action::HealMode);
    assert_eq!(hud::PANEL_NAMES[6], "Heal");
    assert!(hud::CURSOR_KINDS.contains(&CursorKind::Heal));
    for (index, action) in hud::PANEL_ACTIONS[7..10].iter().enumerate() {
        assert_eq!(*action, Action::CastSpell { index: index as u8 });
    }
    for (index, state) in [
        tak_commands::FireState::FireAtWill,
        tak_commands::FireState::Defensive,
        tak_commands::FireState::HoldFire,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            hud::PANEL_ACTIONS[index + 10],
            Action::SetFireState { state }
        );
    }
    assert_eq!(
        hud::order_sound(&SimCommand::Heal {
            units: UnitSet::single(UnitId::new(0, 0)),
            target: UnitId::new(1, 0),
            queue: QueueMode::Replace
        }),
        Some("GUARD")
    );
    assert_eq!(hud::button_state(true, true, false), 3);
    assert_eq!(hud::button_state(false, true, true), 0);
}

#[test]
fn heal_status_and_goal_follow_the_target_and_handle_stale_generations() {
    let mut w = world();
    let healer = spawn(&mut w, 0, 0, 200);
    let target = spawn(&mut w, 2, 0, 400);
    let structure = spawn(&mut w, 5, 0, 600);
    let text = |w: &World| {
        hud::order_text(w, w.unit(healer).unwrap(), |d| {
            w.defs().get(d).unwrap().name.clone()
        })
    };
    assert_eq!(text(&w), "Idle");
    let order = Order::Heal { target };
    w.unit_mut(healer).unwrap().orders.push_back(order);
    assert_eq!(text(&w), "Healing nonbuilder");
    assert_eq!(
        hud::order_goal(&w, &order),
        Some(w.unit(target).unwrap().pos)
    );
    w.unit_mut(target).unwrap().pos = Vec2::from_ints(500, 300);
    assert_eq!(hud::order_goal(&w, &order), Some(Vec2::from_ints(500, 300)));
    assert_ne!(
        hud::order_rgba(&order),
        hud::order_rgba(&Order::Guard { target })
    );
    w.unit_mut(healer).unwrap().orders[0] = Order::Heal { target: structure };
    assert_eq!(text(&w), "Repairing structure");
    w.unit_mut(structure).unwrap().build.frame = Some(Frame::new(10));
    assert_eq!(text(&w), "Assisting structure");
    w.unit_mut(healer)
        .unwrap()
        .orders
        .push_back(Order::Move { to: Vec2::ZERO });
    assert_eq!(text(&w), "Assisting structure (+1)");
    let stale = Order::Heal {
        target: UnitId::new(target.index, 99),
    };
    w.unit_mut(healer).unwrap().orders[0] = stale;
    assert_eq!(hud::order_goal(&w, &stale), None);
    assert_eq!(text(&w), "Healing ? (+1)");

    // The sim owns a temporary front Move while Heal walks into range. It is
    // not a player order and must not replace Heal's status/colour/queue count.
    let mut w = world();
    let healer = spawn(&mut w, 0, 0, 200);
    let frame = spawn(&mut w, 2, 0, 800);
    w.unit_mut(frame).unwrap().build.frame = Some(Frame::new(10));
    tak_sim::construction::apply_heal_command(
        &mut w,
        PlayerId(0),
        &SimCommand::Heal {
            units: UnitSet::single(healer),
            target: frame,
            queue: QueueMode::Replace,
        },
    );
    tak_sim::construction::tick(&mut w, &mut tak_sim::NoScripts);
    let u = w.unit(healer).unwrap();
    assert!(matches!(u.orders.front(), Some(Order::Move { .. })));
    assert_eq!(u.orders.get(1), Some(&Order::Heal { target: frame }));
    assert_eq!(
        hud::orders_for_display(u).copied().collect::<Vec<_>>(),
        vec![Order::Heal { target: frame }]
    );
    assert_eq!(
        hud::order_text(&w, u, |d| w.defs().get(d).unwrap().name.clone()),
        "Assisting nonbuilder"
    );
}

#[test]
fn installed_heal_cursor_and_repair_button_sequences_decode() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("Heal cursor/button art") else {
        return;
    };
    let vfs = tak_client::game::open_vfs(&dir).unwrap();
    for (path, name) in [
        ("anims/cursors.gaf", hud::cursor_seq(CursorKind::Heal)),
        ("anims/igcommonbuttons.gaf", hud::PANEL_BUTTONS[6]),
    ] {
        let gaf = tak_formats::gaf::Gaf::parse(&vfs.read(path).unwrap()).unwrap();
        let entry = gaf.entry(name).unwrap_or_else(|| {
            panic!(
                "missing {name} in {path}; sequences: {:?}",
                gaf.entries.iter().map(|e| &e.name).collect::<Vec<_>>()
            )
        });
        assert!(!entry.frames.is_empty());
        assert!(
            entry
                .frames
                .iter()
                .all(|f| f.frame.width > 0 && f.frame.height > 0)
        );
    }
}
