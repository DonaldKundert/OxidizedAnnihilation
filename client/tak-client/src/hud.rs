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
        C::SetFactoryRally { order, .. } => match order {
            Some(tak_commands::FactoryRallyOrder::Move { .. }) => "MOVE",
            Some(tak_commands::FactoryRallyOrder::Patrol { .. }) => "PATROL",
            Some(tak_commands::FactoryRallyOrder::Guard { .. }) => "GUARD",
            None => "STOP",
        },
        C::CancelProduction { .. } => "STOP",
        C::SetFactoryRepeat { def, .. } => {
            if def.is_some() {
                "MOVE"
            } else {
                "STOP"
            }
        }
        C::SetFireState { .. } => return None,
    })
}

/// The simulation goal of an order, without local visibility filtering.
/// A stale unit target has no goal; never substitute a slot's new generation.
/// Rendering must use [`visible_order_goals`] instead to avoid revealing
/// unseen unit targets or their temporary combat steering moves.
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

/// A known ground goal or a currently visible, selectable unit target's
/// exact-generation position. Friendly/allied visibility follows the view.
pub fn visible_order_goal(
    view: &impl tak_input::WorldView,
    order: &tak_sim::Order,
) -> Option<tak_commands::WorldPos> {
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
        | Order::Heal { target: id } => view
            .unit(*id)
            .filter(|u| u.id == *id && u.selectable)
            .map(|u| u.pos),
    }
}

