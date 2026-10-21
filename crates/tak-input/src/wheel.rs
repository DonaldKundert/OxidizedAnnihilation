//! Radial wheel state machine: open, highlight a slot by stick angle,
//! confirm or cancel.

use serde::{Deserialize, Serialize};
use tak_math::{Angle, Vec2};

use crate::binding::Action;

/// Which wheel.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WheelKind {
    /// Orders and fire states (LT).
    Command,
    /// Build options of the selected builder (LB).
    Build,
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
        self.highlighted.and_then(|i| self.slots.get(i).copied())
    }
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
            Action::CastSpellMode => has_casters,
            Action::BuildOption { .. } => has_builders,
            _ => true,
        })
        .collect()
}
