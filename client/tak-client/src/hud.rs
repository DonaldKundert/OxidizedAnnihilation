//! HUD logic that needs no Bevy (WP-V, design system section 6): which
//! race skin to use, where the order buttons sit on the skin's button
//! panel, the multi-selection strip's type groups, and the bottom bar's
//! verb line. Drawing is in `app.rs` / `hud_view.rs`.

use tak_commands::{UnitDefId, UnitId};
use tak_input::{Action, CursorKind};

/// Image order shared by the common GAF buttons and per-weapon pictures:
/// disabled, up, hovered, active.
pub const BUTTON_STATES: usize = 4;

/// Which button image to show. An unusable button cannot appear active.
pub fn button_state(usable: bool, active: bool, hovered: bool) -> usize {
    match (usable, active, hovered) {
        (false, _, _) => 0,
        (true, true, _) => 3,
        (true, false, true) => 2,
        _ => 1,
    }
}

/// Per-weapon FBI button pictures, in [`button_state`] order. The values
/// name JPEGs under `anims/weaponpic/` (**Known**, installed files). This
/// metadata is display-only and never enters the simulation.
pub fn weapon_button_paths(
    extra: &std::collections::BTreeMap<String, String>,
) -> [Option<String>; BUTTON_STATES] {
    [
        "buttonimagedisabled",
        "buttonimageup",
        "buttonimageselected",
        "buttonimagedown",
    ]
    .map(|key| {
        let name = extra.get(key)?.trim().to_ascii_lowercase();
        if name.is_empty() {
            return None;
        }
        let stem = name.strip_suffix(".jpg").unwrap_or(&name);
        Some(format!("anims/weaponpic/{stem}.jpg"))
    })
}

/// A missing state falls back to the normal picture, then any decoded
/// picture. With no pictures at all the caller keeps its plain plate.
pub fn button_image<T>(images: &[Option<T>; BUTTON_STATES], state: usize) -> Option<&T> {
    images
        .get(state)
        .and_then(Option::as_ref)
        .or(images[1].as_ref())
        .or_else(|| images.iter().find_map(Option::as_ref))
}

/// The in-game skin GAF (`anims/<name>.gaf`, palette `anims/<name>.pcx`)
/// for a side, from the monarch's unit name prefix (**Known**: the five
/// files exist; Veruna's entries carry a `Veruna`/`V` prefix, so entries
/// are taken by index: 0 ButtonPanel, 1 InfoPanel, 2 EndCap, 3 UnitInfo,
/// 4 UnitPic).
pub fn race_skin(monarch_unitname: &str) -> &'static str {
    let up = monarch_unitname.to_ascii_uppercase();
    match up.get(..3).unwrap_or("") {
        "VER" => "verunaig",
        "TAR" => "tarosig",
        "ZON" => "zhonig",
        "CRE" => "creonig",
        _ => "aramonig",
    }
}

/// Skin GAF entry indices.
pub const SKIN_BUTTON_PANEL: usize = 0;
/// See [`SKIN_BUTTON_PANEL`].
pub const SKIN_INFO_PANEL: usize = 1;
/// See [`SKIN_BUTTON_PANEL`].
pub const SKIN_END_CAP: usize = 2;

/// Button slots on Aramon's button panel (frame 1), `[x, y, w, h]` of the
/// dark wells, when detection fails (**Known** for `aramonig.gaf`).
pub const ARAMON_SLOTS: [[u32; 4]; 13] = [
    [18, 26, 27, 27],
    [88, 26, 27, 27],
    [18, 61, 27, 27],
    [88, 61, 27, 27],
    [18, 96, 27, 27],
    [88, 96, 27, 27],
    [53, 131, 27, 27],
    [18, 181, 27, 27],
    [53, 181, 27, 27],
    [88, 181, 27, 27],
    [18, 222, 27, 27],
    [53, 222, 27, 27],
    [88, 222, 27, 27],
];

