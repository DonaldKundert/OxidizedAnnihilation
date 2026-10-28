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
    /// Attack-move to the current cursor immediately (console hold-X).
    AttackMove,
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
    /// Recall a control group (additive while the queue modifier is held).
    RecallGroup {
        /// Group.
        group: ControlGroup,
    },
    /// Add a control group to the selection without modifying the saved group.
    RecallGroupAdd {
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
    /// Next context order clears a reclaimable map feature or wreck.
    ClearMode,
    /// Next context order heals an owned unit or assists its construction.
    HealMode,
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
    /// append it immediately (five with the queue modifier), while mobile
    /// builders enter placement mode.
    BuildOption {
        /// Index into the builder's options.
        index: u8,
    },
    /// Continuously produce this option at eligible selected factories.
    /// Enables the recipe rather than toggling it; mobile placement is unchanged.
    RepeatBuildOption {
        /// Index into the first builder's options.
        index: u8,
    },
    /// Remove this build option from selected factories: one item, or five
    /// with the queue modifier, and stop its matching continuous recipe.
    /// Mobile builder placement is unchanged.
    CancelBuildOption {
        /// Index into the first builder's options.
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
    /// *Held.* Spell wheel: every spell of the selection's first caster
    /// (Kingdoms Console: hold Y).
    SpellWheel,
    /// Narrow the selection to one unit type at a time, cycling through the
    /// types in it; one more press after the last type restores the full
    /// selection (Halo Wars RT).
    SubSelectCycle,
    /// Select the local monarch (TA Ctrl+M); with `track`, the camera also
    /// follows it.
    SelectMonarch {
        /// Follow it with the camera.
        #[serde(default)]
        track: bool,
    },
    /// Zoom back to `zoom_default_milli` (Kingdoms Console: R3).
    ResetZoom,
    /// Camera to a squad's units without changing the selection.
    JumpToGroup {
        /// Squad.
        group: ControlGroup,
    },
    /// *Held.* Full-screen Crystal Ball (radar) while held.
    CrystalBall,
    /// Toggle the full-screen Crystal Ball (TA Tab).
    ToggleCrystalBall,
    /// Next context order attacks the unit or ground under the cursor
    /// (TA A, then click).
    AttackMode,
    /// Next context order attack-moves to the cursor (distinct from force-fire).
    AttackMoveMode,
    /// Next context order moves, even over a unit (TA M, then click).
    MoveMode,
    /// Select every own unit of a category on the map (TA Ctrl+letter;
    /// additive with the queue modifier, TA Ctrl+Shift+letter).
    SelectCategory {
        /// Which units.
        category: Category,
    },
    /// Store the camera position as bookmark `slot` (TA Ctrl+F5..F8).
    SetBookmark {
        /// Bookmark, 1..=4 in the original.
        slot: u8,
    },
    /// Camera to bookmark `slot` (TA F5..F8).
    GoToBookmark {
        /// Bookmark.
        slot: u8,
    },
    /// Camera to the next own unit that is off screen (TA N).
    NextUnit,
    /// Damage bars: all, selected only, off (TA ~).
    ToggleDamageBars,
    /// Game speed up or down (TA + / -). Changes ticks per wall second only.
    GameSpeed {
        /// Faster (`+`) or slower (`-`).
        faster: bool,
    },
    /// Pause or resume (TA Pause).
    TogglePause,
    /// Wheel slot: open a sub-ring (Stance, Unit, Squads).
    OpenRing {
        /// Which ring.
        ring: Ring,
    },
    /// A reserved slot or key: an action the design keeps a place for but
    /// the sim does not support yet (design system section 9). It does
    /// nothing and is hidden in the interface, so adding it later changes
    /// no layout.
    Reserved {
        /// What the slot is for.
        order: ReservedOrder,
    },
}

/// Unit categories for TA's select-category keys (`Keys.TDF`
/// `SelectUnits <CATEGORY>`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Mobile builders (TA Ctrl+B, `BUILDER`).
    Builder,
    /// Melee fighters (Ctrl+E, `MELEE`).
    Melee,
    /// Factories and other structures that produce units (Ctrl+F).
    Factory,
    /// Units with magic (Ctrl+G, `MAGIC`).
    Magic,
    /// The monarch (Ctrl+M).
    Monarch,
    /// Boats and sea monsters (Ctrl+N, `BOAT` / `NAVAL`).
    Naval,
    /// Ballistic weapons (Ctrl+R, `BALLISTIC`).
    Ballistic,
    /// Mobile armed units, not naval (Ctrl+T, `TROOPS`).
    Troops,
    /// Armed units except the monarch (Ctrl+W, `ATTACK`).
    Armed,
    /// Fliers (Ctrl+Y, `FLY`).
    Fly,
}

