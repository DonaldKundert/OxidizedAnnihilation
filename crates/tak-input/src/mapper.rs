//! The stateful mapper: device events in, ordered commands out.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tak_commands::{
    ClientCommand, ControlGroup, QueueMode, Scope, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
    WorldPos,
};
use tak_math::{Fx, Vec2};

use crate::binding::{Action, Category, GRID_KEYS, Gesture, Profile, Ring};
use crate::brush::{brush_radius, drag_circle, units_in_circle};
use crate::context::{Mode, builders_for, healers_for, own_units, pick_unit, resolve_context};
use crate::cursor::{
    Motion, approach_frac, cursor_motion, friction, pct, per_second, soft_snap, stick_vector,
    stick_vector_zones,
};
use crate::event::{Axis, EventKind, Input, InputEvent};
use crate::tuning::Tuning;
use crate::wheel::{
    Wheel, WheelKind, context_slots, context_slots_fixed, expand_spells, ring_slots,
};
use crate::world::{BuilderKind, Camera, UnitInfo, WorldView, fx_int, units_sorted};

/// One thing the mapper produced, in order.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "out", rename_all = "snake_case")]
pub enum Output {
    /// A client action. Selection commands have already been applied to the
    /// mapper's own selection; the client mirrors them (and moves the camera
    /// for `JumpToAlert`, `CycleIdleBuilder`, `CenterOnMonarch`).
    Client {
        /// The command.
        command: ClientCommand,
    },
    /// An order for the sim, stamped with the tick the caller passed in.
    Sim {
        /// Caller's tick.
        tick: Tick,
        /// The order.
        command: SimCommand,
    },
    /// Local or global select ([`Action::SelectAll`]); already applied to
    /// the mapper's selection.
    SelectAll {
        /// On screen or whole map.
        scope: Scope,
        /// The new selection.
        units: UnitSet,
    },
    /// Open or close the client's tuning overlay.
    ToggleOverlay,
    /// Game speed up or down (client: ticks per wall second only).
    GameSpeed {
        /// Faster or slower.
        faster: bool,
    },
    /// Pause or resume (client).
    TogglePause,
    /// Cycle the damage bars: all, selected only, off (client).
    ToggleDamageBars,
}

/// Why an input did nothing (shown with a reason, design system section 7).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The order needs own units selected.
    NothingSelected,
    /// No selected unit can carry out the order.
    CannotDo,
    /// The caster's mana is short.
    NoMana,
    /// The order is reserved: the sim does not support it yet.
    Unsupported,
    /// Nothing to jump to (no squad, bookmark, idle builder, alert).
    NothingThere,
}

impl Refusal {
    /// Bottom-bar text.
    pub fn reason(self) -> &'static str {
        match self {
            Refusal::NothingSelected => "Nothing selected",
            Refusal::CannotDo => "The selection cannot do that",
            Refusal::NoMana => "Not enough mana",
            Refusal::Unsupported => "Not available yet",
            Refusal::NothingThere => "Nothing there",
        }
    }
}

/// The selection's sub-select cycle (Halo Wars RT): the full selection and
/// the unit type currently narrowed to.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SubSelect {
    /// The selection before narrowing.
    pub full: UnitSet,
    /// The types in it, in order of first appearance by unit id.
    pub types: Vec<UnitDefId>,
    /// Index into `types` of the active type.
    pub index: usize,
}

/// Something the player should feel or see (rumble, flashes). Drained with
/// [`Mapper::take_feedback`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Feedback {
    /// The cursor snapped onto a new target.
    Snap,
    /// The selection changed (to `count` units).
    Selection {
        /// Units now selected.
        count: u32,
    },
    /// An order went to the sim.
    Order,
    /// An input was refused.
    Refused {
        /// Why.
        reason: Refusal,
    },
}

/// What the cursor would do here, for the game's context cursors.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorKind {
    /// Nothing to do.
    Normal,
    /// Over an own unit that a click selects.
    Select,
    /// The context order would move.
    Move,
    /// The context order would attack.
    Attack,
    /// The context order would guard.
    Guard,
    /// Heal an owned unit or assist its construction.
    Heal,
    /// Patrol mode.
    Patrol,
    /// Build mode, placeable here.
    Build,
    /// Build mode, not placeable here.
    BuildBlocked,
    /// Spell mode.
    Cast,
    /// The pending order cannot be given here.
    Invalid,
}

/// Bookmarks the mapper keeps (TA has four, F5..F8).
pub const BOOKMARKS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CursorSource {
    Stick,
    Pointer,
}

#[derive(Clone, Debug)]
struct Press {
    at: u32,
    with: Option<Input>,
    /// Used as a chord modifier or completed a double tap: no tap or hold.
    consumed: bool,
    hold_fired: bool,
}

#[derive(Clone, Debug)]
struct PendingTap {
    released_at: u32,
    with: Option<Input>,
}

#[derive(Clone, Debug, Default)]
struct InputState {
    press: Option<Press>,
    pending: Option<PendingTap>,
    /// Release time of the last tap that already fired, for `second_tap`.
    last_tap: Option<PendingTap>,
}

#[derive(Clone, Debug)]
struct BrushState {
    input: Input,
    since: u32,
    anchor: WorldPos,
    last_stamp: Option<WorldPos>,
}

#[derive(Clone, Debug)]
struct DragState {
    input: Input,
    start_world: WorldPos,
    start_screen: Vec2,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TimerKind {
    Hold,
    TapExpired,
}

/// Turns [`InputEvent`]s into [`Output`]s for one local player.
///
/// Owns the client-side input state: virtual cursor, selection, control
/// groups, pending order mode, open wheel, brush, camera motion requests. It
/// reads the world only through [`WorldView`] and never mutates sim state;
/// sim orders come out as [`Output::Sim`] for the caller to submit.
#[derive(Clone, Debug)]
pub struct Mapper {
    profile: Profile,
    tuning: Tuning,
    camera: Camera,
    now: u32,
    last_frame: Option<u32>,
    inputs: BTreeMap<Input, InputState>,
    left: (i16, i16),
    right: (i16, i16),
    cursor: Vec2,
    source: CursorSource,
    pan: Vec2,
    selection: UnitSet,
    groups: [UnitSet; ControlGroup::COUNT as usize],
    mode: Mode,
    last_idle: Option<UnitId>,
    wheel: Option<(Input, Wheel)>,
    /// The open wheel stays open after its button is released.
    wheel_sticky: bool,
    /// Every item of a paged wheel (build, spell), for page turns.
    wheel_items: Vec<Action>,
    /// Parent rings of an open sub-ring (cancel goes back one level).
    wheel_parents: Vec<Wheel>,
    sub_select: Option<SubSelect>,
    crystal_by: Vec<Input>,
    crystal_toggled: bool,
    bookmarks: [Option<WorldPos>; BOOKMARKS],
    last_next_unit: Option<UnitId>,
    /// Reticle-mode camera velocity (world units per second).
    ret_vel: Vec2,
    brush: Option<BrushState>,
    drag: Option<DragState>,
    precision_by: Vec<Input>,
    queue_by: Vec<Input>,
    zoom_by: Vec<Input>,
    // ---- aim assist ----
    /// Assist targets (unit, screen position), sorted by unit id; refreshed
    /// at the start of every [`Mapper::handle`].
    targets: Vec<(UnitId, Vec2)>,
    snap_target: Option<UnitId>,
    accel_since: Option<u32>,
    // ---- camera ----
    pan_vel: Vec2,
    edge_since: Option<u32>,
    edge_active: bool,
    zoom_goal: Option<Fx>,
    zoom_out: Option<Fx>,
    glide: Option<WorldPos>,
    follow: bool,
    lock_world: bool,
    // ---- feedback ----
    feedback: Vec<Feedback>,
    selected_at: Option<u32>,
}

/// Per-call context: world, caller's tick, output sink.
struct Cx<'a, W: ?Sized> {
    world: &'a W,
    tick: Tick,
    out: &'a mut Vec<Output>,
}

impl Mapper {
    /// A mapper for `profile` with the default camera; the cursor starts at
    /// the viewport centre.
    pub fn new(profile: Profile) -> Mapper {
        let camera = Camera::default();
        Mapper {
            tuning: profile.tuning,
            profile,
            cursor: camera.center_screen(),
            camera,
            now: 0,
            last_frame: None,
            inputs: BTreeMap::new(),
            left: (0, 0),
            right: (0, 0),
            source: CursorSource::Stick,
            pan: Vec2::ZERO,
            selection: UnitSet::new(),
            groups: Default::default(),
            mode: Mode::None,
            last_idle: None,
            wheel: None,
            wheel_sticky: false,
            wheel_items: Vec::new(),
            wheel_parents: Vec::new(),
            sub_select: None,
            crystal_by: Vec::new(),
            crystal_toggled: false,
            bookmarks: [None; BOOKMARKS],
            last_next_unit: None,
            ret_vel: Vec2::ZERO,
            brush: None,
            drag: None,
            precision_by: Vec::new(),
            queue_by: Vec::new(),
            zoom_by: Vec::new(),
            targets: Vec::new(),
            snap_target: None,
            accel_since: None,
            pan_vel: Vec2::ZERO,
            edge_since: None,
            edge_active: false,
            zoom_goal: None,
            zoom_out: None,
            glide: None,
            follow: false,
            lock_world: false,
            feedback: Vec::new(),
            selected_at: None,
        }
    }

