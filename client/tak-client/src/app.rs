//! The Bevy app: sim driver, terrain and unit rendering, camera, input
//! adapter, HUD and the screenshot verification mode.
//!
//! The sim runs at exactly 30 ticks per second on a manual accumulator (or,
//! in screenshot mode, as fast as possible up to each requested tick).
//! Rendering interpolates between the last two snapshots with floats on the
//! render side only.

use std::collections::BTreeMap;
use std::f32::consts::FRAC_PI_2;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::Tonemapping;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::window::{PrimaryWindow, WindowResolution};

use tak_commands::{ClientCommand, PlayerId, UnitId};
use tak_input::{
    Action, Axis as InAxis, Button as InButton, EventKind, InputEvent, Key, Mapper, Mode, Output,
    Profile, WheelKind,
};
use tak_vfs::Vfs;

use crate::cli::{Args, Focus, parse_focus};
use crate::game::FeatureArt;
use crate::game::{Session, Snapshot, UnitSnap, angle_rad, lerp, lerp_angle, lerp_pose};
use crate::models::{BuiltModel, TextureIndex, load_model};
use crate::sprites::{SpriteSeq, SpriteStore, billboard, lift_for};
use crate::terrain::{self, TerrainData};
use crate::view::{MinimapGeom, SimView, ViewGeom, f, pick_radius};

/// Seconds per sim tick.
const TICK_DT: f32 = 1.0 / 30.0;

/// Install data the renderer reads lazily (not `Send`: kept on the main
/// thread).
pub struct Content {
    vfs: Vfs,
    tex: TextureIndex,
    sprites: SpriteStore,
    /// The map's OTA `kingdom` (picks the feature palette).
    kingdom: Option<String>,
}

/// The match, its two most recent snapshots and the tick accumulator.
pub struct SimRes {
    /// The match.
    pub session: Session,
    prev: Snapshot,
    curr: Snapshot,
    acc: f32,
    /// Ticks per wall second multiplier (game speed).
    speed: f32,
    paused: bool,
}

impl SimRes {
    fn step(&mut self) {
        self.session.step();
        self.prev = std::mem::replace(&mut self.curr, self.session.snapshot());
    }

    fn alpha(&self) -> f32 {
        (self.acc / TICK_DT).clamp(0.0, 1.0)
    }
}

/// Camera state.
#[derive(Resource, Clone, Copy)]
struct CamState {
    geom: ViewGeom,
}

/// Terrain heights for picking and placement.
#[derive(Resource)]
struct Ground(Arc<TerrainData>);

/// Input adapter state.
#[derive(Resource)]
struct InputRes {
    mapper: Mapper,
    last_axes: BTreeMap<InAxis, i16>,
    last_pointer: Option<(i32, i32)>,
}

/// Screenshot plan (verification mode).
#[derive(Resource)]
struct ShotPlan {
    shots: Vec<(u32, PathBuf)>,
    next: usize,
    wait: u32,
    saved: Arc<AtomicUsize>,
    requested: usize,
    focus: Focus,
    select: Vec<u32>,
    exit_wait: u32,
}

/// Per-model render assets.
struct ModelAssets {
    pieces: Vec<PieceAsset>,
    material: Handle<StandardMaterial>,
    /// Lowest and highest model-space height at rest pose.
    y_range: (f32, f32),
}

struct PieceAsset {
    parent: Option<usize>,
    offset: Vec3,
    name: String,
    mesh: Option<Handle<Mesh>>,
}

#[derive(Resource, Default)]
struct ModelCache {
    models: BTreeMap<(String, usize), Option<Arc<ModelAssets>>>,
}

/// A rendered unit.
#[derive(Component)]
struct UnitVis {
    id: UnitId,
    /// Per model piece: (piece entity, mesh entity, COB piece, rest offset).
    pieces: Vec<(Entity, Option<Entity>, Option<usize>, Vec3)>,
    /// Set while the unit is under construction.
    build: Option<BuildVis>,
}

#[derive(Resource, Default)]
struct UnitEntities(BTreeMap<UnitId, Entity>);

/// Rendered live sim features (gameplay map features, corpses, wrecks).
#[derive(Resource, Default)]
struct FeatureEntities(BTreeMap<tak_sim::features::FeatureId, Entity>);

#[derive(Component)]
struct HudTop;
#[derive(Component)]
struct HudPanel;
#[derive(Component)]
struct HudWheel;

/// Build and run the app.
pub fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let vfs = crate::game::open_vfs(&args.data)?;
    let log = match &args.log {
        Some(p) => tak_commands::CommandLog::from_json(&std::fs::read_to_string(p)?)
            .map_err(|e| format!("{}: {e}", p.display()))?,
        None => tak_commands::CommandLog::new(),
    };
    let t0 = std::time::Instant::now();
    let mut session = Session::load(&vfs, &args.map, &args.players, args.world_seed, log)?;
    if args.demo {
        session.log = crate::game::demo_log(&session);
    }
    let ground = terrain::load(&vfs, &args.map, session.map.palette.as_deref())?;
    let tex = TextureIndex::load(&vfs);
    eprintln!(
        "loaded {} ({}x{} cells, {} defs, {} textures + {} in anims/, {} missing tile pieces) in {:.2}s",
        args.map,
        ground.cells.0,
        ground.cells.1,
        session.defs.len(),
        tex.len(),
        tex.anim_len(),
        ground.missing,
        t0.elapsed().as_secs_f32()
    );
    let kingdom = session.map.kingdom.clone();
    let (w, h) = args.window_size();
    let focus = parse_focus(&args.focus).ok_or("bad --focus")?;
    let snap = session.snapshot();
    let mut profile = Profile::default_controller();
    let mk = Profile::default_mouse_keyboard();
    profile.name = "controller+mouse_keyboard".into();
    profile.bindings.extend(mk.bindings);
    profile
        .validate()
        .map_err(|e| format!("input profile: {e}"))?;
    let mapper = Mapper::new(profile);

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("Oxidized Annihilation - {}", args.map),
            resolution: WindowResolution::new(w, h).with_scale_factor_override(1.0),
            ..default()
        }),
        ..default()
    }));
    // The construction shader must be registered before its material
    // plugin asks for it.
    bevy::asset::embedded_asset!(app, "build_clip.wesl");
    let shader: Handle<bevy::shader::Shader> =
        bevy::asset::load_embedded_asset!(&app, "build_clip.wesl");
    let _ = BUILD_CLIP_SHADER.set(shader);
    app.add_plugins(MaterialPlugin::<BuildMat>::default())
        .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.07)))
        .insert_non_send(Content {
            vfs,
            tex,
            sprites: SpriteStore::default(),
            kingdom,
        })
        .insert_non_send(SimRes {
            session,
            prev: snap.clone(),
            curr: snap,
            acc: 0.0,
            speed: 1.0,
            paused: false,
        })
        .insert_resource(Ground(Arc::new(ground)))
        .insert_resource(CamState {
            geom: ViewGeom {
                target: [0.0, 0.0],
                zoom: args.zoom,
                size: [w as f32, h as f32],
            },
        })
        .insert_resource(InputRes {
            mapper,
            last_axes: BTreeMap::new(),
            last_pointer: None,
        })
        .init_resource::<ModelCache>()
        .init_resource::<UnitEntities>()
        .init_resource::<FeatureEntities>()
        .init_resource::<SpriteCache>()
        .init_resource::<UnitTops>()
        .init_resource::<FxState>()
        .init_resource::<MinimapState>()
        .add_systems(
            Startup,
            (
                setup_scene,
                setup_hud,
                setup_gizmos,
                spawn_scenery,
                setup_minimap,
            ),
        )
        .add_systems(
            Update,
            (
                shot_driver,
                input_system,
                sim_driver,
                camera_system,
                units_system,
                features_system,
                effects_system,
                animate_sprites,
                showcase_system,
                hud_system,
                minimap_system,
                gizmo_system,
                fps_report,
            )
                .chain(),
        );
    if let Some(path) = &args.screenshot {
        let shots = args
            .at_tick
            .iter()
            .map(|&t| (t, PathBuf::from(path.replace("{tick}", &t.to_string()))))
            .collect();
        app.insert_resource(ShotPlan {
            shots,
            next: 0,
            wait: 0,
            saved: Arc::new(AtomicUsize::new(0)),
            requested: 0,
            focus,
            select: args.select.clone(),
            exit_wait: 0,
        });
    } else {
        app.insert_resource(StartFocus(focus));
        app.add_systems(Startup, initial_focus.after(setup_scene));
    }
    if !args.showcase.is_empty() {
        app.insert_resource(Showcase(args.showcase.clone()));
    }
    app.insert_resource(Outputs {
        record: args.record.clone(),
        hashes: args.hashes.clone(),
        done: false,
    })
    .add_systems(Last, write_outputs);
    app.run();
    Ok(())
}

#[derive(Resource)]
struct StartFocus(Focus);

/// Where to write the log and hashes when the app exits.
#[derive(Resource)]
struct Outputs {
    record: Option<PathBuf>,
    hashes: Option<PathBuf>,
    done: bool,
}

/// On exit (an `AppExit` message, or the window closing), write the full
/// command log and the per-tick hashes.
fn write_outputs(
    mut exits: MessageReader<AppExit>,
    mut closed: MessageReader<bevy::window::WindowCloseRequested>,
    windows: Query<(), With<PrimaryWindow>>,
    sim: NonSend<SimRes>,
    mut out: ResMut<Outputs>,
) {
    let exiting = exits.read().count() > 0 || closed.read().count() > 0 || windows.is_empty();
    if !exiting || out.done {
        return;
    }
    out.done = true;
    let s = &sim.session;
    if let Some(p) = &out.record {
        match std::fs::write(p, s.log.to_json_pretty()) {
            Ok(()) => eprintln!("wrote {} commands to {}", s.log.len(), p.display()),
            Err(e) => eprintln!("{}: {e}", p.display()),
        }
    }
    if let Some(p) = &out.hashes {
        match std::fs::write(p, tak_headless::format_hashes(&s.hashes)) {
            Ok(()) => eprintln!("wrote {} hashes to {}", s.hashes.len(), p.display()),
            Err(e) => eprintln!("{}: {e}", p.display()),
        }
    }
    if let Some(h) = s.last_hash() {
        eprintln!("final tick {} hash {h:016x}", s.tick().0.saturating_sub(1));
    }
}

fn initial_focus(sim: NonSend<SimRes>, focus: Res<StartFocus>, mut cam: ResMut<CamState>) {
    // Before tick 0 runs no unit exists: fall back to start positions.
    if let Some(p) = focus_point(&sim.session, focus.0) {
        cam.geom.target = p;
    }
}

