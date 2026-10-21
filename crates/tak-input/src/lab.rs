//! The feel lab: a simulated controller player on synthetic worlds, for
//! comparing tuning presets and catching feel regressions headless.
//!
//! The player ([`Human`]) aims the left stick like a person: it reacts
//! after a delay, sees the cursor a little late (perception delay), plans
//! corrective sub-movements with gain and direction errors, has thumb
//! tremor, lets go of the stick when it believes it is on target, waits a
//! moment to confirm and then presses. It knows the stick's response curve
//! and base speed (a practised player) but not friction, snap or
//! acceleration, which act on it the way they act on a person.
//!
//! Tasks, each on dense (units 36–60 px apart) and sparse worlds:
//!
//! - **Select**: nothing selected, tap A on a given own unit. Wrong if
//!   another unit ends up selected; miss if nothing.
//! - **Attack**: an own unit selected, press X on a given enemy. Wrong if
//!   the order attacks or guards another unit; miss if it moves.
//! - **Brush**: aim at the centre of a cluster of own units with other own
//!   units around it, hold A and let go when the brush covers the cluster.
//!   Accuracy is the F1 score of the selection against the cluster.
//!
//! Everything is integer and seeded, so a run is reproducible and the same
//! trial (world, start point, human noise seed) is used for every preset.

use std::collections::VecDeque;

use tak_commands::{PlayerId, SimCommand, Target, Tick, UnitDefId, UnitId};
use tak_math::{Angle, Fx, Rng, Vec2};

use crate::binding::Profile;
use crate::cursor::{AXIS_MAX, curve, zoom_factor};
use crate::event::{Axis, Button, EventKind, InputEvent};
use crate::mapper::{Mapper, Output};
use crate::tuning::Tuning;
use crate::world::{Camera, UnitInfo, WorldView};

/// The simulated player's parameters (milliseconds, pixels, raw axis
/// units, thousandths).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Human {
    /// Reaction time range before the first stick movement.
    pub react_ms: (u32, u32),
    /// How old the cursor position the player acts on is.
    pub perceive_ms: u32,
    /// Closing time: the player wants a speed of `distance / tau`.
    pub tau_ms: u32,
    /// Sub-movement length range (noise is resampled per sub-movement).
    pub replan_ms: (u32, u32),
    /// Gain error per sub-movement, ± thousandths.
    pub gain_err_permille: u32,
    /// Direction error per sub-movement, ± degrees.
    pub angle_err_deg: i32,
    /// Thumb tremor per frame, ± raw axis units.
    pub jitter_raw: i32,
    /// The player lets go of the stick within this distance (perceived).
    pub stop_px: i32,
    /// The player presses if still within this distance after confirming.
    pub accept_px: i32,
    /// Confirmation pause range after letting go.
    pub confirm_ms: (u32, u32),
    /// Button down time for a tap.
    pub press_ms: u32,
    /// Reaction range to the brush covering the cluster.
    pub brush_react_ms: (u32, u32),
    /// A trial is a miss after this long.
    pub timeout_ms: u32,
    /// Share of aim trials, per mille, played "on the fly": the player
    /// presses as soon as the cursor looks close enough, without letting go
    /// and confirming first (the rest are careful).
    pub fly_permille: u32,
    /// On the fly: motor latency range between deciding and pressing (the
    /// stick is still held meanwhile).
    pub fly_latency_ms: (u32, u32),
}

impl Default for Human {
    fn default() -> Human {
        Human {
            react_ms: (180, 280),
            perceive_ms: 100,
            tau_ms: 170,
            replan_ms: (90, 150),
            gain_err_permille: 180,
            angle_err_deg: 7,
            jitter_raw: 500,
            stop_px: 8,
            accept_px: 12,
            confirm_ms: (90, 170),
            press_ms: 50,
            brush_react_ms: (150, 260),
            timeout_ms: 5000,
            fly_permille: 500,
            fly_latency_ms: (40, 90),
        }
    }
}

/// Lab run settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabConfig {
    /// Trials per task and density (brush: per density).
    pub trials: u32,
    /// Master seed.
    pub seed: u64,
    /// The simulated player.
    pub human: Human,
}

