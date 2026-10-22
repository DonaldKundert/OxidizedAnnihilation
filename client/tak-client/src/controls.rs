//! WP-S controller feel, Bevy side: the live tuning overlay, profile hot
//! reload, the game's own context cursors (`anims/CURSORS.GAF`), controller
//! glyph hints, the radial wheel, rumble, and session recording / ghost
//! state. Feel logic and every tuning value live in `tak-input`; this
//! module adapts devices, draws, and hosts the overlay.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use bevy::input::gamepad::{GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy::window::CursorOptions;
use tak_input::{CursorKind, Feedback, PRESETS, Recorder, Recording, TUNING_FIELDS, Tuning};

use super::*;
use crate::feel::{self, Glyph, HintKey, Layout, Symbol};

/// Feel state shared by the input adapter and the controls systems.
#[derive(Resource)]
pub(super) struct Feel {
    /// Glyph layout in use.
    pub layout: Layout,
    /// `--glyphs` (auto, xbox, playstation).
    pub glyph_choice: String,
    /// Profile file (hot reload and overlay saves).
    pub profile_path: Option<PathBuf>,
    pub profile_mtime: Option<SystemTime>,
    poll_timer: f32,
    /// Message for the overlay / HUD and seconds left.
    pub status: Option<(String, f32)>,
    pub overlay_open: bool,
    overlay_row: usize,
    /// Feedback drained from the mapper this frame.
    pub feedback: Vec<Feedback>,
    /// Session recording (`--record-session`).
    pub recorder: Option<Recorder>,
    pub record_path: Option<PathBuf>,
    pub players: String,
    pub seed: u64,
    /// Ghost playback (`--ghost`).
    pub ghost: Option<Recording>,
    pub ghost_camera: bool,
    last_own_health: Option<i64>,
    /// Screenshot mode has scripted pad events, so draw the stick cursor.
    pub shot_pad: bool,
    /// Damage bars (TA ~): 0 all (selected, damaged, frames), 1 selected
    /// only, 2 off.
    pub damage_bars: u8,
    /// `--hud`: auto, console or classic.
    pub hud_choice: String,
    /// A one-off bottom-bar message (e.g. the damage-bar mode).
    pub toast: Option<String>,
    /// Order acknowledgements to play this frame (`Sounds/*.WAV` names).
    pub sounds: Vec<&'static str>,
}

impl Feel {
    pub fn new(args: &Args, profile_mtime: Option<SystemTime>) -> Feel {
        Feel {
            layout: feel::detect_layout(&args.glyphs, None, None),
            glyph_choice: args.glyphs.clone(),
            profile_path: args.input_profile.clone(),
            profile_mtime,
            poll_timer: 0.0,
            status: None,
            overlay_open: args.overlay,
            overlay_row: 0,
            feedback: Vec::new(),
            recorder: None,
            record_path: args.record_session.clone(),
            players: args.players.clone(),
            seed: args.world_seed,
            ghost: None,
            ghost_camera: args.ghost_camera,
            last_own_health: None,
            shot_pad: args.pad_events.is_some(),
            damage_bars: 0,
            hud_choice: args.hud.to_ascii_lowercase(),
            toast: None,
            sounds: Vec::new(),
        }
    }

    fn say(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        eprintln!("input: {m}");
        self.status = Some((m, 4.0));
    }
}

// ---- profile hot reload and saving ------------------------------------------

/// Poll the profile file twice a second; apply a valid change at once.
pub(super) fn hot_reload_system(
    time: Res<Time>,
    mut feel: ResMut<Feel>,
    mut input: ResMut<InputRes>,
) {
    if let Some((_, left)) = feel.status.as_mut() {
        *left -= time.delta_secs();
        if *left <= 0.0 {
            feel.status = None;
        }
    }
    feel.poll_timer += time.delta_secs();
    if feel.poll_timer < 0.5 {
        return;
    }
    feel.poll_timer = 0.0;
    let Some(path) = feel.profile_path.clone() else {
        return;
    };
    let mut mt = feel.profile_mtime;
    let polled = feel::poll_profile(&path, &mut mt);
    feel.profile_mtime = mt;
    match polled {
        Some(Ok(p)) => {
            input.mapper.set_profile(p);
            let name = input.mapper.tuning().preset_name().unwrap_or("custom");
            feel.say(format!("reloaded {} ({name})", path.display()));
        }
        Some(Err(e)) => feel.say(format!("not reloaded: {e}")),
        None => {}
    }
}

fn save(feel: &mut Feel, input: &InputRes) {
    let path = feel
        .profile_path
        .clone()
        .unwrap_or_else(feel::default_profile_path);
    match feel::save_profile(&path, input.mapper.profile()) {
        Ok(t) => {
            if feel.profile_path.is_none() {
                feel.profile_path = Some(path.clone());
            }
            feel.profile_mtime = Some(t);
            feel.say(format!("saved {}", path.display()));
        }
        Err(e) => feel.say(format!("save failed: {}: {e}", path.display())),
    }
}

// ---- overlay ----------------------------------------------------------------

/// Rows shown at once.
const OVERLAY_ROWS: usize = 22;
/// Row 0 is the preset, then every field, then "save".
fn overlay_len() -> usize {
    TUNING_FIELDS.len() + 2
}

#[derive(Component)]
pub(super) struct OverlayRoot;
#[derive(Component)]
pub(super) struct OverlayTitle;
#[derive(Component)]
pub(super) struct OverlayHelp;
#[derive(Component)]
pub(super) struct OverlayRow(usize);
#[derive(Component)]
pub(super) struct OverlayLabel(usize);
#[derive(Component)]
pub(super) struct OverlayValue(usize);
/// A −/+ button on a visible row.
#[derive(Component, Clone, Copy)]
pub(super) struct OverlayBtn(usize, i64);

pub(super) fn setup_overlay(mut commands: Commands) {
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(8.0),
                top: Val::Px(40.0),
                width: Val::Px(560.0),
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.04, 0.06, 0.88)),
            BorderColor::all(Color::srgb(0.55, 0.48, 0.3)),
            GlobalZIndex(900),
            Visibility::Hidden,
            OverlayRoot,
        ))
        .id();
    commands.spawn((
        Text::new("Tuning"),
        hud_text(16.0),
        OverlayTitle,
        ChildOf(root),
    ));
    for i in 0..OVERLAY_ROWS {
        let row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(6.0),
                    padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                OverlayRow(i),
                ChildOf(root),
            ))
            .id();
        commands.spawn((
            Text::new(""),
            hud_text(13.0),
            Node {
                width: Val::Px(300.0),
                ..default()
            },
            OverlayLabel(i),
            ChildOf(row),
        ));
        for (delta, label) in [(-1, " - "), (1, " + ")] {
            let b = commands
                .spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(4.0), Val::Px(0.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.16, 0.14, 0.11)),
                    BorderColor::all(Color::srgb(0.55, 0.48, 0.3)),
                    OverlayBtn(i, delta),
                    ChildOf(row),
                ))
                .id();
            commands.spawn((Text::new(label), hud_text(13.0), ChildOf(b)));
            if delta < 0 {
                commands.spawn((
                    Text::new(""),
                    hud_text(13.0),
                    Node {
                        width: Val::Px(120.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    OverlayValue(i),
                    ChildOf(row),
                ));
            }
        }
    }
    commands.spawn((Text::new(""), hud_text(12.0), OverlayHelp, ChildOf(root)));
}

