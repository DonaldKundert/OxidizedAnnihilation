//! Controller-feel plumbing that needs no Bevy (WP-S): the input profile
//! file (presets, hot reload, save), button glyph sets, context button
//! hints, wheel slot labels, session-recording metadata, and the headless
//! scripted-session driver that produces recordings for ghost playback.
//!
//! All feel logic and tuning values live in `tak-input`; this module only
//! moves them in and out of files and turns them into labels.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tak_commands::{CommandLog, Scope};
use tak_input::{
    Action, Button, EventKind, FrameIo, Gesture, Input, InputEvent, Mapper, Mode, Output, Profile,
    Recorder, Recording, Tuning, WheelKind,
};

use crate::game::Session;
use crate::view::{ViewGeom, f};

// ---- profile file -----------------------------------------------------------

/// Where the overlay saves when no `--input-profile` was given.
pub fn default_profile_path() -> PathBuf {
    PathBuf::from("tak-input-profile.json")
}

/// The default control scheme (Kingdoms Console + Classic TA:K) with a
/// preset's tuning.
pub fn preset_profile(preset: &str) -> Result<Profile, String> {
    scheme_profile("kingdoms", preset)
}

/// A control scheme ([`tak_input::CONTROL_SCHEMES`]: `kingdoms`, `wp_s`,
/// `wp_j`) with a preset's feel values. The scheme keeps its own cursor mode
/// (reticle or free) and right-stick role; `wp_j` keeps all its values.
pub fn scheme_profile(scheme: &str, preset: &str) -> Result<Profile, String> {
    let tuning = Tuning::preset(preset).ok_or_else(|| {
        format!(
            "unknown --input-preset {preset:?} (try {})",
            tak_input::PRESETS.join(", ")
        )
    })?;
    let mut p = Profile::named(scheme).ok_or_else(|| {
        format!(
            "unknown --controls {scheme:?} (try {})",
            tak_input::CONTROL_SCHEMES.join(", ")
        )
    })?;
    if !scheme.eq_ignore_ascii_case("wp_j") {
        p.tuning = Tuning {
            reticle: p.tuning.reticle,
            right_stick_zoom: p.tuning.right_stick_zoom,
            ..tuning
        };
    }
    Ok(p)
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The starting profile: the file at `path` if it exists (it must be a
/// valid profile), else the preset (written to `path` when one is given,
/// so it can be edited). Returns the profile and the file's mtime.
pub fn initial_profile(
    scheme: &str,
    preset: &str,
    path: Option<&Path>,
) -> Result<(Profile, Option<SystemTime>), String> {
    if let Some(p) = path
        && p.exists()
    {
        let s = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let prof = Profile::from_json(&s).map_err(|e| format!("{}: {e}", p.display()))?;
        return Ok((prof, mtime(p)));
    }
    let prof = scheme_profile(scheme, preset)?;
    let mut t = None;
    if let Some(p) = path {
        t = Some(save_profile(p, &prof).map_err(|e| format!("{}: {e}", p.display()))?);
    }
    Ok((prof, t))
}

/// Write a profile (pretty JSON); returns the new mtime.
pub fn save_profile(path: &Path, p: &Profile) -> std::io::Result<SystemTime> {
    tak_headless::checkpoint::write_text_atomic(path, &(p.to_json_pretty() + "\n"))
        .map_err(std::io::Error::other)?;
    Ok(mtime(path).unwrap_or(SystemTime::UNIX_EPOCH))
}

/// Hot reload: if the file changed since `last`, parse it. `Some(Err)` for
/// a changed but invalid file (the old profile stays in use).
pub fn poll_profile(path: &Path, last: &mut Option<SystemTime>) -> Option<Result<Profile, String>> {
    let now = mtime(path)?;
    if *last == Some(now) {
        return None;
    }
    *last = Some(now);
    let s = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return Some(Err(format!("{}: {e}", path.display()))),
    };
    Some(Profile::from_json(&s).map_err(|e| format!("{}: {e}", path.display())))
}

// ---- glyphs -----------------------------------------------------------------

/// Controller button layout for glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Xbox: A B X Y, LB RB LT RT, View / Menu.
    Xbox,
    /// PlayStation: Cross Circle Square Triangle, L1 R1 L2 R2, Share / Options.
    PlayStation,
}

/// Sony's USB vendor id.
const SONY_VENDOR: u16 = 0x054c;

