//! The in-game HUD (WP-V, design system section 6), Bevy side: the race's
//! own button panel with TA:K's order, stance and weapon buttons
//! (`anims/<race>ig.gaf`, `anims/igcommonbuttons.gaf`), the build menu with
//! portraits (`anims/buildpic`), the unit card on the race's info panel, the
//! multi-selection strip, and the contextual bottom bar. On controller
//! play the side panel collapses: orders and building live in the wheels
//! and the bottom bar shows button prompts. Logic without Bevy is in
//! `crate::hud`.

use tak_input::{CursorKind, Feedback, Refusal};

use super::*;
use crate::hud::{
    ARAMON_SLOTS, BUTTON_STATES, DamageBars, PANEL_ACTIONS, PANEL_BUTTONS, PANEL_NAMES,
    SKIN_BUTTON_PANEL, SKIN_END_CAP, SKIN_INFO_PANEL, button_image, button_slots, button_state,
    group_by_type, health_rgb, race_skin, stance_name, verb_line,
};

/// Build menu cells (retail menus have at most 9 entries).
pub(super) const BUILD_CELLS: usize = 12;
/// Multi-selection strip cells.
const STRIP_CELLS: usize = 10;
/// The side panel sits at the right edge, bottom-aligned, as TA:K's
/// `UnitMenu` gadget does (`guis/<race>ingame.gui`: 512,128 128x352 on the
/// 640x480 desktop, its bottom on the screen's bottom edge).
pub(super) const SIDE_PANEL_H: f32 = 352.0;
/// Button panel size (every race's `ButtonPanel` is 128x352).
const PANEL_SIZE: [f32; 2] = [128.0, 352.0];
/// Unit card (the race's `InfoPanel`, 512x49).
const CARD_SIZE: [f32; 2] = [512.0, 49.0];
/// The bottom bar's left edge (right of the unit card).
pub(super) const BOTTOM_BAR_X: f32 = CARD_SIZE[0] + 8.0;

/// The race skin and button art as decoded at start.
#[derive(Resource, Default)]
pub(super) struct HudArt {
    panel: Option<Handle<Image>>,
    info: Option<Handle<Image>>,
    cap: Option<(Handle<Image>, [f32; 2])>,
    /// Frames of each `PANEL_BUTTONS` entry (0 dark, 1 up, 2 lit, 3 down).
    buttons: Vec<Vec<Handle<Image>>>,
    /// Weapon-slot pictures keyed exactly as `Session::weapon_art`, not
    /// by the index in the filtered spell menu.
    weapon_buttons: BTreeMap<(u32, u8), [Option<Handle<Image>>; BUTTON_STATES]>,
    /// Button wells on the panel, `[x, y, w, h]`.
    slots: Vec<[u32; 4]>,
}

impl HudArt {
    /// The same picture for the classic panel and controller spell wheel.
    pub(super) fn weapon_button(
        &self,
        def: tak_commands::UnitDefId,
        spell: tak_commands::SpellId,
        state: usize,
    ) -> Option<&Handle<Image>> {
        let slot = u8::try_from(spell.0).ok()?;
        button_image(self.weapon_buttons.get(&(def.0, slot))?, state)
    }
}

/// Which HUD layout is showing.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct HudMode {
    /// Controller layout: side panel collapsed, prompts in the bottom bar.
    pub console: bool,
}

/// The panel button under the mouse: its name and key for the help line.
#[derive(Resource, Default)]
pub(super) struct HudHover(pub Option<String>);

/// The keyboard key (with modifier) a profile binds to `action`, e.g. `S`
/// or `Ctrl+F1`.
fn key_label(p: &Profile, action: Action) -> Option<String> {
    p.bindings.iter().find_map(|b| {
        let tak_input::Input::Key(k) = &b.input else {
            return None;
        };
        if b.action != action {
            return None;
        }
        let k = k.0.to_ascii_uppercase();
        Some(match &b.with {
            Some(tak_input::Input::Key(m)) => {
                let mut m = m.0.clone();
                if let Some(c) = m.get_mut(..1) {
                    c.make_ascii_uppercase();
                }
                format!("{m}+{k}")
            }
            _ => k,
        })
    })
}

/// A message for the bottom bar (refusals, alerts) and when it expires.
#[derive(Resource, Default)]
pub(super) struct BottomMsg {
    pub text: String,
    pub until: f32,
}

