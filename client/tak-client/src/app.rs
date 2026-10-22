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
use bevy::ui::UiGlobalTransform;
use bevy::window::{PrimaryWindow, WindowResolution};

use tak_commands::{ClientCommand, PlayerId, UnitId};
use tak_input::{
    Action, Axis as InAxis, Button as InButton, EventKind, InputEvent, Key, Mapper, Mode, Output,
    Profile,
};
use tak_vfs::Vfs;

use crate::cli::{Args, Focus, parse_focus};

#[path = "controls.rs"]
mod controls;
#[path = "hud_view.rs"]
mod hud_view;
use crate::game::FeatureArt;
use crate::game::{Session, Snapshot, UnitSnap, angle_rad, lerp, lerp_angle, lerp_pose};
use crate::models::{BuiltModel, TextureIndex, load_model};
use crate::sprites::{SpriteSeq, SpriteStore, billboard, lift_for};
use crate::terrain::{self, TerrainData};
use crate::view::{MinimapGeom, ViewGeom, f, pick_radius};
use controls::Feel;

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
    /// The local player's visual fog, updated after every tick (so runs
    /// that skip frames, like screenshot mode, still explore the path).
    fog: crate::fog::FogGrid,
    /// Fog changed since the overlay was last painted.
    fog_changed: bool,
    /// Alerts for the local player, checked after every tick (display
    /// only).
    alerts: crate::alerts::AlertTracker,
    /// Alerts raised since the HUD last took them (bottom bar).
    new_alerts: Vec<crate::alerts::Alert>,
}

impl SimRes {
    fn step(&mut self) {
        self.session.step();
        self.prev = std::mem::replace(&mut self.curr, self.session.snapshot());
        let w = self.session.sim.world();
        let local = self.session.local;
        let eyes: Vec<(f32, f32, f32)> = w
            .units()
            .filter(|(_, u)| u.owner == local)
            .map(|(_, u)| {
                let sight = w.defs().get(u.def).map_or(0, |d| d.combat.sight);
                (f(u.pos.x), f(u.pos.y), (sight as f32).max(FOG_MIN_SIGHT))
            })
            .collect();
        self.fog_changed |= self.fog.update(&eyes);
        let input = alert_input(&self.session);
        let new = self.alerts.update(&input);
        self.new_alerts.extend(new);
    }

    fn alpha(&self) -> f32 {
        (self.acc / TICK_DT).clamp(0.0, 1.0)
    }
}

/// What the alert tracker needs from the match: the local player's units,
/// mana and storage.
fn alert_input(s: &Session) -> crate::alerts::AlertInput {
    let w = s.sim.world();
    let me = s.local;
    let monarch = w
        .players()
        .iter()
        .find(|p| p.id == me)
        .and_then(|p| p.monarch);
    let units = w
        .units()
        .filter(|(_, u)| u.owner == me)
        .map(|(id, u)| crate::alerts::AlertUnit {
            id,
            name: s.def(u.def).map_or("?".to_string(), |d| d.display.clone()),
            pos: u.pos,
            health: u.combat.health,
            max_health: u.combat.max_health,
            complete: u.build.is_complete(),
            mobile: w
                .defs()
                .get(u.def)
                .is_some_and(|d| d.maxvelocity > tak_math::Fx::ZERO),
            monarch: Some(id) == monarch,
        })
        .collect();
    let e = tak_sim::economy::player(w, me);
    crate::alerts::AlertInput {
        tick: s.tick().0,
        units,
        mana: e.map_or(0, |e| e.mana >> 16),
        storage: e.map_or(0, |e| e.storage >> 16),
        home: monarch
            .and_then(|id| w.unit(id))
            .map_or(tak_math::Vec2::ZERO, |u| u.pos),
    }
}

/// Camera state.
#[derive(Resource, Clone, Copy)]
struct CamState {
    geom: ViewGeom,
    /// Terrain height under the screen centre: `tak-input`'s camera maps
    /// the plane at this height ([`ViewGeom::input_camera_at`]), so the
    /// reticle picks exactly what is drawn under it.
    input_h: f32,
}

impl CamState {
    fn input_camera(&self) -> tak_input::Camera {
        self.geom.input_camera_at(self.input_h)
    }
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
    /// Screen points to click on the HUD before each capture.
    clicks: Vec<(f32, f32)>,
    /// Gamepad events fed to the mapper before each capture.
    pad_events: Vec<InputEvent>,
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
/// Marks HUD panels: clicks there never select or order behind them.
#[derive(Component)]
struct HudPanel;
/// The match-end banner ("Victory" / "Defeat" / "Draw") and its subtitle.
#[derive(Component)]
struct HudBanner;
#[derive(Component)]
struct HudBannerTitle;
#[derive(Component)]
struct HudBannerText;

/// Ticks between the sim deciding the match and the end banner (TA showed
/// its end screen about 150 ticks, 5 s, after the decisive kill, so the
/// death animations play out first; **Believed** for TA:K, display only).
const BANNER_DELAY_TICKS: u32 = 150;

/// A clickable HUD button: it carries out the same action as its key or
/// wheel slot.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonKind {
    /// `hud_view::PANEL_ACTIONS[i]` (orders, stances, spells).
    Panel(usize),
    /// Build option `i`.
    Build(usize),
}

impl ButtonKind {
    /// What the button does.
    fn action(self) -> Action {
        match self {
            ButtonKind::Panel(i) => hud_view::PANEL_ACTIONS[i],
            ButtonKind::Build(i) => Action::BuildOption { index: i as u8 },
        }
    }
}