/// Layout from `--glyphs` (`xbox`, `playstation`/`ps`, or `auto`), with the
/// connected pad's name and vendor id (gilrs) for `auto`.
pub fn detect_layout(choice: &str, name: Option<&str>, vendor: Option<u16>) -> Layout {
    match choice.to_ascii_lowercase().as_str() {
        "xbox" => return Layout::Xbox,
        "playstation" | "ps" | "ps4" | "ps5" => return Layout::PlayStation,
        _ => {}
    }
    if vendor == Some(SONY_VENDOR) {
        return Layout::PlayStation;
    }
    let n = name.unwrap_or("").to_ascii_lowercase();
    let sony = ["dualsense", "dualshock", "playstation", "ps4", "ps5", "ps3"];
    if sony.iter().any(|k| n.contains(k)) || n == "wireless controller" {
        Layout::PlayStation
    } else {
        Layout::Xbox
    }
}

/// PlayStation face symbols.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    /// ✕
    Cross,
    /// ○
    Circle,
    /// □
    Square,
    /// △
    Triangle,
}

/// How to draw one button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// Text on the chip (empty when a symbol is drawn instead).
    pub text: &'static str,
    /// Chip colour (sRGB).
    pub color: [u8; 3],
    /// Round face button (else a pill).
    pub round: bool,
    /// PlayStation symbol drawn into the chip.
    pub symbol: Option<Symbol>,
    /// Name for docs and plain-text hints.
    pub name: &'static str,
}

const GREY: [u8; 3] = [70, 70, 78];

/// The glyph for a gamepad button in a layout (`None` for mouse buttons).
pub fn glyph(layout: Layout, b: Button) -> Option<Glyph> {
    let face = |text, color, symbol, name| Glyph {
        text,
        color,
        round: true,
        symbol,
        name,
    };
    let pill = |text, name| Glyph {
        text,
        color: GREY,
        round: false,
        symbol: None,
        name,
    };
    let xbox = layout == Layout::Xbox;
    Some(match b {
        Button::A if xbox => face("A", [60, 160, 60], None, "A"),
        Button::B if xbox => face("B", [190, 50, 50], None, "B"),
        Button::X if xbox => face("X", [50, 100, 200], None, "X"),
        Button::Y if xbox => face("Y", [210, 170, 30], None, "Y"),
        Button::A => face("", [40, 40, 48], Some(Symbol::Cross), "Cross"),
        Button::B => face("", [40, 40, 48], Some(Symbol::Circle), "Circle"),
        Button::X => face("", [40, 40, 48], Some(Symbol::Square), "Square"),
        Button::Y => face("", [40, 40, 48], Some(Symbol::Triangle), "Triangle"),
        Button::Lb => pill(
            if xbox { "LB" } else { "L1" },
            if xbox { "LB" } else { "L1" },
        ),
        Button::Rb => pill(
            if xbox { "RB" } else { "R1" },
            if xbox { "RB" } else { "R1" },
        ),
        Button::Lt => pill(
            if xbox { "LT" } else { "L2" },
            if xbox { "LT" } else { "L2" },
        ),
        Button::Rt => pill(
            if xbox { "RT" } else { "R2" },
            if xbox { "RT" } else { "R2" },
        ),
        Button::L3 => pill(
            if xbox { "LS" } else { "L3" },
            if xbox { "LS click" } else { "L3" },
        ),
        Button::R3 => pill(
            if xbox { "RS" } else { "R3" },
            if xbox { "RS click" } else { "R3" },
        ),
        Button::DpadUp => pill("D-up", "D-pad up"),
        Button::DpadDown => pill("D-dn", "D-pad down"),
        Button::DpadLeft => pill("D-lt", "D-pad left"),
        Button::DpadRight => pill("D-rt", "D-pad right"),
        Button::View => pill(
            if xbox { "View" } else { "Share" },
            if xbox { "View" } else { "Share" },
        ),
        Button::Menu => pill(
            if xbox { "Menu" } else { "Options" },
            if xbox { "Menu" } else { "Options" },
        ),
        Button::MouseLeft | Button::MouseRight | Button::MouseMiddle => return None,
    })
}

/// A plain chip for a stick ("LS" / "RS").
pub fn stick_glyph(left: bool) -> Glyph {
    Glyph {
        text: if left { "L-stick" } else { "R-stick" },
        color: GREY,
        round: false,
        symbol: None,
        name: if left { "left stick" } else { "right stick" },
    }
}

// ---- hints ------------------------------------------------------------------

