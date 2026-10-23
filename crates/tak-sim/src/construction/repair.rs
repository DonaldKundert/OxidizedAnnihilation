//! **Chosen** construction-normalized repair policy, not measured original
//! arithmetic. Work generates fractional HP; production mana pays for that
//! exact fraction. Credit lives on the target, surviving cancel/reissue and
//! healer changes. Neither passive regeneration nor personal mana is involved.

use tak_math::Fx;

use crate::hash::StateHasher;

/// Fractional repair and payment state for a completed target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RepairCredit {
    /// Fractional HP numerator; denominator is `buildtime_raw * 30`.
    pub hp_remainder: i64,
    /// Fractional raw mana numerator; denominator is `max_health * total`.
    pub mana_remainder: i128,
    /// Raw mana already paid but not yet converted into fractional HP.
    pub prepaid: i64,
}

impl RepairCredit {
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.u64(self.hp_remainder as u64);
        h.u64(self.mana_remainder as u64);
        h.u64((self.mana_remainder >> 64) as u64);
        h.u64(self.prepaid as u64);
    }

    /// One work step. The callback receives a bounded positive production-mana
    /// demand and returns its grant. A partial grant retains all paid credit.
    /// Zero buildtime refuses repair; zero cost still obeys the work rate.
    /// Inputs are validated `BuildDef` values; i128 products fit even at their
    /// validation maxima. Work/payment are capped to the remaining HP exactly.
    pub(super) fn step(
        &mut self,
        max_health: i32,
        missing: i32,
        total: i64,
        cost: i64,
        worker_step: i64,
        mut drain: impl FnMut(Fx) -> Fx,
    ) -> i32 {
        if max_health <= 0 || missing <= 0 || total <= 0 || worker_step <= 0 {
            return 0;
        }
        let max = i128::from(max_health);
        let total = i128::from(total);
        let cost = i128::from(cost.max(0));
        let denominator = max * total;
        // Numerators are fractional HP, not rounded raw work units. Capping
        // here avoids charging an entire work quantum for a tiny HP deficit.
        let cap = i128::from(missing) * total - i128::from(self.hp_remainder);
        let desired = (max * i128::from(worker_step)).min(cap.max(0));
        let owed = (cost * desired + self.mana_remainder) / denominator;
        let want = (owed - i128::from(self.prepaid)).clamp(0, i128::from(i32::MAX));
        let granted = if want > 0 {
            i128::from(drain(Fx::from_raw(want as i32)).raw()).clamp(0, want)
        } else {
            0
        };
        let budget = i128::from(self.prepaid) + granted;
        let available = if cost == 0 {
            desired
        } else {
            // floor((cost*x + remainder)/denominator) <= budget.
            (((budget + 1) * denominator - self.mana_remainder - 1) / cost).clamp(0, desired)
        };
        let invoice = cost * available + self.mana_remainder;
        let charged = invoice / denominator;
        self.mana_remainder = invoice % denominator;
        self.prepaid = (budget - charged) as i64;
        let hp = i128::from(self.hp_remainder) + available;
        let gain = (hp / total).min(i128::from(missing)) as i32;
        self.hp_remainder = if gain >= missing {
            0
        } else {
            (hp % total) as i64
        };
        gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_fractional_rate_and_billing_survive_many_full_health_intervals() {
        let mut c = RepairCredit::default();
        let mut paid = 0i64;
        // 100 HP in 300 ticks, cost 7 mana. Each 1-HP repair must retain
        // billing remainder even though reaching full discards HP credit.
        for _ in 0..100 {
            let mut gain = 0;
            for _ in 0..3 {
                gain += c.step(100, 1, 300, 7 << 16, 1, |want| {
                    paid += i64::from(want.raw());
                    want
                });
            }
            assert_eq!(gain, 1);
        }
        assert_eq!(paid, 7 << 16);
        assert_eq!(c, RepairCredit::default());
    }

    #[test]
    fn partial_grant_keeps_prepaid_sub_hp_work_and_never_heals_for_free_forever() {
        let mut c = RepairCredit::default();
        let mut health = 0;
        let mut paid = 0i64;
        for _ in 0..1_000 {
            health += c.step(100, 100 - health, 30, 100 << 16, 10, |want| {
                let g = Fx::from_raw(want.raw().min(1000));
                paid += i64::from(g.raw());
                g
            });
        }
        assert!(health > 0 && health < 100);
        assert!(i64::from(health) * 65536 <= paid);
        let before = health;
        for _ in 0..1000 {
            health += c.step(100, 100 - health, 30, 100 << 16, 10, |_| Fx::ZERO);
        }
        assert!(health <= before + 1);
    }

    #[test]
    fn caps_tiny_deficits_zero_policy_and_extremes_without_overpayment() {
        let mut c = RepairCredit::default();
        let mut paid = 0;
        assert_eq!(
            c.step(100, 1, 1, 100 << 16, 1000, |w| {
                paid += w.raw();
                w
            }),
            1
        );
        assert_eq!(paid, 1 << 16);
        assert_eq!(c.step(100, 1, 0, 0, 1000, |_| panic!("zero time")), 0);
        assert_eq!(c.step(100, 1, 300, 0, 1, |_| panic!("free")), 0);
        for _ in 0..2 {
            c.step(100, 1, 300, 0, 1, |_| panic!("free"));
        }
        assert_eq!(c.hp_remainder, 0);
        let mut c = RepairCredit::default();
        for _ in 0..100 {
            c.step(
                i32::MAX,
                i32::MAX,
                (i64::from(i32::MAX) << 16) * 30,
                i64::from(i32::MAX) << 16,
                i64::from(i32::MAX) << 16,
                |w| w,
            );
        }
        assert!(c.hp_remainder >= 0 && c.mana_remainder >= 0 && c.prepaid >= 0);
    }
}