fn first_visible(row: usize) -> usize {
    let n = overlay_len();
    row.saturating_sub(OVERLAY_ROWS / 2)
        .min(n.saturating_sub(OVERLAY_ROWS))
}

/// Apply one adjustment to overlay row `row`.
fn adjust(feel: &mut Feel, input: &mut InputRes, row: usize, steps: i64) {
    let n = TUNING_FIELDS.len();
    if row == 0 {
        let cur = input.mapper.tuning().preset_name();
        let i = cur
            .and_then(|c| PRESETS.iter().position(|p| *p == c))
            .map_or(0, |i| {
                (i as i64 + steps).rem_euclid(PRESETS.len() as i64) as usize
            });
        // A preset sets the feel values; the cursor mode (reticle or free)
        // and the right-stick role belong to the control scheme.
        let cur = *input.mapper.tuning();
        let t = Tuning {
            reticle: cur.reticle,
            right_stick_zoom: cur.right_stick_zoom,
            ..Tuning::preset(PRESETS[i]).unwrap_or_default()
        };
        input.mapper.set_tuning(t);
        feel.say(format!("preset {}", PRESETS[i]));
    } else if row <= n {
        let mut t = *input.mapper.tuning();
        if t.nudge(TUNING_FIELDS[row - 1].name, steps) {
            input.mapper.set_tuning(t);
        }
    } else if steps != 0 {
        save(feel, input);
    }
}

/// Overlay input: keyboard, gamepad and mouse while it is open. The input
/// adapter keeps these away from the mapper meanwhile.
pub(super) fn overlay_input_system(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    pads: Query<&Gamepad>,
    btns: Query<(&OverlayBtn, &ComputedNode, &UiGlobalTransform)>,
    rows: Query<(&OverlayRow, &ComputedNode, &UiGlobalTransform)>,
    mut feel: ResMut<Feel>,
    mut input: ResMut<InputRes>,
) {
    if !feel.overlay_open {
        return;
    }
    let n = overlay_len();
    let mut row = feel.overlay_row.min(n - 1);
    let mut steps: i64 = 0;
    let mut close = keys.just_pressed(KeyCode::F11) || keys.just_pressed(KeyCode::Escape);
    let big = |k: bool| if k { 10 } else { 1 };
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if keys.just_pressed(KeyCode::ArrowDown) {
        row = (row + 1) % n;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        row = (row + n - 1) % n;
    }
    if keys.just_pressed(KeyCode::PageDown) {
        row = (row + OVERLAY_ROWS).min(n - 1);
    }
    if keys.just_pressed(KeyCode::PageUp) {
        row = row.saturating_sub(OVERLAY_ROWS);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        steps += big(shift);
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        steps -= big(shift);
    }
    let mut do_save = keys.just_pressed(KeyCode::Enter)
        || (keys.just_pressed(KeyCode::KeyS)
            && (keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::SuperLeft)));
    if let Some(pad) = pads.iter().next() {
        if pad.just_pressed(GamepadButton::DPadDown) {
            row = (row + 1) % n;
        }
        if pad.just_pressed(GamepadButton::DPadUp) {
            row = (row + n - 1) % n;
        }
        if pad.just_pressed(GamepadButton::DPadRight) {
            steps += 1;
        }
        if pad.just_pressed(GamepadButton::DPadLeft) {
            steps -= 1;
        }
        if pad.just_pressed(GamepadButton::RightTrigger) {
            steps += 10;
        }
        if pad.just_pressed(GamepadButton::LeftTrigger) {
            steps -= 10;
        }
        if pad.just_pressed(GamepadButton::South) {
            steps += 1; // toggles booleans, next preset, saves on the last row
        }
        if pad.just_pressed(GamepadButton::North) {
            do_save = true;
        }
        if pad.just_pressed(GamepadButton::East) || pad.just_pressed(GamepadButton::Start) {
            close = true;
        }
    }
    // Mouse: the −/+ buttons, or a click on a row selects it.
    if mouse.just_pressed(MouseButton::Left)
        && let Some(p) = windows.single().ok().and_then(|w| w.cursor_position())
    {
        let inside = |n: &ComputedNode, t: &UiGlobalTransform| {
            let c = t.translation;
            let h = n.size / 2.0;
            p.x >= c.x - h.x && p.x <= c.x + h.x && p.y >= c.y - h.y && p.y <= c.y + h.y
        };
        let first = first_visible(row);
        if let Some((b, _, _)) = btns.iter().find(|(_, n, t)| inside(n, t)) {
            row = first + b.0;
            steps += b.1;
        } else if let Some((r, _, _)) = rows.iter().find(|(_, n, t)| inside(n, t)) {
            row = first + r.0;
        }
    }
    feel.overlay_row = row.min(n - 1);
    if steps != 0 {
        adjust(&mut feel, &mut input, row, steps);
    }
    if do_save {
        save(&mut feel, &input);
    }
    if close {
        feel.overlay_open = false;
    }
}

fn value_text(t: &Tuning, name: &str) -> String {
    let Some(m) = tak_input::tuning_field(name) else {
        return String::new();
    };
    let v = t.get(name).unwrap_or(0);
    if m.boolean {
        if v != 0 { "on".into() } else { "off".into() }
    } else if m.unit.is_empty() {
        v.to_string()
    } else {
        format!("{v} {}", m.unit)
    }
}