impl Default for LabConfig {
    fn default() -> LabConfig {
        LabConfig {
            trials: 200,
            seed: 0x5eed,
            human: Human::default(),
        }
    }
}

/// One preset's results.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabRow {
    /// Preset name.
    pub name: String,
    /// Select and attack trials run.
    pub aim_trials: u32,
    /// Mean time to acquire over correct trials, ms.
    pub acquire_ms: u32,
    /// Mean time to acquire, dense worlds only, ms.
    pub acquire_dense_ms: u32,
    /// Mean overshoot past the target along the approach, pixels x10.
    pub overshoot_dpx: u32,
    /// Wrong-target trials, per mille.
    pub wrong_permille: u32,
    /// Missed or timed-out trials, per mille.
    pub miss_permille: u32,
    /// Mean brush F1 score, per mille.
    pub brush_f1_permille: u32,
    /// Mean brush time (start to release), ms.
    pub brush_ms: u32,
}

/// Run every `(name, profile)` through the same trials.
pub fn run_lab(presets: &[(String, Profile)], cfg: &LabConfig) -> Vec<LabRow> {
    presets
        .iter()
        .map(|(name, p)| run_preset(name, p, cfg))
        .collect()
}

/// The presets the lab compares by default: WP-J and the named ones, each
/// on the matching controller profile.
pub fn default_presets() -> Vec<(String, Profile)> {
    let mut v = vec![("wp_j".to_owned(), Profile::wp_j_controller())];
    for name in ["console", "assisted", "precise"] {
        let mut p = Profile::default_controller();
        p.tuning = Tuning::preset(name).unwrap_or_default();
        v.push((name.to_owned(), p));
    }
    v
}

/// The results as a Markdown table.
pub fn table_markdown(rows: &[LabRow]) -> String {
    let mut s = String::from(
        "| Preset | Acquire ms | Acquire ms (dense) | Overshoot px | Wrong target | Miss | Errors | Brush F1 | Brush ms |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    for r in rows {
        s.push_str(&format!(
            "| {} | {} | {} | {}.{} | {}.{}% | {}.{}% | {}.{}% | {}.{}% | {} |\n",
            r.name,
            r.acquire_ms,
            r.acquire_dense_ms,
            r.overshoot_dpx / 10,
            r.overshoot_dpx % 10,
            r.wrong_permille / 10,
            r.wrong_permille % 10,
            r.miss_permille / 10,
            r.miss_permille % 10,
            (r.wrong_permille + r.miss_permille) / 10,
            (r.wrong_permille + r.miss_permille) % 10,
            r.brush_f1_permille / 10,
            r.brush_f1_permille % 10,
            r.brush_ms,
        ));
    }
    s
}

// ---- synthetic world ------------------------------------------------------

/// A plain world for the lab: player 0 is local, player 1 the enemy.
#[derive(Clone, Debug, Default)]
pub struct LabWorld {
    /// Units.
    pub units: Vec<UnitInfo>,
}

impl WorldView for LabWorld {
    fn local_player(&self) -> PlayerId {
        PlayerId(0)
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo)) {
        for u in &self.units {
            f(u);
        }
    }
    fn is_enemy(&self, owner: PlayerId) -> bool {
        owner != PlayerId(0)
    }
}

/// Lab camera: 1280x960 at the reference zoom of 0.5 world units per pixel.
fn lab_camera() -> Camera {
    Camera {
        origin: Vec2::ZERO,
        world_per_px: Fx::HALF,
        width_px: 1280,
        height_px: 960,
    }
}

const PICK_RADIUS: i32 = 12; // world units = 24 px

fn unit(id: u32, owner: u8, x: i32, y: i32) -> UnitInfo {
    UnitInfo {
        id: UnitId::new(id, 0),
        pos: Vec2::from_ints(x, y),
        radius: Fx::from_int(PICK_RADIUS),
        owner: PlayerId(owner),
        def: UnitDefId(10 + u32::from(owner)),
        selectable: true,
        builder: None,
        under_construction: false,
    }
}

