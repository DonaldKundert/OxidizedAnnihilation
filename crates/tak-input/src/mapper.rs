//! The stateful mapper: device events in, ordered commands out.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use tak_commands::{
    ClientCommand, ControlGroup, QueueMode, Scope, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
    WorldPos,
};
use tak_math::{Fx, Vec2};

use crate::binding::{Action, Gesture, Profile};
use crate::brush::{brush_radius, drag_circle, units_in_circle};
use crate::context::{Mode, builders_for, own_units, pick_unit, resolve_context};
use crate::cursor::{cursor_step, magnetize, per_second, stick_vector};
use crate::event::{Axis, EventKind, Input, InputEvent};
use crate::tuning::Tuning;
use crate::wheel::{Wheel, WheelKind};
use crate::world::{BuilderKind, Camera, WorldView, fx_int, units_sorted};

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
    /// The selection was cleared (B / Escape with nothing else to cancel).
    /// `ClientCommand` has no variant for this yet.
    Deselect,
    /// An order for the sim, stamped with the tick the caller passed in.
    Sim {
        /// Caller's tick.
        tick: Tick,
        /// The order.
        command: SimCommand,
    },
}

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
/// groups, pending order mode, open wheel, brush. It reads the world only
/// through [`WorldView`] and never mutates sim state; sim orders come out as
/// [`Output::Sim`] for the caller to submit.
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
    brush: Option<BrushState>,
    drag: Option<DragState>,
    precision_by: Vec<Input>,
    queue_by: Vec<Input>,
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
            brush: None,
            drag: None,
            precision_by: Vec::new(),
            queue_by: Vec::new(),
        }
    }

    /// The profile in use.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Replace the camera (the client calls this when its view changes).
    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
        self.cursor = camera.clamp_screen(self.cursor);
    }

    /// Current camera.
    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    /// Cursor in screen pixels.
    pub fn cursor_screen(&self) -> Vec2 {
        self.cursor
    }

    /// Cursor on the ground.
    pub fn cursor_world(&self) -> WorldPos {
        self.camera.screen_to_world(self.cursor)
    }

    /// Current selection (may hold units that died since; orders filter them).
    pub fn selection(&self) -> &UnitSet {
        &self.selection
    }

    /// Pending order mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The open wheel, if any.
    pub fn wheel(&self) -> Option<&Wheel> {
        self.wheel.as_ref().map(|(_, w)| w)
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

    /// Feed one event. `tick` is the sim tick to stamp any orders with.
    /// Events must be in time order; an earlier `t` is treated as "now".
    pub fn handle<W: WorldView + ?Sized>(
        &mut self,
        event: &InputEvent,
        world: &W,
        tick: Tick,
    ) -> Vec<Output> {
        let mut out = Vec::new();
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
            EventKind::Frame => self.frame(t, &mut cx),
        }
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

    /// Move the cursor and camera for the time since the last event, with
    /// the stick values that held over that interval.
    fn integrate(&mut self, to: u32) {
        let dt = to.saturating_sub(self.now);
        self.now = self.now.max(to);
        if dt == 0 {
            return;
        }
        if self.source == CursorSource::Stick && self.wheel.is_none() {
            let stick = stick_vector(self.left.0, self.left.1, self.tuning.stick_deadzone);
            if stick != Vec2::ZERO {
                let step = cursor_step(stick, dt, self.precision(), &self.tuning);
                self.cursor = self.camera.clamp_screen(sat_add(self.cursor, step));
            }
        }
        let pan = stick_vector(self.right.0, self.right.1, self.tuning.stick_deadzone);
        if pan != Vec2::ZERO {
            let d = pan.scale(per_second(self.tuning.pan_speed, dt));
            self.pan = sat_add(self.pan, Vec2::new(d.x, -d.y));
        }
    }

    // ---- raw events ----------------------------------------------------

    fn axis(&mut self, axis: Axis, value: i16) {
        match axis {
            Axis::LeftX => self.left.0 = value,
            Axis::LeftY => self.left.1 = value,
            Axis::RightX => self.right.0 = value,
            Axis::RightY => self.right.1 = value,
            Axis::LeftTrigger | Axis::RightTrigger => return,
        }
        if matches!(axis, Axis::LeftX | Axis::LeftY) {
            let raw = stick_vector(self.left.0, self.left.1, self.tuning.stick_deadzone);
            if raw != Vec2::ZERO {
                self.source = CursorSource::Stick;
            }
            if let Some((_, w)) = self.wheel.as_mut() {
                w.steer(stick_vector(
                    self.left.0,
                    self.left.1,
                    self.tuning.wheel_deadzone,
                ));
            }
        }
    }

    fn frame<W: WorldView + ?Sized>(&mut self, t: u32, cx: &mut Cx<'_, W>) {
        let dt = t.saturating_sub(self.last_frame.unwrap_or(t));
        self.last_frame = Some(t);
        let idle = stick_vector(self.left.0, self.left.1, self.tuning.stick_deadzone) == Vec2::ZERO;
        if self.source == CursorSource::Stick && idle && !self.precision() && self.wheel.is_none() {
            let targets: Vec<Vec2> = units_sorted(cx.world)
                .into_iter()
                .filter(|u| u.selectable && self.camera.contains_world(u.pos))
                .map(|u| self.camera.world_to_screen(u.pos))
                .collect();
            let (c, _) = magnetize(self.cursor, &targets, dt, &self.tuning);
            self.cursor = self.camera.clamp_screen(c);
        }
        self.paint_brush(cx);
    }

    fn down<W: WorldView + ?Sized>(&mut self, input: Input, t: u32, cx: &mut Cx<'_, W>) {
        if self.inputs.get(&input).is_some_and(|s| s.press.is_some()) {
            return; // repeat
        }
        // A different input decides any pending single taps.
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
        let pending = self.inputs.get_mut(&input).and_then(|s| s.pending.take());
        if let Some(p) = pending {
            let double = (p.with == with)
                .then(|| self.profile.find(&input, with.as_ref(), Gesture::DoubleTap))
                .flatten();
            match double {
                Some(a) => {
                    consumed = true;
                    self.trigger(&input, a, t, cx);
                }
                None => self.fire_tap(&input, p.with.as_ref(), t, cx),
            }
        }
        self.inputs.entry(input.clone()).or_default().press = Some(Press {
            at: t,
            with: with.clone(),
            consumed,
            hold_fired: false,
        });
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

    // ---- actions -------------------------------------------------------

    fn trigger<W: WorldView + ?Sized>(
        &mut self,
        input: &Input,
        action: Action,
        at: u32,
        cx: &mut Cx<'_, W>,
    ) {
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
            Action::CommandWheel => {
                let mut w = Wheel::open(WheelKind::Command, self.profile.command_wheel.clone());
                w.steer(stick_vector(
                    self.left.0,
                    self.left.1,
                    self.tuning.wheel_deadzone,
                ));
                self.wheel = Some((input.clone(), w));
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
                if n == 0 {
                    return;
                }
                let slots = (0..n)
                    .map(|i| Action::BuildOption {
                        index: u8::try_from(i).unwrap_or(u8::MAX),
                    })
                    .collect();
                let mut w = Wheel::open(WheelKind::Build, slots);
                w.steer(stick_vector(
                    self.left.0,
                    self.left.1,
                    self.tuning.wheel_deadzone,
                ));
                self.wheel = Some((input.clone(), w));
            }
            _ => {}
        }
    }

    fn end_held<W: WorldView + ?Sized>(&mut self, input: &Input, cx: &mut Cx<'_, W>) {
        self.precision_by.retain(|i| i != input);
        self.queue_by.retain(|i| i != input);
        if let Some((_, w)) = self.wheel.take_if(|(i, _)| i == input) {
            if let Some(a) = w.confirm() {
                self.instant(a, cx);
            }
        }
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
            Action::Cancel => self.cancel(cx),
            Action::SelectSameType { scope } => {
                self.client(ClientCommand::SelectSameType { scope }, cx)
            }
            Action::AssignGroup { group } => self.client(ClientCommand::AssignGroup { group }, cx),
            Action::RecallGroup { group } => self.client(ClientCommand::RecallGroup { group }, cx),
            Action::JumpToAlert => self.client(ClientCommand::JumpToAlert, cx),
            Action::CenterOnMonarch => self.client(ClientCommand::CenterOnMonarch, cx),
            Action::CycleIdleBuilder => self.cycle_idle_builder(cx),
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
            Action::CastSpellMode => {
                let units = own_units(cx.world, &self.selection);
                if let Some(s) = units
                    .iter()
                    .find_map(|&u| cx.world.spells(u).first().copied())
                {
                    self.mode = Mode::CastSpell(s);
                }
            }
            Action::BuildOption { index } => self.build_option(usize::from(index), cx),
            Action::Brush
            | Action::DragSelect
            | Action::Precision
            | Action::QueueModifier
            | Action::CommandWheel
            | Action::BuildWheel => {}
        }
    }

    fn set_mode_if_units<W: WorldView + ?Sized>(&mut self, mode: Mode, cx: &mut Cx<'_, W>) {
        if !own_units(cx.world, &self.selection).is_empty() {
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
        if self.wheel.take().is_some() {
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
            self.selection = UnitSet::new();
            cx.out.push(Output::Deselect);
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
            cx.out.push(Output::Sim {
                tick: cx.tick,
                command,
            });
        }
    }

    fn set_selection(&mut self, s: UnitSet) {
        if s != self.selection {
            self.selection = s;
            self.mode = Mode::None;
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
            ClientCommand::JumpToAlert
            | ClientCommand::CycleIdleBuilder
            | ClientCommand::CenterOnMonarch => {}
        }
        cx.out.push(Output::Client { command });
    }
}

fn sat_add(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x.saturating_add(b.x), a.y.saturating_add(b.y))
}

/// The lowest-id builder among `units`.
fn first_builder<W: WorldView + ?Sized>(world: &W, units: &UnitSet) -> Option<UnitId> {
    units
        .iter()
        .copied()
        .find(|&id| world.unit(id).is_some_and(|u| u.builder.is_some()))
}