/// Camera target for a focus: the point lands at the screen centre at its
/// terrain height (the target itself lives on the height-0 plane).
fn focus_point(s: &Session, focus: Focus) -> Option<[f32; 2]> {
    let w = s.sim.world();
    let pos = match focus {
        Focus::Point(x, y) => tak_math::Vec2::new(
            tak_math::Fx::from_raw((x * 65536.0) as i32),
            tak_math::Fx::from_raw((y * 65536.0) as i32),
        ),
        Focus::Monarch(n) => {
            // Before tick 0 runs no unit exists: use the start position.
            let p = w.players().get(usize::from(n))?;
            p.monarch
                .and_then(|id| w.unit(id))
                .map_or(p.start, |u| u.pos)
        }
        Focus::Unit(slot) => w
            .units()
            .find(|(id, _)| id.index == slot)
            .map(|(_, u)| u.pos)?,
    };
    let h = f(w.map().height_at(pos));
    Some([f(pos.x), f(pos.y) - crate::view::SHEAR * h])
}

fn rgba_image(w: u32, h: u32, data: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

fn setup_scene(
    mut commands: Commands,
    ground: Res<Ground>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        bevy::core_pipeline::tonemapping::DebandDither::Disabled,
        Projection::custom(ObliqueProjection {
            geom: ViewGeom {
                target: [0.0, 0.0],
                zoom: 1.0,
                size: [1280.0, 720.0],
            },
        }),
        Transform::default(),
    ));
    let t = &ground.0;
    for chunk in &t.chunks {
        let cm = terrain::chunk_mesh(t, chunk);
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, cm.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, cm.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, cm.uvs);
        mesh.insert_indices(Indices::U32(cm.indices));
        let (iw, ih) = chunk.image_size();
        let img = rgba_image(iw, ih, chunk.rgba.clone());
        let mat = materials.add(StandardMaterial {
            base_color_texture: Some(images.add(img)),
            unlit: true,
            ..default()
        });
        commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat)));
    }
    // Sea level: a translucent plane over the whole map.
    let (mw, mh) = t.size();
    let water = materials.add(StandardMaterial {
        base_color: Color::srgba(0.08, 0.22, 0.42, 0.35),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(mw, mh))),
        MeshMaterial3d(water),
        Transform::from_xyz(mw / 2.0, t.sea_level, mh / 2.0),
    ));
}

fn setup_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (cfg, _) = store.config_mut::<DefaultGizmoConfigGroup>();
    cfg.depth_bias = -1.0;
    cfg.line.width = 2.0;
}

fn hud_text(size: f32) -> (TextFont, TextColor) {
    (
        TextFont {
            font_size: bevy::text::FontSize::Px(size),
            ..default()
        },
        TextColor(Color::srgb(0.95, 0.92, 0.8)),
    )
}

fn setup_hud(mut commands: Commands) {
    let bg = BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6));
    commands.spawn((
        Text::new(""),
        hud_text(16.0),
        bg,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(8.0),
            top: Val::Px(6.0),
            padding: UiRect::all(Val::Px(4.0)),
            ..default()
        },
        HudTop,
    ));
    commands.spawn((
        Text::new(""),
        hud_text(15.0),
        bg,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(8.0),
            bottom: Val::Px(8.0),
            padding: UiRect::all(Val::Px(4.0)),
            ..default()
        },
        HudPanel,
    ));
    commands.spawn((
        Text::new(""),
        hud_text(15.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            ..default()
        },
        HudWheel,
    ));
}

/// Screenshot mode: run the sim to each requested tick, let the frame
/// settle, capture, and exit after the last one.
fn shot_driver(
    mut commands: Commands,
    plan: Option<ResMut<ShotPlan>>,
    mut sim: NonSendMut<SimRes>,
    mut cam: ResMut<CamState>,
    mut input: ResMut<InputRes>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut plan) = plan else { return };
    sim.paused = true;
    if plan.next >= plan.shots.len() {
        if plan.saved.load(Ordering::SeqCst) >= plan.requested {
            plan.exit_wait += 1;
            if plan.exit_wait > 2 {
                exit.write(AppExit::Success);
            }
        }
        return;
    }
    if plan.saved.load(Ordering::SeqCst) < plan.requested {
        return;
    }
    let (tick, path) = plan.shots[plan.next].clone();
    if plan.wait == 0 {
        let t0 = std::time::Instant::now();
        let start = sim.session.tick().0;
        while sim.session.tick().0 < tick {
            sim.step();
        }
        sim.acc = TICK_DT; // alpha = 1: draw the latest state
        if let Some(p) = focus_point(&sim.session, plan.focus) {
            cam.geom.target = p;
        }
        if !plan.select.is_empty() {
            input.mapper.set_camera(cam.geom.input_camera());
            select_units(&mut input.mapper, &sim, &plan.select);
        }
        for (id, u) in sim.session.sim.world().units() {
            eprintln!(
                "  unit {}:{} {} p{} at ({:.0}, {:.0}) hp {}/{} build_left {}% orders {:?}",
                id.index,
                id.generation,
                sim.session.def(u.def).map_or("?", |d| d.unitname.as_str()),
                u.owner.0,
                f(u.pos.x),
                f(u.pos.y),
                u.combat.health,
                u.combat.max_health,
                u.build.percent_left(),
                u.orders.iter().take(2).collect::<Vec<_>>()
            );
        }
        for imp in sim.session.sim.world().combat.impacts.iter().rev().take(3) {
            let key = crate::game::weapon_key(imp.weapon);
            eprintln!(
                "  impact tick {} weapon {:?} at ({:.0}, {:.0}) by {:?}: {:?}",
                imp.tick,
                key,
                f(imp.point.x),
                f(imp.point.y),
                imp.shooter,
                sim.session.weapon_art.get(&key)
            );
        }
        let fs = &sim.session.sim.world().features;
        eprintln!(
            "  features {} (corpses spawned {})",
            fs.iter().count(),
            fs.corpses_spawned
        );
        eprintln!(
            "ran ticks {start}..{} in {:.2}s; hash {:016x}",
            sim.session.tick().0,
            t0.elapsed().as_secs_f32(),
            sim.session.last_hash().unwrap_or(0)
        );
    }
    plan.wait += 1;
    // Give assets a few frames to upload before capturing.
    if plan.wait >= if plan.requested == 0 { 150 } else { 20 } {
        let saved = plan.saved.clone();
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()))
            .observe(move |_: On<ScreenshotCaptured>| {
                saved.fetch_add(1, Ordering::SeqCst);
            });
        eprintln!("screenshot tick {tick} -> {}", path.display());
        plan.requested += 1;
        plan.next += 1;
        plan.wait = 0;
    }
}

/// Select units by slot through the mapper's own selection path (a click
/// on each unit, the later ones additive), so rings and HUD come from the
/// mapper's state exactly as in play.
fn select_units(mapper: &mut Mapper, sim: &SimRes, slots: &[u32]) {
    let world = sim.session.sim.world();
    let view = SimView {
        world,
        local: sim.session.local,
    };
    let tick = sim.session.tick();
    let units: Vec<_> = world
        .units()
        .filter(|(id, _)| slots.contains(&id.index))
        .map(|(_, u)| u.pos)
        .collect();
    let cam = *mapper.camera();
    for (i, pos) in units.into_iter().enumerate() {
        let s = cam.world_to_screen(pos);
        let (x, y) = (s.x.to_int_floor(), s.y.to_int_floor());
        let t = 1_000_000 + i as u32 * 1000;
        let mut evs = vec![InputEvent::new(t, EventKind::PointerMove { x, y })];
        if i > 0 {
            evs.push(InputEvent::new(
                t,
                EventKind::KeyDown {
                    key: Key::new("shift"),
                },
            ));
        }
        evs.push(InputEvent::new(
            t + 1,
            EventKind::ButtonDown {
                button: InButton::MouseLeft,
            },
        ));
        evs.push(InputEvent::new(
            t + 2,
            EventKind::ButtonUp {
                button: InButton::MouseLeft,
            },
        ));
        if i > 0 {
            evs.push(InputEvent::new(
                t + 3,
                EventKind::KeyUp {
                    key: Key::new("shift"),
                },
            ));
        }
        for e in &evs {
            // Selection only: drop any sim orders (there are none for clicks).
            let _ = mapper.handle(e, &view, tick);
        }
    }
}

fn key_name(k: KeyCode) -> Option<String> {
    use KeyCode::*;
    let s = match k {
        ShiftLeft | ShiftRight => "shift",
        ControlLeft | ControlRight => "ctrl",
        AltLeft | AltRight => "alt",
        Escape => "escape",
        Space => "space",
        Home => "home",
        Period => "period",
        Enter => "enter",
        Tab => "tab",
        F1 => "f1",
        F2 => "f2",
        F3 => "f3",
        F4 => "f4",
        F5 => "f5",
        F6 => "f6",
        F7 => "f7",
        F8 => "f8",
        F9 => "f9",
        F10 => "f10",
        F11 => "f11",
        F12 => "f12",
        Digit0 => "0",
        Digit1 => "1",
        Digit2 => "2",
        Digit3 => "3",
        Digit4 => "4",
        Digit5 => "5",
        Digit6 => "6",
        Digit7 => "7",
        Digit8 => "8",
        Digit9 => "9",
        KeyA => "a",
        KeyB => "b",
        KeyC => "c",
        KeyD => "d",
        KeyE => "e",
        KeyF => "f",
        KeyG => "g",
        KeyH => "h",
        KeyI => "i",
        KeyJ => "j",
        KeyK => "k",
        KeyL => "l",
        KeyM => "m",
        KeyN => "n",
        KeyO => "o",
        KeyP => "p",
        KeyQ => "q",
        KeyR => "r",
        KeyS => "s",
        KeyT => "t",
        KeyU => "u",
        KeyV => "v",
        KeyW => "w",
        KeyX => "x",
        KeyY => "y",
        KeyZ => "z",
        _ => return None,
    };
    Some(s.to_string())
}

const PAD_BUTTONS: [(GamepadButton, InButton); 18] = [
    (GamepadButton::South, InButton::A),
    (GamepadButton::East, InButton::B),
    (GamepadButton::West, InButton::X),
    (GamepadButton::North, InButton::Y),
    (GamepadButton::LeftTrigger, InButton::Lb),
    (GamepadButton::RightTrigger, InButton::Rb),
    (GamepadButton::LeftTrigger2, InButton::Lt),
    (GamepadButton::RightTrigger2, InButton::Rt),
    (GamepadButton::LeftThumb, InButton::L3),
    (GamepadButton::RightThumb, InButton::R3),
    (GamepadButton::DPadUp, InButton::DpadUp),
    (GamepadButton::DPadDown, InButton::DpadDown),
    (GamepadButton::DPadLeft, InButton::DpadLeft),
    (GamepadButton::DPadRight, InButton::DpadRight),
    (GamepadButton::Select, InButton::View),
    (GamepadButton::Start, InButton::Menu),
    (GamepadButton::Mode, InButton::Menu),
    (GamepadButton::C, InButton::Menu),
];

