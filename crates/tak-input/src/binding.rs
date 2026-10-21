//! Data-driven bindings: which input gesture triggers which action.
//!
//! A [`Profile`] is JSON. Example (one tap, one hold, one chord):
//!
//! ```json
//! {
//!   "name": "controller",
//!   "bindings": [
//!     {"input": {"button": "a"}, "gesture": "tap",  "action": {"type": "select"}},
//!     {"input": {"button": "a"}, "gesture": "hold", "action": {"type": "brush"}},
//!     {"input": {"button": "dpad_up"}, "with": {"button": "rb"}, "gesture": "press",
//!      "action": {"type": "assign_group", "group": 1}}
//!   ],
//!   "command_wheel": [{"type": "stop"}]
//! }
//! ```

use serde::{Deserialize, Serialize};
use tak_commands::{ControlGroup, FireState, Scope};
use thiserror::Error;

use crate::event::Input;
use crate::tuning::Tuning;

/// How an input must be operated for a binding to fire.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    /// On the down edge. Held actions stay active until release.
    Press,
    /// On the up edge.
    Release,
    /// Down and up without becoming a hold (only when the input also has a
    /// `hold` binding does duration matter). If the input has a `double_tap`
    /// binding, the tap fires once the double-tap window expires, or as soon
    /// as a different input is pressed.
    Tap,
    /// Still down after `hold_ms`. Held actions stay active until release.
    Hold,
    /// Second down within `double_tap_ms` of a release.
    DoubleTap,
    /// Second down within `double_tap_ms` of a tap's release, *without*
    /// delaying that first tap: the tap fires at once, and the second press
    /// fires this instead of its own tap or hold. For actions that refine
    /// the tap's (Halo Wars: tap A selects one unit, a second tap selects
    /// that type on screen). A `double_tap` binding on the same trigger
    /// takes precedence.
    SecondTap,
}

/// What a binding does. Actions marked *held* last from their gesture until
/// the input is released; the rest fire once.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Select the unit under the cursor (additive with the queue modifier).
    /// With an order mode pending, acts as [`Action::Context`] instead.
    Select,
    /// *Held.* Gamepad selection brush at the cursor; radius grows while held.
    Brush,
    /// *Held.* Mouse drag selection: a click selects at the cursor, a drag
    /// selects the circle spanning the drag.
    DragSelect,
    /// Context order at the cursor: Attack an enemy, Guard a friendly, Move
    /// to ground, or complete the pending order mode (build, patrol, ...).
    Context {
        /// Queue the order (shift). The queue modifier also sets this.
        #[serde(default)]
        append: bool,
    },
    /// Close a wheel, abort a brush, clear the order mode, or deselect.
    Cancel,
    /// Select own units sharing a type with the selection.
    SelectSameType {
        /// On screen or whole map.
        scope: Scope,
    },
    /// Select every own unit on screen (Halo Wars "local") or on the map
    /// ("global").
    SelectAll {
        /// On screen or whole map.
        scope: Scope,
    },
    /// Toggle the camera following the selection (cancelled by panning).
    FollowSelection,
    /// *Held.* Right stick zooms instead of panning.
    ZoomModifier,
    /// Open or close the live tuning overlay (client).
    ToggleOverlay,
    /// Store the selection as a control group.
    AssignGroup {
        /// Group.
        group: ControlGroup,
    },
    /// Recall a control group.
    RecallGroup {
        /// Group.
        group: ControlGroup,
    },
    /// Camera to the latest alert.
    JumpToAlert,
    /// Select and centre on the next idle builder.
    CycleIdleBuilder,
    /// Camera to the monarch.
    CenterOnMonarch,
    /// Stop the selection.
    Stop,
    /// Set the selection's fire state.
    SetFireState {
        /// New state.
        state: FireState,
    },
    /// Next context order is a patrol.
    PatrolMode,
    /// Next context order is a guard.
    GuardMode,
    /// Next context order casts the selection's first spell.
    CastSpellMode,
    /// Next context order casts spell `index` of the selection's first
    /// caster (in [`crate::WorldView::spells`] order). Refused while that
    /// caster cannot pay for it ([`crate::WorldView::can_cast`]).
    CastSpell {
        /// Index into the caster's spells.
        index: u8,
    },
    /// Pick build option `index` of the selection's first builder: factories
    /// queue it immediately, mobile builders enter placement mode.
    BuildOption {
        /// Index into the builder's options.
        index: u8,
    },
    /// *Held.* Precision cursor: reduced speed, magnetism off.
    Precision,
    /// *Held.* Queue modifier (shift): orders append, selection is additive.
    QueueModifier,
    /// *Held.* Command wheel; releasing confirms the highlighted slot.
    CommandWheel,
    /// *Held.* Build wheel for the selected builder.
    BuildWheel,
}

impl Action {
    /// `true` for actions that last while their input is held.
    pub fn is_held(self) -> bool {
        matches!(
            self,
            Action::Brush
                | Action::DragSelect
                | Action::Precision
                | Action::QueueModifier
                | Action::CommandWheel
                | Action::BuildWheel
                | Action::ZoomModifier
        )
    }
}

/// One binding. `with` names a chord modifier that must already be down when
/// `input` goes down (RB + D-pad, Ctrl + 1). A chorded binding wins over an
/// unchorded one, and the modifier's own tap/hold is suppressed once used.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// The input.
    pub input: Input,
    /// Chord modifier, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<Input>,
    /// Gesture.
    pub gesture: Gesture,
    /// Action.
    pub action: Action,
}