    /// The profile in use.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// The tuning in use.
    pub fn tuning(&self) -> &Tuning {
        &self.tuning
    }

    /// Swap the profile at runtime (hot reload). Selection, groups, cursor
    /// and camera state are kept; gestures in progress are dropped.
    pub fn set_profile(&mut self, profile: Profile) {
        self.tuning = profile.tuning;
        self.profile = profile;
        self.inputs.clear();
        self.wheel = None;
        self.wheel_sticky = false;
        self.wheel_parents.clear();
        self.brush = None;
        self.drag = None;
        self.precision_by.clear();
        self.queue_by.clear();
        self.zoom_by.clear();
        self.crystal_by.clear();
    }

    /// Change only the tuning (live tuning overlay, presets).
    pub fn set_tuning(&mut self, tuning: Tuning) {
        self.tuning = tuning;
        self.profile.tuning = tuning;
    }

    /// Replace the camera (the client calls this when its view changes).
    /// After the mapper itself panned with the right stick or zoomed, the
    /// stick cursor keeps its ground point (`cursor_world_lock`).
    pub fn set_camera(&mut self, camera: Camera) {
        if self.reticle_active() {
            self.lock_world = false;
            self.camera = camera;
            self.cursor = camera.center_screen();
            return;
        }
        if self.lock_world
            && self.tuning.cursor_world_lock
            && self.source == CursorSource::Stick
            && camera != self.camera
        {
            let w = self.camera.screen_to_world(self.cursor);
            self.cursor = camera.world_to_screen(w);
        }
        self.lock_world = false;
        self.camera = camera;
        self.cursor = camera.clamp_screen(self.cursor);
    }

    /// Current camera.
    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// Mapper clock: the latest event time, ms.
    pub fn now(&self) -> u32 {
        self.now
    }

    /// Cursor in screen pixels.
    pub fn cursor_screen(&self) -> Vec2 {
        self.cursor
    }

    /// Put the stick cursor at a screen point (clamped), e.g. after the
    /// client warps it or for tests and the feel lab.
    pub fn warp_cursor(&mut self, s: Vec2) {
        self.source = CursorSource::Stick;
        self.cursor = self.camera.clamp_screen(s);
        self.snap_target = None;
    }

    /// Cursor on the ground. In reticle mode this is the point under the
    /// screen centre, including camera motion not yet taken by the client.
    pub fn cursor_world(&self) -> WorldPos {
        if self.reticle_active() {
            return sat_add(
                self.camera.screen_to_world(self.camera.center_screen()),
                self.pan,
            );
        }
        self.camera.screen_to_world(self.cursor)
    }

    /// `true` while the reticle drives selection and orders: reticle mode
    /// is on and the stick (not the mouse) moved last.
    pub fn reticle_active(&self) -> bool {
        self.tuning.reticle && self.source == CursorSource::Stick
    }

    /// The cursor in screen pixels including pending camera motion (reticle
    /// mode: the centre, shifted by the pan not yet applied).
    fn eff_cursor(&self) -> Vec2 {
        if !self.reticle_active() {
            return self.cursor;
        }
        let c = self.camera.center_screen();
        let wpp = self.camera.world_per_px;
        if wpp <= Fx::ZERO {
            return c;
        }
        Vec2::new(
            c.x.saturating_add(self.pan.x.saturating_div(wpp)),
            c.y.saturating_add(self.pan.y.saturating_div(wpp)),
        )
    }

    /// The sub-select cycle in progress, if any.
    pub fn sub_select(&self) -> Option<&SubSelect> {
        self.sub_select.as_ref()
    }

    /// `true` while the full-screen Crystal Ball is shown (held or toggled).
    pub fn crystal_ball(&self) -> bool {
        self.crystal_toggled || !self.crystal_by.is_empty()
    }

    /// Bookmark `slot` (camera centre), if set.
    pub fn bookmark(&self, slot: u8) -> Option<WorldPos> {
        self.bookmarks.get(usize::from(slot)).copied().flatten()
    }

    /// How many rings deep the open wheel is (0: a top-level wheel).
    pub fn wheel_depth(&self) -> usize {
        self.wheel_parents.len()
    }

    /// `true` when the stick (not the mouse) drives the cursor, so the
    /// client draws the virtual cursor.
    pub fn stick_cursor(&self) -> bool {
        self.source == CursorSource::Stick
    }

    /// Current selection (may hold units that died since; orders filter them).
    pub fn selection(&self) -> &UnitSet {
        &self.selection
    }

    /// Milliseconds since the selection last changed, while within
    /// `select_flash_ms` (for the selection ring flash).
    pub fn selection_flash(&self) -> Option<u32> {
        let at = self.selected_at?;
        let age = self.now.saturating_sub(at);
        (age < self.tuning.select_flash_ms).then_some(age)
    }

    /// Pending order mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The open wheel, if any.
    pub fn wheel(&self) -> Option<&Wheel> {
        self.wheel.as_ref().map(|(_, w)| w)
    }

    /// The unit the cursor is snapping to, if any.
    pub fn snap_target(&self) -> Option<UnitId> {
        self.snap_target
    }

    /// `true` while the camera follows the selection.
    pub fn following(&self) -> bool {
        self.follow
    }

    /// Active brush as `(centre, radius)`, for drawing.
    pub fn brush(&self) -> Option<(WorldPos, Fx)> {
        self.brush.as_ref().map(|b| {
            (
                self.cursor_world(),
                brush_radius(self.now.saturating_sub(b.since), &self.tuning),
            )
        })
    }

    /// `true` while precision mode is held.
    pub fn precision(&self) -> bool {
        !self.precision_by.is_empty()
    }

    /// Camera pan requested since the last call, in world units. The client
    /// applies it to its camera and calls [`Mapper::set_camera`].
    pub fn take_camera_pan(&mut self) -> Vec2 {
        core::mem::replace(&mut self.pan, Vec2::ZERO)
    }

    /// New zoom (world units per pixel) requested since the last call: mouse
    /// wheel or RB + right stick, smoothed and clamped. The client applies
    /// it about the screen centre and calls [`Mapper::set_camera`].
    pub fn take_camera_zoom(&mut self) -> Option<Fx> {
        self.zoom_out.take()
    }

    /// Feedback since the last call (rumble, flashes).
    pub fn take_feedback(&mut self) -> Vec<Feedback> {
        core::mem::take(&mut self.feedback)
    }

    /// Move the camera so `pos` is at the screen centre: glides over the
    /// next frames (`glide_ms`), or jumps through [`Mapper::take_camera_pan`]
    /// when `glide_ms` is 0. Used for recentre, idle builder and alerts.
    pub fn glide_to(&mut self, pos: WorldPos) {
        if self.tuning.glide_ms == 0 {
            let centre = self.camera.screen_to_world(self.camera.center_screen());
            self.pan = pos - centre;
            self.glide = None;
        } else {
            self.glide = Some(pos);
        }
    }

    /// The context cursor for the current cursor position, mode and
    /// selection.
    pub fn cursor_kind<W: WorldView + ?Sized>(&self, world: &W) -> CursorKind {
        let pos = self.cursor_world();
        let hovered = pick_unit(world, pos);
        let me = world.local_player();
        let units = own_units(world, &self.selection);
        match self.mode {
            Mode::Build(def) => {
                if world.can_place(def, pos) {
                    CursorKind::Build
                } else {
                    CursorKind::BuildBlocked
                }
            }
            Mode::Patrol => CursorKind::Patrol,
            Mode::Guard => match hovered {
                Some(h) if !world.is_enemy(h.owner) => CursorKind::Guard,
                _ => CursorKind::Invalid,
            },
            Mode::Heal => {
                if resolve_context(
                    world,
                    &units,
                    pos,
                    hovered.as_ref(),
                    Mode::Heal,
                    QueueMode::Replace,
                )
                .is_some()
                {
                    CursorKind::Heal
                } else {
                    CursorKind::Invalid
                }
            }
            Mode::CastSpell(_) => CursorKind::Cast,
            Mode::Attack | Mode::AttackMove => CursorKind::Attack,
            Mode::Move => CursorKind::Move,
            Mode::None if units.is_empty() => match hovered {
                Some(h) if h.owner == me => CursorKind::Select,
                _ => CursorKind::Normal,
            },
            Mode::None => match hovered {
                Some(h) if world.is_enemy(h.owner) => CursorKind::Attack,
                Some(h) if units.contains(h.id) => CursorKind::Select,
                Some(_) => CursorKind::Guard,
                None => CursorKind::Move,
            },
        }
    }