/// Bevy input to `tak-input` events; mapper outputs to the sim's log and
/// the camera.
fn input_system(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<&Window, With<PrimaryWindow>>,
    pads: Query<&Gamepad>,
    ground: Res<Ground>,
    plan: Option<Res<ShotPlan>>,
    mut sim: NonSendMut<SimRes>,
    mut cam: ResMut<CamState>,
    mut input: ResMut<InputRes>,
    mut minimap: ResMut<MinimapState>,
) {
    if let Ok(win) = windows.single() {
        cam.geom.size = [win.width(), win.height()];
    }
    let geom = cam.geom;
    input.mapper.set_camera(geom.input_camera());
    if plan.is_some() {
        return;
    }
    let t = time.elapsed().as_millis() as u32;
    let mut events = Vec::new();
    // Pointer: the terrain point under the mouse, as a virtual pixel.
    if let Some(pos) = windows.single().ok().and_then(|w| w.cursor_position()) {
        let (o, d) = geom.ray(pos.x, pos.y);
        if let Some(hit) = ground.0.raycast(o, d) {
            let v = geom.virtual_px(hit[0], hit[2]);
            if input.last_pointer != Some(v) {
                input.last_pointer = Some(v);
                events.push(EventKind::PointerMove { x: v.0, y: v.1 });
            }
        }
    }
    // Minimap: a left click or drag on it moves the camera and is not
    // passed to the mapper (so it neither selects nor deselects).
    let mouse_pos = windows.single().ok().and_then(|w| w.cursor_position());
    let mm_geom = minimap.geom(geom.size);
    if let (Some(pos), Some(mg)) = (mouse_pos, mm_geom)
        && mouse.just_pressed(MouseButton::Left)
        && mg.contains(pos.x, pos.y)
    {
        minimap.mouse_drag = true;
    }
    if minimap.mouse_drag {
        if let (Some(pos), Some(mg)) = (mouse_pos, mm_geom) {
            cam.geom.target = mg.target_at(pos.x, pos.y);
        }
        if !mouse.pressed(MouseButton::Left) {
            minimap.mouse_drag = false;
        }
    }
    let swallow_left =
        minimap.mouse_drag || mouse.just_released(MouseButton::Left) && minimap.swallow_release;
    minimap.swallow_release = minimap.mouse_drag;
    for (mb, b) in [
        (MouseButton::Left, InButton::MouseLeft),
        (MouseButton::Right, InButton::MouseRight),
        (MouseButton::Middle, InButton::MouseMiddle),
    ] {
        if mb == MouseButton::Left && swallow_left {
            continue;
        }
        if mouse.just_pressed(mb) {
            events.push(EventKind::ButtonDown { button: b });
        }
        if mouse.just_released(mb) {
            events.push(EventKind::ButtonUp { button: b });
        }
    }
    for k in keys.get_just_pressed() {
        if let Some(n) = key_name(*k) {
            events.push(EventKind::KeyDown { key: Key(n) });
        }
    }
    for k in keys.get_just_released() {
        if let Some(n) = key_name(*k) {
            events.push(EventKind::KeyUp { key: Key(n) });
        }
    }
    // One pad drives the mapper.
    if let Some(pad) = pads.iter().next() {
        // A with the virtual cursor over the minimap jumps the camera there.
        let c = input.mapper.cursor_world();
        let (cx, cz) = (f(c.x), f(c.y));
        let csp = geom.to_screen([cx, ground.0.height_at(cx, cz), cz]);
        let mut swallow_a = false;
        if let Some(mg) = mm_geom
            && mg.contains(csp[0], csp[1])
            && pad.just_pressed(GamepadButton::South)
        {
            cam.geom.target = mg.target_at(csp[0], csp[1]);
            minimap.pad_swallow = true;
        }
        if minimap.pad_swallow {
            swallow_a = true;
            if pad.just_released(GamepadButton::South) || !pad.pressed(GamepadButton::South) {
                minimap.pad_swallow = false;
            }
        }
        for (gb, b) in PAD_BUTTONS {
            if gb == GamepadButton::South && swallow_a {
                continue;
            }
            if pad.just_pressed(gb) {
                events.push(EventKind::ButtonDown { button: b });
            }
            if pad.just_released(gb) {
                events.push(EventKind::ButtonUp { button: b });
            }
        }
        let l = pad.left_stick();
        let r = pad.right_stick();
        for (axis, v) in [
            (InAxis::LeftX, l.x),
            (InAxis::LeftY, l.y),
            (InAxis::RightX, r.x),
            (InAxis::RightY, r.y),
        ] {
            let q = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
            if input.last_axes.get(&axis) != Some(&q) {
                input.last_axes.insert(axis, q);
                events.push(EventKind::Axis { axis, value: q });
            }
        }
    }
    events.push(EventKind::Frame);

    let tick = sim.session.tick();
    let mut outs = Vec::new();
    {
        let view = SimView {
            world: sim.session.sim.world(),
            local: sim.session.local,
        };
        for kind in events {
            outs.extend(input.mapper.handle(&InputEvent::new(t, kind), &view, tick));
        }
    }
    for o in outs {
        match o {
            Output::Sim { command, .. } => {
                let at = sim.session.submit(command);
                debug!("order for tick {}", at.0);
            }
            Output::Client { command } => match command {
                ClientCommand::CenterOnMonarch => {
                    if let Some(p) = focus_point(&sim.session, Focus::Monarch(sim.session.local.0))
                    {
                        cam.geom.target = p;
                    }
                }
                ClientCommand::CycleIdleBuilder => {
                    if let Some(id) = input.mapper.selection().iter().next()
                        && let Some(u) = sim.session.sim.world().unit(*id)
                    {
                        cam.geom.target = [f(u.pos.x), f(u.pos.y)];
                    }
                }
                _ => {}
            },
        }
    }
    // Camera: mapper pan (right stick), arrow keys, mouse wheel zoom.
    let pan = input.mapper.take_camera_pan();
    cam.geom.target[0] += f(pan.x);
    cam.geom.target[1] += f(pan.y);
    let dt = time.delta_secs();
    let speed = 900.0 * cam.geom.zoom * dt;
    if keys.pressed(KeyCode::ArrowLeft) {
        cam.geom.target[0] -= speed;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        cam.geom.target[0] += speed;
    }
    if keys.pressed(KeyCode::ArrowUp) {
        cam.geom.target[1] -= speed;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        cam.geom.target[1] += speed;
    }
    for ev in wheel.read() {
        let steps = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y / 40.0,
        };
        cam.geom.zoom = (cam.geom.zoom * 0.9f32.powf(steps)).clamp(0.15, 4.0);
    }
    // Game speed and pause (client only: changes ticks per wall second).
    if keys.just_pressed(KeyCode::Pause)
        || keys.just_pressed(KeyCode::KeyP) && keys.pressed(KeyCode::ControlLeft)
    {
        sim.paused = !sim.paused;
    }
    if keys.just_pressed(KeyCode::Equal) {
        sim.speed = (sim.speed * 2.0).min(8.0);
    }
    if keys.just_pressed(KeyCode::Minus) {
        sim.speed = (sim.speed / 2.0).max(0.25);
    }
}

/// Fixed 30 Hz stepping on an accumulator.
fn sim_driver(time: Res<Time>, plan: Option<Res<ShotPlan>>, mut sim: NonSendMut<SimRes>) {
    if plan.is_some() || sim.paused {
        return;
    }
    sim.acc += time.delta_secs() * sim.speed;
    let mut steps = 0;
    while sim.acc >= TICK_DT && steps < 8 {
        sim.step();
        sim.acc -= TICK_DT;
        steps += 1;
    }
    if steps == 8 {
        // Falling behind: drop the backlog rather than spiral.
        sim.acc = sim.acc.min(TICK_DT);
    }
}

/// Depth range of the oblique projection, in units of `y + SHEAR * z`
/// either side of the camera (covers a 4-world-unit-per-pixel view of a
/// 1080-px window plus every height).
const DEPTH_RANGE: f32 = 8000.0;

/// TA's oblique projection as a Bevy camera projection (see
/// [`ViewGeom`]): the camera sits at `(target.x, 0, target.z)` with no
/// rotation and this matrix does the shear.
#[derive(Debug, Clone)]
struct ObliqueProjection {
    geom: ViewGeom,
}

impl bevy::camera::CameraProjection for ObliqueProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        Mat4::from_cols_array_2d(&self.geom.clip_from_view(DEPTH_RANGE))
    }

    fn get_clip_from_view_for_sub(&self, _sub_view: &bevy::camera::SubCameraView) -> Mat4 {
        self.get_clip_from_view()
    }

    fn update(&mut self, width: f32, height: f32) {
        self.geom.size = [width, height];
    }

    fn far(&self) -> f32 {
        DEPTH_RANGE
    }

    fn get_frustum_corners(&self, _z_near: f32, _z_far: f32) -> [bevy::math::Vec3A; 8] {
        let inv = self.get_clip_from_view().inverse();
        let c = |x: f32, y: f32, z: f32| {
            bevy::math::Vec3A::from(inv.project_point3(Vec3::new(x, y, z)))
        };
        [
            c(1.0, -1.0, 1.0),
            c(1.0, 1.0, 1.0),
            c(-1.0, 1.0, 1.0),
            c(-1.0, -1.0, 1.0),
            c(1.0, -1.0, 0.0),
            c(1.0, 1.0, 0.0),
            c(-1.0, 1.0, 0.0),
            c(-1.0, -1.0, 0.0),
        ]
    }

    fn compute_frustum(
        &self,
        camera_transform: &GlobalTransform,
    ) -> bevy::camera::primitives::Frustum {
        let clip_from_world = self.get_clip_from_view() * camera_transform.affine().inverse();
        bevy::camera::primitives::Frustum(bevy::shape::ViewFrustum::from_clip_from_world(
            &clip_from_world,
        ))
    }
}

fn camera_system(
    cam: Res<CamState>,
    mut q: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let g = cam.geom;
    for (mut tf, mut proj) in &mut q {
        *tf = Transform::from_xyz(g.target[0], 0.0, g.target[1]);
        if let Projection::Custom(c) = &mut *proj
            && let Some(o) = c.get_mut::<ObliqueProjection>()
        {
            o.geom = g;
        }
    }
}

/// Fixed light direction for the baked face shading of models.
const LIGHT: Vec3 = Vec3::new(-0.35, 0.85, 0.4);

fn upload_model(
    built: &BuiltModel,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> ModelAssets {
    let img = rgba_image(
        built.atlas.width,
        built.atlas.height,
        built.atlas.rgba.clone(),
    );
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(images.add(img)),
        unlit: true,
        alpha_mode: AlphaMode::Mask(0.5),
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let l = LIGHT.normalize();
    let pieces = built
        .pieces
        .iter()
        .map(|p| PieceAsset {
            parent: p.parent,
            offset: Vec3::from_array(p.offset),
            name: p.name.clone(),
            mesh: p.mesh.as_ref().map(|m| {
                let colors: Vec<[f32; 4]> = m
                    .normals
                    .iter()
                    .map(|n| {
                        let d = Vec3::from_array(*n).dot(l).abs();
                        let s = 0.62 + 0.38 * d;
                        [s, s, s, 1.0]
                    })
                    .collect();
                let mut mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                );
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs.clone());
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
                mesh.insert_indices(Indices::U32(m.indices.clone()));
                meshes.add(mesh)
            }),
        })
        .collect();
    ModelAssets {
        pieces,
        material,
        y_range: model_y_range(built),
    }
}

