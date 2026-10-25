//! In-game, device-neutral UI for verified replay checkpoints.
//! This module is a child of `app` so it can pause input without touching sim rules.

use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;
use bevy::window::PrimaryWindow;

use super::{Feel, InputRes, MinimapState, ShotPlan, SimRes, hud_text};
use crate::cli::Args;
use crate::saves::browser::{self, Browser, Operation, PAGE_ROWS};

#[derive(Resource)]
pub(super) struct BrowserArgs(pub Args);

/// Prepared independently of the live match. The app must also load render
/// assets successfully before committing this replacement.
pub(super) struct PreparedLoad {
    pub session: crate::game::Session,
    pub path: std::path::PathBuf,
}

#[derive(Default)]
pub(super) struct PendingLoad {
    pub prepared: Option<PreparedLoad>,
}

#[derive(Resource, Default)]
pub(super) struct BrowserState {
    open: bool,
    captured: bool,
    was_paused: bool,
    model: Browser,
    deferred: Option<Operation>,
    waiting_for_swap: bool,
}

impl BrowserState {
    pub fn captures_input(&self) -> bool {
        self.open || self.captured
    }

    pub fn report_error(&mut self, message: String) {
        self.model.message = format!("Not completed: {message}");
        self.deferred = None;
        self.waiting_for_swap = false;
    }

    pub fn loaded(&mut self) {
        self.open = true;
        self.captured = true;
        self.was_paused = false;
        self.model.message = "Loaded verified checkpoint. Close [Esc / B] to resume; camera, selection and explored fog reset.".into();
        self.model.confirmation = None;
        self.deferred = None;
        self.waiting_for_swap = false;
    }

    pub fn cancel_for_transition(&mut self) {
        *self = Self::default();
    }

    fn schedule(&mut self, operation: Operation) {
        self.model.message = match &operation {
            Operation::SaveNew | Operation::Overwrite(_) => "Verifying executed history before saving. Replay may take time...",
            Operation::Load(_) => "Verifying content and replaying the checkpoint. The current match is unchanged until ready...",
        }.into();
        self.deferred = Some(operation);
    }
}

#[derive(Component)]
pub(super) struct Root;
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    Open,
    Row(usize),
    Previous,
    Next,
    SaveNew,
    Overwrite,
    Load,
    Refresh,
    Close,
    Confirm,
    Cancel,
}
#[derive(Component)]
pub(super) enum Label {
    Row(usize),
    Details,
    Status,
    Help,
}

fn control(
    commands: &mut Commands,
    parent: Option<Entity>,
    action: Control,
    label: &str,
) -> Entity {
    let mut entity = commands.spawn((
        Node {
            padding: UiRect::axes(Val::Px(9.0), Val::Px(5.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.12, 0.17, 0.23)),
        BorderColor::all(Color::srgb(0.47, 0.57, 0.67)),
        action,
    ));
    if let Some(parent) = parent {
        entity.insert(ChildOf(parent));
    }
    let id = entity.id();
    commands.spawn((Text::new(label), hud_text(13.0), ChildOf(id)));
    id
}

pub(super) fn setup(mut commands: Commands) {
    let launcher = control(&mut commands, None, Control::Open, "Checkpoints [F8]");
    commands.entity(launcher).insert((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(10.0),
            top: Val::Px(6.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            border: UiRect::all(Val::Px(1.0)),
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
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.72)),
            GlobalZIndex(1100),
            Visibility::Hidden,
            Root,
        ))
        .id();
    let panel = commands
        .spawn((
            Node {
                width: Val::Px(790.0),
                max_width: Val::Percent(96.0),
                max_height: Val::Percent(98.0),
                padding: UiRect::all(Val::Px(12.0)),
                row_gap: Val::Px(5.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgb(0.055, 0.07, 0.09)),
            ChildOf(root),
        ))
        .id();
    commands.spawn((
        Text::new("Replay checkpoints"),
        hud_text(21.0),
        ChildOf(panel),
    ));
    commands.spawn((Text::new("Match paused. Saves reconstruct executed orders, not camera, selection or explored fog.\nLoading checks the installed gameplay fingerprint and final simulation hash."), hud_text(12.0), ChildOf(panel)));
    for row in 0..PAGE_ROWS {
        let node = commands
            .spawn((
                Node {
                    min_height: Val::Px(39.0),
                    padding: UiRect::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.08, 0.11, 0.15)),
                Control::Row(row),
                ChildOf(panel),
            ))
            .id();
        commands.spawn((
            Text::new(""),
            hud_text(12.0),
            Label::Row(row),
            ChildOf(node),
        ));
    }
    commands.spawn((
        Text::new(""),
        hud_text(12.0),
        Label::Details,
        ChildOf(panel),
    ));
    commands.spawn((Text::new(""), hud_text(13.0), Label::Status, ChildOf(panel)));
    for buttons in [
        vec![
            (Control::Previous, "Previous page"),
            (Control::Next, "Next page"),
            (Control::Refresh, "Refresh [R]"),
            (Control::Close, "Close [Esc]"),
        ],
        vec![
            (Control::SaveNew, "New checkpoint [S]"),
            (Control::Overwrite, "Overwrite [O]"),
            (Control::Load, "Load [Enter]"),
            (Control::Cancel, "Cancel"),
            (Control::Confirm, "Confirm"),
        ],
    ] {
        let row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(6.0),
                    row_gap: Val::Px(3.0),
                    ..default()
                },
                ChildOf(panel),
            ))
            .id();
        for (action, label) in buttons {
            control(&mut commands, Some(row), action, label);
        }
    }
    commands.spawn((Text::new(""), hud_text(11.0), Label::Help, ChildOf(panel)));
}