/// The dark button wells on a skin's button panel (RGBA8, `w`×`h`): every
/// connected region of near-black opaque pixels whose bounding box is
/// 22..=34 px on both sides and at least a fifth dark, sorted top to bottom, then
/// left to right. Each race paints them a few pixels apart, so the HUD
/// finds them rather than assuming Aramon's.
pub fn button_slots(rgba: &[u8], w: u32, h: u32) -> Vec<[u32; 4]> {
    let (w, h) = (w as usize, h as usize);
    if rgba.len() < w * h * 4 {
        return Vec::new();
    }
    let dark = |i: usize| {
        let p = &rgba[i * 4..i * 4 + 4];
        p[3] > 0 && u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) < 60
    };
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if seen[start] || !dark(start) {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (w, h, 0, 0, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            n += 1;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            let mut push = |j: usize| {
                if !seen[j] && dark(j) {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        let (bw, bh) = (x1 + 1 - x0, y1 + 1 - y0);
        // The wells have a lit bevel inside: about a third of the box is
        // near-black.
        if (22..=34).contains(&bw) && (22..=34).contains(&bh) && n * 10 >= bw * bh * 2 {
            out.push([x0 as u32, y0 as u32, bw as u32, bh as u32]);
        }
    }
    // Rows: wells within 8 px vertically are one row.
    out.sort_by_key(|s| (s[1] / 8, s[0]));
    out
}

/// What each of the panel's 13 wells holds, in [`button_slots`] order:
/// three rows of two orders and a centre one, the three weapon (spell)
/// buttons, then the three stances. Entries are `igcommonbuttons.gaf`
/// sequences. **Known** row order: the owned manual, printed pp. 70–71,
/// puts weapons above the bottom stance trio. Order/utility placement
/// otherwise remains **Chosen** (the GUI creates these buttons in code).
pub const PANEL_BUTTONS: [&str; 13] = [
    "AttackButton",
    "MoveButton",
    "GuardButton",
    "PatrolButton",
    "StopButton",
    "ClearButton",
    "RepairButton",
    "WPrimaryButton",
    "WSecondaryButton",
    "WSpecialButton",
    "AgOffensiveButton",
    "AgDefensiveButton",
    "AgPassiveButton",
];

/// What each panel button does, in [`PANEL_BUTTONS`] order. Clear remains
/// reserved; Heal keeps the original RepairButton well and art.
pub const PANEL_ACTIONS: [Action; 13] = [
    Action::AttackMode,
    Action::MoveMode,
    Action::GuardMode,
    Action::PatrolMode,
    Action::Stop,
    Action::Reserved {
        order: tak_input::ReservedOrder::Clear,
    },
    Action::HealMode,
    Action::CastSpell { index: 0 },
    Action::CastSpell { index: 1 },
    Action::CastSpell { index: 2 },
    Action::SetFireState {
        state: tak_commands::FireState::FireAtWill,
    },
    Action::SetFireState {
        state: tak_commands::FireState::Defensive,
    },
    Action::SetFireState {
        state: tak_commands::FireState::HoldFire,
    },
];

/// TA:K names for the panel buttons (help line).
pub const PANEL_NAMES: [&str; 13] = [
    "Attack",
    "Move",
    "Guard",
    "Patrol",
    "Stop",
    "Clear",
    "Heal",
    "Primary",
    "Secondary",
    "Special",
    "Offensive",
    "Defensive",
    "Passive",
];

/// `anims/CURSORS.GAF` sequence for each context (**Chosen** mapping; the
/// names say what each is for).
pub fn cursor_seq(kind: CursorKind) -> &'static str {
    match kind {
        CursorKind::Normal => "cursornormal",
        CursorKind::Select => "cursorselect",
        CursorKind::Move => "CursorMove",
        CursorKind::Attack => "CursorAttack",
        CursorKind::Guard => "CursorDefend",
        CursorKind::Heal => "CursorRepair",
        CursorKind::Patrol => "CursorPatrol",
        CursorKind::Build => "cursorgrn",
        CursorKind::BuildBlocked => "cursorred",
        CursorKind::Cast => "Cursorresurrect",
        CursorKind::Invalid => "cursortoofar",
    }
}

/// All cursor contexts to preload, including explicit Heal.
pub const CURSOR_KINDS: [CursorKind; 11] = [
    CursorKind::Normal,
    CursorKind::Select,
    CursorKind::Move,
    CursorKind::Attack,
    CursorKind::Guard,
    CursorKind::Heal,
    CursorKind::Patrol,
    CursorKind::Build,
    CursorKind::BuildBlocked,
    CursorKind::Cast,
    CursorKind::Invalid,
];

/// The acknowledgement for an order (**Chosen** for orders without their
/// own sound: build walks like a move, a spell attacks, Heal assists like
/// Guard, a stance has none).
pub fn order_sound(c: &tak_commands::SimCommand) -> Option<&'static str> {
    use tak_commands::SimCommand as C;
    Some(match c {
        C::Move { .. } | C::Build { .. } => "MOVE",
        C::Attack { .. } | C::AttackMove { .. } | C::CastSpell { .. } => "ATTACK",
        C::Guard { .. } | C::Heal { .. } => "GUARD",
        C::Patrol { .. } => "PATROL",
        C::Stop { .. } => "STOP",
        C::SetFireState { .. } => return None,
    })
}