/// Lowest and highest vertex height of a model at rest pose (piece offsets
/// accumulated down the tree).
fn model_y_range(built: &BuiltModel) -> (f32, f32) {
    let mut origin_y = vec![0.0f32; built.pieces.len()];
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (i, p) in built.pieces.iter().enumerate() {
        let base = p.parent.map_or(0.0, |q| origin_y[q]);
        origin_y[i] = base + p.offset[1];
        if let Some(m) = &p.mesh {
            for v in &m.positions {
                lo = lo.min(origin_y[i] + v[1]);
                hi = hi.max(origin_y[i] + v[1]);
            }
        }
    }
    if lo > hi {
        (0.0, 1.0)
    } else {
        (lo, hi.max(lo + 1.0))
    }
}

/// Material for units under construction: the standard material cut at a
/// world height by `build_clip.wesl`.
type BuildMat = bevy::pbr::ExtendedMaterial<StandardMaterial, BuildClip>;

/// Uniform of `build_clip.wesl`.
#[derive(Asset, bevy::render::render_resource::AsBindGroup, Reflect, Debug, Clone, Default)]
struct BuildClip {
    /// World height of the cut.
    #[uniform(100)]
    clip_y: f32,
    /// 0: keep below the cut (built part); 1: keep above (ghost).
    #[uniform(100)]
    keep_above: f32,
    #[uniform(100)]
    _pad1: f32,
    #[uniform(100)]
    _pad2: f32,
}

static BUILD_CLIP_SHADER: std::sync::OnceLock<Handle<bevy::shader::Shader>> =
    std::sync::OnceLock::new();

impl bevy::pbr::MaterialExtension for BuildClip {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        match BUILD_CLIP_SHADER.get() {
            Some(h) => bevy::shader::ShaderRef::Handle(h.clone()),
            None => bevy::shader::ShaderRef::Default,
        }
    }
}

/// Ghost of the unbuilt part (**Chosen**: TA:K's own build-up look is not
/// documented; a pale translucent copy above the rising cut reads as "being
/// built" and keeps the finished shape visible).
const GHOST_RGBA: [f32; 4] = [0.7, 0.9, 1.0, 0.6];

/// Height of each unit's model top at rest pose (for bars above it).
#[derive(Resource, Default)]
struct UnitTops(BTreeMap<UnitId, f32>);

/// A unit's construction look while it is a frame.
struct BuildVis {
    solid: Handle<BuildMat>,
    ghost: Handle<BuildMat>,
    /// Mesh entities with the solid build material.
    meshes: Vec<Entity>,
    /// Ghost mesh entities (children of `meshes`).
    ghosts: Vec<Entity>,
    /// The finished material to swap back to.
    material: Handle<StandardMaterial>,
    y_range: (f32, f32),
}

/// Heading (0 = east, quarter = south) to a rotation of the model, whose
/// forward axis is -Z (`docs/formats/3do.md`).
fn heading_rotation(heading_rad: f32) -> Quat {
    Quat::from_rotation_y(-(heading_rad + FRAC_PI_2))
}

/// Script turn angles to a piece rotation: `RotY(-y) * RotX(-x) * RotZ(-z)`,
/// the convention `tak-cli anim` renders with (**Believed**, TA/Spring 3DO
/// order).
fn piece_rotation(r: [f32; 3]) -> Quat {
    Quat::from_rotation_y(-r[1]) * Quat::from_rotation_x(-r[0]) * Quat::from_rotation_z(-r[2])
}

fn units_system(
    mut commands: Commands,
    sim: NonSend<SimRes>,
    content: NonSend<Content>,
    mut cache: ResMut<ModelCache>,
    mut ents: ResMut<UnitEntities>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut build_mats: ResMut<Assets<BuildMat>>,
    mut tops: ResMut<UnitTops>,
    mut vis_q: Query<&mut UnitVis>,
    mut tf_q: Query<&mut Transform>,
    mut v_q: Query<&mut Visibility>,
) {
    let alpha = sim.alpha();
    let prev: BTreeMap<UnitId, &UnitSnap> = sim.prev.units.iter().map(|u| (u.id, u)).collect();
    // Despawn units that are gone.
    let live: std::collections::BTreeSet<UnitId> = sim.curr.units.iter().map(|u| u.id).collect();
    let gone: Vec<UnitId> = ents
        .0
        .keys()
        .filter(|k| !live.contains(k))
        .copied()
        .collect();
    for id in gone {
        if let Some(e) = ents.0.remove(&id) {
            commands.entity(e).despawn();
        }
        tops.0.remove(&id);
    }
    for u in &sim.curr.units {
        let p = prev.get(&u.id).copied();
        let x = lerp(p.map_or(u.x, |p| p.x), u.x, alpha);
        let y = lerp(p.map_or(u.y, |p| p.y), u.y, alpha);
        let g = lerp(p.map_or(u.ground, |p| p.ground), u.ground, alpha);
        let hd = match p {
            Some(p) => lerp_angle(p.heading, u.heading, alpha),
            None => angle_rad(u.heading),
        };
        let root_tf = Transform::from_xyz(x, g, y).with_rotation(heading_rotation(hd));
        let Some(&e) = ents.0.get(&u.id) else {
            // Spawn.
            let Some(def) = sim.session.def(u.def) else {
                continue;
            };
            let key = (def.object.clone(), usize::from(u.owner));
            let assets = cache
                .models
                .entry(key)
                .or_insert_with(|| {
                    match load_model(
                        &content.vfs,
                        &content.tex,
                        &def.object,
                        usize::from(u.owner),
                    ) {
                        Ok(b) => {
                            if !b.stand_ins.is_empty() {
                                info!("{}: stand-in textures {:?}", def.object, b.stand_ins);
                            }
                            if !b.missing.is_empty() {
                                warn!("{}: missing textures {:?}", def.object, b.missing);
                            }
                            Some(Arc::new(upload_model(
                                &b,
                                &mut meshes,
                                &mut images,
                                &mut materials,
                            )))
                        }
                        Err(e) => {
                            warn!("{e}");
                            None
                        }
                    }
                })
                .clone();
            let root = commands.spawn((root_tf, Visibility::default())).id();
            let mut pieces = Vec::new();
            let mut build = None;
            if let Some(a) = assets {
                tops.0.insert(u.id, a.y_range.1);
                let building = u.build_percent_left > 0;
                if building {
                    let mk = |keep_above: f32, mats: &Assets<StandardMaterial>| {
                        let mut base = mats.get(&a.material).cloned().unwrap_or_default();
                        if keep_above > 0.5 {
                            base.base_color = Color::srgba(
                                GHOST_RGBA[0],
                                GHOST_RGBA[1],
                                GHOST_RGBA[2],
                                GHOST_RGBA[3],
                            );
                            base.alpha_mode = AlphaMode::Blend;
                        }
                        BuildMat {
                            base,
                            extension: BuildClip {
                                clip_y: g + a.y_range.0,
                                keep_above,
                                ..default()
                            },
                        }
                    };
                    let solid = build_mats.add(mk(0.0, &materials));
                    let ghost = build_mats.add(mk(1.0, &materials));
                    build = Some(BuildVis {
                        solid,
                        ghost,
                        meshes: Vec::new(),
                        ghosts: Vec::new(),
                        material: a.material.clone(),
                        y_range: a.y_range,
                    });
                }
                // COB piece index for each model piece, by the script type's
                // name map.
                let mut cob_of = vec![None; a.pieces.len()];
                if let Some(st) = sim.session.sim.cob().and_then(|c| c.script_type(u.def)) {
                    for (ci, m) in st.piece_map.iter().enumerate() {
                        if let Some(mi) = m.map(usize::from)
                            && mi < cob_of.len()
                            && cob_of[mi].is_none()
                        {
                            cob_of[mi] = Some(ci);
                        }
                    }
                }
                let mut ids: Vec<Entity> = Vec::with_capacity(a.pieces.len());
                for (i, pa) in a.pieces.iter().enumerate() {
                    let parent = pa.parent.and_then(|p| ids.get(p).copied()).unwrap_or(root);
                    let pe = commands
                        .spawn((
                            Transform::from_translation(pa.offset),
                            Visibility::default(),
                            ChildOf(parent),
                            Name::new(pa.name.clone()),
                        ))
                        .id();
                    let me = pa.mesh.as_ref().map(|m| match &mut build {
                        None => commands
                            .spawn((
                                Mesh3d(m.clone()),
                                MeshMaterial3d(a.material.clone()),
                                Transform::default(),
                                Visibility::default(),
                                ChildOf(pe),
                            ))
                            .id(),
                        Some(b) => {
                            let e = commands
                                .spawn((
                                    Mesh3d(m.clone()),
                                    MeshMaterial3d(b.solid.clone()),
                                    Transform::default(),
                                    Visibility::default(),
                                    ChildOf(pe),
                                ))
                                .id();
                            let gh = commands
                                .spawn((
                                    Mesh3d(m.clone()),
                                    MeshMaterial3d(b.ghost.clone()),
                                    Transform::default(),
                                    Visibility::default(),
                                    ChildOf(e),
                                ))
                                .id();
                            b.meshes.push(e);
                            b.ghosts.push(gh);
                            e
                        }
                    });
                    ids.push(pe);
                    pieces.push((pe, me, cob_of[i], pa.offset));
                }
            }
            commands.entity(root).insert(UnitVis {
                id: u.id,
                pieces,
                build,
            });
            ents.0.insert(u.id, root);
            continue;
        };
        if let Ok(mut tf) = tf_q.get_mut(e) {
            *tf = root_tf;
        }
        let Ok(mut vis) = vis_q.get_mut(e) else {
            continue;
        };
        debug_assert_eq!(vis.id, u.id);
        // Construction: move the cut with the build progress; swap to the
        // finished material once complete.
        if let Some(b) = &vis.build {
            if u.build_percent_left > 0 {
                let done = lerp(
                    p.map_or(u.build_done, |p| p.build_done),
                    u.build_done,
                    alpha,
                );
                let clip = g + b.y_range.0 + (b.y_range.1 - b.y_range.0) * done.clamp(0.0, 1.0);
                debug!(
                    "build cut {:?}: y {:?} ground {g} done {done} clip {clip}",
                    u.id, b.y_range
                );
                for h in [&b.solid, &b.ghost] {
                    if let Some(mut m) = build_mats.get_mut(h) {
                        m.extension.clip_y = clip;
                    }
                }
            } else if let Some(b) = vis.build.take() {
                for me in b.meshes {
                    commands
                        .entity(me)
                        .remove::<MeshMaterial3d<BuildMat>>()
                        .insert(MeshMaterial3d(b.material.clone()));
                }
                for gh in b.ghosts {
                    commands.entity(gh).despawn();
                }
                build_mats.remove(&b.solid);
                build_mats.remove(&b.ghost);
            }
        }
        for &(pe, me, cob, base) in &vis.pieces {
            let Some(ci) = cob else { continue };
            let Some(now) = u.pieces.get(ci) else {
                continue;
            };
            let before = p.and_then(|p| p.pieces.get(ci));
            let pose = lerp_pose(before, now, alpha);
            if let Ok(mut tf) = tf_q.get_mut(pe) {
                tf.translation = base + Vec3::from_array(pose.offset);
                tf.rotation = piece_rotation(pose.rotation);
            }
            if let Some(me) = me
                && let Ok(mut v) = v_q.get_mut(me)
            {
                let want = if pose.visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                if *v != want {
                    *v = want;
                }
            }
        }
    }
}