fn shorten(value: &str, max: usize) -> String {
    let value: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if value.chars().count() <= max {
        value
    } else {
        format!(
            "{}...",
            value
                .chars()
                .take(max.saturating_sub(3))
                .collect::<String>()
        )
    }
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
    args: Res<BrowserArgs>,
    mut state: ResMut<BrowserState>,
    mut pending: NonSendMut<PendingLoad>,
    mut sim: NonSendMut<SimRes>,
    mut feel: ResMut<Feel>,
    mut gameplay: ResMut<InputRes>,
    mut minimap: ResMut<MinimapState>,
    skirmish: Option<Res<super::skirmish_ui::SkirmishState>>,
) {
    if skirmish.is_some_and(|state| state.captures_input()) {
        return;
    }
    state.captured = state.open;
    if plan.is_some() {
        return;
    }
    let clicked = if mouse.just_pressed(MouseButton::Left) {
        windows
            .single()
            .ok()
            .and_then(|window| window.cursor_position())
            .and_then(|p| {
                controls
                    .iter()
                    .find(|(_, n, t, v)| {
                        let half = n.size / 2.0;
                        v.get()
                            && p.x >= t.translation.x - half.x
                            && p.x <= t.translation.x + half.x
                            && p.y >= t.translation.y - half.y
                            && p.y <= t.translation.y + half.y
                    })
                    .map(|(control, ..)| *control)
            })
    } else {
        None
    };
    let pad = pads.iter().next();
    let pressed = |button| pad.is_some_and(|pad| pad.just_pressed(button));
    let directory = feel.quick_save_dir.clone();
    if !state.open {
        if keys.just_pressed(KeyCode::F8)
            || clicked == Some(Control::Open)
            || feel.overlay_open && pressed(GamepadButton::Select)
        {
            state.open = true;
            state.captured = true;
            state.was_paused = sim.paused;
            // Match the settings overlay's gesture reset: releases that occur
            // inside a modal must not leave held queue/zoom/drag/wheel state.
            let profile = gameplay.mapper.profile().clone();
            gameplay.mapper.set_profile(profile);
            gameplay.last_axes.clear();
            gameplay.last_pointer = None;
            minimap.mouse_drag = false;
            minimap.ui_held = false;
            minimap.swallow_release = false;
            minimap.pad_swallow = false;
            state.model.message =
                "Choose a checkpoint or save a new one. No files are changed by browsing.".into();
            match &directory {
                Ok(directory) => {
                    if let Err(error) = state.model.refresh(directory, &args.0.data) {
                        state.report_error(error.to_string());
                    }
                }
                Err(error) => state.report_error(error.clone()),
            }
            feel.overlay_open = false;
            feel.overlay_captured_input = true;
            sim.paused = true;
        }
        return;
    }
    feel.overlay_open = false;
    feel.overlay_captured_input = true;
    sim.paused = true;
    let cancel = keys.just_pressed(KeyCode::Escape)
        || keys.just_pressed(KeyCode::F8)
        || pressed(GamepadButton::East)
        || pressed(GamepadButton::Start)
        || clicked == Some(Control::Close)
        || clicked == Some(Control::Cancel);
    if cancel {
        // A frame is deliberately left for the progress notice. Cancellation
        // on that frame takes priority over starting a synchronous replay.
        if state.deferred.take().is_some() || state.waiting_for_swap {
            pending.prepared = None;
            state.waiting_for_swap = false;
            state.model.message = "Cancelled before replacing or writing anything.".into();
            return;
        }
        if state.model.confirmation.take().is_none() {
            state.open = false;
            sim.paused = state.was_paused;
        }
        return;
    }
    if state.waiting_for_swap {
        return;
    }
    if let Some(operation) = state.deferred.take() {
        let result = match directory.as_ref() {
            Err(error) => Err(crate::game::SetupError(error.clone())),
            Ok(directory) => match operation {
                Operation::SaveNew => browser::save_new(&sim.session, directory).map(Some),
                Operation::Overwrite(entry) => {
                    browser::overwrite(&sim.session, directory, &entry).map(Some)
                }
                Operation::Load(entry) => {
                    browser::prepare_load(&args.0, directory, &entry).map(|session| {
                        pending.prepared = Some(PreparedLoad {
                            session,
                            path: entry.path,
                        });
                        state.waiting_for_swap = true;
                        None
                    })
                }
            },
        };
        match result {
            Ok(Some(path)) => {
                state.model.message = format!(
                    "Saved verified checkpoint: {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                if let Ok(directory) = &directory {
                    if let Err(error) = state.model.refresh(directory, &args.0.data) {
                        state.report_error(error.to_string());
                    }
                    if let Some(index) = state
                        .model
                        .catalog
                        .entries
                        .iter()
                        .position(|entry| entry.path == path)
                    {
                        state.model.selected = index;
                    }
                }
            }
            Ok(None) => {}
            Err(error) => state.report_error(error.to_string()),
        }
        return;
    }
    if let Some(confirmation) = &mut state.model.confirmation {
        if keys.just_pressed(KeyCode::Tab)
            || keys.just_pressed(KeyCode::ArrowLeft)
            || keys.just_pressed(KeyCode::ArrowRight)
            || pressed(GamepadButton::DPadLeft)
            || pressed(GamepadButton::DPadRight)
        {
            confirmation.accept = !confirmation.accept;
        }
        let choice = if clicked == Some(Control::Confirm) {
            Some(true)
        } else if keys.just_pressed(KeyCode::Enter) || pressed(GamepadButton::South) {
            Some(confirmation.accept)
        } else {
            None
        };
        if let Some(choice) = choice
            && let Some(operation) = state.model.resolve_confirmation(choice)
        {
            state.schedule(operation);
        }
        return;
    }
    if keys.just_pressed(KeyCode::ArrowUp) || pressed(GamepadButton::DPadUp) {
        state.model.navigate(-1);
    }
    if keys.just_pressed(KeyCode::ArrowDown) || pressed(GamepadButton::DPadDown) {
        state.model.navigate(1);
    }
    if keys.just_pressed(KeyCode::PageUp)
        || pressed(GamepadButton::LeftTrigger)
        || clicked == Some(Control::Previous)
    {
        state.model.navigate(-(PAGE_ROWS as isize));
    }
    if keys.just_pressed(KeyCode::PageDown)
        || pressed(GamepadButton::RightTrigger)
        || clicked == Some(Control::Next)
    {
        state.model.navigate(PAGE_ROWS as isize);
    }
    if let Some(Control::Row(row)) = clicked {
        let index = state.model.first_visible() + row;
        if index < state.model.catalog.entries.len() {
            state.model.selected = index;
        }
    }
    if keys.just_pressed(KeyCode::KeyR)
        || pressed(GamepadButton::Select)
        || clicked == Some(Control::Refresh)
    {
        match directory {
            Ok(directory) => match state.model.refresh(&directory, &args.0.data) {
                Ok(()) => state.model.message = "Checkpoint list refreshed.".into(),
                Err(error) => state.report_error(error.to_string()),
            },
            Err(error) => state.report_error(error),
        }
    } else if keys.just_pressed(KeyCode::KeyS)
        || pressed(GamepadButton::West)
        || clicked == Some(Control::SaveNew)
    {
        match browser::ensure_no_pending_orders(&sim.session) {
            Ok(()) => state.schedule(Operation::SaveNew),
            Err(error) => state.report_error(error.to_string()),
        }
    } else {
        let overwrite = keys.just_pressed(KeyCode::KeyO)
            || pressed(GamepadButton::North)
            || clicked == Some(Control::Overwrite);
        let load = keys.just_pressed(KeyCode::Enter)
            || pressed(GamepadButton::South)
            || clicked == Some(Control::Load);
        if overwrite || load {
            let result = if overwrite {
                browser::ensure_no_pending_orders(&sim.session)
            } else {
                Ok(())
            }
            .and_then(|()| state.model.request_selected(overwrite));
            if let Err(error) = result {
                state.report_error(error.to_string());
            }
        }
    }
}