pub(super) fn overlay_draw_system(
    feel: Res<Feel>,
    input: Res<InputRes>,
    mut root: Query<&mut Visibility, With<OverlayRoot>>,
    mut title: Query<
        &mut Text,
        (
            With<OverlayTitle>,
            Without<OverlayLabel>,
            Without<OverlayValue>,
            Without<OverlayHelp>,
        ),
    >,
    mut help: Query<
        &mut Text,
        (
            With<OverlayHelp>,
            Without<OverlayLabel>,
            Without<OverlayValue>,
            Without<OverlayTitle>,
        ),
    >,
    mut labels: Query<
        (&OverlayLabel, &mut Text),
        (
            Without<OverlayValue>,
            Without<OverlayTitle>,
            Without<OverlayHelp>,
        ),
    >,
    mut values: Query<
        (&OverlayValue, &mut Text),
        (
            Without<OverlayLabel>,
            Without<OverlayTitle>,
            Without<OverlayHelp>,
        ),
    >,
    mut rows: Query<(&OverlayRow, &mut BackgroundColor)>,
) {
    let open = feel.overlay_open;
    for mut v in &mut root {
        *v = if open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if !open {
        return;
    }
    let t = input.mapper.tuning();
    let first = first_visible(feel.overlay_row);
    let n = TUNING_FIELDS.len();
    let row_text = |r: usize| -> (String, String) {
        if r == 0 {
            (
                "Preset".into(),
                t.preset_name().unwrap_or("custom").to_string(),
            )
        } else if r <= n {
            let f = &TUNING_FIELDS[r - 1];
            (format!("{}: {}", f.group, f.name), value_text(t, f.name))
        } else if r == n + 1 {
            ("Save profile".into(), "save".into())
        } else {
            (String::new(), String::new())
        }
    };
    if let Ok(mut s) = title.single_mut() {
        let path = feel
            .profile_path
            .clone()
            .unwrap_or_else(feel::default_profile_path);
        s.0 = format!(
            "Tuning: {} ({})   {}/{}",
            input.mapper.profile().name,
            path.display(),
            feel.overlay_row + 1,
            overlay_len()
        );
    }
    for (l, mut s) in &mut labels {
        s.0 = row_text(first + l.0).0;
    }
    for (v, mut s) in &mut values {
        s.0 = row_text(first + v.0).1;
    }
    for (r, mut bg) in &mut rows {
        bg.0 = if first + r.0 == feel.overlay_row {
            Color::srgba(0.85, 0.7, 0.3, 0.35)
        } else {
            Color::NONE
        };
    }
    if let Ok(mut s) = help.single_mut() {
        let r = feel.overlay_row;
        let what = if r == 0 {
            "Switch preset (console, assisted, precise, wp_j)".to_string()
        } else if r <= n {
            TUNING_FIELDS[r - 1].help.to_string()
        } else {
            "Write the profile (bindings + tuning) to the file above".to_string()
        };
        let pad = match feel.layout {
            Layout::Xbox => "D-pad move/adjust, LB/RB x10, A toggle, Y save, B close",
            Layout::PlayStation => {
                "D-pad move/adjust, L1/R1 x10, Cross toggle, Triangle save, Circle close"
            }
        };
        s.0 = format!(
            "{what}\n{pad}\nKeys: arrows (Shift x10), PgUp/PgDn, Enter save, F11 close. Mouse: - / +{}",
            feel.status
                .as_ref()
                .map_or(String::new(), |(m, _)| format!("\n{m}"))
        );
    }
}

// ---- game cursors -----------------------------------------------------------

struct CursorFrame {
    image: Handle<Image>,
    size: Vec2,
    anchor: Vec2,
    dur: u32,
}

/// The game's cursor sequences, by sequence name.
#[derive(Resource, Default)]
pub(super) struct CursorArt(BTreeMap<&'static str, Vec<CursorFrame>>);

impl CursorArt {
    /// The normal cursor loaded (so the sprite replaces the gizmo cursor).
    pub fn loaded(&self) -> bool {
        self.0.contains_key("cursornormal")
    }
}

/// `anims/CURSORS.GAF` sequence for each context (**Chosen** mapping; the
/// names say what each is for).
fn cursor_seq(kind: CursorKind) -> &'static str {
    match kind {
        CursorKind::Normal => "cursornormal",
        CursorKind::Select => "cursorselect",
        CursorKind::Move => "CursorMove",
        CursorKind::Attack => "CursorAttack",
        CursorKind::Guard => "CursorDefend",
        CursorKind::Patrol => "CursorPatrol",
        CursorKind::Build => "cursorgrn",
        CursorKind::BuildBlocked => "cursorred",
        CursorKind::Cast => "Cursorresurrect",
        CursorKind::Invalid => "cursortoofar",
    }
}

const CURSOR_KINDS: [CursorKind; 10] = [
    CursorKind::Normal,
    CursorKind::Select,
    CursorKind::Move,
    CursorKind::Attack,
    CursorKind::Guard,
    CursorKind::Patrol,
    CursorKind::Build,
    CursorKind::BuildBlocked,
    CursorKind::Cast,
    CursorKind::Invalid,
];

#[derive(Component)]
pub(super) struct GameCursor {
    ghost: bool,
}

pub(super) fn setup_cursors(
    mut commands: Commands,
    content: NonSend<Content>,
    mut images: ResMut<Assets<Image>>,
    mut cursor_opts: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let path = "anims/cursors.gaf";
    let mut art = CursorArt::default();
    if let Some(pal) = content.sprites.palette(&content.vfs, "anims/cursors.pcx") {
        for kind in CURSOR_KINDS {
            let name = cursor_seq(kind);
            if art.0.contains_key(name) {
                continue;
            }
            let Some(seq) = content.sprites.seq(&content.vfs, path, name, &pal) else {
                continue;
            };
            let frames = seq
                .frames
                .iter()
                .map(|fr| CursorFrame {
                    image: images.add(rgba_image(fr.w, fr.h, fr.rgba.clone())),
                    size: Vec2::new(fr.w as f32, fr.h as f32),
                    anchor: Vec2::new(fr.x_off as f32, fr.y_off as f32),
                    dur: fr.duration.max(1),
                })
                .collect::<Vec<_>>();
            if !frames.is_empty() {
                art.0.insert(name, frames);
            }
        }
    }
    eprintln!(
        "game cursors: {} of {} sequences",
        art.0.len(),
        CURSOR_KINDS.len()
    );
    if art.0.contains_key("cursornormal") {
        for mut o in &mut cursor_opts {
            o.visible = false;
        }
    }
    commands.insert_resource(art);
    for ghost in [false, true] {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            ImageNode {
                color: if ghost {
                    Color::srgba(1.0, 0.45, 1.0, 0.85)
                } else {
                    Color::WHITE
                },
                ..default()
            },
            GlobalZIndex(1000 + i32::from(ghost)),
            Visibility::Hidden,
            GameCursor { ghost },
        ));
    }
}

/// Screen position of a mapper (input-camera) ground point.
fn input_to_screen(geom: &ViewGeom, ground: &TerrainData, w: tak_math::Vec2) -> [f32; 2] {
    let (x, z) = (f(w.x), f(w.y));
    geom.to_screen([x, ground.height_at(x, z), z])
}