/// What a hint chip shows: buttons (a chord shows two) or a stick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HintKey {
    /// One gamepad button, or a chord (modifier first).
    Buttons(Vec<Button>),
    /// The left (true) or right stick.
    Stick(bool),
    /// A keyboard key or mouse button, by name (Classic TA:K prompts).
    Text(String),
}

/// One button hint: `[glyph] prefix text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// What to press.
    pub key: HintKey,
    /// "hold", "2x" or "".
    pub prefix: &'static str,
    /// What it does.
    pub text: String,
}

/// The gamepad binding for an action, if any: `(button, chord modifier,
/// gesture)`.
fn pad_binding(
    p: &Profile,
    want: impl Fn(&Action) -> bool,
) -> Option<(Button, Option<Button>, Gesture)> {
    p.bindings.iter().find_map(|b| {
        let Input::Button(btn) = b.input else {
            return None;
        };
        if matches!(
            btn,
            Button::MouseLeft | Button::MouseRight | Button::MouseMiddle
        ) {
            return None;
        }
        let with = match &b.with {
            None => None,
            Some(Input::Button(m)) => Some(*m),
            Some(Input::Key(_)) => return None,
        };
        want(&b.action).then_some((btn, with, b.gesture))
    })
}

/// Text equivalent of a profile's gamepad glyphs, including a chord.
/// Used beside wheel details and in the verb line, so those prompts
/// follow both PlayStation layouts and custom bindings.
pub fn pad_action_name(
    p: &Profile,
    layout: Layout,
    want: impl Fn(&Action) -> bool,
) -> Option<String> {
    let (button, modifier, _) = pad_binding(p, want)?;
    let mut names = Vec::new();
    for b in modifier.into_iter().chain(std::iter::once(button)) {
        names.push(glyph(layout, b)?.name);
    }
    Some(names.join("+"))
}

fn hint_for(p: &Profile, text: &str, want: impl Fn(&Action) -> bool) -> Option<Hint> {
    let (b, with, g) = pad_binding(p, want)?;
    let prefix = match g {
        Gesture::Hold => "hold",
        Gesture::DoubleTap | Gesture::SecondTap => "2x",
        Gesture::Release => "release",
        Gesture::Press | Gesture::Tap => "",
    };
    let mut keys = Vec::new();
    keys.extend(with);
    keys.push(b);
    Some(Hint {
        key: HintKey::Buttons(keys),
        prefix,
        text: text.to_owned(),
    })
}

/// What the HUD knows about the selection, for the prompts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HintCx {
    /// Own units selected.
    pub has_units: bool,
    /// A builder with a nonempty build menu among them.
    pub has_builder: bool,
    /// An operational explicit healer among them, even without a build menu.
    pub has_healer: bool,
    /// A spellcaster among them.
    pub has_caster: bool,
    /// More than one unit type among them (the sub-select cycle applies).
    pub mixed: bool,
}