    /// Feed one event. `tick` is the sim tick to stamp any orders with.
    /// Events must be in time order; an earlier `t` is treated as "now".
    pub fn handle<W: WorldView + ?Sized>(
        &mut self,
        event: &InputEvent,
        world: &W,
        tick: Tick,
    ) -> Vec<Output> {
        let mut out = Vec::new();
        self.targets = self.assist_targets(world);
        let mut cx = Cx {
            world,
            tick,
            out: &mut out,
        };
        let t = event.t.max(self.now);
        self.advance_to(t, &mut cx);
        match &event.kind {
            EventKind::ButtonDown { button } => self.down(Input::Button(*button), t, &mut cx),
            EventKind::ButtonUp { button } => self.up(Input::Button(*button), t, &mut cx),
            EventKind::KeyDown { key } => self.down(Input::Key(key.clone()), t, &mut cx),
            EventKind::KeyUp { key } => self.up(Input::Key(key.clone()), t, &mut cx),
            EventKind::Axis { axis, value } => self.axis(*axis, *value),
            EventKind::PointerMove { x, y } => {
                self.source = CursorSource::Pointer;
                self.cursor = self.camera.clamp_screen(Vec2::new(fx_int(*x), fx_int(*y)));
            }
            EventKind::Scroll { milli } => self.scroll(*milli),
            EventKind::Frame => self.frame(t, &mut cx),
        }
        out
    }

    /// Carry out one instant action as if its binding fired (HUD buttons:
    /// every surface reaches the same action). Held actions are ignored.
    pub fn perform<W: WorldView + ?Sized>(
        &mut self,
        action: Action,
        world: &W,
        tick: Tick,
    ) -> Vec<Output> {
        let mut out = Vec::new();
        if action.is_held() {
            return out;
        }
        self.targets = self.assist_targets(world);
        let mut cx = Cx {
            world,
            tick,
            out: &mut out,
        };
        self.instant(action, &mut cx);
        out
    }

    /// Feed many events at one tick (convenience for tests and replays).
    pub fn handle_all<W: WorldView + ?Sized>(
        &mut self,
        events: &[InputEvent],
        world: &W,
        tick: Tick,
    ) -> Vec<Output> {
        events
            .iter()
            .flat_map(|e| self.handle(e, world, tick))
            .collect()
    }

    // ---- aim assist ----------------------------------------------------

    /// Assist targets by context (`assist_priority`): in build mode the own
    /// frames under construction; with own units selected the enemies; with
    /// nothing selected the own units. Otherwise every pickable unit. Only
    /// units on screen, sorted by id.
    fn assist_targets<W: WorldView + ?Sized>(&self, world: &W) -> Vec<(UnitId, Vec2)> {
        let t = &self.tuning;
        let assist = t.magnet_radius_px > 0 || t.friction_pct < 100;
        if !assist {
            return Vec::new();
        }
        let me = world.local_player();
        let has_units = !self.selection.is_empty() && !own_units(world, &self.selection).is_empty();
        let healers = if self.mode == Mode::Heal {
            healers_for(world, &self.selection)
        } else {
            UnitSet::new()
        };
        let keep = |u: &UnitInfo| -> bool {
            if !u.selectable {
                return false;
            }
            if !t.assist_priority {
                return true;
            }
            match self.mode {
                Mode::Build(_) => u.owner == me && u.under_construction,
                Mode::Patrol => false,
                Mode::Guard => u.owner == me,
                Mode::Heal => u.owner == me && healers.iter().any(|&id| id != u.id),
                Mode::CastSpell(_) | Mode::Attack => true,
                Mode::Move | Mode::AttackMove => false,
                Mode::None if has_units => world.is_enemy(u.owner),
                Mode::None => u.owner == me,
            }
        };
        units_sorted(world)
            .into_iter()
            .filter(|u| keep(u) && self.camera.contains_world(u.pos))
            .map(|u| (u.id, self.camera.world_to_screen(u.pos)))
            .collect()
    }

    fn target_points(&self) -> Vec<Vec2> {
        self.targets.iter().map(|&(_, p)| p).collect()
    }

    // ---- time ----------------------------------------------------------

    fn advance_to<W: WorldView + ?Sized>(&mut self, t: u32, cx: &mut Cx<'_, W>) {
        while let Some((at, input, kind)) = self.next_timer() {
            if at > t {
                break;
            }
            self.integrate(at);
            self.fire_timer(input, kind, at, cx);
        }
        self.integrate(t);
    }

    fn next_timer(&self) -> Option<(u32, Input, TimerKind)> {
        let mut best: Option<(u32, Input, TimerKind)> = None;
        for (input, st) in &self.inputs {
            let mut consider = |at: u32, kind| {
                if best.as_ref().is_none_or(|(b, _, _)| at < *b) {
                    best = Some((at, input.clone(), kind));
                }
            };
            if let Some(p) = st.press.as_ref().filter(|p| {
                !p.consumed
                    && !p.hold_fired
                    && self
                        .profile
                        .find(input, p.with.as_ref(), Gesture::Hold)
                        .is_some()
            }) {
                consider(p.at.saturating_add(self.tuning.hold_ms), TimerKind::Hold);
            }
            if let Some(p) = &st.pending {
                consider(
                    p.released_at
                        .saturating_add(self.tuning.double_tap_ms)
                        .saturating_add(1),
                    TimerKind::TapExpired,
                );
            }
        }
        best
    }

    fn fire_timer<W: WorldView + ?Sized>(
        &mut self,
        input: Input,
        kind: TimerKind,
        at: u32,
        cx: &mut Cx<'_, W>,
    ) {
        let st = self.inputs.entry(input.clone()).or_default();
        match kind {
            TimerKind::Hold => {
                let Some(p) = st.press.as_mut() else { return };
                p.hold_fired = true;
                let with = p.with.clone();
                if let Some(a) = self.profile.find(&input, with.as_ref(), Gesture::Hold) {
                    self.trigger(&input, a, at, cx);
                }
            }
            TimerKind::TapExpired => {
                if let Some(p) = st.pending.take() {
                    self.fire_tap(&input, p.with.as_ref(), at, cx);
                }
            }
        }
    }

    /// `true` when cursor and camera motion need short substeps (friction,
    /// acceleration, smoothing and edge ramps depend on the path); the WP-J
    /// rules integrate exactly in one step.
    fn needs_substeps(&self) -> bool {
        let t = &self.tuning;
        t.friction_pct < 100
            || t.cursor_accel_pct > 0
            || t.pan_smooth_ms > 0
            || t.edge_pan_px > 0
            || (t.reticle && t.reticle_smooth_ms > 0)
    }

    /// Move the cursor and camera for the time since the last event, with
    /// the stick values that held over that interval.
    fn integrate(&mut self, to: u32) {
        let dt = to.saturating_sub(self.now);
        if dt == 0 {
            self.now = self.now.max(to);
            return;
        }
        if self.needs_substeps() {
            let mut left = dt;
            while left > 0 {
                let step = left.min(16);
                self.integrate_step(step);
                left -= step;
            }
        } else {
            self.integrate_step(dt);
        }
    }