pub(super) fn cursor_system(
    time: Res<Time>,
    sim: NonSend<SimRes>,
    input: Res<InputRes>,
    feel: Res<Feel>,
    cam: Res<CamState>,
    ground: Res<Ground>,
    art: Option<Res<CursorArt>>,
    plan: Option<Res<ShotPlan>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    minimap: Res<MinimapState>,
    mut q: Query<(&GameCursor, &mut Node, &mut ImageNode, &mut Visibility)>,
) {
    let Some(art) = art else { return };
    let view = sim.session.view();
    let frame_tick = (time.elapsed_secs() * 30.0) as u32;
    for (gc, mut node, mut img, mut vis) in &mut q {
        let (pos, kind) = if gc.ghost {
            let Some(g) = feel
                .ghost
                .as_ref()
                .and_then(|r| r.ghost_at(sim.session.tick().0))
            else {
                *vis = Visibility::Hidden;
                continue;
            };
            (
                input_to_screen(&cam.geom, &ground.0, g.cursor_world),
                CursorKind::Normal,
            )
        } else {
            if feel.ghost.is_some() || feel.overlay_open && !input.mapper.stick_cursor() {
                *vis = Visibility::Hidden;
                continue;
            }
            let stick = input.mapper.stick_cursor();
            if plan.is_some() && !feel.shot_pad {
                *vis = Visibility::Hidden;
                continue;
            }
            let pos = if stick || plan.is_some() {
                input_to_screen(&cam.geom, &ground.0, input.mapper.cursor_world())
            } else {
                match windows.single().ok().and_then(|w| w.cursor_position()) {
                    Some(p) => [p.x, p.y],
                    None => {
                        *vis = Visibility::Hidden;
                        continue;
                    }
                }
            };
            let over_ui = minimap
                .geom(cam.geom.size)
                .is_some_and(|m| m.contains(pos[0], pos[1]));
            let kind = if over_ui {
                CursorKind::Normal
            } else {
                input.mapper.cursor_kind(&view)
            };
            (pos, kind)
        };
        let frames = art
            .0
            .get(cursor_seq(kind))
            .or_else(|| art.0.get("cursornormal"));
        let Some(frames) = frames else {
            *vis = Visibility::Hidden;
            continue;
        };
        let total: u32 = frames.iter().map(|f| f.dur).sum();
        let mut t = frame_tick % total.max(1);
        let fr = frames
            .iter()
            .find(|f| {
                if t < f.dur {
                    true
                } else {
                    t -= f.dur;
                    false
                }
            })
            .unwrap_or(&frames[0]);
        img.image = fr.image.clone();
        node.left = Val::Px(pos[0] - fr.anchor.x);
        node.top = Val::Px(pos[1] - fr.anchor.y);
        node.width = Val::Px(fr.size.x);
        node.height = Val::Px(fr.size.y);
        *vis = Visibility::Visible;
    }
}

// ---- glyphs and hints ---------------------------------------------------------

/// Generated chip images per glyph (no art assets: drawn here).
#[derive(Resource, Default)]
pub(super) struct GlyphArt(BTreeMap<(bool, &'static str), Handle<Image>>);

fn chip_image(g: &Glyph) -> Image {
    let (w, h) = if g.round {
        (26u32, 26u32)
    } else {
        (16 + 9 * g.text.len() as u32, 22)
    };
    let mut px = vec![0u8; (w * h * 4) as usize];
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let r = h as f32 / 2.0 - 0.5;
    let sym_color = |s: Symbol| match s {
        Symbol::Cross => [124, 178, 232],
        Symbol::Circle => [255, 102, 102],
        Symbol::Square => [255, 105, 248],
        Symbol::Triangle => [64, 226, 160],
    };
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded shape: distance to the centre segment.
            let dx = if g.round {
                fx - cx
            } else {
                (fx - cx).abs() - (cx - r).max(0.0)
            };
            let dx = if g.round { dx } else { dx.max(0.0) };
            let d = (dx * dx + (fy - cy) * (fy - cy)).sqrt();
            if d > r {
                continue;
            }
            let edge = (r - d).min(1.0);
            let mut c = g.color;
            if d > r - 1.5 {
                c = [
                    c[0].saturating_add(60),
                    c[1].saturating_add(60),
                    c[2].saturating_add(60),
                ];
            }
            if let Some(s) = g.symbol {
                let (ux, uy) = ((fx - cx) / r, (fy - cy) / r);
                let t = 0.11;
                let on = match s {
                    Symbol::Cross => {
                        ((ux - uy).abs() < t * 1.4 || (ux + uy).abs() < t * 1.4)
                            && ux.abs() < 0.5
                            && uy.abs() < 0.5
                    }
                    Symbol::Circle => ((ux * ux + uy * uy).sqrt() - 0.45).abs() < t,
                    Symbol::Square => {
                        let m = ux.abs().max(uy.abs());
                        (m - 0.42).abs() < t
                    }
                    Symbol::Triangle => {
                        // Outline of an upward triangle.
                        let top = -0.5;
                        let bottom = 0.38;
                        let half = |yy: f32| (yy - top) / (bottom - top) * 0.55;
                        let inside = uy >= top && uy <= bottom && ux.abs() <= half(uy);
                        let inner = uy >= top + 0.24
                            && uy <= bottom - t * 1.2
                            && ux.abs() <= half(uy) - 0.2;
                        inside && !inner
                    }
                };
                if on {
                    c = sym_color(s);
                }
            }
            let i = ((y * w + x) * 4) as usize;
            px[i..i + 3].copy_from_slice(&c);
            px[i + 3] = (edge * 255.0) as u8;
        }
    }
    rgba_image(w, h, px)
}

fn chip_key(g: &Glyph) -> (bool, &'static str) {
    (
        g.symbol.is_some(),
        if g.symbol.is_some() { g.name } else { g.text },
    )
}

fn chip_for(art: &mut GlyphArt, images: &mut Assets<Image>, g: &Glyph) -> Handle<Image> {
    art.0
        .entry(chip_key(g))
        .or_insert_with(|| images.add(chip_image(g)))
        .clone()
}

const HINTS: usize = 7;

#[derive(Component)]
pub(super) struct HintBar;
#[derive(Component)]
pub(super) struct HintItem(usize);
/// Chip `k` (0, 1: chord) of hint `i`.
#[derive(Component)]
pub(super) struct HintChip(usize, usize);
#[derive(Component)]
pub(super) struct HintChipText(usize, usize);
#[derive(Component)]
pub(super) struct HintText(usize);

pub(super) fn setup_hints(mut commands: Commands) {
    commands.init_resource::<GlyphArt>();
    // The bottom bar: a column (bottom up) of the button prompts, then the
    // verb line and the message line `hud_view` adds.
    let column = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(hud_view::BOTTOM_BAR_X),
                bottom: Val::Px(4.0),
                flex_direction: FlexDirection::ColumnReverse,
                align_items: AlignItems::FlexStart,
                row_gap: Val::Px(3.0),
                max_width: Val::Px(470.0),
                ..default()
            },
            hud_view::BottomBar,
        ))
        .id();
    let bar = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(10.0),
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            HintBar,
            ChildOf(column),
        ))
        .id();
    for i in 0..HINTS {
        let item = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(3.0),
                    ..default()
                },
                Visibility::Hidden,
                HintItem(i),
                ChildOf(bar),
            ))
            .id();
        for k in 0..2 {
            let chip = commands
                .spawn((
                    Node {
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    ImageNode::default(),
                    Visibility::Hidden,
                    HintChip(i, k),
                    ChildOf(item),
                ))
                .id();
            commands.spawn((
                Text::new(""),
                TextFont {
                    font_size: bevy::text::FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                HintChipText(i, k),
                ChildOf(chip),
            ));
        }
        commands.spawn((Text::new(""), hud_text(13.0), HintText(i), ChildOf(item)));
    }
}

