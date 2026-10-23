//! Import TA:K's `Keys.TDF` (the user-editable key bindings in the install
//! root) into a profile's keyboard bindings, so a veteran's own keys carry
//! over.
//!
//! The file is TDF: one `[CUSTOMKEYS]` section of `KEY = Command args;`
//! lines. Key names are `LOWER_A`, `UPPER_A` (Shift), `CTRL_A`,
//! `CTRLSHIFT_A`, `ALT_1`, `ALTSHIFT_1`, `0`..`9`, `F1`..`F12` with
//! `SHIFT_` / `CTRL_` / `CTRLSHIFT_` prefixes, `PAUSE`, `TAB`, `BACKSPACE`,
//! `SYMBOL_xx` (ASCII code in hex) and `XCHAR_xx` (non-ASCII layout keys).
//! An empty value unbinds the key.
//!
//! What is supported is mapped to [`Action`]s; orders the sim lacks map to
//! [`Action::Reserved`] (bound, inert, hidden); the rest is reported. Our
//! chords take one modifier, so `CTRLSHIFT_` and `ALTSHIFT_` lines are
//! reported as skipped: TA's Ctrl+Shift lines are the additive versions of
//! the Ctrl lines, which our select actions already do with Shift held.

use tak_commands::{ControlGroup, Scope};

use crate::binding::{Action, Binding, Category, Gesture, Profile, ReservedOrder};
use crate::event::{Input, Key};

/// How one `Keys.TDF` line was taken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineStatus {
    /// Bound to a supported action.
    Mapped,
    /// Bound to a reserved order (the sim does not support it yet).
    Reserved,
    /// Explicitly unbound (`KEY =;`).
    Unbound,
    /// The command has no counterpart here (menus, chat, debug, clipboard).
    Unsupported,
    /// The key cannot be expressed (two modifiers, or a layout alias).
    Skipped,
}

/// One line of the file and what became of it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeyLine {
    /// The TDF key, e.g. `CTRL_B`.
    pub key: String,
    /// The command as written, e.g. `SelectUnits BUILDER`.
    pub command: String,
    /// Outcome.
    pub status: LineStatus,
    /// Why, for skipped and unsupported lines.
    pub note: &'static str,
}

/// The result of reading a `Keys.TDF`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct KeysImport {
    /// Bindings for the mapped and reserved lines (first line wins for a
    /// trigger).
    pub bindings: Vec<Binding>,
    /// Triggers the file explicitly unbinds.
    pub unbound: Vec<(Input, Option<Input>)>,
    /// Every line, in file order.
    pub lines: Vec<KeyLine>,
}

impl KeysImport {
    /// Lines with a status.
    pub fn count(&self, s: LineStatus) -> usize {
        self.lines.iter().filter(|l| l.status == s).count()
    }

    /// One-line summary, e.g. for the client's log.
    pub fn summary(&self) -> String {
        format!(
            "{} mapped, {} reserved (sim lacks), {} unbound, {} unsupported, {} skipped",
            self.count(LineStatus::Mapped),
            self.count(LineStatus::Reserved),
            self.count(LineStatus::Unbound),
            self.count(LineStatus::Unsupported),
            self.count(LineStatus::Skipped),
        )
    }
}

/// Why a file could not be read at all.
#[derive(Debug, thiserror::Error)]
pub enum KeysError {
    /// Not TDF.
    #[error("Keys.TDF: {0}")]
    Tdf(#[from] tak_formats::tdf::TdfError),
    /// No `[CUSTOMKEYS]` section.
    #[error("Keys.TDF has no [CUSTOMKEYS] section")]
    NoSection,
}

fn key(name: &str) -> Input {
    Input::Key(Key::new(name))
}

/// The key and modifier for a TDF key name, or why not.
fn trigger(name: &str) -> Result<(Input, Option<Input>), &'static str> {
    let up = name.trim().to_ascii_uppercase();
    for two in ["CTRLSHIFT_", "ALTSHIFT_"] {
        if up.starts_with(two) {
            return Err("two modifiers (Shift with the Ctrl or Alt binding adds to the selection)");
        }
    }
    let (with, rest) = if let Some(r) = up.strip_prefix("LOWER_") {
        (None, r)
    } else if let Some(r) = up.strip_prefix("UPPER_") {
        (Some(key("shift")), r)
    } else if let Some(r) = up.strip_prefix("CTRL_") {
        (Some(key("ctrl")), r)
    } else if let Some(r) = up.strip_prefix("ALT_") {
        (Some(key("alt")), r)
    } else if let Some(r) = up.strip_prefix("SHIFT_") {
        (Some(key("shift")), r)
    } else {
        (None, up.as_str())
    };
    let base = match rest {
        r if r.len() == 1 && r.chars().all(|c| c.is_ascii_alphanumeric()) => r.to_ascii_lowercase(),
        r if r.starts_with('F')
            && r.len() <= 3
            && r[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) =>
        {
            r.to_ascii_lowercase()
        }
        "PAUSE" => "pause".into(),
        "TAB" => "tab".into(),
        "BACKSPACE" => "backspace".into(),
        r if r.starts_with("SYMBOL_") => {
            let (k, shifted) = symbol(&r[7..]).ok_or("a symbol another keyboard layout types")?;
            if shifted {
                if with.is_some() {
                    return Err("two modifiers");
                }
                return Ok((key(k), Some(key("shift"))));
            }
            k.into()
        }
        r if r.starts_with("XCHAR_") => return Err("a key of a non-English layout"),
        _ => return Err("unknown key name"),
    };
    Ok((key(&base), with))
}