/// Context button prompts for the bottom bar, read from the profile's
/// bindings (so rebinding updates them): the controller's buttons.
pub fn hints(m: &Mapper, cx: HintCx) -> Vec<Hint> {
    let p = m.profile();
    let mut v: Vec<Option<Hint>> = Vec::new();
    if let Some(w) = m.wheel() {
        v.push(Some(Hint {
            key: HintKey::Stick(true),
            prefix: "",
            text: "choose".into(),
        }));
        let sticky = m.tuning().wheel_confirm_on_press || m.wheel_depth() > 0;
        if sticky {
            v.push(hint_for(p, "confirm", |a| matches!(a, Action::Select)));
        } else {
            let want = match w.kind {
                WheelKind::Build => Action::BuildWheel,
                WheelKind::Spell => Action::SpellWheel,
                _ => Action::CommandWheel,
            };
            let mut h = hint_for(p, "confirm", |a| *a == want);
            if let Some(h) = h.as_mut() {
                h.prefix = "release";
            }
            v.push(h);
        }
        if w.pages > 1 {
            v.push(hint_for(p, "next page", |a| {
                matches!(a, Action::QueueModifier)
            }));
        }
        let back = if m.wheel_depth() > 0 {
            "back"
        } else {
            "cancel"
        };
        v.push(hint_for(p, back, |a| matches!(a, Action::Cancel)));
    } else if m.brush().is_some() {
        v.push(Some(Hint {
            key: HintKey::Stick(true),
            prefix: "",
            text: "paint".into(),
        }));
        v.push(
            hint_for(p, "select", |a| matches!(a, Action::Brush)).map(|mut h| {
                h.prefix = "release";
                h
            }),
        );
    } else if m.mode() != Mode::None {
        let what = match m.mode() {
            Mode::Build(_) => "place",
            Mode::Patrol => "patrol here",
            Mode::Guard => "guard",
            Mode::Heal => "heal / repair",
            Mode::CastSpell(_) => "cast",
            Mode::Attack => "attack",
            Mode::AttackMove => "attack-move here",
            Mode::Move => "move here",
            Mode::None => "",
        };
        v.push(hint_for(p, what, |a| {
            matches!(a, Action::Context { append: false })
        }));
        v.push(hint_for(p, "+ queue", |a| {
            matches!(a, Action::QueueModifier)
        }));
        v.push(hint_for(p, "cancel", |a| matches!(a, Action::Cancel)));
    } else if cx.has_units {
        v.push(hint_for(p, "move / attack", |a| {
            matches!(a, Action::Context { append: false })
        }));
        v.push(hint_for(p, "attack-move", |a| {
            matches!(a, Action::AttackMove | Action::AttackMoveMode)
        }));
        v.push(hint_for(p, "orders", |a| matches!(a, Action::CommandWheel)));
        if cx.has_builder {
            v.push(hint_for(p, "build", |a| matches!(a, Action::BuildWheel)));
        }
        if cx.has_caster {
            v.push(hint_for(p, "spell", |a| matches!(a, Action::CastSpellMode)));
        }
        if cx.mixed {
            v.push(hint_for(p, "cycle type", |a| {
                matches!(a, Action::SubSelectCycle)
            }));
        }
        v.push(hint_for(p, "same type", |a| {
            matches!(
                a,
                Action::SelectSameType {
                    scope: Scope::OnScreen
                }
            )
        }));
        v.push(hint_for(p, "deselect", |a| matches!(a, Action::Cancel)));
    } else {
        v.push(hint_for(p, "select", |a| matches!(a, Action::Select)));
        v.push(hint_for(p, "brush", |a| matches!(a, Action::Brush)));
        v.push(hint_for(p, "all on screen", |a| {
            matches!(
                a,
                Action::SelectAll {
                    scope: Scope::OnScreen
                }
            )
        }));
        v.push(hint_for(p, "idle builder", |a| {
            matches!(a, Action::CycleIdleBuilder)
        }));
        v.push(hint_for(p, "squads 5-9", |a| {
            matches!(a, Action::CommandWheel)
        }));
        v.push(hint_for(p, "alert", |a| matches!(a, Action::JumpToAlert)));
        v.push(hint_for(p, "menu", |a| matches!(a, Action::ToggleOverlay)));
    }
    v.into_iter().flatten().collect()
}

/// The keyboard key (with modifier) a profile binds to an action, e.g. `S`
/// or `Ctrl+F1`.
pub fn key_name_for(p: &Profile, want: impl Fn(&Action) -> bool) -> Option<String> {
    p.bindings.iter().find_map(|b| {
        if !want(&b.action) {
            return None;
        }
        let name = |i: &Input| match i {
            Input::Key(k) => {
                let mut s = k.0.clone();
                if let Some(c) = s.get_mut(..1) {
                    c.make_ascii_uppercase();
                }
                Some(s)
            }
            Input::Button(Button::MouseLeft) => Some("LMB".into()),
            Input::Button(Button::MouseRight) => Some("RMB".into()),
            Input::Button(_) => None,
        };
        let k = name(&b.input)?;
        Some(match b.with.as_ref().and_then(name) {
            Some(m) => format!("{m}+{k}"),
            None => k,
        })
    })
}

fn primary_mouse_context(m: &Mapper) -> bool {
    m.profile().left_click_context
        && m.press_action(&Input::Button(Button::MouseLeft)) == Some(Action::DragSelect)
}

/// Mouse/keyboard input that gives the default context order. Original-style
/// left clicks take priority only while their actual binding still selects
/// contextually; a remapped primary button must not advertise the wrong action.
pub fn mouse_order_name(m: &Mapper) -> String {
    if primary_mouse_context(m) {
        "LMB".into()
    } else {
        key_name_for(m.profile(), |a| {
            matches!(a, Action::Context { append: false })
        })
        .unwrap_or_else(|| "order".into())
    }
}