pub(super) fn hints_system(
    sim: NonSend<SimRes>,
    input: Res<InputRes>,
    mode: Res<hud_view::HudMode>,
    mut feel: ResMut<Feel>,
    pads: Query<(&Gamepad, Option<&Name>)>,
    mut art: ResMut<GlyphArt>,
    mut images: ResMut<Assets<Image>>,
    mut bar: Query<&mut Visibility, (With<HintBar>, Without<HintItem>, Without<HintChip>)>,
    mut items: Query<(&HintItem, &mut Visibility), (Without<HintBar>, Without<HintChip>)>,
    mut chips: Query<
        (&HintChip, &mut ImageNode, &mut Node, &mut Visibility),
        (Without<HintBar>, Without<HintItem>),
    >,
    mut chip_texts: Query<(&HintChipText, &mut Text), Without<HintText>>,
    mut texts: Query<(&HintText, &mut Text), Without<HintChipText>>,
) {
    // Glyph layout from the connected pad (gilrs name / vendor id).
    if let Some((pad, name)) = pads.iter().next() {
        let l = feel::detect_layout(
            &feel.glyph_choice,
            name.map(|n| n.as_str()),
            pad.vendor_id(),
        );
        if l != feel.layout {
            feel.layout = l;
        }
    }
    let w = sim.session.sim.world();
    let me = sim.session.local;
    let sel: Vec<_> = input
        .mapper
        .selection()
        .iter()
        .filter_map(|id| w.unit(*id).filter(|u| u.owner == me))
        .collect();
    let cx = feel::HintCx {
        has_units: !sel.is_empty(),
        has_builder: sel.iter().any(|u| {
            w.defs()
                .get(u.def)
                .is_some_and(|d| d.build.builder && !d.build.canbuild.is_empty())
        }),
        has_caster: sel.iter().any(|u| {
            w.defs()
                .get(u.def)
                .is_some_and(|d| d.combat.weapons.iter().any(|x| x.is_spell()))
        }),
        mixed: input.mapper.sub_select().is_some()
            || sel
                .iter()
                .any(|u| sel.first().is_some_and(|f| f.def != u.def)),
    };
    let hints = if feel.ghost.is_some() || feel.overlay_open {
        Vec::new()
    } else if mode.console {
        feel::hints(&input.mapper, cx)
    } else {
        feel::hints_classic(&input.mapper, cx)
    };
    for mut v in &mut bar {
        *v = if hints.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
    let layout = feel.layout;
    let glyphs: Vec<Vec<Glyph>> = hints
        .iter()
        .map(|h| match &h.key {
            HintKey::Buttons(bs) => bs.iter().filter_map(|b| feel::glyph(layout, *b)).collect(),
            HintKey::Stick(left) => vec![feel::stick_glyph(*left)],
            HintKey::Text(_) => Vec::new(),
        })
        .collect();
    for (it, mut v) in &mut items {
        *v = if it.0 < hints.len() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (c, mut img, mut node, mut v) in &mut chips {
        match glyphs.get(c.0).and_then(|g| g.get(c.1)) {
            Some(g) => {
                img.image = chip_for(&mut art, &mut images, g);
                let im = chip_image_size(g);
                node.width = Val::Px(im.0);
                node.height = Val::Px(im.1);
                *v = Visibility::Inherited;
            }
            None => *v = Visibility::Hidden,
        }
    }
    for (c, mut t) in &mut chip_texts {
        t.0 = glyphs
            .get(c.0)
            .and_then(|g| g.get(c.1))
            .map_or("", |g| g.text)
            .to_string();
    }
    for (h, mut t) in &mut texts {
        t.0 = hints.get(h.0).map_or(String::new(), |h| {
            let key = match &h.key {
                HintKey::Text(k) => format!("[{k}] "),
                _ => String::new(),
            };
            if h.prefix.is_empty() {
                format!("{key}{}", h.text)
            } else {
                format!("{key}{} {}", h.prefix, h.text)
            }
        });
    }
}

fn chip_image_size(g: &Glyph) -> (f32, f32) {
    if g.round {
        (26.0, 26.0)
    } else {
        ((16 + 9 * g.text.len()) as f32, 22.0)
    }
}

// ---- radial wheel -------------------------------------------------------------

/// Slot entities (rings and pages hold at most 8; older profiles up to 16).
const WHEEL_SLOTS: usize = 24;

/// Slot box padding and border (logical px).
const WHEEL_PAD: [f32; 2] = [8.0, 4.0];
const WHEEL_BORDER: f32 = 2.0;
/// Gap kept between neighbouring slots, and between any slot and the
/// cursor at the centre (game cursors are about 32 px).
const WHEEL_GAP: f32 = 6.0;
const WHEEL_CLEAR: f32 = 26.0;
/// Smallest wheel radius.
const WHEEL_MIN_R: f32 = 64.0;
/// Build portraits on wheel slots (`anims/buildpic` is 64x48).
const SLOT_PIC: [f32; 2] = [48.0, 36.0];

#[derive(Component)]
pub(super) struct WheelRoot;
#[derive(Component)]
pub(super) struct WheelSlot(usize);
#[derive(Component)]
pub(super) struct WheelSlotText(usize);
#[derive(Component)]
pub(super) struct WheelSlotPic(usize);
/// The wheel's title, above its top slot (never over the cursor).
#[derive(Component)]
pub(super) struct WheelCentre;
/// The centre panel: what the highlighted slot is (portrait, name, cost,
/// build time, mana shortfall).
#[derive(Component)]
pub(super) struct WheelInfo;
#[derive(Component)]
pub(super) struct WheelInfoPic;
#[derive(Component)]
pub(super) struct WheelInfoText;
#[derive(Component)]
pub(super) struct WheelInfoShort;

/// Build portraits by unit name (`anims/buildpic/<name>.jpg`), decoded on
/// first use; `None` when the install has none.
#[derive(Resource, Default)]
pub(super) struct Portraits(BTreeMap<String, Option<Handle<Image>>>);

impl Portraits {
    /// The portrait for a unit name, loading it on first use.
    pub fn get(
        &mut self,
        content: &Content,
        images: &mut Assets<Image>,
        unitname: &str,
    ) -> Option<Handle<Image>> {
        let key = unitname.to_ascii_lowercase();
        self.0
            .entry(key.clone())
            .or_insert_with(|| {
                let bytes = content
                    .vfs
                    .read(&format!("anims/buildpic/{key}.jpg"))
                    .ok()?;
                let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
                let (w, h) = img.dimensions();
                Some(images.add(rgba_image(w, h, img.into_raw())))
            })
            .clone()
    }
}

pub(super) fn setup_wheel(mut commands: Commands) {
    commands.init_resource::<Portraits>();
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(0.0),
                height: Val::Px(0.0),
                ..default()
            },
            GlobalZIndex(800),
            Visibility::Hidden,
            WheelRoot,
        ))
        .id();
    commands.spawn((
        Text::new(""),
        hud_text(14.0),
        TextLayout::no_wrap(),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
        BorderColor::all(Color::srgb(0.75, 0.62, 0.3)),
        WheelCentre,
        ChildOf(root),
    ));
    let info = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                width: Val::Px(150.0),
                padding: UiRect::all(Val::Px(5.0)),
                border: UiRect::all(Val::Px(1.0)),
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.03, 0.03, 0.02, 0.88)),
            BorderColor::all(Color::srgb(0.55, 0.48, 0.3)),
            Visibility::Hidden,
            WheelInfo,
            ChildOf(root),
        ))
        .id();
    commands.spawn((
        Node {
            width: Val::Px(64.0),
            height: Val::Px(48.0),
            ..default()
        },
        ImageNode::default(),
        WheelInfoPic,
        ChildOf(info),
    ));
    commands.spawn((
        Text::new(""),
        hud_text(13.0),
        TextLayout::justify(Justify::Center),
        WheelInfoText,
        ChildOf(info),
    ));
    commands.spawn((
        Text::new(""),
        hud_text(13.0).0,
        TextColor(Color::srgb(1.0, 0.35, 0.3)),
        WheelInfoShort,
        ChildOf(info),
    ));
    for i in 0..WHEEL_SLOTS {
        let s = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(WHEEL_PAD[0]), Val::Px(WHEEL_PAD[1])),
                    border: UiRect::all(Val::Px(WHEEL_BORDER)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.08, 0.07, 0.05, 0.85)),
                BorderColor::all(Color::srgb(0.45, 0.4, 0.28)),
                Visibility::Hidden,
                WheelSlot(i),
                ChildOf(root),
            ))
            .id();
        commands.spawn((
            Node {
                width: Val::Px(SLOT_PIC[0]),
                height: Val::Px(SLOT_PIC[1]),
                display: Display::None,
                ..default()
            },
            ImageNode::default(),
            WheelSlotPic(i),
            ChildOf(s),
        ));
        commands.spawn((
            Text::new(""),
            hud_text(14.0),
            TextLayout::no_wrap(),
            WheelSlotText(i),
            ChildOf(s),
        ));
    }
}