/// Live features from the sim's feature layer: gameplay-relevant map
/// features (henges, rocks, trees that block or can be reclaimed) and
/// corpses/wrecks, drawn with their art ([`spawn_feature_art`]) and removed
/// when the sim removes them. Scenery the sim does not keep is drawn once
/// by [`spawn_scenery`].
fn features_system(
    mut commands: Commands,
    sim: NonSend<SimRes>,
    content: NonSend<Content>,
    ground: Res<Ground>,
    mut art_assets: ArtAssets,
    mut ents: ResMut<FeatureEntities>,
) {
    let fs = &sim.session.sim.world().features;
    let keep: std::collections::BTreeSet<_> = fs.iter().map(|(id, _)| id).collect();
    let gone: Vec<_> = ents
        .0
        .keys()
        .filter(|k| !keep.contains(k))
        .copied()
        .collect();
    for id in gone {
        if let Some(e) = ents.0.remove(&id) {
            commands.entity(e).despawn();
        }
    }
    for (id, feat) in fs.iter() {
        if ents.0.contains_key(&id) {
            continue;
        }
        let r = feat.rect;
        let x = (r.x as f32 + r.w as f32 / 2.0) * 16.0;
        let z = (r.z as f32 + r.h as f32 / 2.0) * 16.0;
        let name = fs
            .def(feat)
            .map(|d| d.name.to_ascii_lowercase())
            .unwrap_or_default();
        let art = sim
            .session
            .feature_art
            .get(&name)
            .cloned()
            .unwrap_or_else(|| FeatureArt {
                object: fs.def(feat).and_then(|d| d.object.clone()),
                ..FeatureArt::default()
            });
        let heading = match feat.origin {
            tak_sim::features::Origin::Corpse(_) => Some(angle_rad(feat.heading.raw())),
            tak_sim::features::Origin::Map => None,
        };
        let pos = Vec3::new(x, ground.0.height_at(x, z), z);
        let root = spawn_feature_art(
            &mut commands,
            &content,
            &mut art_assets,
            &art,
            pos,
            heading,
            (r.x as u32)
                .wrapping_mul(7)
                .wrapping_add((r.z as u32).wrapping_mul(13)),
        )
        .unwrap_or_else(|| {
            // No art: a low box over the footprint.
            let (w, d) = (r.w as f32 * 16.0, r.h as f32 * 16.0);
            let mat = art_assets.materials.add(StandardMaterial {
                base_color: Color::srgb(0.3, 0.22, 0.15),
                unlit: true,
                ..default()
            });
            let root = commands
                .spawn((Transform::from_translation(pos), Visibility::default()))
                .id();
            commands.spawn((
                Mesh3d(art_assets.meshes.add(Cuboid::new(w * 0.7, 6.0, d * 0.7))),
                MeshMaterial3d(mat),
                Transform::from_xyz(0.0, 3.0, 0.0),
                ChildOf(root),
            ));
            root
        });
        ents.0.insert(id, root);
    }
}

/// Asset stores the feature and effect spawners share.
#[derive(bevy::ecs::system::SystemParam)]
struct ArtAssets<'w> {
    models: ResMut<'w, ModelCache>,
    sprites: ResMut<'w, SpriteCache>,
    meshes: ResMut<'w, Assets<Mesh>>,
    images: ResMut<'w, Assets<Image>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
}

/// GPU assets of one sprite sequence: a quad and a material per frame,
/// shared by every instance (so Bevy batches them).
struct SpriteAssets {
    frames: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
    /// Durations only (pixels dropped), for [`SpriteSeq::frame_at`].
    timing: SpriteSeq,
}

/// How a sprite is blended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SpriteBlend {
    /// Cut out at alpha 0.5 (feature and unit art, indexed effects).
    Mask,
    /// Shadow: the silhouette at half opacity (`shadtrans=1`).
    Shadow,
    /// Alpha blend by the frame's own alpha (TAF frames with the `0xFF00`
    /// subframe word: smoke, dust, blood).
    Alpha,
    /// Additive (other TAF frames: fire, sparks, lightning, rings).
    Add,
}

impl SpriteBlend {
    /// The blend an effect sequence gets (**Chosen** reading of the Unknown
    /// `0xFF00` word: the 37 TAF pairs carrying it are grey smoke, dust and
    /// blood, which look right alpha-blended and wrong additive; the rest
    /// are glowing fire and magic, which look right additive).
    fn for_effect(seq: &SpriteSeq) -> SpriteBlend {
        match seq.frames.first() {
            Some(f) if f.direct && f.subframe_word == 0xFF00 => SpriteBlend::Alpha,
            Some(f) if f.direct => SpriteBlend::Add,
            _ => SpriteBlend::Mask,
        }
    }
}

/// Sprite assets by (file, sequence, lift in 1/4 units, blend).
#[derive(Resource, Default)]
struct SpriteCache(BTreeMap<(String, String, i32, SpriteBlend), Option<Arc<SpriteAssets>>>);

/// Opacity of feature shadows (**Chosen**; TA draws `shadtrans` shadows
/// translucent, the exact level is Unknown).
const SHADOW_ALPHA: f32 = 0.85;

fn sprite_assets(
    art: &mut ArtAssets,
    seq: &SpriteSeq,
    key: (String, String, i32, SpriteBlend),
) -> Option<Arc<SpriteAssets>> {
    if let Some(a) = art.sprites.0.get(&key) {
        return a.clone();
    }
    let lift = key.2 as f32 / 4.0;
    let blend = key.3;
    let frames = seq
        .frames
        .iter()
        .map(|fr| {
            let (pos, uv, idx) = billboard(fr, lift);
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos.to_vec());
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv.to_vec());
            mesh.insert_indices(Indices::U32(idx.to_vec()));
            let mut px = fr.rgba.clone();
            if blend == SpriteBlend::Shadow {
                // Shadow opacity baked into the texture (a material alpha
                // below 0.5 did not show on lavapipe).
                let a = (SHADOW_ALPHA * 255.0) as u8;
                for p in px.chunks_exact_mut(4) {
                    p[3] = p[3].min(a);
                }
            }
            let img = art.images.add(rgba_image(fr.w, fr.h, px));
            let material = match blend {
                SpriteBlend::Mask => StandardMaterial {
                    base_color_texture: Some(img),
                    unlit: true,
                    alpha_mode: AlphaMode::Mask(0.5),
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                },
                SpriteBlend::Shadow | SpriteBlend::Alpha => StandardMaterial {
                    base_color: Color::srgba(1.0, 1.0, 1.0, 1.0),
                    base_color_texture: Some(img),
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                },
                SpriteBlend::Add => StandardMaterial {
                    base_color_texture: Some(img),
                    unlit: true,
                    alpha_mode: AlphaMode::Add,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                },
            };
            (art.meshes.add(mesh), art.materials.add(material))
        })
        .collect();
    let mut timing = seq.clone();
    for f in &mut timing.frames {
        f.rgba = Vec::new();
    }
    let a = Some(Arc::new(SpriteAssets { frames, timing }));
    art.sprites.0.insert(key, a.clone());
    a
}

/// A sprite that plays its sequence.
#[derive(Component)]
struct Animated {
    assets: Arc<SpriteAssets>,
    phase: u32,
    shown: usize,
}

