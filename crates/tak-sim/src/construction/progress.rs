//! Build progress arithmetic, all integer.
//!
//! A frame needs [`total_work`] work units: `buildtime * 30` in raw 16.16
//! (`buildtime` FBI units times ticks per second). Each tick a builder adds
//! [`worker_step`] = `workertime` in raw 16.16. So a lone builder finishes
//! in `ceil(30 * buildtime / workertime)` ticks, i.e. `buildtime /
//! workertime` seconds (**Believed**: TA's rule, Nanolathe "Construction
//! arithmetic"; TA truncates `workertime / 30` to an integer first, which
//! would give every Kingdoms builder, `workertime = 10`, zero progress, so
//! we keep the exact quotient).
//!
//! Mana is charged cumulatively: after `done` work the frame has paid
//! `floor(cost * done / total)`, so the per-tick charges add up to exactly
//! `buildcost` with no rounding drift. When the economy grants only part
//! of a tick's charge, progress advances only as far as the paid mana
//! covers (TA instead refuses the whole step; see `docs/construction.md`).

use tak_math::Fx;

use crate::TICKS_PER_SECOND;

/// Progress of a unit under construction (a "frame").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// Work done, raw 16.16 workertime-ticks.
    pub done: i64,
    /// Work needed ([`total_work`]); at least 1.
    pub total: i64,
    /// Mana paid so far, raw 16.16.
    pub paid: i64,
}

/// Work units a definition needs.
pub fn total_work(buildtime_raw: i64) -> i64 {
    buildtime_raw
        .max(0)
        .saturating_mul(i64::from(TICKS_PER_SECOND))
        .max(1)
}

/// Work units one builder adds per tick.
pub fn worker_step(workertime: i32) -> i64 {
    i64::from(workertime.max(0)) << 16
}

/// Ticks a lone builder needs from an empty frame with full mana.
pub fn ticks_to_build(buildtime_raw: i64, workertime: i32) -> Option<i64> {
    let step = worker_step(workertime);
    (step > 0).then(|| {
        let t = total_work(buildtime_raw);
        (t + step - 1) / step
    })
}

/// Raw 16.16 mana of a whole-unit cost.
pub fn cost_raw(buildcost: i64) -> i64 {
    buildcost.max(0).saturating_mul(1 << 16)
}

/// Mana a frame has owed in total after `done` work.
fn owed(cost: i64, done: i64, total: i64) -> i64 {
    (i128::from(cost) * i128::from(done) / i128::from(total.max(1))) as i64
}

/// The most work `paid` mana covers: the largest `d` with
/// `owed(d) <= paid`.
fn work_for(cost: i64, paid: i64, total: i64) -> i64 {
    if cost <= 0 {
        return total;
    }
    // owed(d) <= p  <=>  cost * d < (p + 1) * total
    let lim = (i128::from(paid) + 1) * i128::from(total);
    let d = (lim + i128::from(cost) - 1) / i128::from(cost) - 1;
    d.clamp(0, i128::from(total)) as i64
}

impl Frame {
    /// An empty frame.
    pub fn new(total: i64) -> Frame {
        Frame {
            done: 0,
            total: total.max(1),
            paid: 0,
        }
    }

    /// `true` once all work is done.
    pub fn is_complete(&self) -> bool {
        self.done >= self.total
    }

    /// Percent still to build, rounded up: 100 for an empty frame, 0 only
    /// when complete (scripts wait for port 17 to read 0).
    pub fn percent_left(&self) -> i32 {
        let left = i128::from((self.total - self.done).max(0));
        let total = i128::from(self.total.max(1));
        ((left * 100 + total - 1) / total) as i32
    }

    /// One builder's step: how far it would go and the mana it asks for
    /// (raw 16.16, capped at `Fx::MAX`, with the step shortened to match).
    pub fn plan(&self, cost: i64, step: i64) -> (i64, Fx) {
        let mut to = self.done.saturating_add(step.max(0)).min(self.total);
        let mut want = owed(cost, to, self.total) - self.paid;
        if want > i64::from(i32::MAX) {
            to = work_for(cost, self.paid + i64::from(i32::MAX), self.total).max(self.done);
            want = owed(cost, to, self.total) - self.paid;
        }
        (to, Fx::from_raw(want.clamp(0, i64::from(i32::MAX)) as i32))
    }

    /// Commit a planned step given the mana granted. Returns the work
    /// added.
    pub fn commit(&mut self, cost: i64, to: i64, want: Fx, granted: Fx) -> i64 {
        let before = self.done;
        let granted = i64::from(granted.raw().clamp(0, want.raw().max(0)));
        if granted >= i64::from(want.raw()) {
            self.done = to;
            self.paid += i64::from(want.raw());
        } else {
            self.paid += granted;
            self.done = work_for(cost, self.paid, self.total).clamp(self.done, to);
        }
        self.done - before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lone_builder_times() {
        // ARAWALL (buildtime 65) by a monarch (workertime 10): 6.5 s.
        assert_eq!(ticks_to_build(65 << 16, 10), Some(195));
        // ARAKEEP 600 / 10 = 60 s; ARASWORD from a keep 99 / 10 = 9.9 s.
        assert_eq!(ticks_to_build(600 << 16, 10), Some(1800));
        assert_eq!(ticks_to_build(99 << 16, 10), Some(297));
        assert_eq!(ticks_to_build(0, 10), Some(1));
        assert_eq!(ticks_to_build(65 << 16, 0), None);
    }

    #[test]
    fn full_grant_pays_exactly_the_cost() {
        let cost = cost_raw(75);
        let mut f = Frame::new(total_work(65 << 16));
        assert_eq!(f.percent_left(), 100);
        let mut ticks = 0;
        while !f.is_complete() {
            let (to, want) = f.plan(cost, worker_step(10));
            f.commit(cost, to, want, want);
            ticks += 1;
            if !f.is_complete() {
                assert!(f.percent_left() > 0);
            }
        }
        assert_eq!(ticks, 195);
        assert_eq!(f.paid, cost);
        assert_eq!(f.percent_left(), 0);
    }

    #[test]
    fn partial_grants_slow_progress_in_proportion() {
        let cost = cost_raw(300);
        let mut f = Frame::new(total_work(30 << 16));
        let mut ticks = 0;
        while !f.is_complete() {
            let (to, want) = f.plan(cost, worker_step(10));
            let half = Fx::from_raw((want.raw() + 1) / 2);
            f.commit(cost, to, want, half);
            assert!(f.paid <= cost);
            ticks += 1;
            assert!(ticks < 1000);
        }
        // 90 ticks at full mana; about twice that at half.
        assert!((178..=200).contains(&ticks), "{ticks}"); // plus a short tail as the deficit is paid off
        assert_eq!(f.paid, cost);
        // Nothing granted: nothing built.
        let mut g = Frame::new(100);
        let (to, want) = g.plan(cost, 50);
        assert_eq!(g.commit(cost, to, want, Fx::ZERO), 0);
        // Free units build at full speed.
        let (to, want) = g.plan(0, 50);
        assert_eq!(want, Fx::ZERO);
        assert_eq!(g.commit(0, to, want, Fx::ZERO), 50);
    }

    #[test]
    fn huge_costs_cap_the_request() {
        let cost = cost_raw(360_000);
        let mut f = Frame::new(total_work(1 << 16));
        let (to, want) = f.plan(cost, worker_step(50));
        assert!(want > Fx::from_int(32_000), "{want:?}");
        assert!(to < f.total && to > 0);
        f.commit(cost, to, want, want);
        assert!(f.paid <= cost_raw(360_000));
    }
}