/// The first own builder of the selection (the one the build wheel and
/// `BuildOption` use) and its definition.
fn first_builder<'w>(
    w: &'w tak_sim::World,
    local: PlayerId,
    sel: &tak_commands::UnitSet,
) -> Option<&'w tak_sim::defs::UnitDef> {
    sel.iter().filter_map(|id| w.unit(*id)).find_map(|u| {
        w.defs()
            .get(u.def)
            .filter(|d| u.owner == local && d.build.builder && !d.build.canbuild.is_empty())
    })
}

/// What a wheel slot shows.
struct SlotView {
    /// Short label on the slot.
    label: String,
    /// Usable now (spells need the caster's mana; builds are allowed short
    /// of mana, TA:K builds drain mana as they go).
    usable: bool,
    /// Shown as greyed (short of mana).
    short: bool,
    /// A unit portrait (build options).
    unit: Option<tak_commands::UnitDefId>,
    /// Centre panel text for the highlighted slot.
    info: String,
    /// Centre panel shortfall line (red).
    shortfall: String,
}

/// A wheel slot's label, state and centre-panel text.
fn wheel_slot_view(
    a: &Action,
    s: &Session,
    w: &tak_sim::World,
    sel: &tak_commands::UnitSet,
) -> SlotView {
    let mut v = SlotView {
        label: feel::action_label(a),
        usable: true,
        short: false,
        unit: None,
        info: String::new(),
        shortfall: String::new(),
    };
    let mana = tak_sim::economy::player(w, s.local).map_or(0.0, |e| super::mana(e.mana));
    match a {
        Action::BuildOption { index } => {
            let builder = first_builder(w, s.local, sel);
            let def = builder.and_then(|d| d.build.canbuild.get(usize::from(*index)).copied());
            if let (Some(b), Some(o)) = (builder, def) {
                let d = w.defs().get(o);
                let cost = d.map_or(0, |d| d.build.buildcost);
                let secs = d.map_or(0.0, |d| {
                    f64::from(f(tak_math::Fx::from_raw(
                        d.build.buildtime.clamp(0, i64::from(i32::MAX)) as i32,
                    )))
                }) / f64::from(b.build.workertime.max(1));
                let name = s.def(o).map_or("?", |d| d.display.as_str()).to_string();
                v.label = name.clone();
                v.unit = Some(o);
                v.short = (cost as f64) > mana;
                v.info = format!("{name}\n{cost} mana   {secs:.0} s");
                if v.short {
                    v.shortfall = format!("short {:.0} mana", cost as f64 - mana);
                }
            }
        }
        Action::CastSpell { index } => {
            if let Some(sp) = crate::view::caster_spells(w, s.local, sel)
                .and_then(|(_, sp)| sp.get(usize::from(*index)).cloned())
            {
                v.label = format!("{} ({})", sp.name, sp.cost);
                v.usable = sp.ready;
                v.short = !sp.ready;
                v.info = format!("{}\n{} mana", sp.name, sp.cost);
                if !sp.ready {
                    let own = crate::view::caster_spells(w, s.local, sel)
                        .and_then(|(id, _)| tak_sim::economy::unit_mana(w, id))
                        .map_or(0.0, |m| f(m.mana));
                    v.shortfall = format!("short {:.0} mana", sp.cost as f32 - own);
                }
            } else {
                v.usable = false;
            }
        }
        Action::CastSpellMode => {
            v.usable = crate::view::caster_spells(w, s.local, sel)
                .is_some_and(|(_, sp)| sp.first().is_some_and(|x| x.ready));
        }
        Action::OpenRing { .. } => v.info = format!("{}\nconfirm to open", v.label),
        Action::AttackMode => v.info = "Attack\nthen X on a target".into(),
        Action::MoveMode => v.info = "Move\nthen X on the ground".into(),
        Action::GuardMode => v.info = "Guard\nthen X on a friend".into(),
        Action::PatrolMode => v.info = "Patrol\nthen X on the ground".into(),
        other => v.info = feel::action_label(other),
    }
    v
}