/// The visible goal of an order. A stale unit target has no goal; never
/// substitute a slot's new generation or an unrelated position.
pub fn order_goal(w: &tak_sim::World, order: &tak_sim::Order) -> Option<tak_commands::WorldPos> {
    use tak_sim::Order;
    match order {
        Order::Move { to } | Order::AttackMove { to } | Order::Patrol { to } => Some(*to),
        Order::Build { at, .. } => *at,
        Order::Attack {
            target: tak_commands::Target::Ground(p),
        } => Some(*p),
        Order::Attack {
            target: tak_commands::Target::Unit(id),
        }
        | Order::Guard { target: id }
        | Order::Heal { target: id } => w.unit(*id).map(|t| t.pos),
    }
}

/// Heal goals use green/cyan, distinct from the ordinary yellow order line.
pub fn order_rgba(order: &tak_sim::Order) -> [f32; 4] {
    if matches!(order, tak_sim::Order::Heal { .. }) {
        [0.3, 1.0, 0.7, 0.8]
    } else {
        [1.0, 1.0, 0.4, 0.6]
    }
}

/// Player-visible orders, hiding the temporary Move which an active Heal
/// inserts in front of its own order while the healer walks into range.
/// Keeping that implementation detail out of the HUD preserves Heal's
/// green goal line, status verb and player-order queue count.
pub fn orders_for_display(u: &tak_sim::world::Unit) -> impl Iterator<Item = &tak_sim::Order> {
    let hide_heal_walk = u.build.heal.is_some_and(|j| {
        !j.cancelled
            && j.work.walking
            && j.work
                .walk_to
                .is_some_and(|to| u.orders.front() == Some(&tak_sim::Order::Move { to }))
            && u.orders.get(1)
                == Some(&tak_sim::Order::Heal {
                    target: j.work.target,
                })
    });
    u.orders.iter().skip(usize::from(hide_heal_walk))
}

/// The current order of a unit, as TA:K would say it.
pub fn order_text(
    w: &tak_sim::World,
    u: &tak_sim::world::Unit,
    display: impl Fn(UnitDefId) -> String,
) -> String {
    let name = |id: UnitId| w.unit(id).map_or("?".to_string(), |t| display(t.def));
    let mut orders = orders_for_display(u);
    let now = match orders.next() {
        None => "Idle".to_string(),
        Some(tak_sim::Order::Move { .. }) => "Moving".into(),
        Some(tak_sim::Order::AttackMove { .. }) => "Attack-moving".into(),
        Some(tak_sim::Order::Patrol { .. }) => "Patrolling".into(),
        Some(tak_sim::Order::Guard { target }) => format!("Guarding {}", name(*target)),
        Some(tak_sim::Order::Heal { target }) => {
            let verb = w.unit(*target).map_or("Healing", |t| {
                if !t.build.is_complete() {
                    "Assisting"
                } else if w.defs().get(t.def).is_some_and(|d| !d.build.mobile) {
                    "Repairing"
                } else {
                    "Healing"
                }
            });
            format!("{verb} {}", name(*target))
        }
        Some(tak_sim::Order::Attack {
            target: tak_commands::Target::Unit(t),
        }) => format!("Attacking {}", name(*t)),
        Some(tak_sim::Order::Attack { .. }) => "Attacking ground".into(),
        Some(tak_sim::Order::Build { def, .. }) => format!("Building {}", display(*def)),
    };
    let queued = orders.count();
    if queued > 0 {
        format!("{now} (+{queued})")
    } else {
        now
    }
}

