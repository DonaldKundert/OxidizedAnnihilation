//! Device-neutral, non-destructive skirmish setup and match-result modal.
use super::{Feel, InputRes, MinimapState, ShotPlan, SimRes, banner_for, hud_text, save_browser};
use crate::skirmish::SetupConfig;
use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;
use bevy::window::PrimaryWindow;

#[derive(Clone, Copy, Debug)]
pub(super) enum Request {
    Launch(SetupConfig),
    Rematch(SetupConfig),
}
#[derive(Resource, Default)]
pub(super) struct PendingRequest {
    pub request: Option<Request>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Setup,
    Result,
}
#[derive(Resource)]
pub(super) struct SkirmishState {
    pub config: SetupConfig,
    open: bool,
    captured: bool,
    startup: bool,
    was_paused: bool,
    page: Page,
    selected: usize,
    confirmation: Option<(Request, bool)>,
    waiting: bool,
    release_wait: bool,
    outcome_seen: bool,
    result: String,
    message: String,
    rematch: Option<SetupConfig>,
}
impl SkirmishState {
    pub fn new(config: SetupConfig, startup: bool) -> Self {
        Self {
            config,
            open: false,
            captured: false,
            startup,
            was_paused: false,
            page: Page::Setup,
            selected: 0,
            confirmation: None,
            waiting: false,
            release_wait: false,
            outcome_seen: false,
            result: String::new(),
            message: String::new(),
            rematch: Some(config),
        }
    }
    pub fn captures_input(&self) -> bool {
        self.open || self.captured || self.release_wait
    }
    pub fn report_error(&mut self, message: String) {
        self.waiting = false;
        self.message = format!("Not completed: {message}. Your current match is unchanged.");
    }
    pub fn launched(&mut self, config: SetupConfig) {
        self.config = config;
        self.rematch = Some(config);
        self.open = false;
        self.captured = true;
        self.release_wait = true;
        self.waiting = false;
        self.confirmation = None;
        self.outcome_seen = false;
        self.startup = false;
        self.result.clear();
        self.message.clear();
    }
    pub fn checkpoint_loaded(&mut self, config: Option<SetupConfig>) {
        if let Some(config) = config {
            self.config = config;
        }
        self.rematch = config;
        self.open = false;
        self.captured = false;
        self.release_wait = false;
        self.waiting = false;
        self.confirmation = None;
        self.outcome_seen = false;
        self.startup = false;
    }
    fn open(&mut self, page: Page, paused: bool) {
        self.open = true;
        self.captured = true;
        self.page = page;
        self.selected = 0;
        self.was_paused = paused;
        self.confirmation = None;
        self.message.clear();
    }
    fn request(&mut self, request: Request, pending: &mut PendingRequest) {
        if self.startup {
            pending.request = Some(request);
            self.waiting = true;
        } else {
            self.confirmation = Some((request, false));
        }
    }
}
#[derive(Component)]
pub(super) struct Root;
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    Open,
    Row(usize),
    Cancel,
    Confirm,
}
#[derive(Component)]
pub(super) enum Label {
    Title,
    Row(usize),
    Status,
    Help,
}
fn button(
    commands: &mut Commands,
    parent: Option<Entity>,
    action: Control,
    text: &str,
    label: Option<Label>,
) -> Entity {
    let mut entity = commands.spawn((
        Node {
            padding: UiRect::all(Val::Px(9.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.10, 0.14, 0.19)),
        BorderColor::all(Color::srgb(0.4, 0.5, 0.6)),
        action,
    ));
    if let Some(parent) = parent {
        entity.insert(ChildOf(parent));
    }
    let id = entity.id();
    let mut text = commands.spawn((Text::new(text), hud_text(16.0), ChildOf(id)));
    if let Some(label) = label {
        text.insert(label);
    }
    id
}
pub(super) fn setup(mut commands: Commands) {
    let launcher = button(&mut commands, None, Control::Open, "Skirmish [F7]", None);
    commands.entity(launcher).insert((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(10.0),
            top: Val::Px(42.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        GlobalZIndex(1000),
    ));
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
            GlobalZIndex(1200),
            Visibility::Hidden,
            Root,
        ))
        .id();
    let panel = commands
        .spawn((
            Node {
                width: Val::Px(700.0),
                max_width: Val::Percent(96.0),
                max_height: Val::Percent(98.0),
                padding: UiRect::all(Val::Px(18.0)),
                row_gap: Val::Px(12.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgb(0.055, 0.07, 0.09)),
            ChildOf(root),
        ))
        .id();
    commands.spawn((
        Text::new("Skirmish setup"),
        hud_text(25.0),
        Label::Title,
        ChildOf(panel),
    ));
    for row in 0..5 {
        button(
            &mut commands,
            Some(panel),
            Control::Row(row),
            "",
            Some(Label::Row(row)),
        );
    }
    commands.spawn((Text::new(""), hud_text(14.0), Label::Status, ChildOf(panel)));
    button(&mut commands, Some(panel), Control::Cancel, "Cancel", None);
    button(
        &mut commands,
        Some(panel),
        Control::Confirm,
        "Confirm replacement",
        None,
    );
    commands.spawn((Text::new(""), hud_text(12.0), Label::Help, ChildOf(panel)));
}
fn reset_gestures(input: &mut InputRes, minimap: &mut MinimapState) {
    let profile = input.mapper.profile().clone();
    input.mapper.set_profile(profile);
    input.last_axes.clear();
    input.last_pointer = None;
    minimap.mouse_drag = false;
    minimap.ui_held = [false; 3];
    minimap.swallow_release = false;
    minimap.pad_swallow = false;
}
pub(super) fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    pads: Query<&Gamepad>,
    windows: Query<&Window, With<PrimaryWindow>>,
    controls: Query<(
        &Control,
        &ComputedNode,
        &UiGlobalTransform,
        &InheritedVisibility,
    )>,
    plan: Option<Res<ShotPlan>>,
    browser: Option<Res<save_browser::BrowserState>>,
    args: Res<save_browser::BrowserArgs>,
    state: Option<ResMut<SkirmishState>>,
    pending: Option<ResMut<PendingRequest>>,
    mut sim: NonSendMut<SimRes>,
    mut feel: ResMut<Feel>,
    mut gameplay: ResMut<InputRes>,
    mut minimap: ResMut<MinimapState>,
) {
    let (Some(mut state), Some(mut pending)) = (state, pending) else {
        return;
    };
    state.captured = state.open;
    if plan.is_some() || !args.0.allows_match_replacement() {
        return;
    }
    let neutral = keys.get_pressed().next().is_none()
        && mouse.get_pressed().next().is_none()
        && pads.iter().all(|p| {
            p.get_pressed().next().is_none()
                && p.left_stick().length() < 0.2
                && p.right_stick().length() < 0.2
        });
    if state.release_wait {
        state.captured = true;
        reset_gestures(&mut gameplay, &mut minimap);
        if neutral {
            state.release_wait = false;
        }
        return;
    }
    if browser.is_some_and(|b| b.captures_input()) {
        return;
    }
    let clicked = if mouse.just_pressed(MouseButton::Left) {
        windows
            .single()
            .ok()
            .and_then(|w| w.cursor_position())
            .and_then(|p| {
                controls
                    .iter()
                    .find(|(_, n, t, v)| {
                        let h = n.size / 2.0;
                        v.get()
                            && p.x >= t.translation.x - h.x
                            && p.x <= t.translation.x + h.x
                            && p.y >= t.translation.y - h.y
                            && p.y <= t.translation.y + h.y
                    })
                    .map(|(c, ..)| *c)
            })
    } else {
        None
    };
    let pad = pads.iter().next();
    let pressed = |b| pad.is_some_and(|p| p.just_pressed(b));
    if !state.open {
        let outcome = banner_for(&sim.session);
        let automatic = outcome.is_some() && !state.outcome_seen;
        if state.startup
            || automatic
            || keys.just_pressed(KeyCode::F7)
            || clicked == Some(Control::Open)
            || feel.overlay_open && pressed(GamepadButton::RightThumb)
        {
            let page = if let Some((title, _, subtitle)) = outcome {
                state.result = format!("{title}\n{subtitle}");
                state.outcome_seen = true;
                Page::Result
            } else {
                Page::Setup
            };
            state.open(page, sim.paused);
            reset_gestures(&mut gameplay, &mut minimap);
            feel.overlay_open = false;
            feel.overlay_captured_input = true;
            sim.paused = true;
            // An automatic result/startup modal must never activate a button
            // held while it appeared. Neutral input is required first.
            state.release_wait = automatic || state.startup;
        }
        return;
    }
    feel.overlay_open = false;
    feel.overlay_captured_input = true;
    sim.paused = true;
    if keys.just_pressed(KeyCode::Escape)
        || keys.just_pressed(KeyCode::F7)
        || pressed(GamepadButton::East)
        || pressed(GamepadButton::Start)
        || clicked == Some(Control::Cancel)
    {
        if state.confirmation.take().is_some() {
            return;
        }
        if state.waiting {
            pending.request = None;
            state.waiting = false;
            state.message = "Cancelled before replacement.".into();
            return;
        }
        if state.startup {
            state.message = "Choose Launch skirmish to begin. No match is running yet.".into();
            return;
        }
        state.open = false;
        state.startup = false;
        state.release_wait = true;
        sim.paused = state.was_paused;
        reset_gestures(&mut gameplay, &mut minimap);
        return;
    }
    if state.waiting {
        return;
    }
    let activate = keys.just_pressed(KeyCode::Enter) || pressed(GamepadButton::South);
    let horizontal = keys.just_pressed(KeyCode::ArrowLeft)
        || keys.just_pressed(KeyCode::ArrowRight)
        || pressed(GamepadButton::DPadLeft)
        || pressed(GamepadButton::DPadRight);
    if let Some((request, mut accept)) = state.confirmation {
        if horizontal || keys.just_pressed(KeyCode::Tab) {
            accept = !accept;
            state.confirmation = Some((request, accept));
        }
        if clicked == Some(Control::Confirm) || activate && accept {
            pending.request = Some(request);
            state.waiting = true;
            state.confirmation = None;
        } else if activate {
            state.confirmation = None;
        }
        return;
    }
    let count = if state.page == Page::Setup {
        if state.startup { 4 } else { 5 }
    } else {
        3
    };
    if keys.just_pressed(KeyCode::ArrowDown)
        || keys.just_pressed(KeyCode::Tab)
        || pressed(GamepadButton::DPadDown)
    {
        state.selected = (state.selected + 1) % count;
    }
    if keys.just_pressed(KeyCode::ArrowUp) || pressed(GamepadButton::DPadUp) {
        state.selected = (state.selected + count - 1) % count;
    }
    let click_row = if let Some(Control::Row(row)) = clicked {
        (row < count).then_some(row)
    } else {
        None
    };
    if let Some(row) = click_row {
        state.selected = row;
    }
    if !(activate || horizontal || click_row.is_some()) {
        return;
    }
    let config = state.config;
    if state.page == Page::Setup {
        match state.selected {
            0 => state.message = "Adamantine Gate is the currently validated skirmish map.".into(),
            1 => state.config.faction = state.config.faction.opposite(),
            2 => state.config.opponent = state.config.opponent.next(),
            3 if !horizontal => state.request(Request::Launch(config), &mut pending),
            4 if !horizontal && !state.startup => {
                state.open = false;
                state.startup = false;
                state.release_wait = true;
                sim.paused = state.was_paused;
                reset_gestures(&mut gameplay, &mut minimap);
            }
            _ => {}
        }
    } else if !horizontal {
        match state.selected {
            0 => {
                if let Some(config) = state.rematch {
                    state.request(Request::Rematch(config), &mut pending);
                } else {
                    state.message="This match cannot be rematched here. Choose Skirmish setup for a validated new match.".into();
                }
            }
            1 => {
                state.page = Page::Setup;
                state.selected = 0;
                state.message.clear();
            }
            _ => {
                state.open = false;
                state.release_wait = true;
                sim.paused = state.was_paused;
                reset_gestures(&mut gameplay, &mut minimap);
            }
        }
    }
}
pub(super) fn draw(
    args: Res<save_browser::BrowserArgs>,
    state: Res<SkirmishState>,
    plan: Option<Res<ShotPlan>>,
    browser: Res<save_browser::BrowserState>,
    mut roots: Query<&mut Visibility, With<Root>>,
    mut labels: Query<(&Label, &mut Text)>,
    mut controls: Query<(&Control, &mut Node, &mut BackgroundColor)>,
) {
    for mut v in &mut roots {
        *v = if state.open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (c, mut node, mut bg) in &mut controls {
        let visible = match c {
            Control::Open => {
                args.0.allows_match_replacement()
                    && !state.open
                    && plan.is_none()
                    && !browser.captures_input()
            }
            Control::Confirm | Control::Cancel => state.confirmation.is_some(),
            Control::Row(row) => {
                state.confirmation.is_none()
                    && (if state.page == Page::Setup {
                        !state.startup || *row < 4
                    } else {
                        *row < 3
                    })
            }
        };
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        let selected = match c {
            Control::Row(r) => *r == state.selected,
            Control::Confirm => state.confirmation.is_some_and(|(_, a)| a),
            Control::Cancel => state.confirmation.is_some_and(|(_, a)| !a),
            _ => false,
        };
        bg.0 = if selected {
            Color::srgb(0.2, 0.34, 0.43)
        } else {
            Color::srgb(0.10, 0.14, 0.19)
        };
    }
    for (label, mut text) in &mut labels {
        text.0 = match label {
            Label::Title => {
                if state.page == Page::Setup {
                    "Skirmish setup".into()
                } else {
                    state.result.clone()
                }
            }
            Label::Row(row) => {
                if state.page == Page::Setup {
                    match row {
                        0 => format!("Map: {} (validated)", state.config.map.label()),
                        1 => format!("Your faction: {}  < >", state.config.faction.label()),
                        2 => format!("Opponent: {}  < >", state.config.opponent.label()),
                        3 => "Launch skirmish".into(),
                        _ => "Back to current match".into(),
                    }
                } else {
                    match row {
                        0 => {
                            if state.rematch.is_some() {
                                "Rematch".into()
                            } else {
                                "Rematch unavailable for this match".into()
                            }
                        }
                        1 => "Skirmish setup".into(),
                        2 => "Continue viewing match".into(),
                        _ => String::new(),
                    }
                }
            }
            Label::Status => {
                if state.confirmation.is_some() {
                    "Replace the current match? Unsaved progress and pending orders will be discarded. Cancel is selected by default.".into()
                } else if state.waiting {
                    "Preparing map and opponent. Your current match stays intact until everything is ready...".into()
                } else if state.page == Page::Result {
                    let summary = if state.rematch.is_some() {
                        "Rematch repeats this match's starting setup and seed. Skirmish setup lets you choose a new supported match."
                    } else {
                        "Rematch is unavailable for this match's map or controller configuration. Choose Skirmish setup for a new supported match."
                    };
                    format!("{summary}\n{}", state.message)
                } else {
                    format!(
                        "Enemy faction: {} | seed {}\n{}\n{}",
                        state.config.faction.opposite().label(),
                        state.config.seed,
                        state.config.opponent.disclosure(),
                        state.message
                    )
                }
            }
            Label::Help => {
                if state.confirmation.is_some() {
                    "Left / Right / Tab / D-pad: choose. Enter / A: activate. Esc / B: cancel."
                        .into()
                } else if state.startup {
                    "Up / Down / D-pad: select. Left / Right: change option. Enter / A: activate.\nChoose Launch skirmish to begin; close the window to quit. No match is running yet.".into()
                } else {
                    "Up / Down / D-pad: select. Left / Right: change option. Enter / A: activate. Esc / B: back.\nIn a match: F7 or click Skirmish. Controller: Menu settings, then R3. Match is paused here.".into()
                }
            }
        };
    }
}
#[cfg(test)]
#[path = "skirmish_ui_tests.rs"]
mod tests;