#[allow(clippy::too_many_arguments)]
pub(super) fn wheel_system(
    sim: NonSend<SimRes>,
    content: NonSend<Content>,
    input: Res<InputRes>,
    feel: Res<Feel>,
    cam: Res<CamState>,
    ground: Res<Ground>,
    mut portraits: ResMut<Portraits>,
    mut images: ResMut<Assets<Image>>,
    mut root: Query<
        (&mut Node, &mut Visibility),
        (
            With<WheelRoot>,
            Without<WheelSlot>,
            Without<WheelCentre>,
            Without<WheelInfo>,
            Without<WheelSlotPic>,
            Without<WheelInfoPic>,
        ),
    >,
    mut centre: Query<
        (&mut Text, &mut Node, &ComputedNode),
        (
            With<WheelCentre>,
            Without<WheelSlotText>,
            Without<WheelRoot>,
            Without<WheelSlot>,
            Without<WheelInfo>,
            Without<WheelInfoText>,
            Without<WheelInfoShort>,
            Without<WheelSlotPic>,
        ),
    >,
    mut slots: Query<
        (
            &WheelSlot,
            &mut Node,
            &mut Visibility,
            &mut BackgroundColor,
            &mut BorderColor,
            &ComputedNode,
        ),
        (
            Without<WheelRoot>,
            Without<WheelCentre>,
            Without<WheelInfo>,
            Without<WheelSlotPic>,
            Without<WheelInfoPic>,
        ),
    >,
    mut pics: Query<
        (&WheelSlotPic, &mut Node, &mut ImageNode),
        (
            Without<WheelSlot>,
            Without<WheelRoot>,
            Without<WheelCentre>,
            Without<WheelInfo>,
            Without<WheelInfoPic>,
        ),
    >,
    mut texts: Query<
        (&WheelSlotText, &mut Text, &mut TextColor),
        (
            Without<WheelCentre>,
            Without<WheelInfoText>,
            Without<WheelInfoShort>,
        ),
    >,
    mut info: Query<
        (&mut Node, &mut Visibility, &ComputedNode),
        (
            With<WheelInfo>,
            Without<WheelSlot>,
            Without<WheelRoot>,
            Without<WheelCentre>,
            Without<WheelSlotPic>,
            Without<WheelInfoPic>,
        ),
    >,
    mut info_pic: Query<
        (&mut Node, &mut ImageNode),
        (
            With<WheelInfoPic>,
            Without<WheelSlot>,
            Without<WheelRoot>,
            Without<WheelCentre>,
            Without<WheelInfo>,
            Without<WheelSlotPic>,
        ),
    >,
    mut info_texts: Query<
        (&mut Text, Has<WheelInfoShort>),
        (
            Or<(With<WheelInfoText>, With<WheelInfoShort>)>,
            Without<WheelSlotText>,
            Without<WheelCentre>,
        ),
    >,
) {
    let s = &sim.session;
    let w = s.sim.world();
    let ghost = feel.ghost.as_ref().and_then(|r| r.ghost_at(s.tick().0));
    let (wheel, at) = match (&ghost, input.mapper.wheel()) {
        (Some(g), _) => (g.wheel.clone(), g.cursor_world),
        (None, Some(wh)) => (Some(wh.clone()), input.mapper.cursor_world()),
        (None, None) => (None, input.mapper.cursor_world()),
    };
    let Ok((mut rn, mut rv)) = root.single_mut() else {
        return;
    };
    let Some(wh) = wheel else {
        *rv = Visibility::Hidden;
        return;
    };
    *rv = Visibility::Visible;
    let p = input_to_screen(&cam.geom, &ground.0, at);
    rn.left = Val::Px(p[0]);
    rn.top = Val::Px(p[1]);
    let n = wh.slots.len().min(WHEEL_SLOTS);
    let sel = input.mapper.selection();
    let views: Vec<SlotView> = wh.slots[..n]
        .iter()
        .map(|a| wheel_slot_view(a, s, w, sel))
        .collect();
    // The centre panel (highlighted slot) and its size, so the ring clears
    // it as it clears the cursor.
    let hi_view = wh.highlighted.filter(|&h| h < n).map(|h| &views[h]);
    let info_size = info
        .single()
        .map(|(_, _, cn)| [cn.size.x, cn.size.y])
        .unwrap_or([0.0, 0.0]);
    let show_info = hi_view.is_some_and(|v| !v.info.is_empty());
    let clear = if show_info {
        WHEEL_CLEAR.max(info_size[0].max(info_size[1]) / 2.0 + 4.0)
    } else {
        WHEEL_CLEAR
    };
    let dirs: Vec<[f32; 2]> = (0..n)
        .map(|k| {
            let (sn, cs) = tak_input::slot_direction(k, n).sin_cos();
            [f(cs), -f(sn)]
        })
        .collect();
    let mut sizes = vec![[0.0f32, 0.0f32]; n];
    for (sl, _, _, _, _, cn) in &slots {
        if sl.0 < n {
            let chars = views[sl.0].label.chars().count() as f32;
            let pic = if views[sl.0].unit.is_some() {
                SLOT_PIC[1] + 2.0
            } else {
                0.0
            };
            sizes[sl.0] = if cn.size.x > 0.0 {
                [cn.size.x, cn.size.y]
            } else {
                [
                    chars * 8.4 + 2.0 * (WHEEL_PAD[0] + WHEEL_BORDER),
                    18.0 + pic + 2.0 * (WHEEL_PAD[1] + WHEEL_BORDER),
                ]
            };
        }
    }
    let radius = crate::view::wheel_radius(&dirs, &sizes, WHEEL_MIN_R, WHEEL_GAP, clear);
    let mut top = 0.0f32;
    for (sl, mut node, mut v, mut bg, mut border, cn) in &mut slots {
        let blank = sl.0 >= n || tak_input::is_blank(&wh.slots[sl.0]);
        if blank {
            *v = Visibility::Hidden;
            continue;
        }
        *v = Visibility::Inherited;
        let reserved = !wh.slots[sl.0].is_supported();
        let [dx, dy] = [dirs[sl.0][0] * radius, dirs[sl.0][1] * radius];
        let [bw, bh] = if cn.size.x > 0.0 {
            [cn.size.x, cn.size.y]
        } else {
            sizes[sl.0]
        };
        node.left = Val::Px(dx - bw / 2.0);
        node.top = Val::Px(dy - bh / 2.0);
        top = top.min(dy - bh / 2.0);
        let hi = wh.highlighted == Some(sl.0);
        let view = &views[sl.0];
        // A reserved slot keeps its place as an empty, faint frame.
        bg.0 = match (hi, view.usable && !view.short, reserved) {
            (_, _, true) => Color::srgba(0.05, 0.05, 0.05, 0.35),
            (true, true, _) => Color::srgba(0.95, 0.72, 0.2, 0.97),
            (true, false, _) => Color::srgba(0.32, 0.3, 0.28, 0.97),
            (false, _, _) => Color::srgba(0.06, 0.05, 0.04, 0.82),
        };
        *border = BorderColor::all(if hi {
            Color::srgb(1.0, 1.0, 0.9)
        } else if reserved {
            Color::srgba(0.45, 0.4, 0.28, 0.4)
        } else {
            Color::srgb(0.45, 0.4, 0.28)
        });
        let bw = Val::Px(if hi { WHEEL_BORDER + 1.0 } else { WHEEL_BORDER });
        let want = UiRect::all(bw);
        if node.border != want {
            node.border = want;
            let pad = if hi { -1.0 } else { 0.0 };
            node.padding = UiRect::axes(Val::Px(WHEEL_PAD[0] + pad), Val::Px(WHEEL_PAD[1] + pad));
        }
    }
    for (sp, mut node, mut img) in &mut pics {
        let unit = (sp.0 < n).then(|| views[sp.0].unit).flatten();
        let handle = unit.and_then(|u| {
            s.def(u)
                .and_then(|d| portraits.get(&content, &mut images, &d.unitname))
        });
        match handle {
            Some(h) => {
                img.image = h;
                img.color = if views[sp.0].short {
                    Color::srgb(0.45, 0.45, 0.45)
                } else {
                    Color::WHITE
                };
                if node.display != Display::Flex {
                    node.display = Display::Flex;
                }
            }
            None => {
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
    }
    for (st, mut t, mut col) in &mut texts {
        let (label, usable, hi) = match views.get(st.0).filter(|_| st.0 < n) {
            Some(_) if !wh.slots[st.0].is_supported() => (String::new(), false, false),
            Some(v) => (
                v.label.clone(),
                v.usable && !v.short,
                wh.highlighted == Some(st.0),
            ),
            None => (String::new(), true, false),
        };
        if t.0 != label {
            t.0 = label;
        }
        col.0 = match (hi, usable) {
            (true, true) => Color::srgb(0.08, 0.05, 0.0),
            (true, false) => Color::srgb(0.85, 0.82, 0.78),
            (false, false) => Color::srgb(0.55, 0.53, 0.5),
            (false, true) => Color::srgb(0.95, 0.92, 0.85),
        };
    }
    // Centre panel.
    if let Ok((mut node, mut v, cn)) = info.single_mut() {
        *v = if show_info {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        node.left = Val::Px(-cn.size.x / 2.0);
        node.top = Val::Px(-cn.size.y / 2.0);
    }
    if let Some(view) = hi_view {
        for (mut t, short) in &mut info_texts {
            let want = if short { &view.shortfall } else { &view.info };
            if t.0 != *want {
                t.0 = want.clone();
            }
        }
        if let Ok((mut node, mut img)) = info_pic.single_mut() {
            let h = view.unit.and_then(|u| {
                s.def(u)
                    .and_then(|d| portraits.get(&content, &mut images, &d.unitname))
            });
            match h {
                Some(h) => {
                    img.image = h;
                    img.color = if view.short {
                        Color::srgb(0.5, 0.5, 0.5)
                    } else {
                        Color::WHITE
                    };
                    node.display = Display::Flex;
                }
                None => node.display = Display::None,
            }
        }
    }
    // Title above the top slot: the ring path and the page.
    if let Ok((mut t, mut node, cn)) = centre.single_mut() {
        let mut title = if input.mapper.wheel_depth() > 0 && ghost.is_none() {
            format!("Orders > {}", wh.kind.title())
        } else {
            wh.kind.title().to_string()
        };
        if wh.pages > 1 {
            title.push_str(&format!("   page {}/{}  (RB: next)", wh.page + 1, wh.pages));
        }
        if ghost.is_some() {
            title.push_str(" (ghost)");
        }
        if t.0 != title {
            t.0 = title;
        }
        node.left = Val::Px(-cn.size.x / 2.0);
        node.top = Val::Px(top - cn.size.y - 4.0);
    }
}

// ---- order sounds -------------------------------------------------------------

/// The order-acknowledge sounds the in-game GUI names (`Sounds/MOVE.WAV`,
/// `ATTACK`, `GUARD`, `PATROL`, `STOP`), decoded by Bevy's WAV support.
#[derive(Resource, Default)]
pub(super) struct OrderSounds(BTreeMap<&'static str, Handle<AudioSource>>);

/// The acknowledgement for an order (**Chosen** for orders without their
/// own sound: build walks like a move, a spell attacks, a stance has none).
pub(super) fn order_sound(c: &tak_commands::SimCommand) -> Option<&'static str> {
    use tak_commands::SimCommand as C;
    Some(match c {
        C::Move { .. } | C::Build { .. } => "MOVE",
        C::Attack { .. } | C::CastSpell { .. } => "ATTACK",
        C::Guard { .. } => "GUARD",
        C::Patrol { .. } => "PATROL",
        C::Stop { .. } => "STOP",
        C::SetFireState { .. } => return None,
    })
}

pub(super) fn setup_sounds(
    mut commands: Commands,
    content: NonSend<Content>,
    mut sources: ResMut<Assets<AudioSource>>,
) {
    let mut s = OrderSounds::default();
    for name in ["MOVE", "ATTACK", "GUARD", "PATROL", "STOP"] {
        if let Ok(bytes) = content.vfs.read(&format!("Sounds/{name}.WAV")) {
            s.0.insert(
                name,
                sources.add(AudioSource {
                    bytes: bytes.into(),
                }),
            );
        }
    }
    eprintln!("order sounds: {} of 5", s.0.len());
    commands.insert_resource(s);
}

/// Seconds a sound entity lives (without an audio device nothing plays and
/// Bevy never despawns it, so it is cleaned up here).
#[derive(Component)]
pub(super) struct SoundTtl(f32);

pub(super) fn sound_system(
    mut commands: Commands,
    time: Res<Time>,
    plan: Option<Res<ShotPlan>>,
    sounds: Option<Res<OrderSounds>>,
    mut feel: ResMut<Feel>,
    mut live: Query<(Entity, &mut SoundTtl)>,
) {
    for (e, mut ttl) in &mut live {
        ttl.0 -= time.delta_secs();
        if ttl.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
    let pending = std::mem::take(&mut feel.sounds);
    let (Some(sounds), None) = (sounds, plan) else {
        return;
    };
    // One acknowledgement per frame is enough (a burst of orders would
    // stack the same voice).
    if let Some(h) = pending.last().and_then(|n| sounds.0.get(n)) {
        commands.spawn((
            AudioPlayer::new(h.clone()),
            PlaybackSettings::DESPAWN,
            SoundTtl(3.0),
        ));
    }
}

// ---- rumble -------------------------------------------------------------------

pub(super) fn rumble_system(
    sim: NonSend<SimRes>,
    input: Res<InputRes>,
    mut feel: ResMut<Feel>,
    pads: Query<Entity, With<Gamepad>>,
    mut rumble: MessageWriter<GamepadRumbleRequest>,
) {
    let fb = std::mem::take(&mut feel.feedback);
    let w = sim.session.sim.world();
    let me = sim.session.local;
    let health: i64 = w
        .units()
        .filter(|(_, u)| u.owner == me)
        .map(|(_, u)| i64::from(u.combat.health))
        .sum();
    let hurt = feel.last_own_health.is_some_and(|h| health < h);
    feel.last_own_health = Some(health);
    let t = input.mapper.tuning();
    if !t.rumble || feel.ghost.is_some() {
        return;
    }
    let Some(pad) = pads.iter().next() else {
        return;
    };
    let mut pct = 0;
    for f in &fb {
        pct = pct.max(match f {
            Feedback::Snap => t.rumble_snap_pct,
            Feedback::Selection { .. } => t.rumble_select_pct,
            Feedback::Order => 0,
            Feedback::Refused { .. } => t.rumble_snap_pct,
        });
    }
    if hurt {
        pct = pct.max(t.rumble_damage_pct);
    }
    if pct > 0 {
        rumble.write(GamepadRumbleRequest::Add {
            gamepad: pad,
            intensity: GamepadRumbleIntensity::weak_motor(pct as f32 / 100.0),
            duration: Duration::from_millis(u64::from(t.rumble_ms)),
        });
    }
}

// ---- recording ----------------------------------------------------------------

/// Write the session recording (called on exit).
pub(super) fn write_recording(feel: &mut Feel, s: &Session) {
    let (Some(path), Some(rec)) = (feel.record_path.clone(), feel.recorder.as_mut()) else {
        return;
    };
    rec.set_meta(feel::session_meta(s, &feel.players, feel.seed));
    let r = rec.recording();
    match std::fs::write(&path, r.to_json()) {
        Ok(()) => eprintln!(
            "wrote session recording {} ({} frames, {:?})",
            path.display(),
            r.frames.len(),
            r.summary()
        ),
        Err(e) => eprintln!("{}: {e}", path.display()),
    }
}
