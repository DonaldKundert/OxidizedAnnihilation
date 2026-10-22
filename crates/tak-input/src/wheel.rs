//! Radial wheel state machine: open, highlight a slot by stick angle,
//! confirm or cancel.

use serde::{Deserialize, Serialize};
use tak_math::{Angle, Vec2};

use tak_commands::{ControlGroup, FireState};

use crate::binding::{Action, ReservedOrder, Ring};

/// Most slots on one ring or page (design system: "the rule of eight").
pub const PAGE: usize = 8;

/// Which wheel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WheelKind {
    /// Orders and fire states (LT).
    Command,
    /// Build options of the selected builder (LB).
    Build,
    /// Spells of the selection's first caster (hold Y).
    Spell,
    /// The Stance sub-ring of the orders wheel.
    Stance,
    /// The Unit sub-ring (unit-specific orders, squads ring).
    Unit,
    /// Squads 5..9.
    Squads,
}

impl WheelKind {
    /// The wheel for a sub-ring.
    pub fn of_ring(r: Ring) -> WheelKind {
        match r {
            Ring::Stance => WheelKind::Stance,
            Ring::Unit => WheelKind::Unit,
            Ring::Squads => WheelKind::Squads,
        }
    }

    /// Title shown above the wheel.
    pub fn title(self) -> &'static str {
        match self {
            WheelKind::Command => "Orders",
            WheelKind::Build => "Build",
            WheelKind::Spell => "Spells",
            WheelKind::Stance => "Stance",
            WheelKind::Unit => "Unit",
            WheelKind::Squads => "Squads",
        }
    }
}

/// An open wheel. Slots run clockwise from the top.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Wheel {
    /// Which wheel.
    pub kind: WheelKind,
    /// Slot actions.
    pub slots: Vec<Action>,
    /// Highlighted slot. Stays put when the stick returns to centre, so a
    /// flick-and-release works.
    pub highlighted: Option<usize>,
    /// Page shown (build and spell wheels hold 8 per page).
    #[serde(default)]
    pub page: u8,
    /// Pages in all (0 or 1: a single page).
    #[serde(default)]
    pub pages: u8,
}

/// The slots of a sub-ring, in clockwise order from the top. Reserved
/// entries keep their places.
pub fn ring_slots(r: Ring) -> Vec<Action> {
    match r {
        // TA:K's names: Offensive / Defensive / Passive.
        Ring::Stance => vec![
            Action::SetFireState {
                state: FireState::FireAtWill,
            },
            Action::SetFireState {
                state: FireState::ReturnFire,
            },
            Action::SetFireState {
                state: FireState::HoldFire,
            },
        ],
        Ring::Unit => vec![
            Action::OpenRing { ring: Ring::Squads },
            Action::Reserved {
                order: ReservedOrder::Heal,
            },
            Action::Reserved {
                order: ReservedOrder::Load,
            },
            Action::Reserved {
                order: ReservedOrder::Unload,
            },
            Action::Reserved {
                order: ReservedOrder::Cloak,
            },
            Action::Reserved {
                order: ReservedOrder::Activate,
            },
        ],
        Ring::Squads => (5..=9)
            .filter_map(ControlGroup::new)
            .map(|group| Action::RecallGroup { group })
            .collect(),
    }
}

/// `true` if a ring has at least one usable slot.
pub fn ring_usable(r: Ring) -> bool {
    ring_slots(r).iter().any(|a| a.is_supported())
}

/// Slot under a stick direction (`+y` up) for a wheel of `n` slots, slot 0
/// centred at the top and increasing clockwise. `None` if `n == 0` or the
/// stick is centred.
pub fn slot_for(stick: Vec2, n: usize) -> Option<usize> {
    if n == 0 || stick == Vec2::ZERO {
        return None;
    }
    // Clockwise angle from straight up.
    let rel = Angle::QUARTER - stick.angle();
    let n64 = n as u64;
    let idx = (u64::from(rel.raw()) * n64 + 32768) >> 16;
    Some((idx % n64) as usize)
}

/// Centre direction of slot `k` of `n` (inverse of [`slot_for`]).
pub fn slot_direction(k: usize, n: usize) -> Angle {
    let n = n.max(1) as u64;
    let step = ((k as u64 % n) * 65536 / n) as u16;
    Angle::QUARTER - Angle::from_raw(step)
}

impl Wheel {
    /// Open with the given slots, nothing highlighted.
    pub fn open(kind: WheelKind, slots: Vec<Action>) -> Wheel {
        Wheel {
            kind,
            slots,
            highlighted: None,
            page: 0,
            pages: 0,
        }
    }