    fn integrate_step(&mut self, dt: u32) {
        let start = self.now;
        self.now = self.now.saturating_add(dt);
        let t = self.tuning;
        let stick = self.left_stick();
        let reticle = self.reticle_active();
        if reticle {
            self.cursor = self.camera.center_screen();
        }
        // Cursor (free mode) or camera under the reticle (reticle mode).
        let mut ret_target = Vec2::ZERO;
        if self.source == CursorSource::Stick && self.wheel.is_none() && stick != Vec2::ZERO {
            let threshold =
                Fx::from_ratio(t.cursor_accel_threshold_pct.clamp(1, 100), 100).unwrap_or(Fx::ONE);
            let accel = if t.cursor_accel_pct > 0 && stick.length() >= threshold {
                let since = *self.accel_since.get_or_insert(start);
                ramp(self.now.saturating_sub(since), t.cursor_accel_ms)
            } else {
                self.accel_since = None;
                Fx::ZERO
            };
            let fr = if self.precision() || t.friction_pct >= 100 {
                Fx::ONE
            } else {
                friction(self.eff_cursor(), &self.target_points(), &t)
            };
            let motion = Motion {
                precision: self.precision() && !reticle,
                world_per_px: self.camera.world_per_px,
                accel,
                friction: fr,
            };
            if reticle {
                // The camera moves under the fixed reticle: the same speed
                // law as the free cursor, in ground units; precision is fast
                // pan instead.
                let fast = if self.precision() {
                    Fx::from_ratio(t.fast_pan_pct.clamp(100, 1000), 100).unwrap_or(Fx::ONE)
                } else {
                    Fx::ONE
                };
                let wpp = self.camera.world_per_px;
                if t.reticle_smooth_ms == 0 {
                    let step = cursor_motion(stick, dt, &motion, &t);
                    self.pan = sat_add(self.pan, step.scale(wpp.saturating_mul(fast)));
                } else {
                    let per_s = cursor_motion(stick, 1000, &motion, &t);
                    ret_target = per_s.scale(wpp.saturating_mul(fast));
                }
                self.glide = None;
                self.follow = false;
            } else {
                let step = cursor_motion(stick, dt, &motion, &t);
                self.cursor = self.camera.clamp_screen(sat_add(self.cursor, step));
            }
        } else {
            self.accel_since = None;
        }
        if reticle && t.reticle_smooth_ms > 0 {
            let k = approach_frac(dt, t.reticle_smooth_ms);
            self.ret_vel = self.ret_vel + (ret_target - self.ret_vel).scale(k);
            if ret_target == Vec2::ZERO && self.ret_vel.length() < Fx::ONE {
                self.ret_vel = Vec2::ZERO;
            }
            self.pan = sat_add(self.pan, per_ms(self.ret_vel, dt));
        } else {
            self.ret_vel = Vec2::ZERO;
        }
        // Edge pan push, per axis in -1..=1 (screen and world agree).
        let mut push = Vec2::ZERO;
        if t.edge_pan_px > 0 && !reticle {
            let m = fx_int(t.edge_pan_px);
            let (w, h) = (fx_int(self.camera.width_px), fx_int(self.camera.height_px));
            let c = self.cursor;
            match self.source {
                CursorSource::Stick if self.wheel.is_none() => {
                    let sx = stick.x;
                    let sy = -stick.y; // screen +y down
                    if (c.x <= m && sx < Fx::ZERO) || (c.x >= w - m && sx > Fx::ZERO) {
                        push.x = sx;
                    }
                    if (c.y <= m && sy < Fx::ZERO) || (c.y >= h - m && sy > Fx::ZERO) {
                        push.y = sy;
                    }
                }
                CursorSource::Pointer if t.edge_pan_mouse => {
                    if c.x <= m {
                        push.x = Fx::NEG_ONE;
                    } else if c.x >= w - Fx::ONE - m {
                        push.x = Fx::ONE;
                    }
                    if c.y <= m {
                        push.y = Fx::NEG_ONE;
                    } else if c.y >= h - Fx::ONE - m {
                        push.y = Fx::ONE;
                    }
                }
                _ => {}
            }
        }
        self.edge_active = push != Vec2::ZERO;
        let edge_ramp = if self.edge_active {
            let since = *self.edge_since.get_or_insert(start);
            ramp(self.now.saturating_sub(since), t.edge_pan_ramp_ms)
        } else {
            self.edge_since = None;
            Fx::ZERO
        };
        // Right stick: zoom with the modifier, else pan.
        let rs = stick_vector(self.right.0, self.right.1, t.stick_deadzone);
        let zoom_held =
            !self.zoom_by.is_empty() || (t.right_stick_zoom && (reticle || self.queue_held()));
        if zoom_held && rs != Vec2::ZERO && self.zoom_by.is_empty() && !reticle {
            // RB + right stick zoomed: RB's own tap (select all) is spent.
            self.consume_modifiers();
        }
        if zoom_held && rs.y != Fx::ZERO {
            let rate = per_second(t.zoom_stick_pct_per_s, dt)
                .checked_div_int(100)
                .unwrap_or(Fx::ZERO)
                .saturating_mul(rs.y.abs())
                .min(Fx::HALF);
            let base = self.zoom_goal.unwrap_or(self.camera.world_per_px);
            let goal = if rs.y > Fx::ZERO {
                base.saturating_mul(Fx::ONE - rate)
            } else {
                base.checked_div(Fx::ONE - rate).unwrap_or(base)
            };
            self.zoom_goal = Some(self.clamp_zoom(goal));
        }
        let stick_pan = if zoom_held { Vec2::ZERO } else { rs };
        if stick_pan != Vec2::ZERO || self.edge_active {
            self.glide = None;
            self.follow = false;
        }
        let pan_k = self.pan_zoom_factor();
        let delta = if t.pan_smooth_ms == 0 {
            let d = stick_pan.scale(per_second(t.pan_speed, dt).saturating_mul(pan_k));
            let e = push.scale(
                per_second(t.edge_pan_speed, dt)
                    .saturating_mul(pan_k)
                    .saturating_mul(edge_ramp),
            );
            Vec2::new(d.x, -d.y) + e
        } else {
            let v = stick_pan.scale(fx_int(t.pan_speed).saturating_mul(pan_k));
            let target = Vec2::new(v.x, -v.y)
                + push.scale(
                    fx_int(t.edge_pan_speed)
                        .saturating_mul(pan_k)
                        .saturating_mul(edge_ramp),
                );
            let k = approach_frac(dt, t.pan_smooth_ms);
            self.pan_vel = self.pan_vel + (target - self.pan_vel).scale(k);
            if target == Vec2::ZERO && self.pan_vel.length() < Fx::ONE {
                self.pan_vel = Vec2::ZERO;
            }
            per_ms(self.pan_vel, dt)
        };
        if delta != Vec2::ZERO {
            self.pan = sat_add(self.pan, delta);
            if stick_pan != Vec2::ZERO && !self.edge_active {
                self.lock_world = true;
            }
        }
    }

    /// Pan speed multiplier for the zoom: `1 + (zoom / ref - 1) * pct`.
    fn pan_zoom_factor(&self) -> Fx {
        let t = &self.tuning;
        if t.pan_zoom_pct == 0 || self.camera.world_per_px <= Fx::ZERO {
            return Fx::ONE;
        }
        let reference = Fx::from_ratio(t.zoom_ref_milli.max(1), 1000).unwrap_or(Fx::ONE);
        let ratio = self
            .camera
            .world_per_px
            .checked_div(reference)
            .unwrap_or(Fx::ONE);
        (Fx::ONE + pct(ratio - Fx::ONE, t.pan_zoom_pct.clamp(0, 100)))
            .clamp(Fx::from_ratio(1, 8).unwrap_or(Fx::ONE), Fx::from_int(16))
    }

    fn clamp_zoom(&self, z: Fx) -> Fx {
        let t = &self.tuning;
        let lo = Fx::from_ratio(t.zoom_min_milli.max(1), 1000).unwrap_or(Fx::ONE);
        let hi =
            Fx::from_ratio(t.zoom_max_milli.max(t.zoom_min_milli.max(1)), 1000).unwrap_or(Fx::ONE);
        z.clamp(lo, hi)
    }

    // ---- raw events ----------------------------------------------------

    fn left_stick(&self) -> Vec2 {
        stick_vector_zones(
            self.left.0,
            self.left.1,
            self.tuning.stick_deadzone,
            self.tuning.stick_outer_deadzone,
        )
    }

    fn steer_wheel(&mut self) {
        let t = self.tuning;
        let s = stick_vector(self.left.0, self.left.1, t.wheel_deadzone);
        if let Some((_, w)) = self.wheel.as_mut() {
            w.steer_with(s, t.wheel_hysteresis_deg, t.wheel_recentre_clears);
        }
    }

    fn axis(&mut self, axis: Axis, value: i16) {
        match axis {
            Axis::LeftX => self.left.0 = value,
            Axis::LeftY => self.left.1 = value,
            Axis::RightX => self.right.0 = value,
            Axis::RightY => self.right.1 = value,
            Axis::LeftTrigger | Axis::RightTrigger => return,
        }
        if matches!(axis, Axis::LeftX | Axis::LeftY) {
            if self.left_stick() != Vec2::ZERO {
                self.source = CursorSource::Stick;
            }
            self.steer_wheel();
        }
    }

    fn scroll(&mut self, milli: i32) {
        if milli == 0 {
            return;
        }
        let t = &self.tuning;
        let s = Fx::from_raw(
            (i64::from(t.zoom_step_pct) * i64::from(milli.unsigned_abs()) * i64::from(Fx::ONE_RAW)
                / 100_000)
                .clamp(0, i64::from(Fx::HALF.raw())) as i32,
        );
        let base = self.zoom_goal.unwrap_or(self.camera.world_per_px);
        let goal = if milli > 0 {
            base.saturating_mul(Fx::ONE - s)
        } else {
            base.checked_div(Fx::ONE - s).unwrap_or(base)
        };
        self.zoom_goal = Some(self.clamp_zoom(goal));
    }

    fn frame<W: WorldView + ?Sized>(&mut self, t: u32, cx: &mut Cx<'_, W>) {
        let dt = t.saturating_sub(self.last_frame.unwrap_or(t));
        self.last_frame = Some(t);
        let tune = self.tuning;
        // Soft snap while the stick is near idle.
        let stick = self.left_stick();
        let idle_lim = Fx::from_ratio(tune.snap_idle_pct.clamp(0, 100), 100).unwrap_or(Fx::ZERO);
        let near_idle =
            stick == Vec2::ZERO || (tune.snap_idle_pct > 0 && stick.length() <= idle_lim);
        if self.source == CursorSource::Stick
            && near_idle
            && !self.precision()
            && self.wheel.is_none()
            && tune.magnet_radius_px > 0
            && !(tune.assist_priority && self.brush.is_some())
        {
            let points = self.target_points();
            let current = self
                .snap_target
                .and_then(|id| self.targets.iter().position(|&(u, _)| u == id));
            let eff = self.eff_cursor();
            let (c, hit) = soft_snap(eff, &points, current, dt, &tune);
            if self.reticle_active() {
                // Aim assist acts on the camera: it slides the target under
                // the fixed reticle.
                let d = c - eff;
                self.pan = sat_add(self.pan, d.scale(self.camera.world_per_px));
            } else {
                self.cursor = self.camera.clamp_screen(c);
            }
            let new = hit.map(|i| self.targets[i].0);
            if new.is_some() && new != self.snap_target {
                self.feedback.push(Feedback::Snap);
            }
            self.snap_target = new;
        } else if !near_idle || self.precision() || self.source == CursorSource::Pointer {
            self.snap_target = None;
        }
        // Zoom smoothing.
        if let Some(goal) = self.zoom_goal {
            let cur = self.zoom_out.unwrap_or(self.camera.world_per_px);
            let k = approach_frac(dt.max(1), tune.zoom_smooth_ms);
            let mut z = cur + (goal - cur) * k;
            if (goal - z).abs() <= Fx::from_raw(64) {
                z = goal;
                self.zoom_goal = None;
            }
            if z != self.camera.world_per_px {
                self.zoom_out = Some(z);
                self.lock_world = true;
            }
        }
        // Follow and glide.
        if self.follow {
            match self.selection_centre(cx.world) {
                Some(c) => self.glide = Some(c),
                None => self.follow = false,
            }
        }
        if let Some(goal) = self.glide {
            let centre = self.camera.screen_to_world(self.camera.center_screen()) + self.pan;
            let gap = goal - centre;
            if gap.length() <= Fx::from_int(2) || tune.glide_ms == 0 {
                self.pan = sat_add(self.pan, gap);
                if !self.follow {
                    self.glide = None;
                }
            } else {
                self.pan = sat_add(self.pan, gap.scale(approach_frac(dt, tune.glide_ms)));
            }
        }
        self.paint_brush(cx);
    }