/// Build and run the app.
pub fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let vfs = crate::game::open_vfs(&args.data)?;
    let from_file = args.input_profile.as_ref().is_some_and(|p| p.exists());
    let (mut profile, profile_mtime) = crate::feel::initial_profile(
        &args.controls,
        &args.input_preset,
        args.input_profile.as_deref(),
    )?;
    // The player's own Keys.TDF (TA:K's key file) carries over onto the
    // Classic TA:K keys, unless a saved profile already says otherwise.
    if !from_file && !args.no_keys_tdf && args.controls.eq_ignore_ascii_case("kingdoms") {
        let path = args
            .keys_tdf
            .clone()
            .unwrap_or_else(|| args.data.join("Keys.TDF"));
        if let Ok(data) = std::fs::read(&path) {
            match tak_input::parse_keys_tdf(&data) {
                Ok(imp) => {
                    profile = profile.with_keys_tdf(&imp);
                    eprintln!("{}: {}", path.display(), imp.summary());
                }
                Err(e) => eprintln!("{}: not imported: {e}", path.display()),
            }
        }
    }
    if args.grid_keys {
        profile.grid_build_keys = true;
    }
    if let Some(ev_path) = &args.synth_events {
        return synth(&args, &vfs, profile, ev_path);
    }
    let mut ghost = None;
    let mut args = args;
    if let Some(gp) = &args.ghost {
        let rec = tak_input::Recording::from_json(&std::fs::read_to_string(gp)?)
            .map_err(|e| format!("{}: {e}", gp.display()))?;
        let gm = crate::feel::ghost_match(&rec)?;
        eprintln!(
            "ghost: {} on {} ({}), {} orders, {} ticks, final hash {:?}",
            gp.display(),
            gm.map,
            gm.players,
            gm.log.len(),
            gm.ticks,
            gm.final_hash
        );
        args.map = gm.map.clone();
        args.players = gm.players.clone();
        args.world_seed = gm.seed;
        ghost = Some((rec, gm.log));
    }
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
    if let Some((_, log)) = &ghost {
        session.log = log.clone();
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
    let fog_cells = ground.cells;
    let (w, h) = args.window_size();
    let focus = parse_focus(&args.focus).ok_or("bad --focus")?;
    let snap = session.snapshot();
    profile
        .validate()
        .map_err(|e| format!("input profile: {e}"))?;
    let mapper = Mapper::new(profile);
    let mut feel = Feel::new(&args, profile_mtime);
    if feel.record_path.is_some() {
        feel.recorder = Some(tak_input::Recorder::new(serde_json::Value::Null, &mapper));
    }
    feel.ghost = ghost.map(|(r, _)| r);
    let pad_events: Vec<InputEvent> = match &args.pad_events {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)
            .map_err(|e| format!("{}: {e}", p.display()))?,
        None => Vec::new(),
    };

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
            fog: crate::fog::FogGrid::new(fog_cells.0, fog_cells.1),
            fog_changed: true,
            alerts: crate::alerts::AlertTracker::default(),
            new_alerts: Vec::new(),
        })
        .insert_resource(Ground(Arc::new(ground)))
        .insert_resource(CamState {
            geom: ViewGeom {
                target: [0.0, 0.0],
                zoom: args.zoom,
                size: [w as f32, h as f32],
            },
            input_h: 0.0,
        })
        .insert_resource(InputRes {
            mapper,
            last_axes: BTreeMap::new(),
            last_pointer: None,
        })
        .insert_resource(feel)
        .init_resource::<ModelCache>()
        .init_resource::<UnitEntities>()
        .init_resource::<FeatureEntities>()
        .init_resource::<SpriteCache>()
        .init_resource::<UnitTops>()
        .init_resource::<FxState>()
        .init_resource::<MinimapState>()
        .insert_resource(FogRes {
            enabled: !args.no_fog,
            ..default()
        })
        .add_systems(
            Startup,
            (
                setup_scene,
                setup_hud,
                setup_gizmos,
                spawn_scenery,
                setup_minimap,
                setup_fog,
                controls::setup_overlay,
                controls::setup_cursors,
                controls::setup_hints,
                controls::setup_wheel,
                controls::setup_sounds,
                hud_view::setup_hud_art,
                hud_view::setup_hud_view,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                (
                    shot_driver,
                    controls::hot_reload_system,
                    controls::overlay_input_system,
                    input_system,
                    sim_driver,
                    camera_system,
                    units_system,
                    features_system,
                    effects_system,
                    animate_sprites,
                    showcase_system,
                    fog_system,
                ),
                (
                    hud_system,
                    hud_view::hud_mode_system,
                    hud_view::panel_system,
                    hud_view::build_grid_system,
                    hud_view::unit_card_system,
                    hud_view::strip_system,
                    hud_view::bottom_bar_system,
                    banner_system,
                    minimap_system,
                    gizmo_system,
                    controls::cursor_system,
                    controls::hints_system,
                    controls::wheel_system,
                    controls::overlay_draw_system,
                    controls::sound_system,
                    controls::rumble_system,
                    fps_report,
                ),
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
            clicks: args
                .click
                .chunks(2)
                .filter(|c| c.len() == 2)
                .map(|c| (c[0], c[1]))
                .collect(),
            pad_events,
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

/// `--synth-events`: drive the match headless with scripted gamepad events
/// through the real mapper and write a session recording.
fn synth(
    args: &Args,
    vfs: &Vfs,
    profile: Profile,
    events: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let evs: Vec<InputEvent> = serde_json::from_str(&std::fs::read_to_string(events)?)
        .map_err(|e| format!("{}: {e}", events.display()))?;
    let mut session = Session::load(
        vfs,
        &args.map,
        &args.players,
        args.world_seed,
        tak_commands::CommandLog::new(),
    )?;
    let mut mapper = Mapper::new(profile);
    let rec = crate::feel::synth_recording(
        &mut session,
        &mut mapper,
        &evs,
        args.synth_ticks,
        &args.players,
        args.world_seed,
    );
    let out = args.record_session.as_ref().ok_or("--record-session")?;
    std::fs::write(out, rec.to_json())?;
    eprintln!(
        "wrote {} ({} frames, {} orders, final hash {:016x}): {:?}",
        out.display(),
        rec.frames.len(),
        session.log.len(),
        session.last_hash().unwrap_or(0),
        rec.summary()
    );
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
    mut feel: ResMut<Feel>,
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
    controls::write_recording(&mut feel, s);
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
    // Match-end banner, centred near the top; display only (input still
    // works behind it).
    let banner = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(22.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            GlobalZIndex(700),
            HudBanner,
        ))
        .id();
    let plate = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(36.0), Val::Px(10.0)),
                border: UiRect::all(Val::Px(2.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            BorderColor::all(Color::srgb(0.75, 0.62, 0.3)),
            ChildOf(banner),
        ))
        .id();
    commands.spawn((
        Text::new(""),
        hud_text(56.0),
        HudBannerTitle,
        ChildOf(plate),
    ));
    commands.spawn((Text::new(""), hud_text(16.0), HudBannerText, ChildOf(plate)));
}