impl Category {
    /// The `Keys.TDF` category name.
    pub fn tdf_name(self) -> &'static str {
        match self {
            Category::Builder => "BUILDER",
            Category::Melee => "MELEE",
            Category::Factory => "FACTORY",
            Category::Magic => "MAGIC",
            Category::Monarch => "MONARCH",
            Category::Naval => "NAVAL",
            Category::Ballistic => "BALLISTIC",
            Category::Troops => "TROOPS",
            Category::Armed => "ATTACK",
            Category::Fly => "FLY",
        }
    }

    /// Parse a `Keys.TDF` category name (case-insensitive; `BOAT` is
    /// naval).
    pub fn from_tdf(s: &str) -> Option<Category> {
        Some(match s.trim().to_ascii_uppercase().as_str() {
            "BUILDER" => Category::Builder,
            "MELEE" => Category::Melee,
            "FACTORY" => Category::Factory,
            "MAGIC" => Category::Magic,
            "MONARCH" => Category::Monarch,
            "BOAT" | "NAVAL" => Category::Naval,
            "BALLISTIC" => Category::Ballistic,
            "TROOPS" => Category::Troops,
            "ATTACK" => Category::Armed,
            "FLY" => Category::Fly,
            _ => return None,
        })
    }
}

/// Sub-rings of the orders wheel.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ring {
    /// Offensive / Defensive / Passive.
    Stance,
    /// Unit-specific orders (Heal, Load, Unload, Cloak, Activate) and the
    /// squads ring.
    Unit,
    /// Squads 5..9 (recall; assign with the queue modifier held).
    Squads,
}

/// Orders the design reserves a place for but the sim cannot carry out yet
/// (design system section 9, plus TA's Clear). Legacy saved-profile tags
/// remain here when a supported action replaces their reserved slot.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservedOrder {
    /// Legacy reserved Clear slot; new profiles use [`Action::ClearMode`].
    Clear,
    /// Legacy reserved Heal / repair slot; new profiles use [`Action::HealMode`].
    Heal,
    /// Load into a transport (TA L).
    Load,
    /// Unload a transport (TA U).
    Unload,
    /// Pick up (transport button).
    Pickup,
    /// Drop (transport button).
    Drop,
    /// Cloak on / off (TA K).
    Cloak,
    /// Open / activate (TA O).
    Activate,
    /// Explicit attack-move.
    AttackMove,
    /// Formations (TA Alt+1..9).
    Formation,
    /// Dismiss / self-destruct (TA Ctrl+D).
    Dismiss,
    /// Persistent weapon select (Primary / Secondary / Special).
    WeaponSelect,
    /// Not an order: an empty position on a wheel page.
    Blank,
}

impl ReservedOrder {
    /// The order's TA:K name.
    pub fn label(self) -> &'static str {
        match self {
            ReservedOrder::Clear => "Clear",
            ReservedOrder::Heal => "Heal",
            ReservedOrder::Load => "Load",
            ReservedOrder::Unload => "Unload",
            ReservedOrder::Pickup => "Pickup",
            ReservedOrder::Drop => "Drop",
            ReservedOrder::Cloak => "Cloak",
            ReservedOrder::Activate => "Activate",
            ReservedOrder::AttackMove => "Attack-move",
            ReservedOrder::Formation => "Formation",
            ReservedOrder::Dismiss => "Dismiss",
            ReservedOrder::WeaponSelect => "Weapon select",
            ReservedOrder::Blank => "",
        }
    }
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
                | Action::SpellWheel
                | Action::ZoomModifier
                | Action::CrystalBall
        )
    }

    /// `true` for the held modifiers (queue, zoom, precision): pressing
    /// another input while one is down uses it, so its own tap is dropped.
    pub fn is_modifier(self) -> bool {
        matches!(
            self,
            Action::QueueModifier | Action::ZoomModifier | Action::Precision
        )
    }

    /// `false` for [`Action::Reserved`]: the interface hides it.
    pub fn is_supported(self) -> bool {
        !matches!(self, Action::Reserved { .. })
    }
}

