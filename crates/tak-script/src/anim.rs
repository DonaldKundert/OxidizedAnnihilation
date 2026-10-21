//! Per-piece animation state: what `move`, `turn`, `spin` and friends drive.
//!
//! All state is integer. Linear values are the model's signed 16.16 fixed
//! point ([`Fx`]); angles are [`Angle`] (65,536 per turn). Speeds are per
//! **second**, as the compiled scripts store them (`turn ... speed <500>`
//! compiles to `500 * 182 = 91022`). One [`PieceAnim::advance`] call is one
//! 30 Hz tick: each moving axis steps by `speed / 30`, with the remainder
//! carried to the next tick so 30 ticks cover exactly `speed` units.
//!
//! Details and status tags: `docs/formats/cob-vm.md`.

use tak_math::{Angle, Fx};

/// Sim ticks per second. Matches the sim's fixed tick rate.
pub const TICKS_PER_SECOND: i64 = 30;

/// Splits a per-second rate into per-tick steps with an exact remainder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
struct Rate {
    /// Units per second, never negative.
    per_second: i64,
    /// Carried remainder, `0..TICKS_PER_SECOND`.
    carry: i64,
}

impl Rate {
    fn new(per_second: i64) -> Rate {
        Rate {
            per_second: per_second.abs(),
            carry: 0,
        }
    }
    /// This tick's step.
    fn step(&mut self) -> i64 {
        let total = self.per_second + self.carry;
        self.carry = total % TICKS_PER_SECOND;
        total / TICKS_PER_SECOND
    }
}

/// One linear axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MoveAxis {
    pub pos: Fx,
    /// `Some` while a `move` is in progress.
    target: Option<Fx>,
    rate: Rate,
}

impl MoveAxis {
    /// True while a `move` has not reached its target (`wait-for-move` blocks).
    pub fn busy(&self) -> bool {
        self.target.is_some()
    }
    pub fn target(&self) -> Option<Fx> {
        self.target
    }
    fn set_now(&mut self, pos: Fx) {
        self.pos = pos;
        self.target = None;
    }
    fn start(&mut self, target: Fx, speed: i32) {
        if speed == 0 || target == self.pos {
            // Believed: speed 0 snaps (see cob-vm.md). Reaching the target
            // immediately also releases waiters.
            self.set_now(target);
            return;
        }
        self.target = Some(target);
        self.rate = Rate::new(i64::from(speed));
    }
    fn advance(&mut self) {
        let Some(target) = self.target else { return };
        let step = self.rate.step();
        let cur = i64::from(self.pos.raw());
        let tgt = i64::from(target.raw());
        let diff = tgt - cur;
        if diff.abs() <= step {
            self.set_now(target);
        } else {
            // |diff| > step so the result lies strictly between cur and tgt.
            let next = cur + step * diff.signum();
            self.pos = Fx::from_raw(next as i32);
        }
    }
}

/// One rotation axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TurnAxis {
    pub angle: Angle,
    /// `Some` while a `turn` is in progress.
    target: Option<Angle>,
    rate: Rate,
    spin: Option<Spin>,
}

/// Spin state: current and target angular speed (units per second, signed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
struct Spin {
    speed: i64,
    target_speed: i64,
    /// Speed change per tick, units per second. 0 = reach target at once.
    accel: i64,
    /// Fractional angle carry, signed.
    carry: i64,
    /// Set by `stop-spin`: the spin ends when speed reaches 0.
    stopping: bool,
}

impl TurnAxis {
    /// True while `wait-for-turn` should block: a `turn` is in progress, or
    /// a `stop-spin` is still decelerating. A steady spin does not block.
    pub fn busy(&self) -> bool {
        self.target.is_some() || self.spin.is_some_and(|s| s.stopping)
    }
    pub fn target(&self) -> Option<Angle> {
        self.target
    }
    pub fn spinning(&self) -> bool {
        self.spin.is_some()
    }
    fn set_now(&mut self, a: Angle) {
        self.angle = a;
        self.target = None;
        self.spin = None;
    }
    fn start(&mut self, target: Angle, speed: i32) {
        self.spin = None;
        if speed == 0 || target == self.angle {
            self.set_now(target);
            return;
        }
        self.target = Some(target);
        self.rate = Rate::new(i64::from(speed));
    }
    fn start_spin(&mut self, speed: i32, accel: i32) {
        self.target = None;
        let accel = i64::from(accel).abs();
        let speed = i64::from(speed);
        let mut s = self.spin.unwrap_or_default();
        s.target_speed = speed;
        s.accel = accel;
        s.stopping = false;
        if accel == 0 {
            s.speed = speed;
        }
        self.spin = Some(s);
    }
    fn stop_spin(&mut self, decel: i32) {
        let Some(mut s) = self.spin else { return };
        let decel = i64::from(decel).abs();
        if decel == 0 {
            self.spin = None;
            return;
        }
        s.target_speed = 0;
        s.accel = decel;
        s.stopping = true;
        self.spin = Some(s);
    }
    fn advance(&mut self) {
        if let Some(mut s) = self.spin {
            if s.accel != 0 && s.speed != s.target_speed {
                let d = s.target_speed - s.speed;
                s.speed += d.signum() * d.abs().min(s.accel);
            }
            let total = s.speed + s.carry;
            // Truncating division keeps the carry's sign equal to `total`'s.
            let step = total / TICKS_PER_SECOND;
            s.carry = total % TICKS_PER_SECOND;
            self.angle = Angle::from_raw(self.angle.raw().wrapping_add(step as u16));
            if s.stopping && s.speed == 0 {
                self.spin = None;
            } else {
                self.spin = Some(s);
            }
            return;
        }
        let Some(target) = self.target else { return };
        let step = self.rate.step();
        // Shortest way round (Believed, TA): delta in -32768..=32767.
        let delta = i64::from(self.angle.delta_to(target));
        if delta.abs() <= step {
            self.set_now(target);
        } else {
            let next = i64::from(self.angle.raw()) + step * delta.signum();
            self.angle = Angle::from_raw(next as u16);
        }
    }
}