/// Screenshot mode: run the sim to each requested tick, let the frame
/// settle, capture, and exit after the last one.
fn shot_driver(
    mut commands: Commands,
    plan: Option<ResMut<ShotPlan>>,
    mut sim: NonSendMut<SimRes>,
    mut cam: ResMut<CamState>,
    mut input: ResMut<InputRes>,
    ground: Res<Ground>,
    mut feel: ResMut<Feel>,
    mut exit: MessageWriter<AppExit>,
    hud_buttons: Query<(
        &ButtonKind,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
    )>,
    hud_panel: Query<(&ComputedNode, &UiGlobalTransform), With<HudPanel>>,
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
        cam.input_h = centre_height(&cam.geom, &ground.0);
        input.mapper.set_camera(cam.input_camera());
        if !plan.select.is_empty() {
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
    // `--click`: HUD clicks once the layout exists, through the same hit
    // test and key path as a real click.
    if plan.wait == 10 && !plan.pad_events.is_empty() {
        let simr: &mut SimRes = &mut sim;
        let tick = simr.session.tick();
        let view = simr.session.view();
        let base = 3_000_000 + plan.next as u32 * 100_000;
        let mut last = base;
        let mut jump = false;
        for e in &plan.pad_events {
            last = base + e.t;
            let outs = input
                .mapper
                .handle(&InputEvent::new(last, e.kind.clone()), &view, tick);
            if !outs.is_empty() {
                eprintln!("  pad {:?} -> {outs:?}", e.kind);
            }
            jump |= outs.iter().any(|o| {
                matches!(
                    o,
                    Output::Client {
                        command: ClientCommand::JumpToAlert
                    }
                )
            });
        }
        let _ = input
            .mapper
            .handle(&InputEvent::new(last + 1, EventKind::Frame), &view, tick);
        if jump {
            for a in &simr.alerts.alerts {
                eprintln!("  alert at tick {}: {}", a.tick, a.text);
            }
            if let Some(a) = simr.alerts.next_jump() {
                eprintln!("  alert jump: {} at {:?}", a.text, a.pos);
                feel.toast = Some(a.text.clone());
                input.mapper.glide_to(a.pos);
            }
        }
        // Let glides (jumps, recentring) settle: 1.5 s of frames.
        for k in 0..90 {
            let _ = input.mapper.handle(
                &InputEvent::new(last + 2 + k * 16, EventKind::Frame),
                &view,
                tick,
            );
        }
        let pan = input.mapper.take_camera_pan();
        cam.geom.target[0] += f(pan.x);
        cam.geom.target[1] += f(pan.y);
        if let Some(z) = input.mapper.take_camera_zoom() {
            cam.geom.zoom = f(z);
        }
        cam.input_h = centre_height(&cam.geom, &ground.0);
        input.mapper.set_camera(cam.input_camera());
        eprintln!(
            "  pad cursor {:?} kind {:?} wheel {:?}",
            input.mapper.cursor_screen(),
            input.mapper.cursor_kind(&view),
            input
                .mapper
                .wheel()
                .map(|w| (w.kind, w.slots.len(), w.highlighted))
        );
    }
    if plan.wait == 10 {
        let clicks = plan.clicks.clone();
        for (x, y) in clicks {
            let hit = hud_hit(x, y, &hud_buttons, &hud_panel);
            eprintln!("  click ({x}, {y}): {hit:?}");
            if let Some(Some(k)) = hit {
                let tick = sim.session.tick();
                let view = sim.session.view();
                let outs = input.mapper.perform(k.action(), &view, tick);
                eprintln!("    -> {outs:?}");
            }
        }
    }
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
    let view = sim.session.view();
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
        Pause => "pause",
        Backspace => "backspace",
        Backquote => "grave",
        Equal => "equal",
        Minus => "minus",
        NumpadAdd => "equal",
        NumpadSubtract => "minus",
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
    hud_buttons: Query<(
        &ButtonKind,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
    )>,
    hud_panel: Query<(&ComputedNode, &UiGlobalTransform), With<HudPanel>>,
    mut feel: ResMut<Feel>,
) {
    if let Ok(win) = windows.single() {
        cam.geom.size = [win.width(), win.height()];
    }
    // Ghost playback: follow the recorded camera if asked.
    if feel.ghost_camera
        && let Some(g) = feel
            .ghost
            .as_ref()
            .and_then(|r| r.ghost_at(sim.session.tick().0))
    {
        let c = g.camera;
        let z = f(c.world_per_px);
        cam.geom.zoom = z;
        cam.geom.target = [
            f(c.origin.x) + c.width_px as f32 / 2.0 * z,
            f(c.origin.y) + c.height_px as f32 / 2.0 * z,
        ];
    }
    cam.input_h = centre_height(&cam.geom, &ground.0);
    let geom = cam.geom;
    let input_h = cam.input_h;
    input.mapper.set_camera(cam.input_camera());
    if plan.is_some() {
        return;
    }
    let ghost = feel.ghost.is_some();
    // Opening the overlay drops gestures in progress (no stuck holds).
    let overlay = feel.overlay_open;
    let t = time.elapsed().as_millis() as u32;
    let mut events = Vec::new();
    // Pointer: the terrain point under the mouse, as a virtual pixel.
    if let Some(pos) = windows.single().ok().and_then(|w| w.cursor_position()) {
        let (o, d) = geom.ray(pos.x, pos.y);
        if let Some(hit) = ground.0.raycast(o, d) {
            let v = geom.virtual_px_at(hit[0], hit[2], input_h);
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
    // HUD panel and buttons: a click there presses the button's key (the
    // same action as the keyboard) and never reaches the world.
    let mut pressed: Vec<Action> = Vec::new();
    if let Some(pos) = mouse_pos
        && mouse.just_pressed(MouseButton::Left)
        && !minimap.mouse_drag
        && let Some(hit) = hud_hit(pos.x, pos.y, &hud_buttons, &hud_panel)
    {
        minimap.ui_held = true;
        if let Some(k) = hit {
            pressed.push(k.action());
        }
    }
    let swallow_left = minimap.mouse_drag
        || minimap.ui_held
        || mouse.just_released(MouseButton::Left) && minimap.swallow_release;
    minimap.swallow_release = minimap.mouse_drag || minimap.ui_held;
    if !mouse.pressed(MouseButton::Left) {
        minimap.ui_held = false;
    }
    for (mb, b) in [
        (MouseButton::Left, InButton::MouseLeft),
        (MouseButton::Right, InButton::MouseRight),
        (MouseButton::Middle, InButton::MouseMiddle),
    ] {
        if (mb == MouseButton::Left && swallow_left) || overlay {
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
        if overlay {
            break;
        }
        if let Some(n) = key_name(*k) {
            events.push(EventKind::KeyDown { key: Key(n) });
        }
    }
    for k in keys.get_just_released() {
        if let Some(n) = key_name(*k) {
            events.push(EventKind::KeyUp { key: Key(n) });
        }
    }
    // One pad drives the mapper (not while the overlay or a ghost has it).
    if let Some(pad) = pads.iter().next().filter(|_| !overlay && !ghost) {
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
        if pad.just_pressed(GamepadButton::South)
            && let Some(hit) = hud_hit(csp[0], csp[1], &hud_buttons, &hud_panel)
        {
            minimap.pad_swallow = true;
            if let Some(k) = hit {
                pressed.push(k.action());
            }
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

    for ev in wheel.read() {
        let steps = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y / 40.0,
        };
        let milli = (steps * 1000.0).round() as i32;
        if milli != 0 {
            events.push(EventKind::Scroll { milli });
        }
    }
    if ghost {
        // Watching a ghost: the camera only (wheel zoom, arrows below).
        events.retain(|e| matches!(e, EventKind::Scroll { .. }));
    }
    events.push(EventKind::Frame);

    let tick = sim.session.tick();
    let mut outs = Vec::new();
    {
        let view = sim.session.view();
        for kind in &events {
            outs.extend(
                input
                    .mapper
                    .handle(&InputEvent::new(t, kind.clone()), &view, tick),
            );
        }
        // HUD buttons carry out their action directly.
        for a in pressed {
            outs.extend(input.mapper.perform(a, &view, tick));
        }
    }
    let mut submitted = Vec::new();
    for o in &outs {
        match o {
            Output::Sim { command, .. } => {
                let at = sim.session.submit(command.clone());
                submitted.push((at.0, command.clone()));
                feel.sounds.extend(controls::order_sound(command));
                debug!("order for tick {}", at.0);
            }
            Output::ToggleOverlay => {
                feel.overlay_open = !feel.overlay_open;
                if feel.overlay_open {
                    let p = input.mapper.profile().clone();
                    input.mapper.set_profile(p);
                }
            }
            Output::Client { command } => match command {
                // The mapper's camera maps the ground under the screen
                // centre, so glides go to true ground positions.
                ClientCommand::CenterOnMonarch => {
                    let w = sim.session.sim.world();
                    if let Some(p) = w
                        .players()
                        .get(usize::from(sim.session.local.0))
                        .and_then(|p| p.monarch)
                        .and_then(|id| w.unit(id))
                    {
                        input.mapper.glide_to(p.pos);
                    }
                }
                ClientCommand::CycleIdleBuilder => {
                    if let Some(id) = input.mapper.selection().iter().next()
                        && let Some(u) = sim.session.sim.world().unit(*id)
                    {
                        input.mapper.glide_to(u.pos);
                    }
                }
                ClientCommand::JumpToAlert => jump_to_alert(&mut sim, &mut input, &mut feel),
                _ => {}
            },
            Output::SelectAll { .. } => {}
            Output::GameSpeed { faster } => {
                sim.speed = if *faster {
                    (sim.speed * 2.0).min(8.0)
                } else {
                    (sim.speed / 2.0).max(0.25)
                };
            }
            Output::TogglePause => sim.paused = !sim.paused,
            Output::ToggleDamageBars => {
                feel.damage_bars = (feel.damage_bars + 1) % 3;
                feel.toast = Some(
                    crate::hud::DamageBars::from_cycle(feel.damage_bars)
                        .label()
                        .into(),
                );
            }
        }
    }
    let fb = input.mapper.take_feedback();
    if let Some(rec) = feel.recorder.as_mut() {
        rec.frame(
            &input.mapper,
            t,
            tick.0,
            tak_input::FrameIo {
                events: &events,
                outputs: &outs,
                submitted: &submitted,
                feedback: &fb,
            },
        );
    }
    feel.feedback.extend(fb);
    // Camera: mapper pan (right stick, edge pan, glides) and zoom (wheel,
    // RB + right stick), then the arrow keys.
    let pan = input.mapper.take_camera_pan();
    cam.geom.target[0] += f(pan.x);
    cam.geom.target[1] += f(pan.y);
    if let Some(z) = input.mapper.take_camera_zoom() {
        cam.geom.zoom = f(z);
    }
    // Hand the moved camera back at once, so the cursor and reticle drawn
    // this frame already ride with it.
    cam.input_h = centre_height(&cam.geom, &ground.0);
    input.mapper.set_camera(cam.input_camera());
    let dt = time.delta_secs();
    let speed = if overlay {
        0.0
    } else {
        900.0 * cam.geom.zoom * dt
    };
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
    // Game speed and pause come from the profile (TA Pause, + / -) as
    // mapper outputs above: client only, ticks per wall second.
}

/// Alert jump (View / F3): the camera glides to the newest alert, then
/// older ones on each press.
fn jump_to_alert(sim: &mut SimRes, input: &mut InputRes, feel: &mut Feel) {
    match sim.alerts.next_jump() {
        Some(a) => {
            let (pos, text) = (a.pos, a.text.clone());
            input.mapper.glide_to(pos);
            feel.toast = Some(text);
        }
        None => feel.toast = Some("No alerts".into()),
    }
}

/// Terrain height under the screen centre (0 off the map).
fn centre_height(geom: &ViewGeom, ground: &TerrainData) -> f32 {
    let (o, d) = geom.ray(geom.size[0] / 2.0, geom.size[1] / 2.0);
    ground.raycast(o, d).map_or(0.0, |p| p[1])
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
    fog: Res<FogRes>,
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
        // Enemies outside the local player's sight are not drawn.
        let shown = PlayerId(u.owner) == sim.session.local || fog_visible(&fog, &sim, x, y);
        if let Ok(mut v) = v_q.get_mut(e) {
            let want = if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *v != want {
                *v = want;
            }
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

/// Visual fog of war (`fog.rs`): one overlay mesh per terrain chunk, its
/// vertex colours black with the fog opacity.
#[derive(Resource, Default)]
struct FogRes {
    enabled: bool,
    /// Per chunk: mesh, first corner (x, z), corners (nx, nz).
    chunks: Vec<(Handle<Mesh>, i64, i64, i64, i64)>,
    /// Minimap fog image.
    mini: Option<Handle<Image>>,
    /// Shown state last applied (to react to toggling).
    applied: Option<bool>,
}

/// Height the fog layer is moved toward the viewer along the view ray: it
/// stays over the same screen pixels but in front of everything below it
/// (the tallest models are about 200 high), so units and features in
/// unexplored ground are hidden too.
const FOG_LIFT: f32 = 400.0;

#[derive(Component)]
struct FogMini;

fn setup_fog(
    mut commands: Commands,
    ground: Res<Ground>,
    mut fog: ResMut<FogRes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mm_root: Query<Entity, With<MinimapRoot>>,
) {
    let t = &ground.0;
    let mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        depth_bias: FOG_SORT_BIAS,
        ..default()
    });
    let b = Vec3::from_array(ViewGeom::back()) * FOG_LIFT;
    for chunk in &t.chunks {
        let cx0 = i64::from(chunk.tile0.0 * 2);
        let cz0 = i64::from(chunk.tile0.1 * 2);
        let nx = i64::from(chunk.tiles.0 * 2);
        let nz = i64::from(chunk.tiles.1 * 2);
        let mut pos = Vec::new();
        for z in 0..=nz {
            for x in 0..=nx {
                let (cx, cz) = (cx0 + x, cz0 + z);
                let p = Vec3::new(
                    cx as f32 * terrain::CELL,
                    t.corner(cx, cz),
                    cz as f32 * terrain::CELL,
                ) + b;
                pos.push(p.to_array());
            }
        }
        let row = (nx + 1) as u32;
        let mut idx = Vec::new();
        for z in 0..nz as u32 {
            for x in 0..nx as u32 {
                let i = z * row + x;
                idx.extend_from_slice(&[i, i + row, i + 1, i + 1, i + row, i + row + 1]);
            }
        }
        let n = pos.len();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; n]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0, 0.0, 0.0, 1.0]; n]);
        mesh.insert_indices(Indices::U32(idx));
        let h = meshes.add(mesh);
        commands.spawn((
            Mesh3d(h.clone()),
            MeshMaterial3d(mat.clone()),
            FogMesh(fog.chunks.len()),
        ));
        fog.chunks.push((h, cx0, cz0, nx, nz));
    }
    // Minimap overlay: one texel per cell (heights ignored: the minimap is
    // screen-space, so fog edges sit up to a few pixels off on hills).
    if let Ok(root) = mm_root.single() {
        let (w, h) = t.cells;
        let img = images.add(rgba_image(w, h, vec![0; (w * h * 4) as usize]));
        fog.mini = Some(img.clone());
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            ImageNode {
                image: img,
                image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                ..default()
            },
            ZIndex(1),
            FogMini,
            ChildOf(root),
        ));
    }
}