/// Order-line goals for this player's view, without changing the player
/// queue or the order/status text. Besides filtering queued unit targets,
/// hide an active unseen unit attack's temporary front steering Move.
/// Compare that entry's identity, not just its coordinates: a later player
/// Move to the same point is still a known ground order and must survive.
pub fn visible_order_goals<'a>(
    view: &'a impl tak_input::WorldView,
    u: &'a tak_sim::world::Unit,
) -> impl Iterator<Item = (&'a tak_sim::Order, tak_commands::WorldPos)> + 'a {
    let hidden_front = match u.combat.order {
        Some(tak_sim::combat::CombatOrder::Attack(tak_commands::Target::Unit(target)))
            if view
                .unit(target)
                .is_none_or(|u| u.id != target || !u.selectable) =>
        {
            u.orders.front().filter(
                |order| matches!(order, tak_sim::Order::Move { to } if Some(*to) == u.combat.goal),
            )
        }
        _ => None,
    };
    orders_for_display(u).filter_map(move |order| {
        if hidden_front.is_some_and(|front| std::ptr::eq(front, order)) {
            return None;
        }
        visible_order_goal(view, order).map(|goal| (order, goal))
    })
}

/// A standing Guard follows only its original live friendly target. Never
/// draw a line to an enemy captured target or a reused arena slot.
pub fn factory_rally_goal(
    w: &tak_sim::World,
    factory: tak_commands::UnitId,
    order: &tak_commands::FactoryRallyOrder,
) -> Option<tak_commands::WorldPos> {
    use tak_commands::FactoryRallyOrder;
    match order {
        FactoryRallyOrder::Move { to } | FactoryRallyOrder::Patrol { to } => Some(*to),
        FactoryRallyOrder::Guard { target } => {
            tak_sim::construction::can_factory_guard_target(w, factory, *target)
                .then(|| w.unit(*target).map(|u| u.pos))
                .flatten()
        }
    }
}

/// Active Guard is promoted out of the ordinary queue. Show its friendly
/// target rather than treating an empty queue as an idle unit.
pub fn active_guard_goal(
    w: &tak_sim::World,
    u: &tak_sim::world::Unit,
) -> Option<tak_commands::WorldPos> {
    let Some(tak_sim::combat::CombatOrder::Guard(target)) = u.combat.order else {
        return None;
    };
    w.unit(target)
        .filter(|t| {
            !t.combat.is_dying()
                && (t.combat.max_health == 0 || t.combat.health > 0)
                && !w.are_hostile(u.owner, t.owner)
        })
        .map(|t| t.pos)
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
/// Also hide active Guard's temporary steering Move. Keeping these details
/// out of the HUD preserves the target line, verb and player queue count.
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
    let hide_guard_walk = matches!(u.combat.order, Some(tak_sim::combat::CombatOrder::Guard(_)))
        && (u
            .combat
            .goal
            .is_some_and(|to| u.orders.front() == Some(&tak_sim::Order::Move { to }))
            || u.build.assist.is_some_and(|assist| {
                assist.walking
                    && assist
                        .walk_to
                        .is_some_and(|to| u.orders.front() == Some(&tak_sim::Order::Move { to }))
            }));
    u.orders
        .iter()
        .skip(usize::from(hide_heal_walk || hide_guard_walk))
}

/// Current Guard-assistance assignment, not evidence of paid progress. The
/// callback interval may remain open while mana is unavailable. Revalidate the
/// exact frame and its relationship to Guard before describing stale work state.
fn active_guard_assist(w: &tak_sim::World, u: &tak_sim::world::Unit) -> Option<(UnitId, bool)> {
    let tak_sim::combat::CombatOrder::Guard(guarded) = u.combat.order? else {
        return None;
    };
    let assist = u.build.assist?;
    let builder = w.defs().get(u.def)?;
    if u.combat.is_dying()
        || !u.build.is_complete()
        || u.build.job.is_some()
        || !builder.build.builder
        || !builder.build.mobile
        || builder.build.workertime <= 0
        || tak_sim::construction::assist_target(w, u.owner, guarded) != Some(assist.target)
    {
        return None;
    }
    let target = w.unit(assist.target)?;
    if target.owner != u.owner || target.combat.is_dying() || target.build.is_complete() {
        return None;
    }
    if assist.walking {
        return assist
            .walk_to
            .is_some_and(|to| u.orders.front() == Some(&tak_sim::Order::Move { to }))
            .then_some((assist.target, true));
    }
    let target_def = w.defs().get(target.def)?;
    (assist.building
        && tak_sim::construction::placement::in_range(u.pos, builder, target.pos, target_def))
    .then_some((assist.target, false))
}

/// The current order of a unit, as TA:K would say it.
pub fn order_text(
    w: &tak_sim::World,
    u: &tak_sim::world::Unit,
    display: impl Fn(UnitDefId) -> String,
) -> String {
    if !u.build.is_complete() {
        return "Under construction".into();
    }
    let name = |id: UnitId| w.unit(id).map_or("?".to_string(), |t| display(t.def));
    let product = u.build.factory.product.and_then(|id| w.unit(id));
    let finite_product = orders_for_display(u).find_map(|order| match order {
        tak_sim::Order::Build { def, at: None } => Some(*def),
        _ => None,
    });
    // A paid repeat is separate from every finite entry, even after its
    // recipe changes or is disabled. A finite product still owns its first
    // production entry, which need not be the front (legacy rally Moves).
    let represented_finite = product.is_some_and(|p| {
        !u.build.factory.product_from_repeat
            && !u.build.factory.cancel_product
            && finite_product == Some(p.def)
    });
    let mut orders = orders_for_display(u);
    let now = match (
        product,
        u.combat.order,
        u.build.factory_repeat.filter(|_| finite_product.is_none()),
    ) {
        (Some(p), _, _) => format!("Building {}", display(p.def)),
        (_, Some(tak_sim::combat::CombatOrder::Guard(target)), _) => {
            format!("Guarding {}", name(target))
        }
        // With no paid frame, the recipe alone does not tell us whether the
        // yard, script, mana, or the next construction tick is holding it up.
        (_, _, Some(def)) => format!("Waiting to repeat {}", display(def)),
        _ => match orders.next() {
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
        },
    };
    let queued = orders
        .count()
        .saturating_sub(usize::from(represented_finite));
    let now = if queued > 0 {
        format!("{now} (+{queued})")
    } else {
        now
    };
    let now = if product.is_none()
        && let Some((target, walking)) = active_guard_assist(w, u)
    {
        let verb = if walking {
            "Moving to assist"
        } else {
            "Assisting"
        };
        format!("{now} | {verb} {}", name(target))
    } else {
        now
    };
    match &u.build.factory_rally {
        Some(rally) if !rally.is_empty() => {
            let targets: Vec<_> = rally
                .iter()
                .filter_map(|order| match order {
                    tak_commands::FactoryRallyOrder::Guard { target } => Some(*target),
                    _ => None,
                })
                .collect();
            let guard = if targets.iter().any(|target| {
                w.unit(*target).is_none_or(|t| {
                    t.combat.is_dying()
                        || (t.combat.max_health > 0 && t.combat.health <= 0)
                        || w.are_hostile(u.owner, t.owner)
                })
            }) {
                " | Guard target unavailable".to_string()
            } else if let Some(target) = targets.first() {
                format!(" | Guard {}", name(*target))
            } else {
                String::new()
            };
            format!("{now} | Rally {}{guard}", rally.len())
        }
        _ => now,
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
    fn active_guard_status_and_route_do_not_mistake_steering_for_player_orders() {
        use tak_commands::PlayerId;
        use tak_math::{Angle, Vec2};
        use tak_sim::{Order, combat::CombatOrder};
        let scenario = tak_sim::testing::example_scenario(1, 2);
        let mut world = tak_sim::World::new(&scenario).unwrap();
        let guard = world
            .spawn(
                UnitDefId(0),
                PlayerId(0),
                Vec2::from_ints(100, 100),
                Angle::ZERO,
            )
            .unwrap();
        let target = world
            .spawn(
                UnitDefId(1),
                PlayerId(0),
                Vec2::from_ints(300, 300),
                Angle::ZERO,
            )
            .unwrap();
        world.unit_mut(guard).unwrap().combat.order = Some(CombatOrder::Guard(target));
        let text = |world: &tak_sim::World| {
            order_text(world, world.unit(guard).unwrap(), |_| "Commander".into())
        };
        assert_eq!(text(&world), "Guarding Commander");
        assert_eq!(
            active_guard_goal(&world, world.unit(guard).unwrap()),
            Some(Vec2::from_ints(300, 300))
        );
        let steer = Vec2::from_ints(250, 250);
        let later = Order::Patrol {
            to: Vec2::from_ints(700, 700),
        };
        let unit = world.unit_mut(guard).unwrap();
        unit.combat.goal = Some(steer);
        unit.orders.extend([Order::Move { to: steer }, later]);
        let before = unit.orders.clone();
        assert_eq!(text(&world), "Guarding Commander (+1)");
        assert_eq!(
            orders_for_display(world.unit(guard).unwrap())
                .copied()
                .collect::<Vec<_>>(),
            vec![later]
        );
        assert_eq!(world.unit(guard).unwrap().orders, before);
        // Guard-assisted construction owns a separate walk marker and
        // deliberately clears combat.goal. Hide that steering too, but not
        // a same-coordinate player Move after the assist stops walking.
        let unit = world.unit_mut(guard).unwrap();
        unit.combat.goal = None;
        unit.build.assist = Some(tak_sim::construction::Assist {
            target,
            walk_to: Some(steer),
            walking: true,
            building: false,
        });
        assert_eq!(text(&world), "Guarding Commander (+1)");
        assert_eq!(orders_for_display(world.unit(guard).unwrap()).count(), 1);
        world
            .unit_mut(guard)
            .unwrap()
            .orders
            .insert(1, Order::Move { to: steer });
        assert_eq!(
            orders_for_display(world.unit(guard).unwrap())
                .copied()
                .collect::<Vec<_>>(),
            vec![Order::Move { to: steer }, later]
        );
        assert_eq!(text(&world), "Guarding Commander (+2)");
        world.unit_mut(guard).unwrap().orders.remove(1);
        world
            .unit_mut(guard)
            .unwrap()
            .build
            .assist
            .as_mut()
            .unwrap()
            .walking = false;
        assert_eq!(text(&world), "Guarding Commander (+2)");
        assert_eq!(orders_for_display(world.unit(guard).unwrap()).count(), 2);
        world.unit_mut(target).unwrap().pos = Vec2::from_ints(500, 400);
        assert_eq!(
            active_guard_goal(&world, world.unit(guard).unwrap()),
            Some(Vec2::from_ints(500, 400))
        );
        world.unit_mut(target).unwrap().owner = PlayerId(1);
        assert_eq!(active_guard_goal(&world, world.unit(guard).unwrap()), None);
        world.despawn(target).unwrap();
        assert_eq!(active_guard_goal(&world, world.unit(guard).unwrap()), None);
        world.unit_mut(guard).unwrap().combat.order = None;
        assert_eq!(text(&world), "Moving (+1)");
    }

    fn guard_assist_status_fixture() -> (tak_sim::World, UnitId, UnitId, UnitId, UnitId) {
        use tak_commands::PlayerId;
        use tak_math::{Angle, Vec2};
        use tak_sim::{
            combat::CombatOrder,
            construction::{Assist, BuildDef, Frame},
        };
        let mut scenario = tak_sim::testing::example_scenario(1, 2);
        scenario.unit_defs = ["Helper", "Builder", "Factory", "Frame"]
            .into_iter()
            .map(tak_sim::testing::monarch_def)
            .collect();
        for (i, def) in scenario.unit_defs.iter_mut().enumerate() {
            def.build = BuildDef {
                builder: i < 3,
                mobile: i < 2,
                workertime: 10,
                builddistance: 100,
                buildcost: 100,
                buildtime: 100 << 16,
                ..Default::default()
            };
        }
        let mut world = tak_sim::World::new(&scenario).unwrap();
        let ids: Vec<_> = (0..4)
            .map(|i| {
                world
                    .spawn(
                        UnitDefId(i),
                        PlayerId(0),
                        Vec2::from_ints(100 + i as i32 * 20, 100),
                        Angle::ZERO,
                    )
                    .unwrap()
            })
            .collect();
        let (helper, builder, factory, frame) = (ids[0], ids[1], ids[2], ids[3]);
        world.unit_mut(frame).unwrap().build.frame = Some(Frame {
            done: 50 << 16,
            total: 100 << 16,
            paid: 50 << 16,
        });
        let u = world.unit_mut(helper).unwrap();
        u.combat.order = Some(CombatOrder::Guard(frame));
        u.build.assist = Some(Assist {
            target: frame,
            walk_to: None,
            walking: false,
            building: true,
        });
        (world, helper, builder, factory, frame)
    }

    #[test]
    fn guard_status_names_actual_direct_builder_and_factory_assistance() {
        use tak_sim::{combat::CombatOrder, construction::BuilderJob};
        let (base, helper, builder, factory, frame) = guard_assist_status_fixture();
        for (guarded, expected) in [
            (frame, "Guarding Frame | Assisting Frame"),
            (builder, "Guarding Builder | Assisting Frame"),
            (factory, "Guarding Factory | Assisting Frame"),
        ] {
            let mut world = base.clone();
            world.unit_mut(helper).unwrap().combat.order = Some(CombatOrder::Guard(guarded));
            let target = world.unit(frame).unwrap();
            let job = BuilderJob {
                def: target.def,
                site: target.pos,
                target: Some(frame),
                walking: false,
                building: true,
            };
            world.unit_mut(builder).unwrap().build.job = Some(job);
            world.unit_mut(factory).unwrap().build.factory.product = Some(frame);
            assert_eq!(factory_status(&world, helper), expected);
        }
        // Health-zero nanoframes are valid: death is identified by lifecycle
        // state, not by a health value which may precede the first work tick.
        assert_eq!(base.unit(frame).unwrap().combat.health, 0);
        assert_eq!(
            factory_status(&base, helper),
            "Guarding Frame | Assisting Frame"
        );
    }

    #[test]
    fn guard_assist_status_distinguishes_walking_and_preserves_queue_counts() {
        use tak_math::Vec2;
        use tak_sim::Order;
        let (mut world, helper, _, _, frame) = guard_assist_status_fixture();
        let goal = world.unit(frame).unwrap().pos;
        let later = Order::Patrol {
            to: Vec2::from_ints(700, 700),
        };
        let u = world.unit_mut(helper).unwrap();
        u.pos = Vec2::from_ints(500, 500);
        let a = u.build.assist.as_mut().unwrap();
        a.walking = true;
        a.walk_to = Some(goal);
        a.building = false;
        u.orders.extend([Order::Move { to: goal }, later]);
        assert_eq!(
            factory_status(&world, helper),
            "Guarding Frame (+1) | Moving to assist Frame"
        );
        world.unit_mut(helper).unwrap().orders.pop_front();
        assert_eq!(factory_status(&world, helper), "Guarding Frame (+1)");
        let a = world
            .unit_mut(helper)
            .unwrap()
            .build
            .assist
            .as_mut()
            .unwrap();
        a.walking = false;
        a.building = true;
        // A stale callback flag cannot describe an out-of-range assignment.
        assert_eq!(factory_status(&world, helper), "Guarding Frame (+1)");
        world.unit_mut(helper).unwrap().pos = Vec2::from_ints(100, 100);
        assert_eq!(
            factory_status(&world, helper),
            "Guarding Frame (+1) | Assisting Frame"
        );
        world
            .unit_mut(helper)
            .unwrap()
            .build
            .assist
            .as_mut()
            .unwrap()
            .building = false;
        assert_eq!(factory_status(&world, helper), "Guarding Frame (+1)");
    }

    #[test]
    fn guard_assist_status_rejects_completed_dead_captured_and_reused_frames() {
        use tak_commands::PlayerId;
        use tak_math::Angle;
        use tak_sim::combat::Dying;
        let (base, helper, _, _, frame) = guard_assist_status_fixture();
        let death = Dying {
            since: 1,
            severity: 100,
            kind: 0,
            done: false,
        };
        for invalid in 0..4 {
            let mut world = base.clone();
            match invalid {
                0 => world.unit_mut(frame).unwrap().build.frame = None,
                1 => world.unit_mut(frame).unwrap().combat.dying = Some(death),
                2 => world.unit_mut(frame).unwrap().owner = PlayerId(1),
                _ => world.unit_mut(helper).unwrap().combat.dying = Some(death),
            }
            assert_eq!(factory_status(&world, helper), "Guarding Frame");
        }
        let mut world = base;
        let old = world.despawn(frame).unwrap();
        let replacement = world
            .spawn(old.def, old.owner, old.pos, Angle::ZERO)
            .unwrap();
        world.unit_mut(replacement).unwrap().build.frame = old.build.frame;
        assert_ne!(replacement, frame);
        assert_eq!(factory_status(&world, helper), "Guarding ?");
    }

    #[test]
    fn guard_assist_status_revalidates_guard_relationship_and_clears_on_stop() {
        use tak_commands::{LoggedCommand, PlayerId, SimCommand, Tick, UnitSet};
        use tak_sim::{
            combat::{CombatOrder, Dying},
            construction::BuilderJob,
        };
        let (base, helper, builder, factory, frame) = guard_assist_status_fixture();
        let target = base.unit(frame).unwrap();
        let job = BuilderJob {
            def: target.def,
            site: target.pos,
            target: Some(frame),
            walking: false,
            building: true,
        };
        for invalid in 0..5 {
            let mut world = base.clone();
            world.unit_mut(helper).unwrap().combat.order = Some(CombatOrder::Guard(builder));
            world.unit_mut(builder).unwrap().build.job = Some(job);
            match invalid {
                0 => world.unit_mut(builder).unwrap().build.job = None,
                1 => world.unit_mut(builder).unwrap().owner = PlayerId(1),
                2 => {
                    world.unit_mut(builder).unwrap().combat.dying = Some(Dying {
                        since: 1,
                        severity: 100,
                        kind: 0,
                        done: false,
                    })
                }
                3 => world.unit_mut(helper).unwrap().build.job = Some(job),
                _ => {
                    world
                        .unit_mut(helper)
                        .unwrap()
                        .build
                        .assist
                        .as_mut()
                        .unwrap()
                        .target = factory
                }
            }
            assert_eq!(factory_status(&world, helper), "Guarding Builder");
        }
        let mut world = base.clone();
        world.unit_mut(helper).unwrap().combat.order = Some(CombatOrder::Guard(factory));
        world.unit_mut(factory).unwrap().build.factory.product = Some(frame);
        assert_eq!(
            factory_status(&world, helper),
            "Guarding Factory | Assisting Frame"
        );
        let replacement = world
            .spawn(target.def, target.owner, target.pos, tak_math::Angle::ZERO)
            .unwrap();
        world.unit_mut(replacement).unwrap().build.frame = target.build.frame;
        world.unit_mut(factory).unwrap().build.factory.product = Some(replacement);
        assert_eq!(factory_status(&world, helper), "Guarding Factory");
        // A queued/inherited Guard without an active assistance assignment
        // remains the ordinary Guard status, even before stale cleanup.
        world.unit_mut(helper).unwrap().combat.order = None;
        world
            .unit_mut(helper)
            .unwrap()
            .orders
            .push_back(tak_sim::Order::Guard { target: frame });
        assert_eq!(factory_status(&world, helper), "Guarding Frame");
        let mut world = base;
        tak_sim::systems::apply_commands(
            &mut world,
            &[LoggedCommand {
                tick: Tick(0),
                player: PlayerId(0),
                seq: 0,
                command: SimCommand::Stop {
                    units: UnitSet::single(helper),
                },
            }],
        );
        assert!(
            world.unit(helper).unwrap().build.assist.is_some(),
            "pending cleanup is deliberately retained"
        );
        assert_eq!(factory_status(&world, helper), "Idle");
    }

    #[test]
    fn guard_assist_status_is_assignment_not_a_claim_of_paid_progress() {
        use tak_commands::PlayerId;
        use tak_math::Fx;
        let (mut world, helper, _, _, frame) = guard_assist_status_fixture();
        tak_sim::economy::drain(&mut world, PlayerId(0), Fx::from_int(1000));
        assert_eq!(
            tak_sim::economy::player(&world, PlayerId(0)).unwrap().mana,
            0
        );
        let before = world.unit(frame).unwrap().build.frame;
        tak_sim::construction::tick(&mut world, &mut tak_sim::script::NoScripts);
        assert_eq!(world.unit(frame).unwrap().build.frame, before);
        assert!(world.unit(helper).unwrap().build.assist.unwrap().building);
        assert_eq!(
            factory_status(&world, helper),
            "Guarding Frame | Assisting Frame"
        );
    }

    #[test]
    fn guard_assist_status_preserves_product_and_factory_rally_precedence() {
        use tak_commands::FactoryRallyOrder;
        use tak_sim::combat::CombatOrder;
        let (mut world, helper, builder, factory, frame) = guard_assist_status_fixture();
        let u = world.unit_mut(helper).unwrap();
        u.build.factory.product = Some(frame);
        u.build.factory_rally = Some(vec![FactoryRallyOrder::Guard { target: builder }]);
        assert_eq!(
            factory_status(&world, helper),
            "Building Frame | Rally 1 | Guard Builder"
        );
        let assist = world.unit(helper).unwrap().build.assist;
        let u = world.unit_mut(factory).unwrap();
        u.combat.order = Some(CombatOrder::Guard(frame));
        u.build.assist = assist;
        u.build.factory_rally = Some(vec![FactoryRallyOrder::Guard { target: builder }]);
        assert_eq!(
            factory_status(&world, factory),
            "Guarding Frame | Rally 1 | Guard Builder"
        );
    }

    fn factory_status_fixture(repeated: bool) -> (tak_sim::World, UnitId, UnitId) {
        use tak_commands::PlayerId;
        use tak_math::{Angle, Vec2};
        use tak_sim::construction::{BuildDef, FactoryState, Frame};
        let mut scenario = tak_sim::testing::example_scenario(1, 1);
        scenario.unit_defs[0].name = "Barracks".into();
        scenario.unit_defs[0].build = BuildDef {
            builder: true,
            mobile: false,
            canbuild: vec![UnitDefId(1), UnitDefId(2)],
            workertime: 10,
            ..Default::default()
        };
        scenario.unit_defs[1].name = "Swordsman".into();
        scenario
            .unit_defs
            .push(tak_sim::testing::monarch_def("Archer"));
        let mut world = tak_sim::World::new(&scenario).unwrap();
        let factory = world
            .spawn(
                UnitDefId(0),
                PlayerId(0),
                Vec2::from_ints(100, 100),
                Angle::ZERO,
            )
            .unwrap();
        let product = world
            .spawn(
                UnitDefId(1),
                PlayerId(0),
                Vec2::from_ints(200, 100),
                Angle::ZERO,
            )
            .unwrap();
        world.unit_mut(product).unwrap().build.frame = Some(Frame {
            done: 5 << 16,
            total: 100 << 16,
            paid: 5 << 16,
        });
        let unit = world.unit_mut(factory).unwrap();
        unit.build.factory = FactoryState {
            product: Some(product),
            product_from_repeat: repeated,
            active: true,
            ..Default::default()
        };
        if repeated {
            unit.build.factory_repeat = Some(UnitDefId(1));
        } else {
            unit.orders.push_back(tak_sim::Order::Build {
                def: UnitDefId(1),
                at: None,
            });
        }
        (world, factory, product)
    }

    fn factory_status(world: &tak_sim::World, factory: UnitId) -> String {
        let before = world.clone();
        let status = order_text(world, world.unit(factory).unwrap(), |def| {
            world.defs().get(def).unwrap().name.clone()
        });
        assert_eq!(world, &before, "HUD reads must not change simulation state");
        status
    }

    #[test]
    fn factory_status_shows_active_repeat_without_a_finite_order() {
        use tak_commands::FactoryRallyOrder;
        use tak_math::Vec2;
        let (mut world, factory, _) = factory_status_fixture(true);
        assert!(world.unit(factory).unwrap().orders.is_empty());
        assert_eq!(factory_status(&world, factory), "Building Swordsman");
        world.unit_mut(factory).unwrap().build.factory_rally =
            Some(vec![FactoryRallyOrder::Move {
                to: Vec2::from_ints(500, 500),
            }]);
        assert_eq!(
            factory_status(&world, factory),
            "Building Swordsman | Rally 1"
        );
        world.unit_mut(factory).unwrap().build.frame = Some(tak_sim::construction::Frame::new(100));
        assert_eq!(factory_status(&world, factory), "Under construction");
    }

    #[test]
    fn factory_status_keeps_finite_work_pending_behind_a_paid_repeat() {
        use tak_sim::Order;
        let (mut world, factory, _) = factory_status_fixture(true);
        world.unit_mut(factory).unwrap().orders.extend([
            Order::Build {
                def: UnitDefId(2),
                at: None,
            },
            Order::Build {
                def: UnitDefId(1),
                at: None,
            },
        ]);
        assert_eq!(factory_status(&world, factory), "Building Swordsman (+2)");
        assert_eq!(
            tak_sim::construction::production_count(&world, factory, UnitDefId(1)),
            2
        );
        assert_eq!(
            tak_sim::construction::production_count(&world, factory, UnitDefId(2)),
            1
        );
    }

    #[test]
    fn factory_status_does_not_double_count_the_active_finite_entry() {
        use tak_math::Vec2;
        use tak_sim::Order;
        let (mut world, factory, _) = factory_status_fixture(false);
        assert_eq!(factory_status(&world, factory), "Building Swordsman");
        world.unit_mut(factory).unwrap().orders.extend([
            Order::Build {
                def: UnitDefId(2),
                at: None,
            },
            Order::Build {
                def: UnitDefId(1),
                at: None,
            },
        ]);
        assert_eq!(factory_status(&world, factory), "Building Swordsman (+2)");
        // Legacy rally Moves can precede the finite production entry. Do not
        // consume a Move or subtract a second queued product from the count.
        world
            .unit_mut(factory)
            .unwrap()
            .orders
            .push_front(Order::Move {
                to: Vec2::from_ints(500, 500),
            });
        assert_eq!(factory_status(&world, factory), "Building Swordsman (+3)");
        // A removed active entry can await cleanup while a new entry of the
        // same type is pending. That replacement does not own the paid frame.
        let unit = world.unit_mut(factory).unwrap();
        unit.build.factory.cancel_product = true;
        unit.orders.clear();
        unit.orders.push_back(Order::Build {
            def: UnitDefId(1),
            at: None,
        });
        assert_eq!(factory_status(&world, factory), "Building Swordsman (+1)");
    }

    #[test]
    fn factory_status_uses_paid_product_after_repeat_changes_or_disables() {
        use tak_commands::{PlayerId, SimCommand, UnitSet};
        for recipe in [Some(UnitDefId(2)), None] {
            let (mut world, factory, product) = factory_status_fixture(true);
            let paid_frame = world.unit(product).unwrap().build.frame;
            tak_sim::construction::apply_factory_command(
                &mut world,
                PlayerId(0),
                &SimCommand::SetFactoryRepeat {
                    units: UnitSet::single(factory),
                    def: recipe,
                },
            );
            assert_eq!(world.unit(factory).unwrap().build.factory_repeat, recipe);
            assert!(
                world
                    .unit(factory)
                    .unwrap()
                    .build
                    .factory
                    .product_from_repeat
            );
            assert_eq!(world.unit(product).unwrap().build.frame, paid_frame);
            assert_eq!(factory_status(&world, factory), "Building Swordsman");
            world
                .unit_mut(factory)
                .unwrap()
                .orders
                .push_back(tak_sim::Order::Build {
                    def: UnitDefId(2),
                    at: None,
                });
            assert_eq!(factory_status(&world, factory), "Building Swordsman (+1)");
        }
    }

    #[test]
    fn factory_status_waiting_repeat_does_not_guess_why_no_product_exists() {
        use tak_commands::Tick;
        use tak_math::Vec2;
        use tak_sim::{Order, construction::FactoryState};
        let (mut world, factory, product) = factory_status_fixture(true);
        world.despawn(product).unwrap();
        for (active, retry_at) in [(false, None), (true, None), (true, Some(Tick(100)))] {
            world.unit_mut(factory).unwrap().build.factory = FactoryState {
                active,
                retry_at,
                ..Default::default()
            };
            assert_eq!(
                factory_status(&world, factory),
                "Waiting to repeat Swordsman"
            );
        }
        world
            .unit_mut(factory)
            .unwrap()
            .orders
            .push_back(Order::Move {
                to: Vec2::from_ints(500, 500),
            });
        assert_eq!(
            factory_status(&world, factory),
            "Waiting to repeat Swordsman (+1)"
        );
        let unit = world.unit_mut(factory).unwrap();
        unit.orders.clear();
        unit.orders.push_back(Order::Build {
            def: UnitDefId(2),
            at: None,
        });
        assert_eq!(factory_status(&world, factory), "Building Archer");
    }

    #[test]
    fn factory_status_recovers_to_idle_after_stop_and_can_repeat_again() {
        use tak_commands::{CommandLog, PlayerId, SimCommand, UnitSet};
        for repeated in [false, true] {
            let (mut world, factory, product) = factory_status_fixture(repeated);
            assert_eq!(factory_status(&world, factory), "Building Swordsman");
            let mut log = CommandLog::new();
            log.record(
                world.tick(),
                PlayerId(0),
                SimCommand::Stop {
                    units: UnitSet::single(factory),
                },
            );
            let tick = world.tick();
            tak_sim::systems::apply_commands(&mut world, log.for_tick(tick));
            tak_sim::construction::tick(&mut world, &mut tak_sim::script::NoScripts);
            assert!(world.unit(product).is_none());
            let unit = world.unit(factory).unwrap();
            assert_eq!(unit.build.factory.product, None);
            assert_eq!(unit.build.factory_repeat, None);
            assert!(unit.orders.is_empty());
            assert_eq!(factory_status(&world, factory), "Idle");
            tak_sim::construction::apply_factory_command(
                &mut world,
                PlayerId(0),
                &SimCommand::SetFactoryRepeat {
                    units: UnitSet::single(factory),
                    def: Some(UnitDefId(2)),
                },
            );
            assert_eq!(factory_status(&world, factory), "Waiting to repeat Archer");
        }
    }

    #[test]
    fn factory_guard_display_tracks_only_the_live_friendly_generation() {
        use tak_commands::{FactoryRallyOrder, PlayerId, QueueMode, SimCommand, UnitSet};
        use tak_math::{Angle, Vec2};
        let mut scenario = tak_sim::testing::example_scenario(1, 2);
        scenario.unit_defs[0].build = tak_sim::construction::BuildDef {
            builder: true,
            mobile: false,
            canbuild: vec![UnitDefId(1)],
            workertime: 10,
            ..Default::default()
        };
        let mut world = tak_sim::World::new(&scenario).unwrap();
        let factory = world
            .spawn(
                UnitDefId(0),
                PlayerId(0),
                Vec2::from_ints(100, 100),
                Angle::ZERO,
            )
            .unwrap();
        let target = world
            .spawn(
                UnitDefId(1),
                PlayerId(0),
                Vec2::from_ints(300, 300),
                Angle::ZERO,
            )
            .unwrap();
        let guard = FactoryRallyOrder::Guard { target };
        world.unit_mut(factory).unwrap().build.factory_rally = Some(vec![guard]);
        let status = |world: &tak_sim::World| {
            order_text(world, world.unit(factory).unwrap(), |_| "Commander".into())
        };
        assert_eq!(
            order_sound(&SimCommand::SetFactoryRally {
                units: UnitSet::single(factory),
                order: Some(guard),
                queue: QueueMode::Replace
            }),
            Some("GUARD")
        );
        assert_eq!(status(&world), "Idle | Rally 1 | Guard Commander");
        assert_eq!(
            factory_rally_goal(&world, factory, &guard),
            Some(Vec2::from_ints(300, 300))
        );
        world.unit_mut(target).unwrap().pos = Vec2::from_ints(600, 400);
        assert_eq!(
            factory_rally_goal(&world, factory, &guard),
            Some(Vec2::from_ints(600, 400))
        );
        world.unit_mut(target).unwrap().owner = PlayerId(1);
        assert_eq!(factory_rally_goal(&world, factory, &guard), None);
        assert_eq!(status(&world), "Idle | Rally 1 | Guard target unavailable");
        world.unit_mut(target).unwrap().owner = PlayerId(0);
        world.unit_mut(target).unwrap().combat.max_health = 100;
        world.unit_mut(target).unwrap().combat.health = 0;
        assert_eq!(factory_rally_goal(&world, factory, &guard), None);
        assert!(status(&world).ends_with("Guard target unavailable"));
        world.despawn(target).unwrap();
        let replacement = world
            .spawn(
                UnitDefId(1),
                PlayerId(0),
                Vec2::from_ints(900, 900),
                Angle::ZERO,
            )
            .unwrap();
        assert_eq!(replacement.index, target.index);
        assert_ne!(replacement, target);
        assert_eq!(factory_rally_goal(&world, factory, &guard), None);
        assert!(status(&world).ends_with("Guard target unavailable"));
        let to = Vec2::from_ints(700, 700);
        assert_eq!(
            factory_rally_goal(&world, factory, &FactoryRallyOrder::Move { to }),
            Some(to)
        );
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