/// Classic TA:K prompts (keys and mouse) for the bottom bar.
pub fn hints_classic(m: &Mapper, cx: HintCx) -> Vec<Hint> {
    let p = m.profile();
    let mut v: Vec<(Option<String>, &str)> = Vec::new();
    if m.mode() != Mode::None {
        v.push((
            Some("LMB".into()),
            if m.mode() == Mode::Heal {
                "heal / repair"
            } else {
                "confirm"
            },
        ));
        v.push((
            key_name_for(p, |a| matches!(a, Action::QueueModifier)),
            "+ click: queue",
        ));
        v.push((key_name_for(p, |a| matches!(a, Action::Cancel)), "cancel"));
    } else if cx.has_units {
        v.push((
            Some(mouse_order_name(m)),
            if primary_mouse_context(m) {
                "select / move / attack"
            } else {
                "order"
            },
        ));
        v.push((
            key_name_for(p, |a| matches!(a, Action::AttackMode)),
            "attack",
        ));
        v.push((
            key_name_for(p, |a| {
                matches!(a, Action::AttackMove | Action::AttackMoveMode)
            }),
            "attack-move",
        ));
        v.push((key_name_for(p, |a| matches!(a, Action::MoveMode)), "move"));
        v.push((key_name_for(p, |a| matches!(a, Action::GuardMode)), "guard"));
        if cx.has_healer {
            v.push((
                key_name_for(p, |a| matches!(a, Action::HealMode)),
                "heal / repair",
            ));
        }
        v.push((
            key_name_for(p, |a| matches!(a, Action::PatrolMode)),
            "patrol",
        ));
        v.push((key_name_for(p, |a| matches!(a, Action::Stop)), "stop"));
        v.push((
            key_name_for(p, |a| matches!(a, Action::AssignGroup { .. }))
                .map(|k| k.trim_end_matches(|c: char| c.is_ascii_digit()).to_string() + "1-9"),
            "squad",
        ));
    } else {
        v.push((Some("LMB".into()), "select / drag"));
        v.push((Some("2x LMB".into()), "same type"));
        v.push((
            key_name_for(p, |a| {
                matches!(
                    a,
                    Action::SelectAll {
                        scope: Scope::OnScreen
                    }
                )
            }),
            "all on screen",
        ));
        v.push((
            key_name_for(p, |a| matches!(a, Action::SelectMonarch { track: true })),
            "monarch",
        ));
        v.push((
            key_name_for(p, |a| matches!(a, Action::ToggleCrystalBall)),
            "Crystal Ball",
        ));
    }
    v.into_iter()
        .filter_map(|(k, t)| {
            Some(Hint {
                key: HintKey::Text(k?),
                prefix: "",
                text: t.to_owned(),
            })
        })
        .collect()
}

/// A short label for a wheel slot (build options are named by the caller).
pub fn action_label(a: &Action) -> String {
    match a {
        Action::Stop => "Stop".into(),
        Action::PatrolMode => "Patrol".into(),
        Action::GuardMode => "Guard".into(),
        Action::HealMode => "Heal / repair".into(),
        Action::CastSpellMode => "Cast spell".into(),
        Action::CastSpell { index } => format!("Spell {}", index + 1),
        Action::SetFireState { state } => crate::hud::stance_name(*state).into(),
        Action::CycleIdleBuilder => "Idle builder".into(),
        Action::JumpToAlert => "Last alert".into(),
        Action::CenterOnMonarch => "Monarch".into(),
        Action::SelectAll {
            scope: Scope::OnScreen,
        } => "All on screen".into(),
        Action::SelectAll { scope: Scope::Map } => "All units".into(),
        Action::SelectSameType {
            scope: Scope::OnScreen,
        } => "Same type".into(),
        Action::SelectSameType { scope: Scope::Map } => "Same type (map)".into(),
        Action::FollowSelection => "Track".into(),
        Action::BuildOption { index } => format!("Option {}", index + 1),
        Action::AttackMode => "Attack".into(),
        Action::AttackMove | Action::AttackMoveMode => "Attack-move".into(),
        Action::MoveMode => "Move".into(),
        Action::RecallGroup { group } => format!("Squad {}", squad_number(group.get())),
        Action::AssignGroup { group } => format!("Set squad {}", squad_number(group.get())),
        Action::JumpToGroup { group } => format!("Go to squad {}", squad_number(group.get())),
        Action::OpenRing { ring } => match ring {
            tak_input::Ring::Stance => "Stance >".into(),
            tak_input::Ring::Unit => "Unit >".into(),
            tak_input::Ring::Squads => "Squads >".into(),
        },
        Action::Reserved { order } => order.label().into(),
        Action::SubSelectCycle => "Cycle type".into(),
        Action::SelectMonarch { track: false } => "Select monarch".into(),
        Action::SelectMonarch { track: true } => "Track monarch".into(),
        Action::ResetZoom => "Reset zoom".into(),
        Action::CrystalBall | Action::ToggleCrystalBall => "Crystal Ball".into(),
        Action::SelectCategory { category } => format!("All {:?}", category),
        Action::SetBookmark { slot } => format!("Set bookmark {slot}"),
        Action::GoToBookmark { slot } => format!("Bookmark {slot}"),
        Action::NextUnit => "Next unit".into(),
        Action::ToggleDamageBars => "Damage bars".into(),
        Action::GameSpeed { faster: true } => "Faster".into(),
        Action::GameSpeed { faster: false } => "Slower".into(),
        Action::TogglePause => "Pause".into(),
        other => format!("{other:?}"),
    }
}