/// A fog overlay chunk (its index in `FogRes::chunks`).
#[derive(Component)]
struct FogMesh(usize);

/// Recompute the fog from the local player's units after each tick and
/// repaint the overlay; F9 toggles it.
fn fog_system(
    mut sim: NonSendMut<SimRes>,
    keys: Res<ButtonInput<KeyCode>>,
    mut fog: ResMut<FogRes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut vis: Query<&mut Visibility, Or<(With<FogMesh>, With<FogMini>)>>,
    fog_meshes: Query<(Entity, &FogMesh)>,
) {
    let mut chunk_entities: Vec<(usize, Entity)> =
        fog_meshes.iter().map(|(e, m)| (m.0, e)).collect();
    chunk_entities.sort();
    let chunk_entities: Vec<Entity> = chunk_entities.into_iter().map(|(_, e)| e).collect();
    if keys.just_pressed(KeyCode::F9) {
        fog.enabled = !fog.enabled;
    }
    let enabled = fog.enabled;
    if fog.applied != Some(enabled) {
        fog.applied = Some(enabled);
        for mut v in &mut vis {
            *v = if enabled {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
        // Repaint (re-applies per-chunk visibility).
        sim.fog_changed = true;
    }
    if !sim.fog_changed {
        return;
    }
    sim.fog_changed = false;
    let grid = &sim.fog;
    let FogRes { chunks, mini, .. } = &mut *fog;
    for (i, (h, cx0, cz0, nx, nz)) in chunks.iter().enumerate() {
        let Some(mut mesh) = meshes.get_mut(h) else {
            continue;
        };
        let mut cols = Vec::with_capacity(((nx + 1) * (nz + 1)) as usize);
        let mut any = false;
        for z in 0..=*nz {
            for x in 0..=*nx {
                let a = grid.corner_alpha(cx0 + x, cz0 + z);
                any |= a > 0.0;
                cols.push([0.0, 0.0, 0.0, a]);
            }
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, cols);
        // Fully clear chunks are skipped (saves a blended pass over them).
        if let Some(e) = chunk_entities.get(i)
            && let Ok(mut v) = vis.get_mut(*e)
        {
            let want = if any && enabled {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *v != want {
                *v = want;
            }
        }
    }
    if let Some(img) = mini.as_ref()
        && let Some(mut im) = images.get_mut(img)
    {
        let (gw, gh) = grid.size;
        let mut px = Vec::with_capacity((gw * gh * 4) as usize);
        for z in 0..i64::from(gh) {
            for x in 0..i64::from(gw) {
                let a = match grid.cell(x, z) {
                    crate::fog::FogCell::Unexplored => MINIMAP_UNEXPLORED_ALPHA,
                    crate::fog::FogCell::Explored => 140,
                    crate::fog::FogCell::Seen => 0,
                };
                px.extend_from_slice(&[0, 0, 0, a]);
            }
        }
        im.data = Some(px);
    }
}

/// Minimap fog opacity over unexplored cells (**Chosen**): dark but not
/// black, so the map's shape reads from the first tick (TA:K-style: the
/// minimap is a map, not a scouting record; units there stay hidden).
/// 255 would match the main view's black.
const MINIMAP_UNEXPLORED_ALPHA: u8 = 205;

/// Whether the local player can see a world point (always, with the fog
/// off).
fn fog_visible(fog: &FogRes, sim: &SimRes, x: f32, z: f32) -> bool {
    !fog.enabled || sim.fog.at(x, z) == crate::fog::FogCell::Seen
}

/// Sight radius for units whose `sightdistance` is 0 (**Chosen**).
const FOG_MIN_SIGHT: f32 = 32.0;

/// Sort bias of the fog material: transparent items are drawn back to
/// front by view depth plus this, so the fog comes after every other
/// translucent surface (water, shadows, ghosts, effects). A second camera
/// for the overlay cost about 3 fps on lavapipe.
const FOG_SORT_BIAS: f32 = 100_000.0;

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
    /// The left button went down on the HUD panel.
    ui_held: bool,
    /// Full-screen Crystal Ball (TA Tab, hold View).
    big: bool,
}

impl MinimapState {
    /// Where the minimap is for a viewport (bottom-right corner, fitted
    /// to the window keeping the image's aspect: [`MinimapGeom::fit`]).
    fn geom(&self, view: [f32; 2]) -> Option<MinimapGeom> {
        Some(if self.big {
            MinimapGeom::full(view, self.image?, self.map)
        } else {
            MinimapGeom::fit(view, self.image?, self.map)
        })
    }
}

/// The minimap panel (frame).
#[derive(Component)]
struct MinimapPanel;
/// The minimap image; fog, dots and the camera rectangle are its children,
/// so they scale with it.
#[derive(Component)]
struct MinimapRoot;
#[derive(Component)]
struct MinimapView;
#[derive(Component)]
struct MinimapDot(usize);
/// An alert ping on the minimap: an expanding, fading ring.
#[derive(Component)]
struct MinimapPing(usize);
const MINIMAP_PINGS: usize = 8;

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
    // Sizes and places are set every frame from `MinimapGeom::fit`.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
        BorderColor::all(Color::srgb(0.45, 0.4, 0.3)),
        MinimapPanel,
    ));
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                overflow: Overflow::clip(),
                ..default()
            },
            ImageNode {
                image: img,
                image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                ..default()
            },
            ZIndex(1),
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
        ZIndex(2),
        MinimapView,
        ChildOf(root),
    ));
    for i in 0..MINIMAP_PINGS {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BorderColor::all(Color::WHITE),
            Visibility::Hidden,
            ZIndex(3),
            MinimapPing(i),
            ChildOf(root),
        ));
    }
}