/// Step animated sprites by sim tick.
fn animate_sprites(
    sim: NonSend<SimRes>,
    mut q: Query<(
        &mut Animated,
        &mut Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let tick = sim.session.tick().0;
    for (mut a, mut mesh, mut mat) in &mut q {
        let i = a.assets.timing.frame_at(tick.wrapping_add(a.phase));
        if i != a.shown {
            a.shown = i;
            let (m, t) = &a.assets.frames[i];
            mesh.0 = m.clone();
            mat.0 = t.clone();
        }
    }
}

/// Explosions, hit effects and projectiles in flight.
#[derive(Resource, Default)]
struct FxState {
    /// Impacts with a tick below this were handled.
    impact_cursor: u32,
    /// Projectile visuals by key: entity and where it was first seen.
    projectiles: BTreeMap<ProjKey, (Entity, [f32; 2])>,
}

type ProjKey = (UnitId, (u32, u8), u32);

/// A one-shot effect sprite playing from sim tick `start`.
#[derive(Component)]
struct Effect {
    assets: Arc<SpriteAssets>,
    start: u32,
    shown: usize,
}

/// Effect sprite assets by name (and weapon GAF file), cached.
fn effect_assets(
    content: &Content,
    art: &mut ArtAssets,
    name: &str,
    file: Option<&str>,
) -> Option<Arc<SpriteAssets>> {
    let key_name = format!("fx:{}:{}", file.unwrap_or(""), name.to_ascii_lowercase());
    // Quick path: any lift/blend variant already built for this name.
    if let Some((_, a)) = art
        .sprites
        .0
        .range((key_name.clone(), String::new(), i32::MIN, SpriteBlend::Mask)..)
        .next()
        .filter(|(k, _)| k.0 == key_name)
    {
        return a.clone();
    }
    let seq = content.sprites.effect_seq(&content.vfs, name, file);
    let Some(seq) = seq else {
        art.sprites
            .0
            .insert((key_name, String::new(), 0, SpriteBlend::Mask), None);
        return None;
    };
    let lift = lift_for(seq.max_below(), 2.0);
    let blend = SpriteBlend::for_effect(&seq);
    sprite_assets(
        art,
        &seq,
        (
            key_name,
            name.to_ascii_lowercase(),
            (lift * 4.0).round() as i32,
            blend,
        ),
    )
}

/// Total duration of a sequence in ticks.
fn seq_ticks(seq: &SpriteSeq) -> u32 {
    seq.frames.iter().map(|f| f.duration.max(1)).sum()
}

/// Hit explosions (from the sim's impact log: projectile landings and
/// instant hits alike, death explosions included), played once by sim
/// tick; projectiles in flight drawn with their model or sprite.
fn effects_system(
    mut commands: Commands,
    sim: NonSend<SimRes>,
    content: NonSend<Content>,
    ground: Res<Ground>,
    mut art_assets: ArtAssets,
    mut fx: ResMut<FxState>,
    mut effects: Query<(
        Entity,
        &mut Effect,
        &mut Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
    mut tf_q: Query<&mut Transform, Without<Effect>>,
) {
    let s = &sim.session;
    let w = s.sim.world();
    let now = s.tick().0;
    // New impacts.
    let cursor = fx.impact_cursor;
    for imp in &w.combat.impacts {
        if imp.tick < cursor || imp.tick >= now {
            continue;
        }
        let key = crate::game::weapon_key(imp.weapon);
        let Some(class) = s.weapon_art.get(&key).and_then(|a| a.explosion.as_ref()) else {
            continue;
        };
        let Some(names) = s.explosions.get(class) else {
            continue;
        };
        if names.is_empty() {
            continue;
        }
        let (px, pz) = (f(imp.point.x), f(imp.point.y));
        let pick = (imp.tick as usize + (px as usize) * 7 + (pz as usize) * 13) % names.len();
        let Some(a) = effect_assets(&content, &mut art_assets, &names[pick], None) else {
            continue;
        };
        if now.saturating_sub(imp.tick) >= seq_ticks(&a.timing) {
            continue;
        }
        let (m, t) = a.frames[0].clone();
        commands.spawn((
            Mesh3d(m),
            MeshMaterial3d(t),
            Transform::from_xyz(px, ground.0.height_at(px, pz), pz),
            Visibility::default(),
            Effect {
                assets: a,
                start: imp.tick,
                shown: 0,
            },
        ));
    }
    fx.impact_cursor = now;
    // Age effects.
    for (e, mut eff, mut mesh, mut mat) in &mut effects {
        let age = now.saturating_sub(eff.start);
        if age >= seq_ticks(&eff.assets.timing) {
            commands.entity(e).despawn();
            continue;
        }
        let i = eff.assets.timing.frame_at(age);
        if i != eff.shown {
            eff.shown = i;
            let (m, t) = &eff.assets.frames[i];
            mesh.0 = m.clone();
            mat.0 = t.clone();
        }
    }
    // Projectiles.
    let alpha = sim.alpha();
    let prev: BTreeMap<ProjKey, &crate::game::ProjSnap> =
        sim.prev.projectiles.iter().map(|p| (p.key, p)).collect();
    let live: std::collections::BTreeSet<ProjKey> =
        sim.curr.projectiles.iter().map(|p| p.key).collect();
    let gone: Vec<ProjKey> = fx
        .projectiles
        .keys()
        .filter(|k| !live.contains(k))
        .copied()
        .collect();
    for k in gone {
        if let Some((e, _)) = fx.projectiles.remove(&k) {
            commands.entity(e).despawn();
        }
    }
    for p in &sim.curr.projectiles {
        let Some(art) = s.weapon_art.get(&p.weapon) else {
            continue;
        };
        let q = prev.get(&p.key).copied();
        let x = lerp(q.map_or(p.x, |q| q.x), p.x, alpha);
        let z = lerp(q.map_or(p.y, |q| q.y), p.y, alpha);
        let g = lerp(q.map_or(p.ground, |q| q.ground), p.ground, alpha);
        let start = fx.projectiles.get(&p.key).map(|v| v.1);
        let start = start.unwrap_or([x, z]);
        // Ballistic shots arc (**Chosen** shape: a parabola peaking at a
        // quarter of the range; the sim moves them on the ground only).
        let total = ((p.dest[0] - start[0]).powi(2) + (p.dest[1] - start[1]).powi(2)).sqrt();
        let done = ((x - start[0]).powi(2) + (z - start[1]).powi(2)).sqrt();
        let arc = if art.kind == "ballistic" && total > 1.0 {
            let t = (done / total).clamp(0.0, 1.0);
            total * 0.25 * 4.0 * t * (1.0 - t)
        } else {
            0.0
        };
        let pos = Vec3::new(x, g + 18.0 + arc, z);
        let heading = (p.dest[1] - z).atan2(p.dest[0] - x);
        let rot = heading_rotation(heading);
        if let Some((e, _)) = fx.projectiles.get(&p.key) {
            if let Ok(mut tf) = tf_q.get_mut(*e) {
                tf.translation = pos;
                if art.model.is_some() {
                    tf.rotation = rot;
                }
            }
            continue;
        }
        let root = commands
            .spawn((
                Transform::from_translation(pos).with_rotation(rot),
                Visibility::default(),
            ))
            .id();
        let mut drawn = false;
        if let Some(o) = &art.model {
            let ArtAssets {
                models,
                meshes,
                images,
                materials,
                ..
            } = &mut art_assets;
            let assets = models
                .models
                .entry((o.clone(), 0))
                .or_insert_with(|| {
                    load_model(&content.vfs, &content.tex, o, 0)
                        .ok()
                        .map(|b| Arc::new(upload_model(&b, meshes, images, materials)))
                })
                .clone();
            if let Some(a) = assets {
                spawn_static_pieces(&mut commands, &a, root);
                drawn = true;
            }
        }
        if !drawn
            && let Some(name) = &art.art
            && let Some(a) = effect_assets(&content, &mut art_assets, name, art.art_file.as_deref())
        {
            commands
                .entity(root)
                .insert(Transform::from_translation(pos));
            let (m, t) = a.frames[0].clone();
            let child = commands
                .spawn((
                    Mesh3d(m),
                    MeshMaterial3d(t),
                    Transform::default(),
                    ChildOf(root),
                ))
                .id();
            if a.frames.len() > 1 {
                commands.entity(child).insert(Animated {
                    assets: a.clone(),
                    phase: 0,
                    shown: 0,
                });
            }
        }
        fx.projectiles.insert(p.key, (root, start));
    }
}

/// Spawn a feature's art at `pos` (footprint centre on the ground): its 3DO
/// when it names one (rest pose, rotated by `heading` if given), else its
/// GAF sequence as a billboard with its shadow, animated when `animating`.
/// Returns the root entity, or `None` when the feature has no drawable art.
fn spawn_feature_art(
    commands: &mut Commands,
    content: &Content,
    art_assets: &mut ArtAssets,
    art: &FeatureArt,
    pos: Vec3,
    heading: Option<f32>,
    phase: u32,
) -> Option<Entity> {
    if let Some(o) = &art.object {
        let o = o.to_ascii_lowercase();
        let ArtAssets {
            models,
            meshes,
            images,
            materials,
            ..
        } = art_assets;
        let assets = models
            .models
            .entry((o.clone(), 0))
            .or_insert_with(|| {
                load_model(&content.vfs, &content.tex, &o, 0)
                    .ok()
                    .map(|b| Arc::new(upload_model(&b, meshes, images, materials)))
            })
            .clone();
        if let Some(a) = assets {
            let rot = heading.map_or(Quat::IDENTITY, heading_rotation);
            let root = commands
                .spawn((
                    Transform::from_translation(pos).with_rotation(rot),
                    Visibility::default(),
                ))
                .id();
            spawn_static_pieces(commands, &a, root);
            return Some(root);
        }
    }
    let file = art.filename.as_ref()?;
    let seqname = art.seqname.as_ref()?;
    let path = format!("anims/{file}.gaf");
    let pal = content
        .sprites
        .feature_palette(&content.vfs, &path, content.kingdom.as_deref());
    let seq = content.sprites.seq(&content.vfs, &path, seqname, &pal)?;
    let shadow = art
        .seqname_shadow
        .as_ref()
        .and_then(|sn| content.sprites.seq(&content.vfs, &path, sn, &pal));
    let below = seq
        .max_below()
        .max(shadow.as_ref().map_or(0, SpriteSeq::max_below));
    let lift = lift_for(below, 6.0);
    let q = |l: f32| (l * 4.0).round() as i32;
    let root = commands
        .spawn((Transform::from_translation(pos), Visibility::default()))
        .id();
    let obj = sprite_assets(
        art_assets,
        &seq,
        (
            path.clone(),
            seqname.to_ascii_lowercase(),
            q(lift),
            SpriteBlend::Mask,
        ),
    )?;
    let animate = art.animating && obj.frames.len() > 1;
    let (m, t) = obj.frames[0].clone();
    let child = commands
        .spawn((
            Mesh3d(m),
            MeshMaterial3d(t),
            Transform::default(),
            ChildOf(root),
        ))
        .id();
    if animate {
        commands.entity(child).insert(Animated {
            assets: obj.clone(),
            phase,
            shown: 0,
        });
    }
    if let (Some(sh), Some(sn)) = (shadow, &art.seqname_shadow)
        && let Some(sa) = sprite_assets(
            art_assets,
            &sh,
            (
                path.clone(),
                sn.to_ascii_lowercase(),
                q(lift - 3.0),
                SpriteBlend::Shadow,
            ),
        )
    {
        let (m, t) = sa.frames[0].clone();
        let child = commands
            .spawn((
                Mesh3d(m),
                MeshMaterial3d(t),
                Transform::default(),
                ChildOf(root),
            ))
            .id();
        if art.animating && sa.frames.len() > 1 {
            commands.entity(child).insert(Animated {
                assets: sa.clone(),
                phase,
                shown: 0,
            });
        }
    }
    Some(root)
}

/// Map features the sim does not keep (pure scenery): drawn once, never
/// removed. The rest come from the sim's feature layer
/// ([`features_system`]).
fn spawn_scenery(
    mut commands: Commands,
    sim: NonSend<SimRes>,
    content: NonSend<Content>,
    ground: Res<Ground>,
    mut art_assets: ArtAssets,
) {
    let t0 = std::time::Instant::now();
    let mut n = 0;
    for fb in &sim.session.features {
        if fb.in_sim {
            continue;
        }
        let Some(art) = fb.def.as_ref().and_then(|d| sim.session.feature_art.get(d)) else {
            continue;
        };
        let (w, d) = (
            f32::from(fb.footprint.0) * 16.0,
            f32::from(fb.footprint.1) * 16.0,
        );
        let x = fb.cell.0 as f32 * 16.0 + w / 2.0;
        let z = fb.cell.1 as f32 * 16.0 + d / 2.0;
        let pos = Vec3::new(x, ground.0.height_at(x, z), z);
        let phase = fb
            .cell
            .0
            .wrapping_mul(7)
            .wrapping_add(fb.cell.1.wrapping_mul(13));
        if spawn_feature_art(
            &mut commands,
            &content,
            &mut art_assets,
            art,
            pos,
            None,
            phase,
        )
        .is_some()
        {
            n += 1;
        }
    }
    eprintln!(
        "scenery: {n} of {} placed features drawn as scenery in {:.2}s",
        sim.session.features.len(),
        t0.elapsed().as_secs_f32()
    );
}

/// Spawn a model's pieces at rest pose under `root`.
fn spawn_static_pieces(commands: &mut Commands, a: &ModelAssets, root: Entity) {
    let mut ids: Vec<Entity> = Vec::new();
    for pa in &a.pieces {
        let parent = pa.parent.and_then(|p| ids.get(p).copied()).unwrap_or(root);
        let pe = commands
            .spawn((
                Transform::from_translation(pa.offset),
                Visibility::default(),
                ChildOf(parent),
            ))
            .id();
        if let Some(m) = &pa.mesh {
            commands.spawn((
                Mesh3d(m.clone()),
                MeshMaterial3d(a.material.clone()),
                Transform::default(),
                ChildOf(pe),
            ));
        }
        ids.push(pe);
    }
}

/// Models placed by `--showcase` (render only: the sim never sees them).
#[derive(Resource)]
struct Showcase(Vec<String>);

/// `--showcase a,b,c`: static models in a row south of the focus point,
/// facing the camera, for checking textures and art by eye.
fn showcase_system(
    mut commands: Commands,
    show: Option<Res<Showcase>>,
    mut done: Local<bool>,
    sim: NonSend<SimRes>,
    content: NonSend<Content>,
    ground: Res<Ground>,
    plan: Option<Res<ShotPlan>>,
    mut cache: ResMut<ModelCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(show) = show else { return };
    if *done || sim.session.tick().0 == 0 {
        return;
    }
    *done = true;
    let focus = plan.map_or(Focus::Monarch(0), |p| p.focus);
    let w = sim.session.sim.world();
    let centre = match focus {
        Focus::Point(x, y) => [x, y],
        _ => w
            .players()
            .first()
            .and_then(|p| p.monarch.and_then(|m| w.unit(m)).map(|u| u.pos))
            .map_or([0.0, 0.0], |p| [f(p.x), f(p.y)]),
    };
    let n = show.0.len() as f32;
    for (i, name) in show.0.iter().enumerate() {
        let o = name.to_ascii_lowercase();
        let assets = cache
            .models
            .entry((o.clone(), 0))
            .or_insert_with(|| match load_model(&content.vfs, &content.tex, &o, 0) {
                Ok(b) => {
                    eprintln!(
                        "showcase {o}: stand-ins {:?} missing {:?}",
                        b.stand_ins, b.missing
                    );
                    Some(Arc::new(upload_model(
                        &b,
                        &mut meshes,
                        &mut images,
                        &mut materials,
                    )))
                }
                Err(e) => {
                    eprintln!("showcase {o}: {e}");
                    None
                }
            })
            .clone();
        let Some(a) = assets else { continue };
        let x = centre[0] + (i as f32 - (n - 1.0) / 2.0) * 150.0;
        let z = centre[1] + 60.0;
        // Heading a quarter turn = facing south, toward the camera.
        let tf = Transform::from_xyz(x, ground.0.height_at(x, z), z)
            .with_rotation(heading_rotation(FRAC_PI_2));
        let root = commands.spawn((tf, Visibility::default())).id();
        spawn_static_pieces(&mut commands, &a, root);
    }
}

/// Minimap width on screen, logical pixels (height follows the image).
const MINIMAP_W: f32 = 230.0;
/// Gap to the window edge.
const MINIMAP_MARGIN: f32 = 8.0;

/// Minimap layout and pointer state.
#[derive(Resource, Default)]
struct MinimapState {
    /// Image size, if the map has a minimap.
    image: Option<(u32, u32)>,
    /// Map size in world units.
    map: [f32; 2],
    mouse_drag: bool,
    swallow_release: bool,
    pad_swallow: bool,
}

impl MinimapState {
    /// Where the minimap is for a viewport (bottom-right corner).
    fn geom(&self, view: [f32; 2]) -> Option<MinimapGeom> {
        let (iw, ih) = self.image?;
        let w = MINIMAP_W;
        let h = w * ih as f32 / iw.max(1) as f32;
        Some(MinimapGeom {
            rect: [
                view[0] - w - MINIMAP_MARGIN,
                view[1] - h - MINIMAP_MARGIN,
                w,
                h,
            ],
            map: self.map,
        })
    }
}

#[derive(Component)]
struct MinimapRoot;
#[derive(Component)]
struct MinimapView;
#[derive(Component)]
struct MinimapDot(usize);

/// Team colours for minimap dots (**Chosen**: TA's player order blue, red,
/// white, green, yellow, purple, cyan, orange).
fn team_color(player: u8) -> Color {
    const C: [(u8, u8, u8); 8] = [
        (60, 120, 255),
        (230, 40, 30),
        (235, 235, 235),
        (40, 200, 60),
        (240, 220, 40),
        (170, 60, 220),
        (40, 220, 220),
        (250, 140, 30),
    ];
    let (r, g, b) = C[usize::from(player) % C.len()];
    Color::srgb_u8(r, g, b)
}

fn setup_minimap(
    mut commands: Commands,
    ground: Res<Ground>,
    mut images: ResMut<Assets<Image>>,
    mut mm: ResMut<MinimapState>,
) {
    let t = &ground.0;
    mm.map = [
        t.cells.0 as f32 * terrain::CELL,
        t.cells.1 as f32 * terrain::CELL,
    ];
    let Some((w, h, rgba)) = &t.minimap else {
        return;
    };
    mm.image = Some((*w, *h));
    let img = images.add(rgba_image(*w, *h, rgba.clone()));
    let mh = MINIMAP_W * *h as f32 / (*w).max(1) as f32;
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(MINIMAP_MARGIN),
                bottom: Val::Px(MINIMAP_MARGIN),
                width: Val::Px(MINIMAP_W),
                height: Val::Px(mh),
                overflow: Overflow::clip(),
                ..default()
            },
            ImageNode::new(img),
            MinimapRoot,
        ))
        .id();
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BorderColor::all(Color::WHITE),
        MinimapView,
        ChildOf(root),
    ));
}