#[derive(Component)]
pub(super) struct SidePanel;
#[derive(Component)]
pub(super) struct PanelBtnLabel(usize);
#[derive(Component)]
pub(super) struct BuildGrid;
#[derive(Component)]
pub(super) struct BuildCellPic(usize);
#[derive(Component)]
pub(super) struct BuildCellText(usize);
#[derive(Component)]
pub(super) struct UnitCard;
#[derive(Component)]
pub(super) struct CardPic;
#[derive(Component)]
pub(super) struct CardName;
#[derive(Component)]
pub(super) struct CardText;
/// Bar fill: 0 health, 1 mana, 2 build progress.
#[derive(Component)]
pub(super) struct CardBar(u8);
#[derive(Component)]
pub(super) struct CardBarTrack(u8);
#[derive(Component)]
pub(super) struct SelStrip;
#[derive(Component)]
pub(super) struct StripCell(usize);
#[derive(Component)]
pub(super) struct StripPic(usize);
#[derive(Component)]
pub(super) struct StripCount(usize);
#[derive(Component)]
pub(super) struct HelpLine;
/// The bottom bar column (prompts, verb line, messages), right of the unit
/// card and left of the minimap.
#[derive(Component)]
pub(super) struct BottomBar;
#[derive(Component)]
pub(super) struct MsgLine;

/// One decoded frame as a Bevy image.
fn frame_image(
    images: &mut Assets<Image>,
    seq: &SpriteSeq,
    i: usize,
) -> Option<(Handle<Image>, [u32; 2], Vec<u8>)> {
    let f = seq.frames.get(i).or_else(|| seq.frames.first())?;
    Some((
        images.add(rgba_image(f.w, f.h, f.rgba.clone())),
        [f.w, f.h],
        f.rgba.clone(),
    ))
}

/// Decode the local side's skin and the shared button art (startup).
pub(super) fn setup_hud_art(
    mut commands: Commands,
    content: NonSend<Content>,
    sim: NonSend<SimRes>,
    mut images: ResMut<Assets<Image>>,
) {
    let s = &sim.session;
    let w = s.sim.world();
    let monarch = w
        .players()
        .iter()
        .find(|p| p.id == s.local)
        .and_then(|p| s.def(p.monarch_def))
        .map_or("ARAKING", |d| d.unitname.as_str());
    let skin = race_skin(monarch);
    let mut art = HudArt::default();
    let path = format!("anims/{skin}.gaf");
    let pal = content
        .sprites
        .palette(&content.vfs, &format!("anims/{skin}.pcx"));
    let names: Vec<String> = content
        .sprites
        .gaf(&content.vfs, &path)
        .map(|g| g.entries.iter().map(|e| e.name.clone()).collect())
        .unwrap_or_default();
    let seq = |i: usize| -> Option<SpriteSeq> {
        let pal = pal.as_ref()?;
        content.sprites.seq(&content.vfs, &path, names.get(i)?, pal)
    };
    let mut report = Vec::new();
    match seq(SKIN_BUTTON_PANEL).and_then(|sq| frame_image(&mut images, &sq, 1)) {
        Some((h, [pw, ph], rgba)) => {
            art.panel = Some(h);
            let found = button_slots(&rgba, pw, ph);
            art.slots = if found.len() == ARAMON_SLOTS.len() {
                report.push(format!("button panel ({} wells found)", found.len()));
                found
            } else {
                report.push(format!(
                    "button panel (wells: found {}, using Aramon's)",
                    found.len()
                ));
                ARAMON_SLOTS.to_vec()
            };
        }
        None => {
            report.push("button panel FALLBACK (plain)".into());
            art.slots = ARAMON_SLOTS.to_vec();
        }
    }
    match seq(SKIN_INFO_PANEL).and_then(|sq| frame_image(&mut images, &sq, 0)) {
        Some((h, _, _)) => {
            art.info = Some(h);
            report.push("info panel".into());
        }
        None => report.push("info panel FALLBACK (plain)".into()),
    }
    if let Some((h, [cw, ch], _)) =
        seq(SKIN_END_CAP).and_then(|sq| frame_image(&mut images, &sq, 0))
    {
        art.cap = Some((h, [cw as f32, ch as f32]));
        report.push("end cap".into());
    }
    let bpath = "anims/igcommonbuttons.gaf";
    let bpal = content
        .sprites
        .palette(&content.vfs, "anims/igcommonbuttons.pcx");
    let mut got = 0;
    for name in PANEL_BUTTONS {
        let frames: Vec<Handle<Image>> = bpal
            .as_ref()
            .and_then(|p| content.sprites.seq(&content.vfs, bpath, name, p))
            .map(|sq| {
                sq.frames
                    .iter()
                    .map(|f| images.add(rgba_image(f.w, f.h, f.rgba.clone())))
                    .collect()
            })
            .unwrap_or_default();
        got += usize::from(!frames.is_empty());
        art.buttons.push(frames);
    }
    report.push(format!(
        "igcommonbuttons {got}/{} buttons",
        PANEL_BUTTONS.len()
    ));
    // These are ordinary JPEGs, not GAF entry names. Cache by path so
    // shared spell art and repeated disabled/down pictures decode once.
    let mut pictures: BTreeMap<String, Option<Handle<Image>>> = BTreeMap::new();
    for (&key, weapon) in &s.weapon_art {
        let frames: [Option<Handle<Image>>; BUTTON_STATES] = std::array::from_fn(|state| {
            let path = weapon.button_images[state].as_ref()?;
            pictures
                .entry(path.clone())
                .or_insert_with(|| {
                    let bytes = content.vfs.read(path).ok()?;
                    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
                    let (w, h) = img.dimensions();
                    Some(images.add(rgba_image(w, h, img.into_raw())))
                })
                .clone()
        });
        if frames.iter().any(Option::is_some) {
            art.weapon_buttons.insert(key, frames);
        }
    }
    report.push(format!(
        "weapon pictures {}/{}",
        pictures.values().filter(|h| h.is_some()).count(),
        pictures.len()
    ));
    eprintln!("hud art: {skin}: {}", report.join(", "));
    commands.insert_resource(art);
    commands.init_resource::<HudMode>();
    commands.init_resource::<HudHover>();
    commands.init_resource::<BottomMsg>();
}