    fn selection_centre<W: WorldView + ?Sized>(&self, world: &W) -> Option<WorldPos> {
        centre_of(world, &own_units(world, &self.selection))
    }

    fn down<W: WorldView + ?Sized>(&mut self, input: Input, t: u32, cx: &mut Cx<'_, W>) {
        if self.inputs.get(&input).is_some_and(|s| s.press.is_some()) {
            return; // repeat
        }
        // Another input pressed while a held modifier (queue, zoom,
        // precision) is down uses that modifier: its own tap is dropped.
        self.consume_modifiers();
        // Grid build keys (optional add-on) win while a builder is selected.
        if let Some(index) = self.grid_index(&input, cx) {
            self.inputs.entry(input).or_default().press = Some(Press {
                at: t,
                with: None,
                consumed: true,
                hold_fired: false,
            });
            self.instant(Action::BuildOption { index }, cx);
            return;
        }
        // A different input decides any pending single taps and ends any
        // second-tap window.
        let others: Vec<Input> = self
            .inputs
            .iter()
            .filter(|(i, s)| **i != input && s.pending.is_some())
            .map(|(i, _)| i.clone())
            .collect();
        for other in others {
            if let Some(p) = self.inputs.get_mut(&other).and_then(|s| s.pending.take()) {
                self.fire_tap(&other, p.with.as_ref(), t, cx);
            }
        }
        for (i, s) in self.inputs.iter_mut() {
            if *i != input {
                s.last_tap = None;
            }
        }
        // Chord modifier: the first held input this one has a chord with.
        let with = self
            .inputs
            .iter()
            .find(|(m, s)| s.press.is_some() && self.profile.has_chord(&input, m))
            .map(|(m, _)| m.clone());
        if let Some(p) = with
            .as_ref()
            .and_then(|m| self.inputs.get_mut(m))
            .and_then(|s| s.press.as_mut())
        {
            p.consumed = true;
        }
        let mut consumed = false;
        // A second press that completed a double or second tap does not
        // also fire its press binding (a double click is not a new drag).
        let mut second = false;
        let (pending, last_tap) = match self.inputs.get_mut(&input) {
            Some(s) => (s.pending.take(), s.last_tap.take()),
            None => (None, None),
        };
        if let Some(p) = pending {
            let double = (p.with == with)
                .then(|| self.profile.find(&input, with.as_ref(), Gesture::DoubleTap))
                .flatten();
            match double {
                Some(a) => {
                    consumed = true;
                    second = true;
                    self.trigger(&input, a, t, cx);
                }
                None => self.fire_tap(&input, p.with.as_ref(), t, cx),
            }
        } else if let Some(p) = last_tap.filter(|p| {
            p.with == with && t.saturating_sub(p.released_at) <= self.tuning.double_tap_ms
        }) && let Some(a) = self
            .profile
            .find(&input, p.with.as_ref(), Gesture::SecondTap)
        {
            consumed = true;
            second = true;
            self.trigger(&input, a, t, cx);
        }
        self.inputs.entry(input.clone()).or_default().press = Some(Press {
            at: t,
            with: with.clone(),
            consumed,
            hold_fired: false,
        });
        if second {
            return;
        }
        if let Some(a) = self.profile.find(&input, with.as_ref(), Gesture::Press) {
            self.trigger(&input, a, t, cx);
        }
    }

    fn up<W: WorldView + ?Sized>(&mut self, input: Input, t: u32, cx: &mut Cx<'_, W>) {
        let Some(press) = self.inputs.get_mut(&input).and_then(|s| s.press.take()) else {
            return;
        };
        self.end_held(&input, cx);
        let with = press.with.as_ref();
        if let Some(a) = self.profile.find(&input, with, Gesture::Release) {
            self.trigger(&input, a, t, cx);
        }
        if press.consumed || press.hold_fired {
            return;
        }
        // A long press of an input whose press binding is a held action
        // (RB: queue modifier) is not also a tap.
        if t.saturating_sub(press.at) >= self.tuning.hold_ms
            && self
                .profile
                .find(&input, with, Gesture::Press)
                .is_some_and(|a| a.is_held())
        {
            return;
        }
        if self
            .profile
            .find(&input, with, Gesture::DoubleTap)
            .is_some()
        {
            self.inputs.entry(input).or_default().pending = Some(PendingTap {
                released_at: t,
                with: press.with,
            });
        } else {
            self.fire_tap(&input, with, t, cx);
            if self
                .profile
                .find(&input, with, Gesture::SecondTap)
                .is_some()
            {
                self.inputs.entry(input).or_default().last_tap = Some(PendingTap {
                    released_at: t,
                    with: press.with,
                });
            }
        }
    }

    fn fire_tap<W: WorldView + ?Sized>(
        &mut self,
        input: &Input,
        with: Option<&Input>,
        at: u32,
        cx: &mut Cx<'_, W>,
    ) {
        if let Some(a) = self.profile.find(input, with, Gesture::Tap) {
            self.trigger(input, a, at, cx);
        }
    }

    /// Mark every pressed input whose press binding is a held modifier as
    /// used, so releasing it fires no tap.
    fn consume_modifiers(&mut self) {
        let profile = &self.profile;
        for (i, st) in self.inputs.iter_mut() {
            if let Some(p) = st.press.as_mut()
                && profile
                    .find(i, p.with.as_ref(), Gesture::Press)
                    .is_some_and(|a| a.is_modifier())
            {
                p.consumed = true;
            }
        }
    }

    /// The Grid add-on's build option for a key, when it applies: the
    /// profile enables it, no Ctrl or Alt is held, no wheel or mode is
    /// active and a builder is selected.
    fn grid_index<W: WorldView + ?Sized>(&self, input: &Input, cx: &Cx<'_, W>) -> Option<u8> {
        if !self.profile.grid_build_keys || self.wheel.is_some() {
            return None;
        }
        let Input::Key(k) = input else { return None };
        let index = GRID_KEYS.iter().position(|g| *g == k.0)?;
        let held = |name: &str| {
            self.inputs
                .iter()
                .any(|(i, s)| s.press.is_some() && matches!(i, Input::Key(x) if x.0 == name))
        };
        if held("ctrl") || held("alt") {
            return None;
        }
        let units = own_units(cx.world, &self.selection);
        first_builder(cx.world, &units)?;
        u8::try_from(index).ok()
    }

    // ---- actions -------------------------------------------------------

    fn trigger<W: WorldView + ?Sized>(
        &mut self,
        input: &Input,
        action: Action,
        at: u32,
        cx: &mut Cx<'_, W>,
    ) {
        // A paged wheel: the queue modifier (RB) turns to the next page, the
        // idle-builder button (LB tap, sticky wheels) to the previous one.
        if let Some((_, w)) = self.wheel.as_ref()
            && w.pages > 1
            && matches!(action, Action::QueueModifier | Action::CycleIdleBuilder)
        {
            let step = if action == Action::QueueModifier {
                1
            } else {
                -1
            };
            self.turn_page(step);
            if let Some(p) = self.inputs.get_mut(input).and_then(|s| s.press.as_mut()) {
                p.consumed = true;
            }
            return;
        }
        // A sticky wheel (confirm on press, sub-rings): select / context /
        // brush confirm the highlighted slot instead.
        if self.wheel_sticky
            && self.wheel.is_some()
            && matches!(
                action,
                Action::Select | Action::Context { .. } | Action::Brush
            )
        {
            if let Some((i, w)) = self.wheel.take() {
                self.wheel_sticky = false;
                self.confirm_wheel(&i, w, cx);
            }
            return;
        }
        if action.is_held() {
            self.start_held(input, action, at, cx);
        } else {
            self.instant(action, cx);
        }
    }

    fn queue_held(&self) -> bool {
        !self.queue_by.is_empty()
    }

    fn queue_mode(&self, append: bool) -> QueueMode {
        if append || self.queue_held() {
            QueueMode::Append
        } else {
            QueueMode::Replace
        }
    }

    fn open_wheel(&mut self, input: &Input, kind: WheelKind, slots: Vec<Action>) {
        if slots.is_empty() {
            return;
        }
        self.wheel = Some((input.clone(), Wheel::open(kind, slots)));
        self.wheel_sticky = self.tuning.wheel_confirm_on_press;
        self.wheel_parents.clear();
        self.steer_wheel();
    }