/// US-layout key for an ASCII symbol (hex), and whether Shift types it.
/// Symbols the file lists "for French / German / Italian keyboards" (that
/// need Shift on a US layout and duplicate another line) are not mapped.
fn symbol(hex: &str) -> Option<(&'static str, bool)> {
    Some(match u8::from_str_radix(hex, 16).ok()? {
        b'=' => ("equal", false),
        b'+' => ("equal", true),
        b'-' => ("minus", false),
        b'_' => ("minus", true),
        b'`' => ("grave", false),
        b'~' => ("grave", true),
        b',' => ("comma", false),
        b'.' => ("period", false),
        b'/' => ("slash", false),
        b';' => ("semicolon", false),
        b'\'' => ("quote", false),
        b'[' => ("bracketleft", false),
        b']' => ("bracketright", false),
        b'\\' => ("backslash", false),
        _ => return None,
    })
}

/// What a command maps to.
enum Cmd {
    Do(Action),
    Unsupported(&'static str),
}

fn group(n: &str) -> Option<ControlGroup> {
    // TA numbers squads 1..10; squad 10 is the `0` key (group 0).
    let n: u8 = n.trim().parse().ok()?;
    ControlGroup::new(n % 10)
}

fn command(value: &str) -> Cmd {
    let all: Vec<String> = value
        .split(',')
        .map(|c| c.trim().to_ascii_lowercase())
        .collect();
    let first = all.first().cloned().unwrap_or_default();
    let mut words = first.split_whitespace();
    let verb = words.next().unwrap_or("");
    let args: Vec<&str> = words.collect();
    let arg = args.first().copied().unwrap_or("");
    let reserved = |order| Cmd::Do(Action::Reserved { order });
    match verb {
        "unitcommand" => match arg {
            "attack" => Cmd::Do(Action::AttackMode),
            "attackmove" => Cmd::Do(Action::AttackMoveMode),
            "move" => Cmd::Do(Action::MoveMode),
            "guard" => Cmd::Do(Action::GuardMode),
            "patrol" => Cmd::Do(Action::PatrolMode),
            "stop" => Cmd::Do(Action::Stop),
            "clear" => reserved(ReservedOrder::Clear),
            "heal" | "repair" => reserved(ReservedOrder::Heal),
            "togglecloak" => reserved(ReservedOrder::Cloak),
            "load" => reserved(ReservedOrder::Load),
            "unload" => reserved(ReservedOrder::Unload),
            "togglegateopen" | "toggleactivate" => reserved(ReservedOrder::Activate),
            // TA:K "select weapon 1..3": spells 1..3 of the caster
            // (Chosen, as the WP-T keys).
            "primaryweapon" => Cmd::Do(Action::CastSpell { index: 0 }),
            "secondaryweapon" => Cmd::Do(Action::CastSpell { index: 1 }),
            "specialweapon" => Cmd::Do(Action::CastSpell { index: 2 }),
            _ => Cmd::Unsupported("unknown unit command"),
        },
        "selectallunits" => Cmd::Do(Action::SelectAll { scope: Scope::Map }),
        "selectunitsonscreen" => Cmd::Do(Action::SelectAll {
            scope: Scope::OnScreen,
        }),
        "selectallunitsselectedtype" => Cmd::Do(Action::SelectSameType { scope: Scope::Map }),
        "selectunits" | "selectunitsadd" => {
            let track = all.iter().any(|c| c == "trackunit");
            match Category::from_tdf(arg) {
                Some(Category::Monarch) => Cmd::Do(Action::SelectMonarch { track }),
                Some(category) => Cmd::Do(Action::SelectCategory { category }),
                None => Cmd::Unsupported("unknown unit category"),
            }
        }
        "retrievesquad" => match group(arg) {
            Some(group) => Cmd::Do(Action::RecallGroup { group }),
            None => Cmd::Unsupported("bad squad number"),
        },
        "createsquad" if args.get(1).is_some_and(|a| *a == "formation") => {
            reserved(ReservedOrder::Formation)
        }
        "createsquad" => match group(arg) {
            Some(group) => Cmd::Do(Action::AssignGroup { group }),
            None => Cmd::Unsupported("bad squad number"),
        },
        "retrievesquadadd" => Cmd::Unsupported("add a squad to the selection"),
        "movetonextunit" => Cmd::Do(Action::NextUnit),
        "trackunit" | "trackunitreverse" => Cmd::Do(Action::FollowSelection),
        "movetolastreport" => Cmd::Do(Action::JumpToAlert),
        "fsr" => Cmd::Do(Action::ToggleCrystalBall),
        "togglepause" => Cmd::Do(Action::TogglePause),
        "increasespeed" => Cmd::Do(Action::GameSpeed { faster: true }),
        "decreasespeed" => Cmd::Do(Action::GameSpeed { faster: false }),
        "toggledamagebars" => Cmd::Do(Action::ToggleDamageBars),
        "mapbookmarkretrieve" => match arg.parse::<u8>() {
            Ok(slot) => Cmd::Do(Action::GoToBookmark { slot }),
            Err(_) => Cmd::Unsupported("bad bookmark number"),
        },
        "mapbookmarkstore" => match arg.parse::<u8>() {
            Ok(slot) => Cmd::Do(Action::SetBookmark { slot }),
            Err(_) => Cmd::Unsupported("bad bookmark number"),
        },
        "f2menu" => Cmd::Do(Action::ToggleOverlay),
        "selfdestruct" => reserved(ReservedOrder::Dismiss),
        "diplomacymenu" => Cmd::Unsupported("no diplomacy screen yet"),
        "briefing" => Cmd::Unsupported("no mission briefing yet"),
        "netscorecard" => Cmd::Unsupported("no leaderboard yet"),
        "loadgame" | "savegame" => Cmd::Unsupported("no saved games yet"),
        "clearchat" => Cmd::Unsupported("no chat yet"),
        "copyunittoclipboard" | "pasteunitfromclipboard" | "cutunittoclipboard" => {
            Cmd::Unsupported("unit clipboard (a debug feature)")
        }
        v if v.starts_with("debug") => Cmd::Unsupported("a debug command"),
        _ => Cmd::Unsupported("unknown command"),
    }
}

/// Read a `Keys.TDF`. Never panics; unknown lines are reported, not fatal.
pub fn parse_keys_tdf(data: &[u8]) -> Result<KeysImport, KeysError> {
    let doc = tak_formats::tdf::parse(data)?;
    let sec = doc
        .sections
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("customkeys"))
        .ok_or(KeysError::NoSection)?;
    let mut out = KeysImport::default();
    // Plain bindings first, so a Shift line identical to its plain one adds
    // nothing (Shift then just queues, as the plain line would).
    let mut plain: Vec<((Input, Option<Input>), Action)> = Vec::new();
    for e in &sec.entries {
        let value = e.value.trim().trim_end_matches(';').trim();
        let mut line = KeyLine {
            key: e.key.trim().to_owned(),
            command: value.to_owned(),
            status: LineStatus::Skipped,
            note: "",
        };
        let trig = match trigger(&e.key) {
            Ok(t) => t,
            Err(why) => {
                line.note = why;
                out.lines.push(line);
                continue;
            }
        };
        if value.is_empty() {
            line.status = LineStatus::Unbound;
            if !out.unbound.contains(&trig) {
                out.unbound.push(trig);
            }
            out.lines.push(line);
            continue;
        }
        match command(value) {
            Cmd::Unsupported(why) => {
                line.status = LineStatus::Unsupported;
                line.note = why;
            }
            Cmd::Do(action) => {
                line.status = if action.is_supported() {
                    LineStatus::Mapped
                } else {
                    LineStatus::Reserved
                };
                let taken = out
                    .bindings
                    .iter()
                    .any(|b| b.input == trig.0 && b.with == trig.1);
                if trig.1.is_none() {
                    plain.push((trig.clone(), action));
                }
                if !taken {
                    out.bindings.push(Binding {
                        input: trig.0.clone(),
                        with: trig.1.clone(),
                        gesture: Gesture::Press,
                        action,
                    });
                }
            }
        }
        out.lines.push(line);
    }
    out.bindings.retain(|b| {
        let shift = matches!(&b.with, Some(Input::Key(k)) if k.0 == "shift");
        !(shift
            && plain
                .iter()
                .any(|((i, w), a)| *i == b.input && w.is_none() && *a == b.action))
    });
    Ok(out)
}

impl Profile {
    /// This profile with an imported `Keys.TDF` applied: every key trigger
    /// the file binds or unbinds is replaced by the file's choice; keys the
    /// file leaves alone (or binds to something unsupported, like F11's
    /// debug toggle) keep the profile's binding. Mouse and pad bindings are
    /// untouched.
    pub fn with_keys_tdf(&self, import: &KeysImport) -> Profile {
        let mut p = self.clone();
        let replaced = |b: &Binding| {
            matches!(b.input, Input::Key(_))
                && (import
                    .bindings
                    .iter()
                    .any(|n| n.input == b.input && n.with == b.with)
                    || import
                        .unbound
                        .iter()
                        .any(|(i, w)| *i == b.input && *w == b.with))
        };
        p.bindings.retain(|b| !replaced(b));
        p.bindings.extend(import.bindings.iter().cloned());
        p
    }
}