/// Unit dots in team colours and the camera rectangle.
fn minimap_system(
    mut commands: Commands,
    sim: NonSend<SimRes>,
    cam: Res<CamState>,
    mm: Res<MinimapState>,
    root: Query<Entity, With<MinimapRoot>>,
    mut view: Query<&mut Node, (With<MinimapView>, Without<MinimapDot>)>,
    mut dots: Query<
        (
            &MinimapDot,
            &mut Node,
            &mut BackgroundColor,
            &mut Visibility,
        ),
        Without<MinimapView>,
    >,
) {
    let Ok(root) = root.single() else { return };
    let Some(g) = mm.geom(cam.geom.size) else {
        return;
    };
    if let Ok(mut n) = view.single_mut() {
        let r = g.view_rect(&cam.geom);
        n.left = Val::Px(r[0]);
        n.top = Val::Px(r[1]);
        n.width = Val::Px(r[2].max(2.0));
        n.height = Val::Px(r[3].max(2.0));
    }
    let units = &sim.curr.units;
    let mut have = 0;
    for (d, mut n, mut bg, mut v) in &mut dots {
        have = have.max(d.0 + 1);
        match units.get(d.0) {
            Some(u) => {
                let p = g.to_mini(u.x, u.y - crate::view::SHEAR * u.ground);
                let size = if sim
                    .session
                    .def(u.def)
                    .is_some_and(|d| d.footprint.0.max(d.footprint.1) >= 3)
                {
                    6.0
                } else {
                    4.0
                };
                n.left = Val::Px(p[0] - size / 2.0);
                n.top = Val::Px(p[1] - size / 2.0);
                n.width = Val::Px(size);
                n.height = Val::Px(size);
                bg.0 = team_color(u.owner);
                if *v != Visibility::Inherited {
                    *v = Visibility::Inherited;
                }
            }
            None => {
                if *v != Visibility::Hidden {
                    *v = Visibility::Hidden;
                }
            }
        }
    }
    for i in have..units.len() {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(3.0),
                height: Val::Px(3.0),
                ..default()
            },
            BackgroundColor(Color::WHITE),
            Visibility::Hidden,
            MinimapDot(i),
            ChildOf(root),
        ));
    }
}

/// Every 10 s of wall time, log frames per second and ticks run.
fn fps_report(sim: NonSend<SimRes>, mut acc: Local<Option<(std::time::Instant, u32, u32)>>) {
    let now = std::time::Instant::now();
    let tick = sim.session.tick().0;
    let (t0, frames, tick0) = acc.get_or_insert((now, 0, tick));
    *frames += 1;
    let dt = now.duration_since(*t0).as_secs_f32();
    if dt >= 10.0 {
        info!(
            "{:.1} fps, {:.1} ticks/s",
            *frames as f32 / dt,
            tick.saturating_sub(*tick0) as f32 / dt
        );
        *acc = Some((now, 0, tick));
    }
}

fn mana(raw: i64) -> f64 {
    raw as f64 / 65536.0
}

fn hud_system(
    sim: NonSend<SimRes>,
    input: Res<InputRes>,
    cam: Res<CamState>,
    ground: Res<Ground>,
    mut top: Query<&mut Text, (With<HudTop>, Without<HudPanel>, Without<HudWheel>)>,
    mut panel: Query<&mut Text, (With<HudPanel>, Without<HudTop>, Without<HudWheel>)>,
    mut wheel_q: Query<
        (&mut Text, &mut Node),
        (With<HudWheel>, Without<HudTop>, Without<HudPanel>),
    >,
) {
    let s = &sim.session;
    let w = s.sim.world();
    let tick = s.tick().0;
    if let Ok(mut t) = top.single_mut() {
        let mut line = format!(
            "{}  tick {} ({:.1}s){}{}",
            s.map_name.rsplit('/').next().unwrap_or(&s.map_name),
            tick,
            tick as f32 / 30.0,
            if sim.paused { "  PAUSED" } else { "" },
            if (sim.speed - 1.0).abs() > 1e-3 {
                format!("  x{}", sim.speed)
            } else {
                String::new()
            }
        );
        for p in w.players() {
            if let Some(e) = tak_sim::economy::player(w, p.id) {
                let side = p
                    .monarch
                    .and_then(|m| w.unit(m))
                    .and_then(|u| s.def(u.def))
                    .map_or("-", |d| &d.unitname[..3.min(d.unitname.len())]);
                line.push_str(&format!(
                    "   P{} {}: mana {:.0}/{:.0} (+{:.1}/s)",
                    p.id.0,
                    side,
                    mana(e.mana),
                    mana(e.storage),
                    mana(e.income)
                ));
            }
        }
        t.0 = line;
    }
    if let Ok(mut t) = panel.single_mut() {
        let mut out = String::new();
        let sel: Vec<UnitId> = input
            .mapper
            .selection()
            .iter()
            .copied()
            .filter(|id| w.unit(*id).is_some())
            .collect();
        if let Some(&first) = sel.first()
            && let Some(u) = w.unit(first)
        {
            let d = s.def(u.def);
            out.push_str(&format!(
                "{} ({})   HP {}/{}   {}{}",
                d.map_or("?", |d| d.display.as_str()),
                d.map_or("?", |d| d.unitname.as_str()),
                u.combat.health,
                u.combat.max_health,
                if u.orders.is_empty() {
                    "idle".to_string()
                } else {
                    format!("{} orders", u.orders.len())
                },
                if sel.len() > 1 {
                    format!("   (+{} more)", sel.len() - 1)
                } else {
                    String::new()
                }
            ));
            if u.build.percent_left() > 0 {
                out.push_str(&format!(
                    "\nunder construction: {}%",
                    100 - u.build.percent_left()
                ));
            }
            let opts = w
                .defs()
                .get(u.def)
                .map(|d| d.build.canbuild.clone())
                .unwrap_or_default();
            if !opts.is_empty() {
                out.push_str("\nBuild (F1-F8, LB wheel):");
                let mode = input.mapper.mode();
                for (i, o) in opts.iter().enumerate() {
                    let name = s.def(*o).map_or("?", |d| d.display.as_str());
                    let cost = w.defs().get(*o).map_or(0, |d| d.build.buildcost);
                    let mark = if mode == Mode::Build(*o) { ">" } else { " " };
                    out.push_str(&format!("\n{mark}F{} {name} ({cost})", i + 1));
                }
            }
        } else {
            out.push_str("Nothing selected. LMB / A: select   RMB / X: order   arrows / right stick: pan   wheel: zoom");
        }
        match input.mapper.mode() {
            Mode::None => {}
            m => out.push_str(&format!("\nmode: {m:?}")),
        }
        t.0 = out;
    }
    if let Ok((mut t, mut node)) = wheel_q.single_mut() {
        let mut text = String::new();
        if let Some(wh) = input.mapper.wheel() {
            let c = input.mapper.cursor_world();
            let (cx, cz) = (f(c.x), f(c.y));
            let sp = cam.geom.to_screen([cx, ground.0.height_at(cx, cz), cz]);
            node.left = Val::Px(sp[0] + 24.0);
            node.top = Val::Px(sp[1] - 60.0);
            text.push_str(match wh.kind {
                WheelKind::Command => "Orders",
                WheelKind::Build => "Build",
            });
            for (i, a) in wh.slots.iter().enumerate() {
                let mark = if wh.highlighted == Some(i) { ">" } else { " " };
                let label = match a {
                    Action::BuildOption { index } => {
                        let sel = input.mapper.selection().iter().next().copied();
                        sel.and_then(|id| w.unit(id))
                            .and_then(|u| w.defs().get(u.def))
                            .and_then(|d| d.build.canbuild.get(usize::from(*index)).copied())
                            .and_then(|o| s.def(o))
                            .map_or(format!("option {index}"), |d| d.display.clone())
                    }
                    other => format!("{other:?}"),
                };
                text.push_str(&format!("\n{mark}{i}: {label}"));
            }
        }
        t.0 = text;
    }
}