/// The selection grouped by unit type for the multi-selection strip: each
/// type with its units, in order of the first unit (by id).
pub fn group_by_type(units: &[(UnitId, UnitDefId)]) -> Vec<(UnitDefId, Vec<UnitId>)> {
    let mut sorted = units.to_vec();
    sorted.sort_by_key(|(id, _)| *id);
    let mut out: Vec<(UnitDefId, Vec<UnitId>)> = Vec::new();
    for (id, def) in sorted {
        match out.iter_mut().find(|(d, _)| *d == def) {
            Some((_, v)) => v.push(id),
            None => out.push((def, vec![id])),
        }
    }
    out
}

/// Damage bars (TA ~): which units show a health bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageBars {
    /// Selected, damaged and unfinished units (the default).
    All,
    /// Selected units only.
    Selected,
    /// None.
    Off,
}

impl DamageBars {
    /// From the cycle counter (0, 1, 2).
    pub fn from_cycle(n: u8) -> DamageBars {
        match n % 3 {
            0 => DamageBars::All,
            1 => DamageBars::Selected,
            _ => DamageBars::Off,
        }
    }

    /// Show a bar for a unit?
    pub fn show(self, selected: bool, damaged: bool, building: bool) -> bool {
        match self {
            DamageBars::All => selected || damaged || building,
            DamageBars::Selected => selected,
            DamageBars::Off => false,
        }
    }

    /// Bottom-bar text when toggled.
    pub fn label(self) -> &'static str {
        match self {
            DamageBars::All => "Damage bars: all",
            DamageBars::Selected => "Damage bars: selected",
            DamageBars::Off => "Damage bars: off",
        }
    }
}

/// The bottom bar's verb line: what the confirm input does here, e.g.
/// "X: Attack Zhon Warrior" or "RMB: Move". `prompt` is the input's name,
/// `verb` the order, `target` what is under the cursor.
pub fn verb_line(prompt: &str, verb: &str, target: Option<&str>) -> String {
    match target {
        Some(t) if !t.is_empty() => format!("{prompt}: {verb} {t}"),
        _ => format!("{prompt}: {verb}"),
    }
}

/// Health bar colour (green, then yellow, then red), RGB 0..1.
pub fn health_rgb(frac: f32) -> [f32; 3] {
    let f = frac.clamp(0.0, 1.0);
    if f > 0.6 {
        [0.2, 0.85, 0.2]
    } else if f > 0.3 {
        [0.95, 0.8, 0.15]
    } else {
        [0.9, 0.2, 0.15]
    }
}