/// Scatter `n` units (world units) at least `gap` apart inside the view,
/// owners alternating with the given enemy share.
fn scatter(rng: &mut Rng, n: u32, gap: i32, first_id: u32, enemy_of_4: u32) -> Vec<UnitInfo> {
    let mut v: Vec<UnitInfo> = Vec::new();
    let mut tries = 0;
    while (v.len() as u32) < n && tries < 4000 {
        tries += 1;
        let x = rng.range_i32(40, 600);
        let y = rng.range_i32(40, 440);
        let p = Vec2::from_ints(x, y);
        if v.iter().any(|u| u.pos.distance(p) < Fx::from_int(gap)) {
            continue;
        }
        let owner = u8::from(rng.below(4) < enemy_of_4);
        v.push(unit(first_id + v.len() as u32, owner, x, y));
    }
    v
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Density {
    Dense,
    Sparse,
}

fn aim_world(rng: &mut Rng, d: Density) -> LabWorld {
    let units = match d {
        // Clumps of armies: 36 px minimum spacing.
        Density::Dense => scatter(rng, 70, 18, 1, 2),
        Density::Sparse => scatter(rng, 14, 80, 1, 2),
    };
    LabWorld { units }
}

// ---- rig ------------------------------------------------------------------

struct Rig {
    m: Mapper,
    w: LabWorld,
    t: u32,
    frame_no: u32,
    stick: (i16, i16),
    hist: VecDeque<(u32, Vec2)>,
    outs: Vec<Output>,
}

impl Rig {
    fn new(profile: &Profile, w: LabWorld) -> Rig {
        let mut m = Mapper::new(profile.clone());
        m.set_camera(lab_camera());
        Rig {
            m,
            w,
            t: 1000,
            frame_no: 0,
            stick: (0, 0),
            hist: VecDeque::new(),
            outs: Vec::new(),
        }
    }

    fn ev(&mut self, kind: EventKind) {
        let tick = Tick(self.t * 30 / 1000);
        let o = self.m.handle(&InputEvent::new(self.t, kind), &self.w, tick);
        self.outs.extend(o);
    }

    fn set_stick(&mut self, x: i16, y: i16) {
        if x != self.stick.0 {
            self.ev(EventKind::Axis {
                axis: Axis::LeftX,
                value: x,
            });
        }
        if y != self.stick.1 {
            self.ev(EventKind::Axis {
                axis: Axis::LeftY,
                value: y,
            });
        }
        self.stick = (x, y);
    }

    /// One 60 Hz frame (17, 17, 16 ms).
    fn frame(&mut self) {
        self.frame_no += 1;
        self.t += if self.frame_no % 3 == 0 { 16 } else { 17 };
        self.ev(EventKind::Frame);
        let c = self.m.cursor_screen();
        self.hist.push_back((self.t, c));
        while self.hist.len() > 64 {
            self.hist.pop_front();
        }
    }

    fn frames_for(&mut self, ms: u32) {
        let end = self.t + ms;
        while self.t < end {
            self.frame();
        }
    }

    fn perceived(&self, delay: u32) -> Vec2 {
        let at = self.t.saturating_sub(delay);
        self.hist
            .iter()
            .rev()
            .find(|(t, _)| *t <= at)
            .or(self.hist.front())
            .map_or(self.m.cursor_screen(), |&(_, c)| c)
    }

    fn button(&mut self, b: Button, down: bool) {
        self.ev(if down {
            EventKind::ButtonDown { button: b }
        } else {
            EventKind::ButtonUp { button: b }
        });
    }

    fn tap(&mut self, b: Button, press_ms: u32) {
        self.button(b, true);
        self.frames_for(press_ms);
        self.button(b, false);
        self.frame();
    }

    /// Warp the virtual cursor (stick source) to a screen point.
    fn warp(&mut self, s: Vec2) {
        self.m.warp_cursor(s);
        self.hist.clear();
        self.hist.push_back((self.t, s));
    }
}

/// Base cursor speed (px/s) the player expects for deflection `m`.
fn expected_speed(t: &Tuning, m: Fx) -> Fx {
    Fx::from_int(t.cursor_speed.clamp(0, 32767))
        .saturating_mul(curve(m, t.cursor_curve_pct))
        .saturating_mul(zoom_factor(Fx::HALF, t))
}

/// The deflection (0..=1) whose expected speed is `v` px/s.
fn deflection_for(t: &Tuning, v: Fx) -> Fx {
    if expected_speed(t, Fx::ONE) <= v {
        return Fx::ONE;
    }
    let (mut lo, mut hi) = (Fx::ZERO, Fx::ONE);
    for _ in 0..18 {
        let mid = Fx::from_raw((lo.raw() + hi.raw()) / 2);
        if expected_speed(t, mid) < v {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// Raw stick magnitude for a shaped deflection `m`.
fn raw_for(t: &Tuning, m: Fx) -> i32 {
    if m <= Fx::ZERO {
        return 0;
    }
    let inner = t.stick_deadzone.clamp(0, AXIS_MAX - 2);
    let full = (AXIS_MAX - t.stick_outer_deadzone.max(0)).clamp(inner + 1, AXIS_MAX);
    let span = i64::from(full - inner);
    (i64::from(inner) + 1 + span * i64::from(m.raw().min(Fx::ONE_RAW)) / i64::from(Fx::ONE_RAW))
        .min(i64::from(AXIS_MAX)) as i32
}

fn rotate(v: Vec2, a: Angle) -> Vec2 {
    let (s, c) = a.sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

fn signed_deg(rng: &mut Rng, max_deg: i32) -> Angle {
    let d = rng.range_i32(-max_deg * 16, max_deg * 16 + 1);
    let raw = (i64::from(d) * 65536 / (360 * 16)) as i32;
    Angle::from_raw(raw as u16)
}

/// Aim at `target` (screen px) until the player decides to press. Returns
/// `(ok, overshoot in px)`; `ok` is false on timeout.
/// How the player finishes an aim.
#[derive(Clone, Copy)]
struct Goal {
    stop_px: i32,
    accept_px: i32,
    fly: bool,
}

/// Aim at `target` (screen px) until the player decides to press. Returns
/// `(ok, overshoot in px)`; `ok` is false on timeout.
fn aim(
    rig: &mut Rig,
    target: Vec2,
    h: &Human,
    rng: &mut Rng,
    deadline: u32,
    goal: Goal,
) -> (bool, Fx) {
    let t = *rig.m.tuning();
    let start = rig.m.cursor_screen();
    let d0 = target - start;
    let dist0 = d0.length();
    let dir0 = d0.normalize().unwrap_or(Vec2::new(Fx::ONE, Fx::ZERO));
    let mut max_proj = Fx::ZERO;
    let mut track = |rig: &Rig| {
        let p = (rig.m.cursor_screen() - start).dot(dir0);
        if p > max_proj {
            max_proj = p;
        }
    };
    let mut gain = Fx::ONE;
    let mut err = Angle::ZERO;
    let mut replan_at = 0;
    loop {
        if rig.t >= deadline {
            rig.set_stick(0, 0);
            return (false, (max_proj - dist0).max(Fx::ZERO));
        }
        let seen = rig.perceived(h.perceive_ms);
        let off = target - seen;
        let dist = off.length();
        if goal.fly && dist <= Fx::from_int(goal.accept_px) {
            // Press on the fly: the thumb is still on the stick while the
            // press goes in.
            let end = rig.t + rng.range(h.fly_latency_ms.0, h.fly_latency_ms.1 + 1);
            while rig.t < end {
                rig.frame();
                track(rig);
            }
            rig.set_stick(0, 0);
            return (true, (max_proj - dist0).max(Fx::ZERO));
        }
        if dist <= Fx::from_int(goal.stop_px) {
            // Let go and confirm.
            rig.set_stick(0, 0);
            let wait = rng.range(h.confirm_ms.0, h.confirm_ms.1 + 1);
            let end = rig.t + wait;
            while rig.t < end {
                rig.frame();
                track(rig);
            }
            let seen = rig.perceived(h.perceive_ms);
            if (target - seen).length() <= Fx::from_int(goal.accept_px) {
                return (true, (max_proj - dist0).max(Fx::ZERO));
            }
            continue;
        }
        if rig.t >= replan_at {
            let g = rng.range_i32(
                -(h.gain_err_permille as i32),
                h.gain_err_permille as i32 + 1,
            );
            gain = Fx::from_ratio(1000 + g, 1000).unwrap_or(Fx::ONE);
            err = signed_deg(rng, h.angle_err_deg);
            replan_at = rig.t + rng.range(h.replan_ms.0, h.replan_ms.1 + 1);
        }
        // Wanted speed: distance / tau.
        let v = dist
            .saturating_mul_int_ratio(1000, h.tau_ms.max(1) as i32)
            .min(Fx::from_int(30000));
        let m = deflection_for(&t, v).saturating_mul(gain).min(Fx::ONE);
        let raw = raw_for(&t, m);
        let dir = rotate(off.normalize().unwrap_or(dir0), err);
        let jx = rng.range_i32(-h.jitter_raw, h.jitter_raw + 1);
        let jy = rng.range_i32(-h.jitter_raw, h.jitter_raw + 1);
        // Screen +y is down, stick +y is up.
        let sx = dir.x.saturating_mul_int(raw).to_int_round() + jx;
        let sy = -dir.y.saturating_mul_int(raw).to_int_round() + jy;
        rig.set_stick(
            sx.clamp(-AXIS_MAX, AXIS_MAX) as i16,
            sy.clamp(-AXIS_MAX, AXIS_MAX) as i16,
        );
        rig.frame();
        track(rig);
    }
}

trait FxExt {
    fn saturating_mul_int(self, n: i32) -> Fx;
    fn saturating_mul_int_ratio(self, num: i32, den: i32) -> Fx;
}

impl FxExt for Fx {
    fn saturating_mul_int(self, n: i32) -> Fx {
        let r = i64::from(self.raw()) * i64::from(n);
        Fx::from_raw(r.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
    }
    fn saturating_mul_int_ratio(self, num: i32, den: i32) -> Fx {
        let r = i64::from(self.raw()) * i64::from(num) / i64::from(den.max(1));
        Fx::from_raw(r.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
    }
}

#[derive(Default)]
struct Acc {
    trials: u32,
    hits: u32,
    hit_ms: u64,
    dense_hits: u32,
    dense_ms: u64,
    over: i64,
    wrong: u32,
    miss: u32,
    brush_n: u32,
    brush_f1: u64,
    brush_ms: u64,
}

fn trial_rng(seed: u64, kind: u64, density: Density, i: u32) -> (Rng, Rng) {
    let base = seed
        ^ (kind << 40)
        ^ (u64::from(density == Density::Dense) << 32)
        ^ u64::from(i).wrapping_mul(0x9E37_79B9);
    (Rng::with_stream(base, 1), Rng::with_stream(base, 2))
}

/// A start point 150..600 px from `target`, inside the view.
fn start_for(rng: &mut Rng, target: Vec2) -> Vec2 {
    for _ in 0..64 {
        let a = Angle::from_raw(rng.next_u32() as u16);
        let r = rng.range_i32(150, 600);
        let p = target + Vec2::from_angle(a).scale(Fx::from_int(r));
        if p.x > Fx::from_int(10)
            && p.y > Fx::from_int(10)
            && p.x < Fx::from_int(1270)
            && p.y < Fx::from_int(950)
        {
            return p;
        }
    }
    Vec2::from_ints(640, 480)
}

fn run_preset(name: &str, profile: &Profile, cfg: &LabConfig) -> LabRow {
    let h = cfg.human;
    let mut acc = Acc::default();
    for density in [Density::Dense, Density::Sparse] {
        for i in 0..cfg.trials {
            for attack in [false, true] {
                let (mut wr, mut hr) = trial_rng(cfg.seed, u64::from(attack), density, i);
                let w = aim_world(&mut wr, density);
                let (owner, other) = if attack { (1, 0) } else { (0, 1) };
                let cands: Vec<&UnitInfo> = w
                    .units
                    .iter()
                    .filter(|u| u.owner == PlayerId(owner))
                    .collect();
                let selector = w.units.iter().find(|u| u.owner == PlayerId(other)).copied();
                if cands.is_empty() || (attack && selector.is_none()) {
                    continue;
                }
                let target = *cands[wr.below(cands.len() as u32) as usize];
                let mut rig = Rig::new(profile, w);
                let cam = lab_camera();
                if let Some(sel) = selector.filter(|_| attack) {
                    rig.warp(cam.world_to_screen(sel.pos));
                    rig.tap(Button::A, h.press_ms);
                    rig.frames_for(600);
                }
                let tscreen = cam.world_to_screen(target.pos);
                let start = start_for(&mut wr, tscreen);
                rig.warp(start);
                rig.outs.clear();
                let t0 = rig.t;
                rig.frames_for(hr.range(h.react_ms.0, h.react_ms.1 + 1));
                let goal = Goal {
                    stop_px: h.stop_px,
                    accept_px: h.accept_px,
                    fly: hr.below(1000) < h.fly_permille,
                };
                let (ok, over) = aim(&mut rig, tscreen, &h, &mut hr, t0 + h.timeout_ms, goal);
                acc.trials += 1;
                acc.over += i64::from(over.raw()) * 10 / i64::from(Fx::ONE_RAW);
                if !ok {
                    acc.miss += 1;
                    continue;
                }
                rig.outs.clear();
                if attack {
                    rig.tap(Button::X, h.press_ms);
                } else {
                    rig.tap(Button::A, h.press_ms);
                }
                let elapsed = rig.t - t0;
                let verdict = if attack {
                    attack_verdict(&rig.outs, target.id)
                } else {
                    select_verdict(rig.m.selection(), target.id)
                };
                match verdict {
                    Verdict::Hit => {
                        acc.hits += 1;
                        acc.hit_ms += u64::from(elapsed);
                        if density == Density::Dense {
                            acc.dense_hits += 1;
                            acc.dense_ms += u64::from(elapsed);
                        }
                    }
                    Verdict::Wrong => acc.wrong += 1,
                    Verdict::Miss => acc.miss += 1,
                }
            }
            // Brush.
            let (mut wr, mut hr) = trial_rng(cfg.seed, 7, density, i);
            if let Some((f1, ms)) = brush_trial(profile, density, &h, &mut wr, &mut hr) {
                acc.brush_n += 1;
                acc.brush_f1 += u64::from(f1);
                acc.brush_ms += u64::from(ms);
            }
        }
    }
    let div = |a: u64, b: u32| if b == 0 { 0 } else { (a / u64::from(b)) as u32 };
    LabRow {
        name: name.to_owned(),
        aim_trials: acc.trials,
        acquire_ms: div(acc.hit_ms, acc.hits),
        acquire_dense_ms: div(acc.dense_ms, acc.dense_hits),
        overshoot_dpx: div(acc.over.max(0) as u64, acc.trials),
        wrong_permille: div(u64::from(acc.wrong) * 1000, acc.trials),
        miss_permille: div(u64::from(acc.miss) * 1000, acc.trials),
        brush_f1_permille: div(acc.brush_f1, acc.brush_n),
        brush_ms: div(acc.brush_ms, acc.brush_n),
    }
}

enum Verdict {
    Hit,
    Wrong,
    Miss,
}

fn select_verdict(sel: &tak_commands::UnitSet, want: UnitId) -> Verdict {
    if sel.len() == 1 && sel.contains(want) {
        Verdict::Hit
    } else if sel.is_empty() {
        Verdict::Miss
    } else {
        Verdict::Wrong
    }
}

fn attack_verdict(outs: &[Output], want: UnitId) -> Verdict {
    for o in outs {
        if let Output::Sim { command, .. } = o {
            return match command {
                SimCommand::Attack {
                    target: Target::Unit(id),
                    ..
                } if *id == want => Verdict::Hit,
                SimCommand::Attack { .. } | SimCommand::Guard { .. } => Verdict::Wrong,
                _ => Verdict::Miss,
            };
        }
    }
    Verdict::Miss
}

/// One brush trial: `(F1 per mille, time ms)`.
fn brush_trial(
    profile: &Profile,
    density: Density,
    h: &Human,
    wr: &mut Rng,
    hr: &mut Rng,
) -> Option<(u32, u32)> {
    // Cluster of 5..10 own units within radius r around c, other own units
    // in a ring outside it.
    let r = match density {
        Density::Dense => wr.range_i32(24, 48),
        Density::Sparse => wr.range_i32(36, 72),
    };
    let c = Vec2::from_ints(wr.range_i32(200, 440), wr.range_i32(150, 330));
    let k = wr.range(5, 11);
    let mut units = Vec::new();
    let mut id = 1;
    let mut tries = 0;
    while (units.len() as u32) < k && tries < 2000 {
        tries += 1;
        let a = Angle::from_raw(wr.next_u32() as u16);
        let d = wr.range_i32(0, r + 1);
        let p = c + Vec2::from_angle(a).scale(Fx::from_int(d));
        if units
            .iter()
            .any(|u: &UnitInfo| u.pos.distance(p) < Fx::from_int(10))
        {
            continue;
        }
        let (x, y) = (p.x.to_int_round(), p.y.to_int_round());
        units.push(unit(id, 0, x, y));
        id += 1;
    }
    let cluster: Vec<UnitId> = units.iter().map(|u| u.id).collect();
    let ring = match density {
        Density::Dense => 14,
        Density::Sparse => 6,
    };
    let (gap_lo, gap_hi) = match density {
        Density::Dense => (8, 40),
        Density::Sparse => (20, 90),
    };
    for _ in 0..ring {
        let a = Angle::from_raw(wr.next_u32() as u16);
        let d = r + wr.range_i32(gap_lo, gap_hi);
        let p = c + Vec2::from_angle(a).scale(Fx::from_int(d));
        units.push(unit(id, 0, p.x.to_int_round(), p.y.to_int_round()));
        id += 1;
    }
    // Needed radius from the cluster centre.
    let mut rig = Rig::new(profile, LabWorld { units });
    let cam = lab_camera();
    let tscreen = cam.world_to_screen(c);
    let start = start_for(wr, tscreen);
    rig.warp(start);
    let t0 = rig.t;
    rig.frames_for(hr.range(h.react_ms.0, h.react_ms.1 + 1));
    // A cluster only needs a rough aim: within a third of its radius.
    let rough = (r * 2 / 3).max(h.accept_px);
    let goal = Goal {
        stop_px: rough.max(h.stop_px),
        accept_px: rough,
        fly: false,
    };
    let (ok, _) = aim(&mut rig, tscreen, h, hr, t0 + h.timeout_ms, goal);
    if !ok {
        return Some((0, h.timeout_ms));
    }
    rig.button(Button::A, true);
    // Hold until the brush (as seen) covers the cluster from where the
    // cursor actually is, then react.
    let deadline = rig.t + 4000;
    let mut covered_at = None;
    while rig.t < deadline {
        rig.frame();
        let centre = rig.m.cursor_world();
        let need = rig
            .w
            .units
            .iter()
            .filter(|u| cluster.contains(&u.id))
            .map(|u| u.pos.distance(centre))
            .max()
            .unwrap_or(Fx::ZERO)
            + Fx::from_int(2);
        if let Some((_, radius)) = rig.m.brush()
            && radius >= need
            && covered_at.is_none()
        {
            covered_at =
                Some(rig.t + h.perceive_ms + hr.range(h.brush_react_ms.0, h.brush_react_ms.1 + 1));
        }
        if covered_at.is_some_and(|at| rig.t >= at) {
            break;
        }
    }
    rig.button(Button::A, false);
    rig.frame();
    let ms = rig.t - t0;
    let sel = rig.m.selection();
    let inter = sel.iter().filter(|id| cluster.contains(id)).count() as u32;
    let denom = sel.len() as u32 + cluster.len() as u32;
    let f1 = (2000 * inter).checked_div(denom).unwrap_or(0);
    Some((f1, ms))
}