pub(super) fn draw(
    state: Res<BrowserState>,
    sim: NonSend<SimRes>,
    plan: Option<Res<ShotPlan>>,
    mut roots: Query<&mut Visibility, With<Root>>,
    mut labels: Query<(&Label, &mut Text)>,
    mut controls: Query<(&Control, &mut Node, &mut BackgroundColor)>,
) {
    for mut visibility in &mut roots {
        *visibility = if state.open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    let first = state.model.first_visible();
    let confirmation = state.model.confirmation.as_ref();
    for (action, mut node, mut background) in &mut controls {
        let visible = match action {
            Control::Open => !state.open && plan.is_none(),
            Control::Confirm | Control::Cancel => confirmation.is_some(),
            Control::SaveNew
            | Control::Overwrite
            | Control::Load
            | Control::Refresh
            | Control::Previous
            | Control::Next => confirmation.is_none(),
            Control::Row(row) => first + row < state.model.catalog.entries.len(),
            Control::Close => true,
        };
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        let selected = match action {
            Control::Row(row) => first + row == state.model.selected,
            Control::Confirm => confirmation.is_some_and(|confirmation| confirmation.accept),
            Control::Cancel => confirmation.is_some_and(|confirmation| !confirmation.accept),
            _ => false,
        };
        background.0 = if selected {
            Color::srgb(0.2, 0.34, 0.43)
        } else {
            Color::srgb(0.10, 0.14, 0.19)
        };
    }
    if !state.open {
        return;
    }
    for (label, mut text) in &mut labels {
        text.0 = match label {
            Label::Row(row) => state.model.catalog.entries.get(first + row).map(|entry| {
                match &entry.metadata {
                    Ok(metadata) => format!("{}  |  {}  |  {}\n{}  |  {}", shorten(&metadata.map, 55), metadata.match_time(), shorten(&metadata.factions, 28), shorten(&entry.name, 66), entry.saved_time()),
                    Err(error) => format!("{}\nUnavailable: {}", shorten(&entry.name, 78), shorten(error, 100)),
                }
            }).unwrap_or_default(),
            Label::Details => match state.model.selected() {
                None => "No checkpoints yet. New checkpoint creates a separate file; F10 remains quick-save.".into(),
                Some(entry) => match &entry.metadata {
                    Ok(metadata) => format!("{} | next tick {} | {} executed orders | gameplay fingerprint {}\n{} | state {} | checked on load (listing does not replay)", if !metadata.playable { "Headless synthetic" } else if metadata.campaign { "Campaign placements" } else { "Skirmish" }, metadata.next_tick, metadata.commands, shorten(&metadata.content_identity, 48), shorten(&metadata.factions, 80), metadata.state_hash),
                    Err(error) => format!("This file cannot be loaded or overwritten: {}", shorten(error, 180)),
                },
            },
            Label::Status => if let Some(confirmation) = confirmation {
                match &confirmation.operation {
                    Operation::Overwrite(entry) => format!("Replace {} with the current executed history?\nThe existing checkpoint will be replaced. Cancel is selected by default.", shorten(&entry.name, 82)),
                    Operation::Load(entry) => format!("Load {}?\nThis replaces the current match and discards unsaved progress, including {} pending orders.", shorten(&entry.name, 82), browser::pending_orders(&sim.session)),
                    Operation::SaveNew => String::new(),
                }
            } else { shorten(&state.model.message, 300) },
            Label::Help => if confirmation.is_some() {
                "Tab / Left / Right or D-pad: choose Cancel / Confirm. Enter / A: activate. Esc / B: cancel.".into()
            } else {
                format!("{}-{} of {}{} | Arrows / D-pad: select | PgUp/PgDn / LB/RB: page\nA: load | X: new | Y: overwrite | View: refresh | B: close. F10 keeps its quick-save behavior.", if state.model.catalog.entries.is_empty() { 0 } else { first + 1 }, (first + PAGE_ROWS).min(state.model.catalog.entries.len()), state.model.catalog.entries.len(), if state.model.catalog.truncated { " (bounded list; move older saves out of folder)" } else { "" })
            },
        };
    }
}

#[cfg(test)]
#[path = "save_browser_tests.rs"]
mod tests;