/// User-facing stance names. Legacy ReturnFire is retaliation, not Kingdoms Defensive.
pub fn stance_name(state: tak_commands::FireState) -> &'static str {
    match state {
        tak_commands::FireState::FireAtWill => "Offensive",
        tak_commands::FireState::ReturnFire => "Retaliate",
        tak_commands::FireState::HoldFire => "Passive",
        tak_commands::FireState::Defensive => "Defensive",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_panel_weapons_are_above_the_bottom_stances() {
        assert_eq!(
            &PANEL_BUTTONS[7..10],
            &["WPrimaryButton", "WSecondaryButton", "WSpecialButton"]
        );
        assert_eq!(
            &PANEL_BUTTONS[10..13],
            &["AgOffensiveButton", "AgDefensiveButton", "AgPassiveButton"]
        );
        assert!(ARAMON_SLOTS[9][1] < ARAMON_SLOTS[10][1]);
    }

    #[test]
    fn weapon_pictures_preserve_fbi_state_names() {
        let extra = [
            ("buttonimagedisabled".into(), "TestOff".into()),
            ("buttonimageup".into(), " TestUp.JPG ".into()),
            ("buttonimageselected".into(), "TestHover".into()),
            ("buttonimagedown".into(), " ".into()),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            weapon_button_paths(&extra),
            [
                Some("anims/weaponpic/testoff.jpg".into()),
                Some("anims/weaponpic/testup.jpg".into()),
                Some("anims/weaponpic/testhover.jpg".into()),
                None,
            ]
        );
        assert_eq!(
            weapon_button_paths(&Default::default()),
            [None, None, None, None]
        );
    }

    #[test]
    fn button_state_priority_and_missing_picture_fallback() {
        assert_eq!(button_state(false, true, true), 0);
        assert_eq!(button_state(true, true, true), 3);
        assert_eq!(button_state(true, false, true), 2);
        assert_eq!(button_state(true, false, false), 1);
        let pictures = [None, Some("normal"), Some("hover"), None];
        assert_eq!(button_image(&pictures, 2), Some(&"hover"));
        assert_eq!(button_image(&pictures, 0), Some(&"normal"));
        assert_eq!(button_image(&pictures, 3), Some(&"normal"));
        assert_eq!(button_image(&pictures, usize::MAX), Some(&"normal"));
        assert_eq!(
            button_image(&[Some("only"), None, None, None], 1),
            Some(&"only")
        );
        assert_eq!(button_image::<()>(&[None, None, None, None], 1), None);
    }

    #[test]
    fn skins_by_side() {
        assert_eq!(race_skin("ARAKING"), "aramonig");
        assert_eq!(race_skin("verking"), "verunaig");
        assert_eq!(race_skin("TARKING"), "tarosig");
        assert_eq!(race_skin("ZONKING"), "zhonig");
        assert_eq!(race_skin("CREKING"), "creonig");
        assert_eq!(race_skin(""), "aramonig");
    }

    #[test]
    fn wells_are_found_on_a_synthetic_panel() {
        // A 128x120 tan panel with three dark 27x27 wells (one row of two,
        // one centred below) and a big dark box that is not a well.
        let (w, h) = (128u32, 120u32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for i in 0..(w * h) as usize {
            px[i * 4..i * 4 + 4].copy_from_slice(&[150, 120, 80, 255]);
        }
        let mut fill = |x0: u32, y0: u32, bw: u32, bh: u32| {
            for y in y0..y0 + bh {
                for x in x0..x0 + bw {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[10, 10, 12, 255]);
                }
            }
        };
        fill(88, 20, 27, 27);
        fill(18, 21, 27, 27);
        fill(53, 60, 27, 27);
        fill(10, 95, 100, 20);
        let s = button_slots(&px, w, h);
        assert_eq!(
            s,
            vec![[18, 21, 27, 27], [88, 20, 27, 27], [53, 60, 27, 27]]
        );
        assert!(button_slots(&px[..10], w, h).is_empty());
    }

    #[test]
    fn strip_groups_types_in_id_order() {
        let u = |i| UnitId::new(i, 0);
        let g = group_by_type(&[
            (u(5), UnitDefId(2)),
            (u(1), UnitDefId(7)),
            (u(3), UnitDefId(2)),
        ]);
        assert_eq!(
            g,
            vec![(UnitDefId(7), vec![u(1)]), (UnitDefId(2), vec![u(3), u(5)])]
        );
    }

    #[test]
    fn damage_bars_cycle() {
        assert_eq!(DamageBars::from_cycle(0), DamageBars::All);
        assert!(DamageBars::All.show(false, true, false));
        assert!(!DamageBars::Selected.show(false, true, false));
        assert!(DamageBars::Selected.show(true, false, false));
        assert!(!DamageBars::from_cycle(2).show(true, true, true));
        assert_eq!(verb_line("X", "Attack", Some("Zhon")), "X: Attack Zhon");
        assert_eq!(verb_line("RMB", "Move", None), "RMB: Move");
    }
}