/// One binding. `with` names a chord modifier that must already be down when
/// `input` goes down (RB + D-pad, Ctrl + 1). A chorded binding wins over an
/// unchorded one. `with_all` allows additional modifiers; the most-specific
/// satisfied chord wins. Every used modifier's own tap/hold is suppressed.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// The input.
    pub input: Input,
    /// Chord modifier, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<Input>,
    /// Additional chord modifiers. All must be held; more-specific chords
    /// win over less-specific ones. Existing `with` bindings remain valid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub with_all: Vec<Input>,
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
    /// The same modifier appears more than once in a chord.
    #[error("binding {0}: duplicate chord modifier")]
    DuplicateModifier(usize),
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

impl Binding {
    /// Required modifiers in canonical order, including the legacy `with`.
    pub fn modifiers(&self) -> Vec<Input> {
        let mut modifiers: Vec<_> = self.with.iter().chain(&self.with_all).cloned().collect();
        modifiers.sort();
        modifiers
    }
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
    /// Wheel slots keep fixed positions: a slot the selection cannot use
    /// stays in place, empty, instead of closing the gap (design system
    /// "spatial memory"). Build and spell wheels show 8 per page.
    #[serde(default, skip_serializing_if = "is_false")]
    pub fixed_slots: bool,
    /// Optional "Grid" build hotkeys: with a builder selected, Q W E R /
    /// A S D F / Z X C V (no modifier) pick build options 1..12 instead of
    /// their normal bindings. Off by default (TA:K had none).
    #[serde(default, skip_serializing_if = "is_false")]
    pub grid_build_keys: bool,
    /// Original-style primary mouse clicks: [`Action::DragSelect`] still
    /// drags a selection, but a click selects an own unit or gives the
    /// context order over ground / other players' units. Off by default;
    /// [`Profile::original_tak`] enables it without changing the pad.
    #[serde(default, skip_serializing_if = "is_false")]
    pub left_click_context: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The Grid add-on's keys, in build-option order.
pub const GRID_KEYS: [&str; 12] = ["q", "w", "e", "r", "a", "s", "d", "f", "z", "x", "c", "v"];

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

const KINGDOMS_CONSOLE_JSON: &str = include_str!("profiles/kingdoms_console.json");
const CLASSIC_TAK_JSON: &str = include_str!("profiles/classic_tak.json");
const WP_S_CONTROLLER_JSON: &str = include_str!("profiles/wp_s_controller.json");
const WP_J_CONTROLLER_JSON: &str = include_str!("profiles/wp_j_controller.json");
const WP_S_MOUSE_KEYBOARD_JSON: &str = include_str!("profiles/wp_s_mouse_keyboard.json");

/// Names accepted by [`Profile::named`] (control schemes), default first.
pub const CONTROL_SCHEMES: [&str; 4] = ["kingdoms", "original", "wp_s", "wp_j"];

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
            let modifiers = b.modifiers();
            if modifiers.contains(&b.input) {
                return Err(ProfileError::SelfChord(i));
            }
            if modifiers.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(ProfileError::DuplicateModifier(i));
            }
            if b.action.is_held() && !matches!(b.gesture, Gesture::Press | Gesture::Hold) {
                return Err(ProfileError::HeldOnInstantGesture(i, b.action));
            }
            if let Some(prev) = self.bindings[..i].iter().find(|p| {
                p.input == b.input && p.modifiers() == modifiers && p.gesture == b.gesture
            }) {
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

    /// The default gamepad profile: "Kingdoms Console" (design system
    /// section 4, `src/profiles/kingdoms_console.json`), reticle mode.
    pub fn default_controller() -> Profile {
        Profile::kingdoms_console()
    }

    /// "Kingdoms Console": Halo Wars' controller scheme on TA:K's orders.
    pub fn kingdoms_console() -> Profile {
        Profile::from_json(KINGDOMS_CONSOLE_JSON).expect("embedded Kingdoms Console profile")
    }

    /// The WP-S controller profile (free cursor, Y local/global select, RB
    /// zoom modifier), kept selectable for comparison.
    pub fn wp_s_controller() -> Profile {
        Profile::from_json(WP_S_CONTROLLER_JSON).expect("embedded WP-S profile is valid")
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
    /// (what the client uses: both devices drive one mapper): Kingdoms
    /// Console plus Classic TA:K.
    pub fn default_combined() -> Profile {
        Profile::combine(
            Profile::kingdoms_console(),
            Profile::classic_tak(),
            "Kingdoms Console + Classic TA:K",
        )
    }

    /// The WP-S controller and mouse/keyboard profiles merged (the previous
    /// default).
    pub fn wp_s_combined() -> Profile {
        Profile::combine(
            Profile::wp_s_controller(),
            Profile::wp_s_mouse_keyboard(),
            "controller+mouse_keyboard",
        )
    }

    /// `pad`'s tuning and wheels with `keys`' bindings appended.
    pub fn combine(pad: Profile, keys: Profile, name: &str) -> Profile {
        let mut p = pad;
        p.name = name.into();
        p.grid_build_keys |= keys.grid_build_keys;
        p.left_click_context |= keys.left_click_context;
        p.bindings.extend(keys.bindings);
        p
    }

    /// A control scheme by name ([`CONTROL_SCHEMES`]), controller and
    /// keyboard merged: `kingdoms` (default), `original` (original-style
    /// left-click orders), `wp_s`, `wp_j` (the WP-J pad with the WP-S keyboard).
    pub fn named(name: &str) -> Option<Profile> {
        match name.to_ascii_lowercase().replace('-', "_").as_str() {
            "kingdoms" | "kingdoms_console" | "default" => Some(Profile::default_combined()),
            "original" | "original_tak" => Some(Profile::combine(
                Profile::kingdoms_console(),
                Profile::original_tak(),
                "Kingdoms Console + Original TA:K",
            )),
            "wp_s" | "wps" => Some(Profile::wp_s_combined()),
            "wp_j" | "wpj" => Some(Profile::combine(
                Profile::wp_j_controller(),
                Profile::wp_s_mouse_keyboard(),
                "wp_j+mouse_keyboard",
            )),
            _ => None,
        }
    }

    /// The default mouse and keyboard profile: "Classic TA:K" (the
    /// original's default keys, `src/profiles/classic_tak.json`).
    pub fn default_mouse_keyboard() -> Profile {
        Profile::classic_tak()
    }

    /// "Classic TA:K": TA:K's default keys with left-select / right-order
    /// mouse controls, retained as the default for existing players.
    pub fn classic_tak() -> Profile {
        Profile::from_json(CLASSIC_TAK_JSON).expect("embedded Classic TA:K profile is valid")
    }

    /// "Original TA:K": Classic keys with optional original-style primary
    /// contextual clicks. Own units select; ground moves; enemies attack.
    /// Drag selection, explicit order modes and right-click orders remain.
    pub fn original_tak() -> Profile {
        let mut p = Profile::classic_tak();
        p.name = "Original TA:K".into();
        p.left_click_context = true;
        p
    }

    /// The WP-S mouse and keyboard profile, kept selectable.
    pub fn wp_s_mouse_keyboard() -> Profile {
        Profile::from_json(WP_S_MOUSE_KEYBOARD_JSON)
            .expect("embedded WP-S mouse/keyboard profile is valid")
    }

    /// The binding for one legacy modifier and gesture, if any.
    pub fn find(&self, input: &Input, with: Option<&Input>, gesture: Gesture) -> Option<Action> {
        self.find_chord(
            input,
            &with.into_iter().cloned().collect::<Vec<_>>(),
            gesture,
        )
    }

    /// The binding for a canonical, sorted set of chord modifiers.
    pub fn find_chord(
        &self,
        input: &Input,
        modifiers: &[Input],
        gesture: Gesture,
    ) -> Option<Action> {
        self.bindings
            .iter()
            .find(|b| &b.input == input && b.modifiers() == modifiers && b.gesture == gesture)
            .map(|b| b.action)
    }

    /// Most-specific satisfied chord, with deterministic lexical tie-breaking.
    pub fn held_chord(&self, input: &Input, held: impl Fn(&Input) -> bool) -> Vec<Input> {
        let mut chords: Vec<_> = self
            .bindings
            .iter()
            .filter(|b| &b.input == input)
            .map(Binding::modifiers)
            .filter(|modifiers| modifiers.iter().all(&held))
            .collect();
        chords.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        chords.into_iter().next().unwrap_or_default()
    }

    /// `true` if any binding for `input` uses modifier `with`.
    pub fn has_chord(&self, input: &Input, with: &Input) -> bool {
        self.bindings
            .iter()
            .any(|b| &b.input == input && b.modifiers().contains(with))
    }
}