/// Unit dots in team colours and the camera rectangle.
fn minimap_system(
    mut commands: Commands,
    sim: NonSend<SimRes>,
    cam: Res<CamState>,
    input: Res<InputRes>,
    mut mm: ResMut<MinimapState>,
    fog: Res<FogRes>,
    mut root: Query<(Entity, &mut Node), (With<MinimapRoot>, Without<MinimapPing>)>,
    mut panel: Query<
        &mut Node,
        (
            With<MinimapPanel>,
            Without<MinimapRoot>,
            Without<MinimapView>,
            Without<MinimapDot>,
            Without<MinimapPing>,
        ),
    >,
    mut view: Query<
        &mut Node,
        (
            With<MinimapView>,
            Without<MinimapDot>,
            Without<MinimapRoot>,
            Without<MinimapPing>,
        ),
    >,
    mut dots: Query<
        (
            &MinimapDot,
            &mut Node,
            &mut BackgroundColor,
            &mut Visibility,
        ),
        (
            Without<MinimapView>,
            Without<MinimapRoot>,
            Without<MinimapPing>,
        ),
    >,
    mut pings: Query<
        (&MinimapPing, &mut Node, &mut BorderColor, &mut Visibility),
        (
            Without<MinimapView>,
            Without<MinimapRoot>,
            Without<MinimapDot>,
            Without<MinimapPanel>,
        ),
    >,
) {
    let Ok((root, mut root_node)) = root.single_mut() else {
        return;
    };
    let big = input.mapper.crystal_ball();
    if mm.big != big {
        mm.big = big;
    }
    let Some(g) = mm.geom(cam.geom.size) else {
        return;
    };
    let place = |n: &mut Node, r: [f32; 4]| {
        let want = (Val::Px(r[0]), Val::Px(r[1]), Val::Px(r[2]), Val::Px(r[3]));
        if (n.left, n.top, n.width, n.height) != want {
            (n.left, n.top, n.width, n.height) = want;
        }
    };
    place(&mut root_node, g.rect);
    if let Ok(mut n) = panel.single_mut() {
        place(&mut n, g.panel);
    }
    if let Ok(mut n) = view.single_mut() {
        let r = g.view_rect(&cam.geom);
        n.left = Val::Px(r[0]);
        n.top = Val::Px(r[1]);
        n.width = Val::Px(r[2].max(2.0));
        n.height = Val::Px(r[3].max(2.0));
    }
    // Alert pings: rings that grow and fade over three seconds.
    let tick = sim.session.tick().0;
    let w = sim.session.sim.world();
    let live: Vec<_> = sim.alerts.pings(tick).collect();
    for (pg, mut n, mut bc, mut v) in &mut pings {
        match live.iter().rev().nth(pg.0) {
            Some((a, age)) => {
                let h = f(w.map().height_at(a.pos));
                let p = g.to_mini(f(a.pos.x), f(a.pos.y) - crate::view::SHEAR * h);
                let k = *age as f32 / crate::alerts::PING_TICKS as f32;
                let size = 8.0 + 34.0 * k;
                n.left = Val::Px(p[0] - size / 2.0);
                n.top = Val::Px(p[1] - size / 2.0);
                n.width = Val::Px(size);
                n.height = Val::Px(size);
                let [r, gg, b] = a.kind.rgb();
                *bc = BorderColor::all(Color::srgba(r, gg, b, 1.0 - 0.8 * k));
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
    let units = &sim.curr.units;
    let mut have = 0;
    for (d, mut n, mut bg, mut v) in &mut dots {
        have = have.max(d.0 + 1);
        let unit = units.get(d.0).filter(|u| {
            PlayerId(u.owner) == sim.session.local || fog_visible(&fog, &sim, u.x, u.y)
        });
        match unit {
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
            ZIndex(2),
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
    feel: Res<Feel>,
    mut top: Query<&mut Text, With<HudTop>>,
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
            let side = s
                .def(p.monarch_def)
                .map_or("-", |d| &d.unitname[..3.min(d.unitname.len())]);
            if w.is_defeated(p.id) {
                line.push_str(&format!("   P{} {}: DEFEATED", p.id.0, side));
            } else if let Some(e) = tak_sim::economy::player(w, p.id) {
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
        if let Some((m, _)) = &feel.status {
            line.push_str(&format!("   [{m}]"));
        }
        if let Some(r) = &feel.ghost {
            line.push_str(&format!("   GHOST {} frames", r.frames.len()));
        }
        line.push_str(&format!(
            "   input: {}",
            input.mapper.tuning().preset_name().unwrap_or("custom")
        ));
        t.0 = line;
    }
}

/// The match-end banner for the local player, once the sim has decided
/// the match (`tak_sim::victory`) and [`BANNER_DELAY_TICKS`] have passed:
/// (title, colour, subtitle).
fn banner_for(s: &Session) -> Option<(String, Color, String)> {
    let w = s.sim.world();
    let tak_sim::victory::MatchState::Over { winners } = s.sim.match_state() else {
        return None;
    };
    let at = w.victory().decided_at()?;
    if s.tick().0 < at.0.saturating_add(BANNER_DELAY_TICKS) {
        return None;
    }
    let side = |p: PlayerId| {
        w.players()
            .iter()
            .find(|x| x.id == p)
            .and_then(|x| s.def(x.monarch_def))
            .map_or_else(
                || "?".to_string(),
                |d| d.unitname[..3.min(d.unitname.len())].to_string(),
            )
    };
    let names: Vec<String> = winners
        .iter()
        .map(|&p| format!("P{} {}", p.0, side(p)))
        .collect();
    let when = format!("{:.0}s", at.0 as f32 / 30.0);
    Some(if winners.is_empty() {
        (
            "Draw".into(),
            Color::srgb(0.85, 0.85, 0.85),
            format!("Everyone was defeated at {when}"),
        )
    } else if winners.contains(&s.local) {
        (
            "Victory".into(),
            Color::srgb(1.0, 0.82, 0.3),
            format!("{} won at {when}", names.join(", ")),
        )
    } else {
        (
            "Defeat".into(),
            Color::srgb(0.9, 0.3, 0.25),
            format!("{} won at {when}", names.join(", ")),
        )
    })
}

/// Show or hide the match-end banner.
fn banner_system(
    sim: NonSend<SimRes>,
    mut root: Query<&mut Visibility, With<HudBanner>>,
    mut title: Query<(&mut Text, &mut TextColor), (With<HudBannerTitle>, Without<HudBannerText>)>,
    mut text: Query<&mut Text, (With<HudBannerText>, Without<HudBannerTitle>)>,
) {
    let b = banner_for(&sim.session);
    if let Ok(mut v) = root.single_mut() {
        let want = if b.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    let Some((t, c, sub)) = b else { return };
    if let Ok((mut tt, mut tc)) = title.single_mut() {
        if tt.0 != t {
            tt.0 = t;
        }
        tc.0 = c;
    }
    if let Ok(mut st) = text.single_mut()
        && st.0 != sub
    {
        st.0 = sub;
    }
}

/// What a click at a screen point hits on the HUD: a button, or just the
/// panel (swallowed so it does not select or order behind it).
fn hud_hit(
    sx: f32,
    sy: f32,
    buttons: &Query<(
        &ButtonKind,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
    )>,
    panel: &Query<(&ComputedNode, &UiGlobalTransform), With<HudPanel>>,
) -> Option<Option<ButtonKind>> {
    let inside = |n: &ComputedNode, t: &UiGlobalTransform| {
        let c = t.translation;
        let h = n.size / 2.0;
        sx >= c.x - h.x && sx <= c.x + h.x && sy >= c.y - h.y && sy <= c.y + h.y
    };
    for (k, n, t, v) in buttons {
        if v.get() && inside(n, t) {
            return Some(Some(*k));
        }
    }
    panel.iter().any(|(n, t)| inside(n, t)).then_some(None)
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
    fog: Res<FogRes>,
    sim: NonSend<SimRes>,
    input: Res<InputRes>,
    ground: Res<Ground>,
    plan: Option<Res<ShotPlan>>,
    feel: Res<Feel>,
    art: Option<Res<controls::CursorArt>>,
) {
    let t = &ground.0;
    let ghost = feel
        .ghost
        .as_ref()
        .and_then(|r| r.ghost_at(sim.session.tick().0));
    let ghost_sel = ghost
        .as_ref()
        .map(|g| g.selection.clone())
        .unwrap_or_default();
    if let Some(gh) = &ghost {
        // The recorded player's cursor and brush.
        let (cx, cz) = (f(gh.cursor_world.x), f(gh.cursor_world.y));
        let c = Color::srgb(1.0, 0.4, 1.0);
        ground_circle(&mut g, t, cx, cz, 9.0, c);
        let ch = t.height_at(cx, cz) + 1.5;
        g.line(
            Vec3::new(cx - 14.0, ch, cz),
            Vec3::new(cx + 14.0, ch, cz),
            c,
        );
        g.line(
            Vec3::new(cx, ch, cz - 14.0),
            Vec3::new(cx, ch, cz + 14.0),
            c,
        );
        if let Some((bc, br)) = gh.brush {
            ground_circle(&mut g, t, f(bc.x), f(bc.y), f(br), c);
        }
    }
    let alpha = sim.alpha();
    let prev: BTreeMap<UnitId, &UnitSnap> = sim.prev.units.iter().map(|u| (u.id, u)).collect();
    let w = sim.session.sim.world();
    let local = sim.session.local;
    for u in &sim.curr.units {
        let p = prev.get(&u.id).copied();
        let x = lerp(p.map_or(u.x, |p| p.x), u.x, alpha);
        let y = lerp(p.map_or(u.y, |p| p.y), u.y, alpha);
        if PlayerId(u.owner) != local && !fog_visible(&fog, &sim, x, y) {
            continue;
        }
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
            // Selection flash (`select_flash_ms`): a bright ring that
            // shrinks onto the normal one.
            match input.mapper.selection_flash() {
                Some(age) => {
                    let k = 1.0 - age as f32 / input.mapper.tuning().select_flash_ms.max(1) as f32;
                    ground_circle(
                        &mut g,
                        t,
                        x,
                        y,
                        r + 3.0 + 8.0 * k,
                        Color::srgb(0.85, 1.0, 0.85),
                    );
                    ground_circle(&mut g, t, x, y, r + 3.0, Color::srgb(0.6, 1.0, 0.6));
                }
                None => ground_circle(&mut g, t, x, y, r + 3.0, Color::srgb(0.3, 1.0, 0.3)),
            }
        }
        if ghost_sel.contains(u.id) {
            ground_circle(&mut g, t, x, y, r + 6.0, Color::srgb(1.0, 0.4, 1.0));
        }
        let bars = crate::hud::DamageBars::from_cycle(feel.damage_bars);
        if bars.show(selected, u.health_percent < 100, u.build_percent_left > 0) {
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
    if (plan.is_some() && input.mapper.brush().is_none() && !feel.shot_pad) || ghost.is_some() {
        return;
    }
    let c = input.mapper.cursor_world();
    let (cx, cz) = (f(c.x), f(c.y));
    if art.as_ref().is_some_and(|a| a.loaded()) {
        // The game cursor sprite shows the cursor; mark the ground point
        // under a stick cursor.
        if input.mapper.stick_cursor() {
            ground_circle(&mut g, t, cx, cz, 3.0, Color::srgb(1.0, 1.0, 0.2));
        }
    } else {
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
    }
    // Aim assist: a thin ring on the snap target.
    if let Some(u) = input
        .mapper
        .snap_target()
        .and_then(|id| sim.curr.units.iter().find(|u| u.id == id))
    {
        let r = sim
            .session
            .def(u.def)
            .map_or(14.0, |d| pick_radius(d.footprint.0, d.footprint.1));
        ground_circle(
            &mut g,
            t,
            u.x,
            u.y,
            r + 9.0,
            Color::srgba(1.0, 1.0, 0.3, 0.7),
        );
    }
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
        let ok = w.defs().get(def).is_some_and(|dd| {
            use tak_sim::construction::placement::{check_site, site_center};
            check_site(w, def, site_center(dd, c), None).is_ok()
        });
        let ghost = if ok {
            Color::srgb(0.3, 1.0, 0.35)
        } else {
            Color::srgb(1.0, 0.25, 0.2)
        };
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
                ghost,
            );
        }
    }
}