    /// Open a wheel of `items`, 8 per page (fixed positions).
    fn open_paged(&mut self, input: &Input, kind: WheelKind, items: Vec<Action>) {
        if items.is_empty() {
            return;
        }
        let w = Wheel::paged(kind, &items, 0);
        self.wheel_items = items;
        self.wheel = Some((input.clone(), w));
        self.wheel_sticky = self.tuning.wheel_confirm_on_press;
        self.wheel_parents.clear();
        self.steer_wheel();
    }

    /// Show the next (`step` 1) or previous (-1) page of a paged wheel.
    fn turn_page(&mut self, step: i32) {
        let items = self.wheel_items.clone();
        if let Some((_, w)) = self.wheel.as_mut() {
            let pages = i32::from(w.pages.max(1));
            let page = (i32::from(w.page) + step).rem_euclid(pages);
            let mut nw = Wheel::paged(w.kind, &items, usize::try_from(page).unwrap_or(0));
            nw.highlighted = None;
            *w = nw;
        }
        self.steer_wheel();
    }

    /// A wheel was confirmed (released, or A / X on a sticky one): open a
    /// sub-ring, or carry out the slot.
    fn confirm_wheel<W: WorldView + ?Sized>(
        &mut self,
        input: &Input,
        w: Wheel,
        cx: &mut Cx<'_, W>,
    ) {
        match w.clone().confirm() {
            Some(Action::OpenRing { ring }) => {
                self.wheel_parents.push(w);
                self.open_ring(input, ring, cx.world);
            }
            Some(Action::RecallGroup { group })
                if w.kind == WheelKind::Squads && self.queue_held() =>
            {
                self.wheel_parents.clear();
                self.instant(Action::AssignGroup { group }, cx);
            }
            Some(a) => {
                self.wheel_parents.clear();
                self.instant(a, cx);
            }
            None => self.wheel_parents.clear(),
        }
    }

    /// Open a sub-ring. It stays open after the wheel button is released:
    /// the stick chooses, A / X confirms, B goes back one ring.
    fn open_ring<W: WorldView + ?Sized>(&mut self, input: &Input, ring: Ring, world: &W) {
        // Sub-rings keep their learned positions, even in non-fixed profiles.
        let slots = heal_slots(
            ring_slots(ring),
            !healers_for(world, &self.selection).is_empty(),
            true,
        );
        self.wheel = Some((input.clone(), Wheel::open(WheelKind::of_ring(ring), slots)));
        self.wheel_sticky = true;
    }

    fn refuse(&mut self, reason: Refusal) {
        self.feedback.push(Feedback::Refused { reason });
    }

    fn start_held<W: WorldView + ?Sized>(
        &mut self,
        input: &Input,
        action: Action,
        at: u32,
        cx: &mut Cx<'_, W>,
    ) {
        match action {
            Action::Precision => self.precision_by.push(input.clone()),
            Action::QueueModifier => self.queue_by.push(input.clone()),
            Action::ZoomModifier => self.zoom_by.push(input.clone()),
            Action::Brush => {
                self.brush = Some(BrushState {
                    input: input.clone(),
                    since: at,
                    anchor: self.cursor_world(),
                    last_stamp: None,
                });
            }
            Action::DragSelect => {
                self.drag = Some(DragState {
                    input: input.clone(),
                    start_world: self.cursor_world(),
                    start_screen: self.cursor,
                });
            }
            Action::CrystalBall => self.crystal_by.push(input.clone()),
            Action::CommandWheel | Action::BuildWheel | Action::SpellWheel
                if self.wheel.is_some() =>
            {
                // Pressing a wheel button again closes a sticky wheel.
                self.wheel = None;
                self.wheel_sticky = false;
                self.wheel_parents.clear();
            }
            Action::CommandWheel if self.profile.fixed_slots => {
                let units = own_units(cx.world, &self.selection);
                let spells = first_caster(cx.world, &units).map_or(0, |(_, s)| s.len());
                let builders = first_builder(cx.world, &units).is_some();
                let slots = context_slots_fixed(
                    &self.profile.command_wheel,
                    &self.profile.idle_wheel,
                    !units.is_empty(),
                    spells > 0,
                    builders,
                );
                let slots = heal_slots(slots, !healers_for(cx.world, &units).is_empty(), true);
                self.open_wheel(input, WheelKind::Command, slots);
            }
            Action::BuildWheel if self.profile.fixed_slots => {
                let units = own_units(cx.world, &self.selection);
                let Some(first) = first_builder(cx.world, &units) else {
                    self.refuse(Refusal::CannotDo);
                    return;
                };
                let items: Vec<Action> = (0..cx.world.build_options(first).len())
                    .map(|i| Action::BuildOption {
                        index: u8::try_from(i).unwrap_or(u8::MAX),
                    })
                    .collect();
                self.open_paged(input, WheelKind::Build, items);
            }
            Action::SpellWheel => {
                let units = own_units(cx.world, &self.selection);
                let Some((_, spells)) = first_caster(cx.world, &units) else {
                    self.refuse(Refusal::CannotDo);
                    return;
                };
                let items: Vec<Action> = (0..spells.len())
                    .map(|i| Action::CastSpell {
                        index: u8::try_from(i).unwrap_or(u8::MAX),
                    })
                    .collect();
                self.open_paged(input, WheelKind::Spell, items);
            }
            Action::CommandWheel => {
                let slots = if self.tuning.wheel_context {
                    let units = own_units(cx.world, &self.selection);
                    let spells = first_caster(cx.world, &units).map_or(0, |(_, s)| s.len());
                    let builders = first_builder(cx.world, &units).is_some();
                    let slots = context_slots(
                        &self.profile.command_wheel,
                        &self.profile.idle_wheel,
                        !units.is_empty(),
                        spells > 0,
                        builders,
                    );
                    let mut slots = expand_spells(slots, spells);
                    slots.truncate(usize::from(self.tuning.wheel_max_slots));
                    slots
                } else {
                    self.profile.command_wheel.clone()
                };
                let slots = heal_slots(
                    slots,
                    !healers_for(cx.world, &self.selection).is_empty(),
                    false,
                );
                self.open_wheel(input, WheelKind::Command, slots);
            }
            Action::BuildWheel => {
                let units = own_units(cx.world, &self.selection);
                let Some(first) = first_builder(cx.world, &units) else {
                    return;
                };
                let n = cx
                    .world
                    .build_options(first)
                    .len()
                    .min(usize::from(self.tuning.wheel_max_slots));
                let slots = (0..n)
                    .map(|i| Action::BuildOption {
                        index: u8::try_from(i).unwrap_or(u8::MAX),
                    })
                    .collect();
                self.open_wheel(input, WheelKind::Build, slots);
            }
            _ => {}
        }
    }

    fn end_held<W: WorldView + ?Sized>(&mut self, input: &Input, cx: &mut Cx<'_, W>) {
        self.precision_by.retain(|i| i != input);
        self.queue_by.retain(|i| i != input);
        self.zoom_by.retain(|i| i != input);
        if !self.wheel_sticky
            && let Some((i, w)) = self.wheel.take_if(|(i, _)| i == input)
        {
            self.confirm_wheel(&i, w, cx);
        }
        self.crystal_by.retain(|i| i != input);
        if let Some(b) = self.brush.take_if(|b| &b.input == input) {
            let radius = brush_radius(self.now.saturating_sub(b.since), &self.tuning);
            let additive = b.last_stamp.is_some() || self.queue_held();
            let center = self.cursor_world();
            self.client(
                ClientCommand::SelectBrush {
                    center,
                    radius,
                    additive,
                },
                cx,
            );
        }
        if let Some(d) = self.drag.take_if(|d| &d.input == input) {
            let end = self.cursor_world();
            if d.start_screen.distance(self.cursor) < fx_int(self.tuning.drag_min_px) {
                if self.mode != Mode::None {
                    self.context(false, cx);
                } else {
                    let additive = self.queue_held();
                    self.client(ClientCommand::SelectAt { pos: end, additive }, cx);
                }
            } else {
                let (center, radius) = drag_circle(d.start_world, end);
                let additive = self.queue_held();
                self.client(
                    ClientCommand::SelectBrush {
                        center,
                        radius,
                        additive,
                    },
                    cx,
                );
            }
        }
    }

    /// Brush painting: once the brush has moved half its radius from its
    /// last stamp, stamp the old spot (first time only) and the new one.
    fn paint_brush<W: WorldView + ?Sized>(&mut self, cx: &mut Cx<'_, W>) {
        let Some(b) = self.brush.clone() else { return };
        let center = self.cursor_world();
        let radius = brush_radius(self.now.saturating_sub(b.since), &self.tuning);
        let reference = b.last_stamp.unwrap_or(b.anchor);
        let moved = center.distance(reference);
        if center == reference || moved.saturating_add(moved) < radius {
            return;
        }
        if b.last_stamp.is_none() {
            let additive = self.queue_held();
            self.client(
                ClientCommand::SelectBrush {
                    center: b.anchor,
                    radius,
                    additive,
                },
                cx,
            );
        }
        self.client(
            ClientCommand::SelectBrush {
                center,
                radius,
                additive: true,
            },
            cx,
        );
        if let Some(bs) = self.brush.as_mut() {
            bs.last_stamp = Some(center);
        }
    }