/// TA numbers squads 1..10; squad 10 is group 0 (the `0` key).
pub fn squad_number(group: u8) -> u8 {
    if group == 0 { 10 } else { group }
}

// ---- recordings -------------------------------------------------------------

/// Match metadata stored in a session recording: enough to replay it.
pub fn session_meta(s: &Session, players: &str, seed: u64) -> serde_json::Value {
    let log: serde_json::Value =
        serde_json::from_str(&s.log.to_json()).unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "client": concat!("tak-client ", env!("CARGO_PKG_VERSION")),
        "map": s.map_name,
        "players": players,
        "seed": seed,
        "ticks": s.tick().0,
        "final_hash": s.last_hash().map(|h| format!("{h:016x}")),
        "log": log,
    })
}

/// What a recording needs to replay its match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhostMatch {
    /// Map path.
    pub map: String,
    /// Sides.
    pub players: String,
    /// World seed.
    pub seed: u64,
    /// Every order of the session.
    pub log: CommandLog,
    /// Ticks the session ran.
    pub ticks: u32,
    /// Hash after the last tick, hex.
    pub final_hash: Option<String>,
}

/// Read the match out of a recording's metadata.
pub fn ghost_match(r: &Recording) -> Result<GhostMatch, String> {
    let m = &r.meta;
    let s = |k: &str| {
        m.get(k)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .ok_or_else(|| format!("recording has no {k:?}"))
    };
    let log = m.get("log").ok_or("recording has no command log")?;
    let log = CommandLog::from_json(&log.to_string()).map_err(|e| format!("recorded log: {e}"))?;
    Ok(GhostMatch {
        map: s("map")?,
        players: s("players")?,
        seed: m.get("seed").and_then(|v| v.as_u64()).unwrap_or(1),
        log,
        ticks: m.get("ticks").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        final_hash: m
            .get("final_hash")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
    })
}

/// The starting camera for scripted sessions: the local monarch's start at
/// the screen centre.
pub fn start_geom(s: &Session, zoom: f32, size: [f32; 2]) -> ViewGeom {
    let w = s.sim.world();
    let p = w
        .players()
        .get(usize::from(s.local.0))
        .map_or(tak_math::Vec2::ZERO, |p| {
            p.monarch
                .and_then(|id| w.unit(id))
                .map_or(p.start, |u| u.pos)
        });
    let h = f(w.map().height_at(p));
    ViewGeom {
        target: [f(p.x), f(p.y) - crate::view::SHEAR * h],
        zoom,
        size,
    }
}

/// Terrain height of the ground point drawn at the screen centre (the
/// oblique view shows ground `z` at screen `z - SHEAR * h`), from the sim's
/// height map; a few fixed-point steps of `z = target + SHEAR * h(z)`.
pub fn centre_height(s: &Session, geom: &ViewGeom) -> f32 {
    let w = s.sim.world();
    let fx = |v: f32| tak_math::Fx::from_raw((v * 65536.0) as i32);
    let mut h = 0.0;
    for _ in 0..4 {
        let z = geom.target[1] + crate::view::SHEAR * h;
        h = f(w
            .map()
            .height_at(tak_math::Vec2::new(fx(geom.target[0]), fx(z))));
    }
    h
}