fn ground_circle(g: &mut Gizmos, t: &TerrainData, x: f32, z: f32, r: f32, color: Color) {
    let n = 32;
    let mut prev = None;
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let (px, pz) = (x + r * a.cos(), z + r * a.sin());
        let p = Vec3::new(px, t.height_at(px, pz) + 1.5, pz);
        if let Some(q) = prev {
            g.line(q, p, color);
        }
        prev = Some(p);
    }
}

fn gizmo_system(
    mut g: Gizmos,
    tops: Res<UnitTops>,
    sim: NonSend<SimRes>,
    input: Res<InputRes>,
    ground: Res<Ground>,
    plan: Option<Res<ShotPlan>>,
) {
    let t = &ground.0;
    let alpha = sim.alpha();
    let prev: BTreeMap<UnitId, &UnitSnap> = sim.prev.units.iter().map(|u| (u.id, u)).collect();
    let w = sim.session.sim.world();
    let local = sim.session.local;
    for u in &sim.curr.units {
        let p = prev.get(&u.id).copied();
        let x = lerp(p.map_or(u.x, |p| p.x), u.x, alpha);
        let y = lerp(p.map_or(u.y, |p| p.y), u.y, alpha);
        let (fx, fz) = sim.session.def(u.def).map_or((1, 1), |d| d.footprint);
        let r = pick_radius(fx, fz);
        let team = if PlayerId(u.owner) == local {
            Color::srgb(0.3, 0.6, 1.0)
        } else {
            Color::srgb(1.0, 0.3, 0.25)
        };
        // Team ring (thin) under every unit; selection ring (bright) on top.
        ground_circle(&mut g, t, x, y, r * 0.9, team.with_alpha(0.7));
        let selected = input.mapper.selection().contains(u.id);
        if selected {
            ground_circle(&mut g, t, x, y, r + 3.0, Color::srgb(0.3, 1.0, 0.3));
        }
        if selected || u.health_percent < 100 || u.build_percent_left > 0 {
            let top = tops.0.get(&u.id).copied().unwrap_or(60.0);
            let h = u.ground + top + 12.0;
            let left = Vec3::new(x - 16.0, h, y);
            let max = u.max_health.max(1) as f32;
            let frac = (u.health as f32 / max).clamp(0.0, 1.0);
            g.line(left, left + Vec3::X * 32.0, Color::srgb(0.25, 0.0, 0.0));
            // Foreground nudged toward the camera so it wins the depth test.
            let nudge = Vec3::new(0.0, 1.0, 1.0);
            // A frame's earned health (`max - frame_health`): the part of
            // it the frame lacks through damage shows dim.
            if u.build_percent_left > 0 {
                let earned = (u.earned_health as f32 / max).clamp(0.0, 1.0);
                g.line(
                    left + nudge * 0.5,
                    left + nudge * 0.5 + Vec3::X * 32.0 * earned,
                    Color::srgb(0.15, 0.4, 0.15),
                );
            }
            g.line(
                left + nudge,
                left + nudge + Vec3::X * 32.0 * frac,
                Color::srgb(0.2, 0.9, 0.2),
            );
            if u.build_percent_left > 0 {
                let b = left + Vec3::Y * 4.0;
                let done = (100 - u.build_percent_left) as f32 / 100.0;
                g.line(b, b + Vec3::X * 32.0, Color::srgb(0.1, 0.1, 0.3));
                g.line(
                    b + nudge,
                    b + nudge + Vec3::X * 32.0 * done,
                    Color::srgb(0.4, 0.6, 1.0),
                );
            }
        }
        // Order lines for own selected units.
        if selected && let Some(unit) = w.unit(u.id) {
            let mut from = Vec3::new(x, u.ground + 2.0, y);
            for o in &unit.orders {
                let to = match o {
                    tak_sim::Order::Move { to } | tak_sim::Order::Patrol { to } => Some(*to),
                    tak_sim::Order::Build { at: Some(at), .. } => Some(*at),
                    tak_sim::Order::Attack {
                        target: tak_commands::Target::Ground(p),
                    } => Some(*p),
                    tak_sim::Order::Attack {
                        target: tak_commands::Target::Unit(id),
                    }
                    | tak_sim::Order::Guard { target: id } => w.unit(*id).map(|t| t.pos),
                    _ => None,
                };
                if let Some(to) = to {
                    let (tx, tz) = (f(to.x), f(to.y));
                    let p = Vec3::new(tx, t.height_at(tx, tz) + 2.0, tz);
                    g.line(from, p, Color::srgba(1.0, 1.0, 0.4, 0.6));
                    from = p;
                }
            }
        }
    }
    // Projectiles with neither a model nor a sprite: a small orange ball.
    for pr in &sim.curr.projectiles {
        let art = sim.session.weapon_art.get(&pr.weapon);
        if art.is_some_and(|a| a.model.is_some() || a.art.is_some()) {
            continue;
        }
        g.sphere(
            Isometry3d::from_translation(Vec3::new(pr.x, pr.ground + 18.0, pr.y)),
            4.0,
            Color::srgb(1.0, 0.6, 0.1),
        );
    }
    // Lightning (`hweffect=lightning`): jagged bolts from the shooter to the
    // impact for `emittime` ticks (**Chosen** look; TA:K draws it with its
    // own effect code, Unknown).
    let now = sim.session.tick().0;
    for imp in &w.combat.impacts {
        let key = crate::game::weapon_key(imp.weapon);
        let Some((emit, rgb)) = sim.session.weapon_art.get(&key).and_then(|a| a.lightning) else {
            continue;
        };
        if now.saturating_sub(imp.tick) > emit {
            continue;
        }
        let Some(sh) = imp
            .shooter
            .and_then(|id| sim.curr.units.iter().find(|u| u.id == id))
        else {
            continue;
        };
        let top = tops.0.get(&sh.id).copied().unwrap_or(40.0);
        let a = Vec3::new(sh.x, sh.ground + top * 0.7, sh.y);
        let (px, pz) = (f(imp.point.x), f(imp.point.y));
        let b = Vec3::new(px, t.height_at(px, pz) + 10.0, pz);
        let color = Color::srgb_u8(rgb[0], rgb[1], rgb[2]);
        for bolt in 0..2u32 {
            let n = 8;
            let mut prev = a;
            for i in 1..=n {
                let k = i as f32 / n as f32;
                let mut q = a.lerp(b, k);
                if i < n {
                    let h = (now / 2)
                        .wrapping_mul(2654435761)
                        .wrapping_add(i * 97 + bolt * 31)
                        .wrapping_mul(40503);
                    let jx = ((h & 0xff) as f32 / 255.0 - 0.5) * 18.0;
                    let jy = (((h >> 8) & 0xff) as f32 / 255.0 - 0.5) * 18.0;
                    q += Vec3::new(jx, jy, 0.0);
                }
                g.line(prev, q, if bolt == 0 { Color::WHITE } else { color });
                prev = q;
            }
        }
    }
    // Cursor, brush and build placement (no cursor in screenshots: the
    // mouse is not there).
    if plan.is_some() && input.mapper.brush().is_none() {
        return;
    }
    let c = input.mapper.cursor_world();
    let (cx, cz) = (f(c.x), f(c.y));
    ground_circle(&mut g, t, cx, cz, 6.0, Color::srgb(1.0, 1.0, 0.2));
    let ch = t.height_at(cx, cz) + 1.5;
    g.line(
        Vec3::new(cx - 10.0, ch, cz),
        Vec3::new(cx + 10.0, ch, cz),
        Color::srgb(1.0, 1.0, 0.2),
    );
    g.line(
        Vec3::new(cx, ch, cz - 10.0),
        Vec3::new(cx, ch, cz + 10.0),
        Color::srgb(1.0, 1.0, 0.2),
    );
    if let Some((bc, br)) = input.mapper.brush() {
        ground_circle(
            &mut g,
            t,
            f(bc.x),
            f(bc.y),
            f(br),
            Color::srgb(0.3, 1.0, 1.0),
        );
    }
    if let Mode::Build(def) = input.mapper.mode()
        && let Some(d) = sim.session.def(def)
    {
        let (w2, d2) = (
            f32::from(d.footprint.0) * 8.0,
            f32::from(d.footprint.1) * 8.0,
        );
        let corners = [(-w2, -d2), (w2, -d2), (w2, d2), (-w2, d2), (-w2, -d2)];
        for k in 0..4 {
            let (ax, az) = (cx + corners[k].0, cz + corners[k].1);
            let (bx, bz) = (cx + corners[k + 1].0, cz + corners[k + 1].1);
            g.line(
                Vec3::new(ax, t.height_at(ax, az) + 2.0, az),
                Vec3::new(bx, t.height_at(bx, bz) + 2.0, bz),
                Color::WHITE,
            );
        }
    }
}