/// Why a profile was rejected.
#[derive(Debug, Error)]
pub enum ProfileError {
    /// Not valid JSON for a profile.
    #[error("profile JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Two bindings share input, modifier and gesture.
    #[error("binding {index}: duplicate trigger {input:?} with {with:?} on {gesture:?}")]
    Duplicate {
        /// Index of the second binding.
        index: usize,
        /// Input.
        input: Input,
        /// Modifier.
        with: Option<Input>,
        /// Gesture.
        gesture: Gesture,
    },
    /// A binding uses its own input as its modifier.
    #[error("binding {0}: input is its own chord modifier")]
    SelfChord(usize),
    /// A held action is bound to a gesture without a duration.
    #[error("binding {0}: held action {1:?} needs a press or hold gesture")]
    HeldOnInstantGesture(usize, Action),
    /// A wheel slot holds a held action.
    #[error("command wheel slot {0}: held action {1:?} cannot be a wheel slot")]
    HeldInWheel(usize, Action),
    /// More wheel slots than `tuning.wheel_max_slots`.
    #[error("command wheel has {0} slots, more than the limit {1}")]
    WheelTooLarge(usize, u8),
}

/// A complete input profile.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Display name.
    pub name: String,
    /// Feel constants (any subset may be given).
    #[serde(default)]
    pub tuning: Tuning,
    /// Bindings.
    pub bindings: Vec<Binding>,
    /// Command wheel slots, clockwise from the top.
    #[serde(default)]
    pub command_wheel: Vec<Action>,
    /// Command wheel with nothing selected, when `tuning.wheel_context` is
    /// on.
    #[serde(default = "default_idle_wheel")]
    pub idle_wheel: Vec<Action>,
}

fn default_idle_wheel() -> Vec<Action> {
    vec![
        Action::CycleIdleBuilder,
        Action::JumpToAlert,
        Action::CenterOnMonarch,
        Action::SelectAll {
            scope: Scope::OnScreen,
        },
        Action::SelectAll { scope: Scope::Map },
    ]
}

const CONTROLLER_JSON: &str = include_str!("profiles/controller.json");
const WP_J_CONTROLLER_JSON: &str = include_str!("profiles/wp_j_controller.json");
const MOUSE_KEYBOARD_JSON: &str = include_str!("profiles/mouse_keyboard.json");

impl Profile {
    /// Parse and validate.
    pub fn from_json(s: &str) -> Result<Profile, ProfileError> {
        let p: Profile = serde_json::from_str(s)?;
        p.validate()?;
        Ok(p)
    }

    /// Indented JSON.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Check the invariants [`Profile::from_json`] enforces.
    pub fn validate(&self) -> Result<(), ProfileError> {
        for (i, b) in self.bindings.iter().enumerate() {
            if b.with.as_ref() == Some(&b.input) {
                return Err(ProfileError::SelfChord(i));
            }
            if b.action.is_held() && !matches!(b.gesture, Gesture::Press | Gesture::Hold) {
                return Err(ProfileError::HeldOnInstantGesture(i, b.action));
            }
            if let Some(prev) = self.bindings[..i]
                .iter()
                .find(|p| p.input == b.input && p.with == b.with && p.gesture == b.gesture)
            {
                return Err(ProfileError::Duplicate {
                    index: i,
                    input: prev.input.clone(),
                    with: prev.with.clone(),
                    gesture: prev.gesture,
                });
            }
        }
        if self.command_wheel.len() > usize::from(self.tuning.wheel_max_slots) {
            return Err(ProfileError::WheelTooLarge(
                self.command_wheel.len(),
                self.tuning.wheel_max_slots,
            ));
        }
        if self.idle_wheel.len() > usize::from(self.tuning.wheel_max_slots) {
            return Err(ProfileError::WheelTooLarge(
                self.idle_wheel.len(),
                self.tuning.wheel_max_slots,
            ));
        }
        if let Some((i, a)) = self
            .command_wheel
            .iter()
            .chain(&self.idle_wheel)
            .enumerate()
            .find(|(_, a)| a.is_held())
        {
            return Err(ProfileError::HeldInWheel(i, *a));
        }
        Ok(())
    }

    /// The default gamepad profile (`src/profiles/controller.json`).
    pub fn default_controller() -> Profile {
        Profile::from_json(CONTROLLER_JSON).expect("embedded controller profile is valid")
    }

    /// The WP-J controller profile and tuning, unchanged (Y / double Y for
    /// same-type select, [`Tuning::wp_j`]). Kept for the WP-J tests and the
    /// feel lab's baseline.
    pub fn wp_j_controller() -> Profile {
        let mut p =
            Profile::from_json(WP_J_CONTROLLER_JSON).expect("embedded WP-J profile is valid");
        p.tuning = Tuning::wp_j();
        p
    }

    /// The default controller and mouse/keyboard profiles merged into one
    /// (what the client uses: both devices drive one mapper).
    pub fn default_combined() -> Profile {
        let mut p = Profile::default_controller();
        p.name = "controller+mouse_keyboard".into();
        p.bindings
            .extend(Profile::default_mouse_keyboard().bindings);
        p
    }

    /// The default mouse and keyboard profile
    /// (`src/profiles/mouse_keyboard.json`).
    pub fn default_mouse_keyboard() -> Profile {
        Profile::from_json(MOUSE_KEYBOARD_JSON).expect("embedded mouse/keyboard profile is valid")
    }

    /// The binding for `input` + `with` + `gesture`, if any.
    pub fn find(&self, input: &Input, with: Option<&Input>, gesture: Gesture) -> Option<Action> {
        self.bindings
            .iter()
            .find(|b| &b.input == input && b.with.as_ref() == with && b.gesture == gesture)
            .map(|b| b.action)
    }

    /// `true` if any binding for `input` uses modifier `with`.
    pub fn has_chord(&self, input: &Input, with: &Input) -> bool {
        self.bindings
            .iter()
            .any(|b| &b.input == input && b.with.as_ref() == Some(with))
    }
}