/// Drive a session headless with scripted device events through the real
/// mapper (30 ticks per second, one input frame per tick), submitting its
/// orders to the sim exactly as the client does, and record everything.
/// Event times are milliseconds from the start.
pub fn synth_recording(
    s: &mut Session,
    mapper: &mut Mapper,
    events: &[InputEvent],
    ticks: u32,
    players: &str,
    seed: u64,
) -> Recording {
    // Tick 0 spawns the units.
    if s.tick().0 == 0 {
        s.step();
    }
    let mut geom = start_geom(s, 0.5, [1280.0, 720.0]);
    let mut rec = Recorder::new(serde_json::Value::Null, mapper);
    let mut next = 0;
    let t_base = 1000u32;
    while s.tick().0 < ticks {
        let tick = s.tick();
        let t = t_base + tick.0 * 1000 / 30;
        mapper.set_camera(geom.input_camera_at(centre_height(s, &geom)));
        let mut kinds = Vec::new();
        let mut outs = Vec::new();
        let mut submitted = Vec::new();
        {
            let view = s.view();
            while next < events.len() && t_base + events[next].t <= t {
                let e = &events[next];
                kinds.push(e.kind.clone());
                outs.extend(mapper.handle(
                    &InputEvent::new(t_base + e.t, e.kind.clone()),
                    &view,
                    tick,
                ));
                next += 1;
            }
            outs.extend(mapper.handle(&InputEvent::new(t, EventKind::Frame), &view, tick));
        }
        for o in &outs {
            match o {
                Output::Sim { command, .. } => {
                    let at = s.submit(command.clone());
                    submitted.push((at.0, command.clone()));
                }
                Output::Client {
                    command: tak_commands::ClientCommand::CenterOnMonarch,
                } => {
                    let w = s.sim.world();
                    if let Some(u) = w
                        .players()
                        .get(usize::from(s.local.0))
                        .and_then(|p| p.monarch)
                        .and_then(|id| w.unit(id))
                    {
                        mapper.glide_to(u.pos);
                    }
                }
                _ => {}
            }
        }
        let pan = mapper.take_camera_pan();
        geom.target[0] += f(pan.x);
        geom.target[1] += f(pan.y);
        if let Some(z) = mapper.take_camera_zoom() {
            geom.zoom = f(z);
        }
        let fb = mapper.take_feedback();
        rec.frame(
            mapper,
            t,
            tick.0,
            FrameIo {
                events: &kinds,
                outputs: &outs,
                submitted: &submitted,
                feedback: &fb,
            },
        );
        s.step();
    }
    rec.set_meta(session_meta(s, players, seed));
    rec.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_order_prompts_match_original_default_and_remapped_bindings() {
        let original = Mapper::new(Profile::named("original").unwrap());
        let classic = Mapper::new(Profile::default_combined());
        let cx = HintCx {
            has_units: true,
            ..Default::default()
        };
        assert_eq!(mouse_order_name(&original), "LMB");
        assert_eq!(mouse_order_name(&classic), "RMB");
        assert_eq!(
            hints_classic(&original, cx)[0],
            Hint {
                key: HintKey::Text("LMB".into()),
                prefix: "",
                text: "select / move / attack".into(),
            }
        );
        assert_eq!(
            hints_classic(&classic, cx)[0],
            Hint {
                key: HintKey::Text("RMB".into()),
                prefix: "",
                text: "order".into(),
            }
        );
        assert_eq!(hints(&original, cx), hints(&classic, cx));
        assert_eq!(
            hints_classic(&original, HintCx::default()),
            hints_classic(&classic, HintCx::default())
        );

        let mut remapped = Profile::original_tak();
        for binding in &mut remapped.bindings {
            if binding.action == Action::DragSelect {
                binding.action = Action::Select;
            }
            if matches!(binding.action, Action::Context { append: false }) {
                binding.input = Input::Key(tak_input::Key::new("q"));
            }
        }
        let remapped = Mapper::new(remapped);
        assert_eq!(mouse_order_name(&remapped), "Q");
        assert_eq!(hints_classic(&remapped, cx)[0].text, "order");
    }

    #[test]
    fn attack_move_is_distinct_from_force_attack_in_both_hint_sets() {
        let m = Mapper::new(Profile::default_combined());
        let cx = HintCx {
            has_units: true,
            ..Default::default()
        };
        let pad = hints(&m, cx);
        assert!(pad.iter().any(|h| h.text == "attack-move"
            && h.prefix == "hold"
            && h.key == HintKey::Buttons(vec![Button::X])));
        let classic = hints_classic(&m, cx);
        assert!(
            classic
                .iter()
                .any(|h| h.text == "attack-move" && h.key == HintKey::Text("Alt+A".into()))
        );
        assert!(
            classic
                .iter()
                .any(|h| h.text == "attack" && h.key == HintKey::Text("A".into()))
        );
        assert_eq!(action_label(&Action::AttackMove), "Attack-move");
        assert_eq!(action_label(&Action::AttackMoveMode), "Attack-move");
        assert_eq!(action_label(&Action::AttackMode), "Attack");
    }

    #[test]
    fn layouts_detect_from_name_and_vendor() {
        assert_eq!(
            detect_layout("auto", Some("Xbox Wireless Controller"), Some(0x045e)),
            Layout::Xbox
        );
        assert_eq!(
            detect_layout("auto", Some("DualSense Wireless Controller"), None),
            Layout::PlayStation
        );
        assert_eq!(
            detect_layout("auto", Some("whatever"), Some(0x054c)),
            Layout::PlayStation
        );
        assert_eq!(detect_layout("ps", None, None), Layout::PlayStation);
        assert_eq!(
            detect_layout("xbox", Some("DualSense"), Some(0x054c)),
            Layout::Xbox
        );
        assert_eq!(
            glyph(Layout::PlayStation, Button::A).unwrap().symbol,
            Some(Symbol::Cross)
        );
        assert_eq!(glyph(Layout::Xbox, Button::Lt).unwrap().text, "LT");
        assert!(glyph(Layout::Xbox, Button::MouseLeft).is_none());
    }

    #[test]
    fn hints_come_from_bindings() {
        let m = Mapper::new(preset_profile("console").unwrap());
        let h = hints(&m, HintCx::default());
        assert_eq!(h[0].key, HintKey::Buttons(vec![Button::A]));
        assert_eq!(h[0].text, "select");
        assert!(h.iter().any(|h| h.prefix == "hold" && h.text == "brush"));
        let h = hints(
            &m,
            HintCx {
                has_units: true,
                has_builder: true,
                ..HintCx::default()
            },
        );
        assert!(
            h.iter()
                .any(|h| h.key == HintKey::Buttons(vec![Button::Lb]) && h.text == "build")
        );
        let h = hints_classic(
            &m,
            HintCx {
                has_units: true,
                ..HintCx::default()
            },
        );
        assert!(
            h.iter()
                .any(|h| h.key == HintKey::Text("S".into()) && h.text == "stop"),
            "{h:?}"
        );
        assert!(
            h.iter()
                .any(|h| h.key == HintKey::Text("Ctrl+1-9".into()) && h.text == "squad"),
            "{h:?}"
        );
    }

    #[test]
    fn plain_pad_prompts_follow_layout_rebinding_and_chords() {
        let mut p = Profile::kingdoms_console();
        let context = |a: &Action| matches!(a, Action::Context { append: false });
        assert_eq!(
            pad_action_name(&p, Layout::Xbox, context).as_deref(),
            Some("X")
        );
        assert_eq!(
            pad_action_name(&p, Layout::PlayStation, context).as_deref(),
            Some("Square")
        );
        for b in &mut p.bindings {
            if context(&b.action) {
                b.input = Input::Button(Button::Y);
                b.with = Some(Input::Button(Button::Lb));
            }
        }
        assert_eq!(
            pad_action_name(&p, Layout::Xbox, context).as_deref(),
            Some("LB+Y")
        );
        assert_eq!(
            pad_action_name(&p, Layout::PlayStation, context).as_deref(),
            Some("L1+Triangle")
        );
        p.bindings.retain(|b| !context(&b.action));
        assert_eq!(pad_action_name(&p, Layout::Xbox, context), None);
    }

    #[test]
    fn profile_file_round_trip_and_hot_reload() {
        let dir = std::env::temp_dir().join(format!("tak-feel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.json");
        let _ = std::fs::remove_file(&path);
        let (p, mut t) = initial_profile("kingdoms", "assisted", Some(&path)).unwrap();
        assert_eq!(p.tuning, Tuning::assisted());
        assert!(path.exists());
        assert!(poll_profile(&path, &mut t).is_none());
        let mut q = p.clone();
        q.tuning.cursor_speed = 1234;
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, q.to_json_pretty()).unwrap();
        // Force a different mtime even on coarse clocks.
        t = Some(SystemTime::UNIX_EPOCH);
        let got = poll_profile(&path, &mut t).unwrap().unwrap();
        assert_eq!(got.tuning.cursor_speed, 1234);
        std::fs::write(&path, "{ nope").unwrap();
        t = Some(SystemTime::UNIX_EPOCH);
        assert!(poll_profile(&path, &mut t).unwrap().is_err());
        assert!(initial_profile("kingdoms", "console", Some(&path)).is_err());
        assert!(preset_profile("bogus").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