fn panel_bg(art: Option<&Handle<Image>>) -> (ImageNode, BackgroundColor) {
    match art {
        Some(h) => (ImageNode::new(h.clone()), BackgroundColor(Color::NONE)),
        None => (
            ImageNode::default().with_color(Color::NONE),
            BackgroundColor(Color::srgba(0.12, 0.1, 0.07, 0.92)),
        ),
    }
}

/// Spawn the HUD widgets (startup, after the art).
pub(super) fn setup_hud_view(
    mut commands: Commands,
    art: Res<HudArt>,
    bar: Query<Entity, With<BottomBar>>,
) {
    // Side panel: the race's button panel with the order buttons on it, at
    // the right edge like the original's UnitMenu.
    let (img, bg) = panel_bg(art.panel.as_ref());
    let panel = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                width: Val::Px(PANEL_SIZE[0]),
                height: Val::Px(PANEL_SIZE[1]),
                ..default()
            },
            img,
            bg,
            HudPanel,
            SidePanel,
        ))
        .id();
    for (i, s) in art.slots.iter().enumerate().take(PANEL_BUTTONS.len()) {
        let b = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(s[0] as f32 - 1.0),
                    top: Val::Px(s[1] as f32 - 1.0),
                    width: Val::Px(29.0),
                    height: Val::Px(29.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                ImageNode::default(),
                Visibility::Hidden,
                ButtonKind::Panel(i),
                ChildOf(panel),
            ))
            .id();
        commands.spawn((
            Text::new(""),
            hud_text(12.0),
            TextLayout::no_wrap(),
            PanelBtnLabel(i),
            ChildOf(b),
        ));
    }
    // Build menu: portraits two per row, a column beside the panel (top
    // aligned with it).
    let grid = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(PANEL_SIZE[0] + 2.0),
                bottom: Val::Px(0.0),
                width: Val::Px(PANEL_SIZE[0]),
                height: Val::Px(PANEL_SIZE[1]),
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                align_content: AlignContent::FlexStart,
                ..default()
            },
            BackgroundColor(Color::NONE),
            Visibility::Hidden,
            HudPanel,
            BuildGrid,
        ))
        .id();
    for i in 0..BUILD_CELLS {
        let c = commands
            .spawn((
                Node {
                    width: Val::Px(64.0),
                    height: Val::Px(48.0),
                    border: UiRect::all(Val::Px(1.0)),
                    display: Display::None,
                    ..default()
                },
                BorderColor::all(Color::srgb(0.3, 0.26, 0.18)),
                BackgroundColor(Color::srgb(0.05, 0.04, 0.03)),
                Visibility::Inherited,
                ButtonKind::Build(i),
                ChildOf(grid),
            ))
            .id();
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            ImageNode::default(),
            BuildCellPic(i),
            ChildOf(c),
        ));
        commands.spawn((
            Text::new(""),
            hud_text(11.0),
            TextLayout::no_wrap(),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(1.0),
                bottom: Val::Px(0.0),
                padding: UiRect::axes(Val::Px(2.0), Val::Px(0.0)),
                ..default()
            },
            BuildCellText(i),
            ChildOf(c),
        ));
    }
    // Unit card: the race's info panel at the bottom left.
    let (img, bg) = panel_bg(art.info.as_ref());
    let card = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                bottom: Val::Px(0.0),
                width: Val::Px(CARD_SIZE[0]),
                height: Val::Px(CARD_SIZE[1]),
                ..default()
            },
            img,
            bg,
            HudPanel,
            UnitCard,
        ))
        .id();
    if let Some((cap, [cw, ch])) = &art.cap {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                bottom: Val::Px(0.0),
                width: Val::Px(*cw),
                height: Val::Px(*ch),
                ..default()
            },
            ImageNode::new(cap.clone()),
            ChildOf(card),
        ));
    }
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(59.0),
            top: Val::Px(7.0),
            width: Val::Px(48.0),
            height: Val::Px(36.0),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        ImageNode::default(),
        BorderColor::all(Color::srgb(0.2, 0.17, 0.1)),
        BackgroundColor(Color::srgb(0.02, 0.02, 0.02)),
        CardPic,
        ChildOf(card),
    ));
    commands.spawn((
        Text::new(""),
        hud_text(14.0),
        TextLayout::no_wrap(),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(114.0),
            top: Val::Px(4.0),
            ..default()
        },
        CardName,
        ChildOf(card),
    ));
    for (k, y) in [(0u8, 25.0), (1, 33.0), (2, 41.0)] {
        let track = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(114.0),
                    top: Val::Px(y),
                    width: Val::Px(110.0),
                    height: Val::Px(5.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.1, 0.04, 0.03)),
                CardBarTrack(k),
                ChildOf(card),
            ))
            .id();
        commands.spawn((
            Node {
                width: Val::Percent(0.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.2, 0.85, 0.2)),
            CardBar(k),
            ChildOf(track),
        ));
    }
    commands.spawn((
        Text::new(""),
        hud_text(11.0),
        TextLayout::no_wrap(),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(234.0),
            top: Val::Px(5.0),
            ..default()
        },
        CardText,
        ChildOf(card),
    ));
    // Multi-selection strip above the card.
    let strip = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                bottom: Val::Px(CARD_SIZE[1] + 3.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(3.0),
                padding: UiRect::all(Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            Visibility::Hidden,
            HudPanel,
            SelStrip,
        ))
        .id();
    for i in 0..STRIP_CELLS {
        let c = commands
            .spawn((
                Node {
                    width: Val::Px(48.0),
                    height: Val::Px(36.0),
                    border: UiRect::all(Val::Px(2.0)),
                    display: Display::None,
                    ..default()
                },
                BorderColor::all(Color::srgb(0.3, 0.26, 0.18)),
                StripCell(i),
                ChildOf(strip),
            ))
            .id();
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            ImageNode::default(),
            StripPic(i),
            ChildOf(c),
        ));
        commands.spawn((
            Text::new(""),
            hud_text(12.0),
            TextLayout::no_wrap(),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                padding: UiRect::axes(Val::Px(2.0), Val::Px(0.0)),
                ..default()
            },
            StripCount(i),
            ChildOf(c),
        ));
    }
    // Bottom bar: the verb line and a message line above the button
    // prompts (the hint bar in `controls.rs`).
    let Ok(bar) = bar.single() else { return };
    commands.spawn((
        Text::new(""),
        hud_text(15.0),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        Node {
            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
            ..default()
        },
        HelpLine,
        ChildOf(bar),
    ));
    commands.spawn((
        Text::new(""),
        hud_text(14.0).0,
        TextColor(Color::srgb(1.0, 0.45, 0.35)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        Node {
            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
            ..default()
        },
        Visibility::Hidden,
        MsgLine,
        ChildOf(bar),
    ));
}