/// Render flags a script can toggle. They change nothing in the sim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PieceFlags {
    pub visible: bool,
    /// `cache` / `dont-cache`. Believed (TA): a renderer hint.
    pub cached: bool,
    /// `shade` / `dont-shade`.
    pub shaded: bool,
    /// Cleared by `dont-shadow`.
    pub shadow: bool,
}

impl Default for PieceFlags {
    fn default() -> Self {
        PieceFlags {
            visible: true,
            cached: true,
            shaded: true,
            shadow: true,
        }
    }
}

/// Animation state of one script piece.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PieceAnim {
    /// Offsets added to the piece's model offset, x/y/z.
    pub moves: [MoveAxis; 3],
    /// Rotations about x/y/z.
    pub turns: [TurnAxis; 3],
    pub flags: PieceFlags,
}

/// A snapshot of one piece's pose, for renderers and hashing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PieceTransform {
    pub offset: [Fx; 3],
    pub rotation: [Angle; 3],
    pub visible: bool,
}

impl PieceAnim {
    pub fn advance(&mut self) {
        for m in &mut self.moves {
            m.advance();
        }
        for t in &mut self.turns {
            t.advance();
        }
    }
    pub fn transform(&self) -> PieceTransform {
        PieceTransform {
            offset: [self.moves[0].pos, self.moves[1].pos, self.moves[2].pos],
            rotation: [
                self.turns[0].angle,
                self.turns[1].angle,
                self.turns[2].angle,
            ],
            visible: self.flags.visible,
        }
    }
    pub(crate) fn move_to(&mut self, axis: usize, target: i32, speed: i32) {
        self.moves[axis].start(Fx::from_raw(target), speed);
    }
    pub(crate) fn move_now(&mut self, axis: usize, target: i32) {
        self.moves[axis].set_now(Fx::from_raw(target));
    }
    pub(crate) fn turn_to(&mut self, axis: usize, target: i32, speed: i32) {
        self.turns[axis].start(angle_of(target), speed);
    }
    pub(crate) fn turn_now(&mut self, axis: usize, target: i32) {
        self.turns[axis].set_now(angle_of(target));
    }
    pub(crate) fn spin(&mut self, axis: usize, speed: i32, accel: i32) {
        self.turns[axis].start_spin(speed, accel);
    }
    pub(crate) fn stop_spin(&mut self, axis: usize, decel: i32) {
        self.turns[axis].stop_spin(decel);
    }
}

/// Script angles are 32-bit; only the low 16 bits (one turn) matter.
fn angle_of(v: i32) -> Angle {
    Angle::from_raw(v as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_is_exact_over_a_second() {
        let mut r = Rate::new(100);
        let total: i64 = (0..30).map(|_| r.step()).sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn turn_takes_shortest_path() {
        let mut t = TurnAxis::default();
        t.start(Angle::from_raw(60000), 30 * 1000);
        t.advance();
        // 60000 is 5536 units the negative way round.
        assert_eq!(t.angle.raw(), 64536);
        for _ in 0..5 {
            t.advance();
        }
        assert_eq!(t.angle.raw(), 60000);
        assert!(!t.busy());
    }

    #[test]
    fn move_stops_on_target() {
        let mut m = MoveAxis::default();
        m.start(Fx::from_raw(-95), 30 * 10);
        for _ in 0..9 {
            m.advance();
            assert!(m.busy());
        }
        m.advance();
        assert_eq!(m.pos.raw(), -95);
        assert!(!m.busy());
    }

    #[test]
    fn spin_accelerates_then_stops() {
        let mut t = TurnAxis::default();
        t.start_spin(300, 0);
        t.advance();
        assert_eq!(t.angle.raw(), 10);
        assert!(!t.busy());
        t.stop_spin(100);
        assert!(t.busy());
        let mut n = 0;
        while t.busy() {
            t.advance();
            n += 1;
            assert!(n < 10);
        }
        assert_eq!(n, 3);
        assert!(!t.spinning());
    }
}