    fn instant<W: WorldView + ?Sized>(&mut self, action: Action, cx: &mut Cx<'_, W>) {
        match action {
            Action::Select => {
                if self.mode != Mode::None {
                    self.context(false, cx);
                } else {
                    let pos = self.cursor_world();
                    let additive = self.queue_held();
                    self.client(ClientCommand::SelectAt { pos, additive }, cx);
                }
            }
            Action::Context { append } => self.context(append, cx),
            Action::AttackMove => {
                let units = own_units(cx.world, &self.selection);
                if units.is_empty() {
                    self.refuse(Refusal::NothingSelected);
                } else {
                    let to = self.cursor_world();
                    let queue = self.queue_mode(false);
                    self.sim(SimCommand::AttackMove { units, to, queue }, cx);
                    if queue == QueueMode::Replace {
                        self.mode = Mode::None;
                    }
                }
            }
            Action::Cancel => self.cancel(cx),
            Action::SelectSameType { scope } => {
                self.client(ClientCommand::SelectSameType { scope }, cx)
            }
            Action::SelectAll { scope } => self.select_all(scope, cx),
            Action::AssignGroup { group } => self.client(ClientCommand::AssignGroup { group }, cx),
            Action::RecallGroup { group } => self.client(ClientCommand::RecallGroup { group }, cx),
            Action::JumpToAlert => self.client(ClientCommand::JumpToAlert, cx),
            Action::CenterOnMonarch => self.client(ClientCommand::CenterOnMonarch, cx),
            Action::CycleIdleBuilder => self.cycle_idle_builder(cx),
            Action::FollowSelection => {
                self.follow = !self.follow && self.selection_centre(cx.world).is_some();
                if !self.follow {
                    self.glide = None;
                }
            }
            Action::ToggleOverlay => cx.out.push(Output::ToggleOverlay),
            Action::Stop => {
                let units = own_units(cx.world, &self.selection);
                self.sim(SimCommand::Stop { units }, cx);
            }
            Action::SetFireState { state } => {
                let units = own_units(cx.world, &self.selection);
                self.sim(SimCommand::SetFireState { units, state }, cx);
            }
            Action::PatrolMode => self.set_mode_if_units(Mode::Patrol, cx),
            Action::GuardMode => self.set_mode_if_units(Mode::Guard, cx),
            Action::HealMode => {
                let units = own_units(cx.world, &self.selection);
                if units.is_empty() {
                    self.refuse(Refusal::NothingSelected);
                } else if healers_for(cx.world, &units).is_empty() {
                    self.refuse(Refusal::CannotDo);
                } else {
                    self.mode = Mode::Heal;
                }
            }
            Action::CastSpellMode => self.cast_spell_mode(0, cx),
            Action::CastSpell { index } => self.cast_spell_mode(usize::from(index), cx),
            Action::BuildOption { index } => self.build_option(usize::from(index), cx),
            Action::SubSelectCycle => self.sub_select_cycle(cx),
            Action::SelectMonarch { track } => {
                let me = cx.world.local_player();
                match cx.world.monarch().and_then(|id| cx.world.unit(id)) {
                    Some(u) if u.owner == me => {
                        self.set_selection(UnitSet::single(u.id));
                        if track {
                            self.follow = true;
                        }
                    }
                    _ => self.refuse(Refusal::NothingThere),
                }
            }
            Action::ResetZoom => {
                let z =
                    Fx::from_ratio(self.tuning.zoom_default_milli.max(1), 1000).unwrap_or(Fx::HALF);
                self.zoom_goal = Some(self.clamp_zoom(z));
            }
            Action::JumpToGroup { group } => {
                let units = own_units(cx.world, &self.groups[usize::from(group.get())]);
                match centre_of(cx.world, &units) {
                    Some(c) => {
                        self.follow = false;
                        self.glide_to(c);
                    }
                    None => self.refuse(Refusal::NothingThere),
                }
            }
            Action::ToggleCrystalBall => self.crystal_toggled = !self.crystal_toggled,
            Action::AttackMode => self.set_mode_if_units(Mode::Attack, cx),
            Action::AttackMoveMode => self.set_mode_if_units(Mode::AttackMove, cx),
            Action::MoveMode => self.set_mode_if_units(Mode::Move, cx),
            Action::SelectCategory { category } => self.select_category(category, cx),
            Action::SetBookmark { slot } => {
                if let Some(b) = self.bookmarks.get_mut(usize::from(slot)) {
                    *b = Some(sat_add(
                        self.camera.screen_to_world(self.camera.center_screen()),
                        self.pan,
                    ));
                }
            }
            Action::GoToBookmark { slot } => match self.bookmark(slot) {
                Some(p) => {
                    self.follow = false;
                    self.glide_to(p);
                }
                None => self.refuse(Refusal::NothingThere),
            },
            Action::NextUnit => self.next_unit(cx),
            Action::ToggleDamageBars => cx.out.push(Output::ToggleDamageBars),
            Action::GameSpeed { faster } => cx.out.push(Output::GameSpeed { faster }),
            Action::TogglePause => cx.out.push(Output::TogglePause),
            Action::Reserved { .. } => self.refuse(Refusal::Unsupported),
            Action::OpenRing { .. }
            | Action::Brush
            | Action::DragSelect
            | Action::Precision
            | Action::QueueModifier
            | Action::ZoomModifier
            | Action::CommandWheel
            | Action::BuildWheel
            | Action::SpellWheel
            | Action::CrystalBall => {}
        }
    }

    /// Halo Wars RT: narrow to the next unit type in the selection; after
    /// the last type, back to the full selection.
    fn sub_select_cycle<W: WorldView + ?Sized>(&mut self, cx: &mut Cx<'_, W>) {
        let world = cx.world;
        let by_type = |full: &UnitSet, def: UnitDefId| -> UnitSet {
            full.iter()
                .copied()
                .filter(|&id| world.unit(id).is_some_and(|u| u.def == def))
                .collect()
        };
        let current = own_units(world, &self.selection);
        let next = match self.sub_select.take() {
            Some(mut ss)
                if ss.types.get(ss.index).map(|&d| by_type(&ss.full, d))
                    == Some(current.clone()) =>
            {
                ss.full = own_units(world, &ss.full);
                ss.index += 1;
                if ss.index < ss.types.len() {
                    Some(ss)
                } else {
                    self.set_selection(ss.full);
                    None
                }
            }
            _ => {
                let mut types: Vec<UnitDefId> = Vec::new();
                for id in current.iter() {
                    if let Some(u) = world.unit(*id)
                        && !types.contains(&u.def)
                    {
                        types.push(u.def);
                    }
                }
                if types.len() < 2 {
                    self.refuse(Refusal::NothingThere);
                    return;
                }
                Some(SubSelect {
                    full: current,
                    types,
                    index: 0,
                })
            }
        };
        if let Some(ss) = next {
            let sub = by_type(&ss.full, ss.types[ss.index]);
            self.set_selection(sub);
            self.sub_select = Some(ss);
        }
    }

    /// TA Ctrl+letter: every own unit of a category on the map (added to
    /// the selection with the queue modifier).
    fn select_category<W: WorldView + ?Sized>(&mut self, c: Category, cx: &mut Cx<'_, W>) {
        let me = cx.world.local_player();
        let mut v: Vec<UnitId> = if self.queue_held() {
            self.selection.iter().copied().collect()
        } else {
            Vec::new()
        };
        for u in units_sorted(cx.world) {
            if u.owner == me && u.selectable && cx.world.in_category(u.id, c) {
                v.push(u.id);
            }
        }
        let s = UnitSet::from(v);
        if s.is_empty() {
            self.refuse(Refusal::NothingThere);
        }
        self.set_selection(s);
    }

    /// TA N: camera to the next own unit that is off screen.
    fn next_unit<W: WorldView + ?Sized>(&mut self, cx: &mut Cx<'_, W>) {
        let me = cx.world.local_player();
        let off: Vec<UnitInfo> = units_sorted(cx.world)
            .into_iter()
            .filter(|u| u.owner == me && u.selectable && !self.camera.contains_world(u.pos))
            .collect();
        let next = self
            .last_next_unit
            .and_then(|last| off.iter().find(|u| u.id > last))
            .or_else(|| off.first())
            .copied();
        match next {
            Some(u) => {
                self.last_next_unit = Some(u.id);
                self.follow = false;
                self.glide_to(u.pos);
            }
            None => self.refuse(Refusal::NothingThere),
        }
    }

    fn select_all<W: WorldView + ?Sized>(&mut self, scope: Scope, cx: &mut Cx<'_, W>) {
        let me = cx.world.local_player();
        let units: UnitSet = units_sorted(cx.world)
            .into_iter()
            .filter(|u| {
                u.owner == me
                    && u.selectable
                    && (scope == Scope::Map || self.camera.contains_world(u.pos))
            })
            .map(|u| u.id)
            .collect();
        self.set_selection(units.clone());
        cx.out.push(Output::SelectAll { scope, units });
    }