/// Pick the layout: `--hud console|classic`, or (auto) the controller
/// layout while the stick drives the cursor.
pub(super) fn hud_mode_system(input: Res<InputRes>, feel: Res<Feel>, mut mode: ResMut<HudMode>) {
    let console = match feel.hud_choice.as_str() {
        "console" => true,
        "classic" => false,
        _ => input.mapper.stick_cursor(),
    };
    if mode.console != console {
        mode.console = console;
    }
}

/// The selection's own, live units.
fn own_selection<'w>(
    w: &'w tak_sim::World,
    local: PlayerId,
    sel: &tak_commands::UnitSet,
) -> Vec<(UnitId, &'w tak_sim::world::Unit)> {
    sel.iter()
        .filter_map(|id| w.unit(*id).map(|u| (*id, u)))
        .filter(|(_, u)| u.owner == local)
        .collect()
}

/// Side panel buttons: art frame by state (dark: unusable, up, lit:
/// hovered, down: the active mode or stance), reserved wells left empty.
#[allow(clippy::too_many_arguments)]
pub(super) fn panel_system(
    sim: NonSend<SimRes>,
    input: Res<InputRes>,
    art: Res<HudArt>,
    mode: Res<HudMode>,
    mut hover: ResMut<HudHover>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut panel: Query<&mut Visibility, (With<SidePanel>, Without<ButtonKind>)>,
    mut buttons: Query<
        (
            &ButtonKind,
            &mut ImageNode,
            &mut Visibility,
            &ComputedNode,
            &UiGlobalTransform,
        ),
        Without<SidePanel>,
    >,
    mut labels: Query<(&PanelBtnLabel, &mut Text)>,
) {
    let show = !mode.console;
    for mut v in &mut panel {
        let want = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    hover.0 = None;
    if !show {
        return;
    }
    let s = &sim.session;
    let w = s.sim.world();
    let sel = own_selection(w, s.local, input.mapper.selection());
    let first = sel.first().map(|(_, u)| *u);
    let has_healer = !tak_input::healers_for(&sim.view(), input.mapper.selection()).is_empty();
    let caster = crate::view::caster_spells(w, s.local, input.mapper.selection());
    let caster_def = caster
        .as_ref()
        .and_then(|(id, _)| w.unit(*id))
        .map(|u| u.def);
    let spells = caster.as_ref().map_or(&[][..], |(_, sp)| sp.as_slice());
    let mouse = windows.single().ok().and_then(|w| w.cursor_position());
    let m = input.mapper.mode();
    for (kind, mut img, mut vis, cn, tr) in &mut buttons {
        let ButtonKind::Panel(i) = *kind else {
            continue;
        };
        let a = PANEL_ACTIONS[i];
        let frames = art.buttons.get(i).map_or(&[][..], Vec::as_slice);
        let spell = match a {
            Action::CastSpell { index } => spells.get(usize::from(index)),
            _ => None,
        };
        let picture = |state| {
            let def = caster_def?;
            art.weapon_button(def, spell?.id, state)
        };
        let shown = a.is_supported()
            && (!frames.is_empty() || picture(1).is_some())
            && match a {
                Action::CastSpell { index } => spells.len() > usize::from(index),
                Action::HealMode => has_healer,
                _ => true,
            };
        let want = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        if !shown {
            continue;
        }
        let usable = first.is_some()
            && match a {
                Action::CastSpell { index } => {
                    spells.get(usize::from(index)).is_some_and(|x| x.ready)
                }
                _ => true,
            };
        let active = match a {
            Action::AttackMode => m == Mode::Attack,
            Action::MoveMode => m == Mode::Move,
            Action::GuardMode => m == Mode::Guard,
            Action::HealMode => m == Mode::Heal,
            Action::PatrolMode => m == Mode::Patrol,
            Action::SetFireState { state } => first.is_some_and(|u| u.fire_state == state),
            Action::CastSpell { index } => spells
                .get(usize::from(index))
                .is_some_and(|x| m == Mode::CastSpell(x.id)),
            _ => false,
        };
        let hovered = mouse.is_some_and(|p| {
            let c = tr.translation;
            let h = cn.size / 2.0;
            p.x >= c.x - h.x && p.x <= c.x + h.x && p.y >= c.y - h.y && p.y <= c.y + h.y
        });
        if hovered {
            let name = match a {
                Action::CastSpell { .. } => spell.map_or(PANEL_NAMES[i].to_string(), |x| {
                    let shortage = x
                        .shortfall_text()
                        .map_or(String::new(), |t| format!("; {t}"));
                    format!("{}: {}{shortage}", x.name, x.mana_text())
                }),
                _ => PANEL_NAMES[i].to_string(),
            };
            hover.0 = Some(match key_label(input.mapper.profile(), a) {
                Some(k) => format!("{name}  [{k}]"),
                None => name,
            });
        }
        let f = button_state(usable, active, hovered);
        let custom = picture(f);
        img.color = if custom.is_some() && !usable {
            Color::srgb(0.5, 0.5, 0.5)
        } else {
            Color::WHITE
        };
        if let Some(h) = custom.or_else(|| frames.get(f).or_else(|| frames.last()))
            && img.image != *h
        {
            img.image = h.clone();
        }
    }
    for (l, mut t) in &mut labels {
        // Keep a numbered plate only when a mod/install lacks its art.
        let want = match PANEL_ACTIONS[l.0] {
            Action::CastSpell { index } => {
                let has_picture = spells.get(usize::from(index)).is_some_and(|sp| {
                    caster_def.is_some_and(|def| art.weapon_button(def, sp.id, 1).is_some())
                });
                if has_picture {
                    String::new()
                } else {
                    format!("{}", index + 1)
                }
            }
            _ => String::new(),
        };
        if t.0 != want {
            t.0 = want;
        }
    }
}

/// Build menu: the first own builder's options with portraits and costs,
/// greyed when the player's mana is short, the option being placed lit.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_grid_system(
    sim: NonSend<SimRes>,
    content: NonSend<Content>,
    input: Res<InputRes>,
    mode: Res<HudMode>,
    mut portraits: ResMut<controls::Portraits>,
    mut images: ResMut<Assets<Image>>,
    mut grid: Query<&mut Visibility, (With<BuildGrid>, Without<ButtonKind>)>,
    mut cells: Query<(&ButtonKind, &mut Node, &mut BorderColor), Without<BuildGrid>>,
    mut pics: Query<(&BuildCellPic, &mut ImageNode)>,
    mut texts: Query<(&BuildCellText, &mut Text)>,
) {
    let s = &sim.session;
    let w = s.sim.world();
    let sel = own_selection(w, s.local, input.mapper.selection());
    let opts: Vec<tak_commands::UnitDefId> = sel
        .iter()
        .find_map(|(_, u)| {
            w.defs()
                .get(u.def)
                .filter(|d| d.build.builder && !d.build.canbuild.is_empty())
        })
        .map(|d| d.build.canbuild.clone())
        .unwrap_or_default();
    let show = !mode.console && !opts.is_empty();
    for mut v in &mut grid {
        let want = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    if !show {
        return;
    }
    let mana = tak_sim::economy::player(w, s.local).map_or(0.0, |e| mana(e.mana));
    let m = input.mapper.mode();
    let mut short = [false; BUILD_CELLS];
    for (kind, mut node, mut border) in &mut cells {
        let ButtonKind::Build(i) = *kind else {
            continue;
        };
        let opt = opts.get(i).copied();
        let disp = if opt.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != disp {
            node.display = disp;
        }
        if let Some(o) = opt {
            let cost = w.defs().get(o).map_or(0, |d| d.build.buildcost);
            short[i] = cost as f64 > mana;
            *border = BorderColor::all(if m == Mode::Build(o) {
                Color::srgb(1.0, 0.8, 0.3)
            } else {
                Color::srgb(0.3, 0.26, 0.18)
            });
        }
    }
    for (p, mut img) in &mut pics {
        if let Some(o) = opts.get(p.0)
            && let Some(d) = s.def(*o)
            && let Some(h) = portraits.get(&content, &mut images, &d.unitname)
        {
            img.image = h;
            img.color = if short[p.0] {
                Color::srgb(0.4, 0.4, 0.4)
            } else {
                Color::WHITE
            };
        }
    }
    for (t, mut text) in &mut texts {
        let want = opts.get(t.0).map_or(String::new(), |o| {
            let cost = w.defs().get(*o).map_or(0, |d| d.build.buildcost);
            match input
                .mapper
                .profile()
                .grid_build_keys
                .then(|| tak_input::GRID_KEYS.get(t.0))
                .flatten()
            {
                Some(k) => format!("{} {cost}", k.to_ascii_uppercase()),
                None => format!("{cost}"),
            }
        });
        if text.0 != want {
            text.0 = want;
        }
    }
}

/// Unit card: portrait, name, health / mana / build bars, stance, current
/// order and queue (experience and kills are not in the sim yet).
#[allow(clippy::too_many_arguments)]
pub(super) fn unit_card_system(
    sim: NonSend<SimRes>,
    content: NonSend<Content>,
    input: Res<InputRes>,
    mut portraits: ResMut<controls::Portraits>,
    mut images: ResMut<Assets<Image>>,
    mut pic: Query<&mut ImageNode, With<CardPic>>,
    mut texts: Query<(&mut Text, Has<CardName>), Or<(With<CardName>, With<CardText>)>>,
    mut bars: Query<(&CardBar, &mut Node, &mut BackgroundColor), Without<CardBarTrack>>,
    mut tracks: Query<(&CardBarTrack, &mut Visibility)>,
) {
    let s = &sim.session;
    let w = s.sim.world();
    let sel: Vec<(UnitId, &tak_sim::world::Unit)> = input
        .mapper
        .selection()
        .iter()
        .filter_map(|id| w.unit(*id).map(|u| (*id, u)))
        .collect();
    let first = sel.first().copied();
    let (name, info) = match first {
        Some((_, u)) => {
            let d = s.def(u.def);
            let n = d.map_or("?".to_string(), |d| d.display.clone());
            let name = if sel.len() > 1 {
                format!("{n}  ({} selected)", sel.len())
            } else {
                n
            };
            let info = if u.owner == s.local {
                format!(
                    "{} - {}\nHealth {}/{}",
                    stance_name(u.fire_state),
                    crate::hud::order_text(w, u, |def| {
                        s.def(def).map_or("?".to_string(), |d| d.display.clone())
                    }),
                    u.combat.health,
                    u.combat.max_health
                )
            } else {
                format!("Enemy\nHealth {}/{}", u.combat.health, u.combat.max_health)
            };
            (name, info)
        }
        None => ("Nothing selected".to_string(), String::new()),
    };
    for (mut t, is_name) in &mut texts {
        let want = if is_name { &name } else { &info };
        if t.0 != *want {
            t.0 = want.clone();
        }
    }
    if let Ok(mut img) = pic.single_mut() {
        let h = first
            .and_then(|(_, u)| s.def(u.def))
            .and_then(|d| portraits.get(&content, &mut images, &d.unitname));
        match h {
            Some(h) => {
                img.image = h;
                img.color = Color::WHITE;
            }
            None => img.color = Color::NONE,
        }
    }
    let vals: [Option<(f32, Color)>; 3] = match first {
        Some((id, u)) => {
            let max = u.combat.max_health.max(1) as f32;
            let hf = u.combat.health as f32 / max;
            let [r, g, b] = health_rgb(hf);
            let mana = tak_sim::economy::unit_mana(w, id)
                .map(|m| (f(m.mana) / f(m.max).max(1.0), Color::srgb(0.3, 0.5, 1.0)));
            let left = u.build.percent_left();
            let build =
                (left > 0).then(|| ((100 - left) as f32 / 100.0, Color::srgb(0.95, 0.75, 0.25)));
            [Some((hf, Color::srgb(r, g, b))), mana, build]
        }
        None => [None, None, None],
    };
    for (b, mut n, mut bg) in &mut bars {
        if let Some((frac, c)) = vals[usize::from(b.0)] {
            n.width = Val::Percent(frac.clamp(0.0, 1.0) * 100.0);
            bg.0 = c;
        }
    }
    for (t, mut v) in &mut tracks {
        let want = if vals[usize::from(t.0)].is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
}

/// Multi-selection strip: the selection by type with counts; during a
/// sub-select cycle the full selection, the active type lit and the rest
/// dimmed.
#[allow(clippy::too_many_arguments)]
pub(super) fn strip_system(
    sim: NonSend<SimRes>,
    content: NonSend<Content>,
    input: Res<InputRes>,
    mut portraits: ResMut<controls::Portraits>,
    mut images: ResMut<Assets<Image>>,
    mut strip: Query<&mut Visibility, With<SelStrip>>,
    mut cells: Query<(&StripCell, &mut Node, &mut BorderColor)>,
    mut pics: Query<(&StripPic, &mut ImageNode)>,
    mut counts: Query<(&StripCount, &mut Text)>,
) {
    let s = &sim.session;
    let w = s.sim.world();
    let sub = input.mapper.sub_select();
    let set = sub.map_or(input.mapper.selection(), |ss| &ss.full);
    let units: Vec<(UnitId, tak_commands::UnitDefId)> = set
        .iter()
        .filter_map(|id| {
            w.unit(*id)
                .filter(|u| u.owner == s.local)
                .map(|u| (*id, u.def))
        })
        .collect();
    let groups = group_by_type(&units);
    let active = sub.and_then(|ss| ss.types.get(ss.index).copied());
    let show = units.len() > 1;
    for mut v in &mut strip {
        let want = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    if !show {
        return;
    }
    for (c, mut n, mut border) in &mut cells {
        let g = groups.get(c.0);
        let disp = if g.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if n.display != disp {
            n.display = disp;
        }
        let lit = g.is_some_and(|(d, _)| Some(*d) == active);
        *border = BorderColor::all(if lit {
            Color::srgb(1.0, 0.82, 0.3)
        } else {
            Color::srgb(0.3, 0.26, 0.18)
        });
    }
    for (p, mut img) in &mut pics {
        if let Some((d, _)) = groups.get(p.0) {
            match s
                .def(*d)
                .and_then(|dd| portraits.get(&content, &mut images, &dd.unitname))
            {
                Some(h) => img.image = h,
                None => img.color = Color::NONE,
            }
            img.color = if active.is_some() && Some(*d) != active {
                Color::srgb(0.35, 0.35, 0.35)
            } else {
                Color::WHITE
            };
        }
    }
    for (c, mut t) in &mut counts {
        let want = groups
            .get(c.0)
            .map_or(String::new(), |(_, v)| format!("x{}", v.len()));
        if t.0 != want {
            t.0 = want;
        }
    }
}

/// The bottom bar: what the confirm input does here (verb, target), the
/// placement refusal while placing, and messages (refusals, the latest
/// rejected build, alerts).
#[allow(clippy::too_many_arguments)]
pub(super) fn bottom_bar_system(
    time: Res<Time>,
    mut sim: NonSendMut<SimRes>,
    input: Res<InputRes>,
    mut feel: ResMut<Feel>,
    mode: Res<HudMode>,
    hover: Res<HudHover>,
    plan: Option<Res<ShotPlan>>,
    mut msg: ResMut<BottomMsg>,
    cam: Res<CamState>,
    minimap: Res<MinimapState>,
    mut column: Query<&mut Node, (With<BottomBar>, Without<HelpLine>, Without<MsgLine>)>,
    mut help: Query<(&mut Text, &mut Node), (With<HelpLine>, Without<MsgLine>)>,
    mut line: Query<(&mut Text, &mut Node, &mut Visibility), (With<MsgLine>, Without<HelpLine>)>,
) {
    let right = if mode.console {
        minimap
            .geom(cam.geom.size)
            .map_or(cam.geom.size[0], |g| g.panel[0])
    } else {
        cam.geom.size[0] - 2.0 * PANEL_SIZE[0] - 2.0
    };
    if let Ok(mut n) = column.single_mut() {
        let want = Val::Px((right - BOTTOM_BAR_X - 8.0).max(160.0));
        if n.max_width != want {
            n.max_width = want;
        }
    }
    let now = time.elapsed_secs();
    // Screenshot runs draw a few frames a second: keep messages up.
    let hold = if plan.is_some() { 60.0 } else { 3.0 };
    // Refusals from the mapper this frame.
    for fb in &feel.feedback {
        if let Feedback::Refused { reason } = fb {
            msg.text = reason.reason().to_string();
            msg.until = now + hold;
        }
    }
    if let Some(t) = feel.toast.take() {
        msg.text = t;
        msg.until = now + hold;
    }
    if let Some(a) = std::mem::take(&mut sim.new_alerts).pop() {
        msg.text = a.text;
        msg.until = now + hold + 1.0;
    }
    let view = sim.view();
    let s = &sim.session;
    let w = s.sim.world();
    let m = &input.mapper;
    let cursor = m.cursor_world();
    let hovered = tak_input::pick_unit(&view, cursor);
    let target_name = hovered
        .and_then(|h| w.unit(h.id))
        .and_then(|u| s.def(u.def))
        .map(|d| d.display.clone());
    let prompt = if mode.console {
        crate::feel::pad_action_name(m.profile(), feel.layout, |a| {
            matches!(a, Action::Context { append: false })
        })
        .unwrap_or_else(|| "confirm".into())
    } else if m.mode() != Mode::None {
        "LMB".into()
    } else {
        crate::feel::mouse_order_name(m)
    };
    let select_prompt = if mode.console {
        crate::feel::pad_action_name(m.profile(), feel.layout, |a| matches!(a, Action::Select))
            .unwrap_or_else(|| "confirm".into())
    } else {
        "LMB".into()
    };
    let mut placement_error = None;
    let verb = match m.cursor_kind(&view) {
        CursorKind::Normal => String::new(),
        CursorKind::Select => verb_line(&select_prompt, "Select", target_name.as_deref()),
        CursorKind::Move => verb_line(&prompt, "Move", None),
        CursorKind::Attack if m.mode() == Mode::AttackMove => {
            verb_line(&prompt, "Attack-move", None)
        }
        CursorKind::Attack => verb_line(&prompt, "Attack", target_name.as_deref()),
        CursorKind::Guard => verb_line(&prompt, "Guard", target_name.as_deref()),
        CursorKind::Heal => verb_line(&prompt, "Heal / repair", target_name.as_deref()),
        CursorKind::Patrol => verb_line(&prompt, "Patrol", None),
        CursorKind::Cast => verb_line(&prompt, "Cast", target_name.as_deref()),
        CursorKind::Invalid => "Not possible here".into(),
        CursorKind::Build | CursorKind::BuildBlocked => {
            let def = match m.mode() {
                Mode::Build(d) => Some(d),
                _ => None,
            };
            let name = def
                .and_then(|d| s.def(d))
                .map_or("?".to_string(), |d| d.display.clone());
            if let Some(d) = def
                && let Some(dd) = w.defs().get(d)
            {
                use tak_sim::construction::placement::{check_site, site_center};
                if let Err(e) = check_site(w, d, site_center(dd, cursor), None) {
                    placement_error = Some(format!("Cannot build {name} here: {}", e.reason()));
                }
            }
            if placement_error.is_some() {
                format!("{name}: blocked")
            } else {
                verb_line(&prompt, "Build", Some(&name))
            }
        }
    };
    if let Ok((mut t, mut node)) = help.single_mut() {
        let want = if feel.ghost.is_some() || feel.overlay_open || m.wheel().is_some() {
            String::new()
        } else if let Some(h) = &hover.0 {
            h.clone()
        } else {
            verb
        };
        let disp = if want.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
        if node.display != disp {
            node.display = disp;
        }
        if t.0 != want {
            t.0 = want;
        }
    }
    // The latest refused build (construction rejections) for 5 s.
    let rejected = {
        let tick = w.tick().0;
        w.construction
            .rejections
            .iter()
            .rev()
            .find(|r| r.player == s.local && tick.saturating_sub(r.tick.0) < 150)
            .map(|r| {
                format!(
                    "Cannot build {}: {}",
                    s.def(r.def).map_or("?", |d| d.display.as_str()),
                    r.reason.reason()
                )
            })
    };
    let text = placement_error
        .or(rejected)
        .or_else(|| (now < msg.until).then(|| msg.text.clone()));
    if let Ok((mut t, mut node, mut v)) = line.single_mut() {
        match text {
            Some(x) if !x.is_empty() => {
                if t.0 != x {
                    t.0 = x;
                }
                *v = Visibility::Inherited;
                node.display = Display::Flex;
            }
            _ => {
                *v = Visibility::Hidden;
                if node.display != Display::None {
                    node.display = Display::None;
                }
            }
        }
    }
    let _ = Refusal::NothingThere;
    let _ = DamageBars::All;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_panel_actions_match_weapon_then_stance_art() {
        for (index, action) in PANEL_ACTIONS[7..10].iter().enumerate() {
            assert_eq!(*action, Action::CastSpell { index: index as u8 });
        }
        assert_eq!(
            PANEL_ACTIONS[10],
            Action::SetFireState {
                state: tak_commands::FireState::FireAtWill
            }
        );
        assert_eq!(
            PANEL_ACTIONS[11],
            Action::SetFireState {
                state: tak_commands::FireState::Defensive
            }
        );
        assert_eq!(
            PANEL_ACTIONS[12],
            Action::SetFireState {
                state: tak_commands::FireState::HoldFire
            }
        );
    }
}