    /// Page `page` of `items` (8 per page, padded with reserved slots so
    /// every page has the same 8 positions).
    pub fn paged(kind: WheelKind, items: &[Action], page: usize) -> Wheel {
        let pages = items.len().div_ceil(PAGE).max(1);
        let page = page % pages;
        let mut slots: Vec<Action> = items.iter().skip(page * PAGE).take(PAGE).copied().collect();
        if pages > 1 || slots.len() > 1 {
            slots.resize(PAGE, BLANK);
        }
        Wheel {
            kind,
            slots,
            highlighted: None,
            page: u8::try_from(page).unwrap_or(u8::MAX),
            pages: u8::try_from(pages).unwrap_or(u8::MAX),
        }
    }

    /// Update the highlight from a (deadzoned) stick vector.
    pub fn steer(&mut self, stick: Vec2) {
        self.steer_with(stick, 0, false);
    }

    /// [`Wheel::steer`] with angle hysteresis (degrees past the highlighted
    /// slot's edge before another slot takes over) and, if `recentre_clears`,
    /// a centred stick clearing the highlight.
    pub fn steer_with(&mut self, stick: Vec2, hysteresis_deg: i32, recentre_clears: bool) {
        let n = self.slots.len();
        if stick == Vec2::ZERO {
            if recentre_clears {
                self.highlighted = None;
            }
            return;
        }
        let Some(i) = slot_for(stick, n) else { return };
        // An empty or reserved slot cannot be chosen: releasing there
        // cancels.
        if !self.slots.get(i).is_some_and(|a| a.is_supported()) {
            self.highlighted = None;
            return;
        }
        if let Some(h) = self.highlighted.filter(|&h| h != i && h < n)
            && hysteresis_deg > 0
        {
            let off = i32::from(slot_direction(h, n).delta_to(stick.angle())).unsigned_abs();
            let half = 32768 / n as u32;
            let hyst = Angle::from_degrees(hysteresis_deg.clamp(0, 90)).raw() as u32;
            if off <= half + hyst {
                return;
            }
        }
        self.highlighted = Some(i);
    }

    /// Close, yielding the highlighted action (or `None`: cancelled).
    pub fn confirm(self) -> Option<Action> {
        self.highlighted
            .and_then(|i| self.slots.get(i).copied())
            .filter(|a| a.is_supported())
    }
}

/// An empty slot (padding on a wheel page).
pub const BLANK: Action = Action::Reserved {
    order: ReservedOrder::Blank,
};

/// `true` for padding slots (drawn as nothing at all, unlike a reserved
/// order which keeps an empty frame).
pub fn is_blank(a: &Action) -> bool {
    *a == BLANK
}

/// Command wheel contents for the current selection (AoE IV style: only
/// what the selection can do). With own units selected: the profile's
/// `slots` minus orders no selected unit can carry out (`CastSpellMode`
/// without a caster, `BuildOption` without a builder). With nothing
/// selected: `idle`.
pub fn context_slots(
    slots: &[Action],
    idle: &[Action],
    has_units: bool,
    has_casters: bool,
    has_builders: bool,
) -> Vec<Action> {
    if !has_units {
        return idle.to_vec();
    }
    slots
        .iter()
        .copied()
        .filter(|a| match a {
            Action::CastSpellMode | Action::CastSpell { .. } => has_casters,
            Action::BuildOption { .. } => has_builders,
            _ => true,
        })
        .collect()
}

/// `slots` with `CastSpellMode` replaced by one `CastSpell` slot per spell
/// of the selection's first caster (`spells` of them), so the wheel lists
/// every spell. Unchanged when `spells` is 0 or there is no such slot.
pub fn expand_spells(slots: Vec<Action>, spells: usize) -> Vec<Action> {
    if spells == 0 {
        return slots;
    }
    let mut out = Vec::with_capacity(slots.len() + spells);
    for a in slots {
        if a == Action::CastSpellMode {
            out.extend(
                (0..spells.min(usize::from(u8::MAX))).map(|i| Action::CastSpell {
                    index: u8::try_from(i).unwrap_or(u8::MAX),
                }),
            );
        } else {
            out.push(a);
        }
    }
    out
}

/// [`context_slots`] for fixed-position wheels: slots the selection cannot
/// use become [`BLANK`] in place, and a sub-ring with no usable slot is
/// shown as reserved, so every slot keeps its position.
pub fn context_slots_fixed(
    slots: &[Action],
    idle: &[Action],
    has_units: bool,
    has_casters: bool,
    has_builders: bool,
) -> Vec<Action> {
    if !has_units {
        return idle.to_vec();
    }
    slots
        .iter()
        .map(|a| match a {
            Action::CastSpellMode | Action::CastSpell { .. } if !has_casters => BLANK,
            Action::BuildOption { .. } if !has_builders => BLANK,
            Action::OpenRing { ring } if !ring_usable(*ring) => BLANK,
            other => *other,
        })
        .collect()
}