    /// Enter cast mode for spell `index` of the selection's first caster,
    /// if it has that spell and can pay for it.
    fn cast_spell_mode<W: WorldView + ?Sized>(&mut self, index: usize, cx: &mut Cx<'_, W>) {
        let units = own_units(cx.world, &self.selection);
        match first_caster(cx.world, &units) {
            Some((caster, spells)) => match spells.get(index) {
                Some(&s) if cx.world.can_cast(caster, s) => self.mode = Mode::CastSpell(s),
                Some(_) => self.refuse(Refusal::NoMana),
                None => self.refuse(Refusal::CannotDo),
            },
            None if units.is_empty() => self.refuse(Refusal::NothingSelected),
            None => self.refuse(Refusal::CannotDo),
        }
    }

    fn set_mode_if_units<W: WorldView + ?Sized>(&mut self, mode: Mode, cx: &mut Cx<'_, W>) {
        if own_units(cx.world, &self.selection).is_empty() {
            self.refuse(Refusal::NothingSelected);
        } else {
            self.mode = mode;
        }
    }

    fn context<W: WorldView + ?Sized>(&mut self, append: bool, cx: &mut Cx<'_, W>) {
        let units = own_units(cx.world, &self.selection);
        let pos = self.cursor_world();
        let hovered = pick_unit(cx.world, pos);
        let queue = self.queue_mode(append);
        if let Some(cmd) =
            resolve_context(cx.world, &units, pos, hovered.as_ref(), self.mode, queue)
        {
            self.sim(cmd, cx);
            if queue == QueueMode::Replace {
                self.mode = Mode::None;
            }
        }
    }

    fn cancel<W: WorldView + ?Sized>(&mut self, cx: &mut Cx<'_, W>) {
        if let Some((input, _)) = self.wheel.take() {
            // A sub-ring goes back to its parent (still open, sticky).
            if let Some(mut parent) = self.wheel_parents.pop() {
                parent.highlighted = None;
                self.wheel = Some((input, parent));
                self.wheel_sticky = true;
            } else {
                self.wheel_sticky = false;
            }
            return;
        }
        if self.brush.take().is_some() || self.drag.take().is_some() {
            return;
        }
        if self.mode != Mode::None {
            self.mode = Mode::None;
            return;
        }
        if !self.selection.is_empty() {
            self.client(ClientCommand::ClearSelection, cx);
        }
    }

    fn build_option<W: WorldView + ?Sized>(&mut self, index: usize, cx: &mut Cx<'_, W>) {
        let units = own_units(cx.world, &self.selection);
        let Some(first) = first_builder(cx.world, &units) else {
            return;
        };
        let Some(&def) = cx.world.build_options(first).get(index) else {
            return;
        };
        let mobiles = builders_for(cx.world, &units, def, Some(BuilderKind::Mobile));
        if !mobiles.is_empty() {
            self.mode = Mode::Build(def);
            return;
        }
        let factories = builders_for(cx.world, &units, def, Some(BuilderKind::Factory));
        let queue = self.queue_mode(false);
        self.sim(
            SimCommand::Build {
                units: factories,
                def,
                at: None,
                queue,
            },
            cx,
        );
    }

    fn cycle_idle_builder<W: WorldView + ?Sized>(&mut self, cx: &mut Cx<'_, W>) {
        let me = cx.world.local_player();
        let idle: Vec<UnitId> = units_sorted(cx.world)
            .into_iter()
            .filter(|u| u.owner == me && u.selectable && u.builder.is_some())
            .map(|u| u.id)
            .filter(|&id| cx.world.is_idle(id))
            .collect();
        let next = self
            .last_idle
            .and_then(|last| idle.iter().copied().find(|&id| id > last))
            .or_else(|| idle.first().copied());
        let Some(next) = next else { return };
        self.last_idle = Some(next);
        self.set_selection(UnitSet::single(next));
        cx.out.push(Output::Client {
            command: ClientCommand::CycleIdleBuilder,
        });
    }

    fn sim<W: WorldView + ?Sized>(&mut self, command: SimCommand, cx: &mut Cx<'_, W>) {
        if !command.units().is_empty() {
            self.feedback.push(Feedback::Order);
            cx.out.push(Output::Sim {
                tick: cx.tick,
                command,
            });
        }
    }

    fn set_selection(&mut self, s: UnitSet) {
        self.sub_select = None;
        if s != self.selection {
            self.selection = s;
            self.mode = Mode::None;
            self.selected_at = Some(self.now);
            if !self.selection.is_empty() {
                self.feedback.push(Feedback::Selection {
                    count: u32::try_from(self.selection.len()).unwrap_or(u32::MAX),
                });
            }
        }
    }

    /// Apply a selection-affecting client command, then emit it.
    fn client<W: WorldView + ?Sized>(&mut self, command: ClientCommand, cx: &mut Cx<'_, W>) {
        let world = cx.world;
        let me = world.local_player();
        match command {
            ClientCommand::SelectAt { pos, additive } => {
                let picked = pick_unit(world, pos)
                    .filter(|u| u.owner == me)
                    .map(|u| u.id);
                let mut s = if additive {
                    self.selection.clone()
                } else {
                    UnitSet::new()
                };
                if let Some(id) = picked {
                    s.insert(id);
                }
                self.set_selection(s);
            }
            ClientCommand::SelectBrush {
                center,
                radius,
                additive,
            } => {
                let found = units_in_circle(world, center, radius);
                let s = if additive {
                    self.selection.iter().chain(found.iter()).copied().collect()
                } else {
                    found
                };
                self.set_selection(s);
            }
            ClientCommand::SelectSameType { scope } => {
                let current = own_units(world, &self.selection);
                let defs: BTreeSet<UnitDefId> = current
                    .iter()
                    .filter_map(|&id| world.unit(id).map(|u| u.def))
                    .collect();
                let mut v: Vec<UnitId> = current.iter().copied().collect();
                world.for_each_unit(&mut |u| {
                    if u.owner == me
                        && u.selectable
                        && defs.contains(&u.def)
                        && (scope == Scope::Map || self.camera.contains_world(u.pos))
                    {
                        v.push(u.id);
                    }
                });
                self.set_selection(UnitSet::from(v));
            }
            ClientCommand::AssignGroup { group } => {
                self.groups[usize::from(group.get())] = own_units(world, &self.selection);
            }
            ClientCommand::RecallGroup { group } => {
                let s = own_units(world, &self.groups[usize::from(group.get())]);
                self.set_selection(s);
            }
            ClientCommand::ClearSelection => self.set_selection(UnitSet::new()),
            ClientCommand::JumpToAlert
            | ClientCommand::CycleIdleBuilder
            | ClientCommand::CenterOnMonarch => {}
        }
        cx.out.push(Output::Client { command });
    }
}

/// Hide unusable Heal slots without disturbing a fixed wheel's positions.
fn heal_slots(slots: Vec<Action>, available: bool, fixed: bool) -> Vec<Action> {
    if available {
        return slots;
    }
    slots
        .into_iter()
        .filter_map(|action| {
            if action != Action::HealMode {
                Some(action)
            } else if fixed {
                Some(crate::wheel::BLANK)
            } else {
                None
            }
        })
        .collect()
}

fn sat_add(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x.saturating_add(b.x), a.y.saturating_add(b.y))
}

/// `elapsed / total` clamped to `0..=1` (1 when `total` is 0).
fn ramp(elapsed: u32, total: u32) -> Fx {
    if total == 0 || elapsed >= total {
        return Fx::ONE;
    }
    Fx::from_raw((u64::from(elapsed) * u64::from(Fx::ONE_RAW as u32) / u64::from(total)) as i32)
}

/// A per-second velocity times `dt_ms`, rounded.
fn per_ms(v: Vec2, dt_ms: u32) -> Vec2 {
    let f = |c: Fx| {
        let r = (i64::from(c.raw()) * i64::from(dt_ms) * 2 + 1000).div_euclid(2000);
        Fx::from_raw(r.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
    };
    Vec2::new(f(v.x), f(v.y))
}

/// Mean position of the units that still exist.
fn centre_of<W: WorldView + ?Sized>(world: &W, units: &UnitSet) -> Option<WorldPos> {
    let (mut x, mut y, mut n) = (0i64, 0i64, 0i64);
    for id in units.iter() {
        if let Some(u) = world.unit(*id) {
            x += i64::from(u.pos.x.raw());
            y += i64::from(u.pos.y.raw());
            n += 1;
        }
    }
    (n > 0).then(|| Vec2::new(Fx::from_raw((x / n) as i32), Fx::from_raw((y / n) as i32)))
}

/// The lowest-id builder with a nonempty build menu among `units`.
fn first_builder<W: WorldView + ?Sized>(world: &W, units: &UnitSet) -> Option<UnitId> {
    units.iter().copied().find(|&id| {
        world.unit(id).is_some_and(|u| u.builder.is_some()) && !world.build_options(id).is_empty()
    })
}

/// The first unit of `units` with spells, and its spells (menu order).
fn first_caster<W: WorldView + ?Sized>(
    world: &W,
    units: &UnitSet,
) -> Option<(UnitId, Vec<tak_commands::SpellId>)> {
    units.iter().copied().find_map(|id| {
        let s = world.spells(id);
        (!s.is_empty()).then_some((id, s))
    })
}
