//! Seeded deterministic PRNG.
//!
//! The algorithm is **PCG32** (`PCG-XSH-RR 64/32`, Melissa O'Neill, 2014):
//! a 64-bit linear congruential state with multiplier
//! `6364136223846793005`, an odd per-stream increment, and a
//! xorshift-high/random-rotate output permutation producing 32 bits per step.
//! Seeding follows the reference `pcg32_srandom_r(seed, stream)`, so outputs
//! match the reference implementation bit for bit (tested against the
//! published `pcg32-demo` vector).
//!
//! There is no OS or time seeding anywhere: the world creates one `Rng` from
//! the match seed and owns it. The full state is two `u64`s, serializable and
//! hashable, so it can be saved, compared and included in the state hash.

use crate::Fx;

const MULTIPLIER: u64 = 6_364_136_223_846_793_005;
/// Stream used by [`Rng::new`]; the reference demo's default (`54`).
pub const DEFAULT_STREAM: u64 = 54;

/// PCG32 generator. See the module docs.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    /// Seed with the default stream ([`DEFAULT_STREAM`]).
    pub const fn new(seed: u64) -> Rng {
        Rng::with_stream(seed, DEFAULT_STREAM)
    }

    /// Seed with an explicit stream selector (`pcg32_srandom_r`). Different
    /// streams with the same seed give independent sequences.
    pub const fn with_stream(seed: u64, stream: u64) -> Rng {
        let mut rng = Rng {
            state: 0,
            inc: (stream << 1) | 1,
        };
        rng.step();
        rng.state = rng.state.wrapping_add(seed);
        rng.step();
        rng
    }

    #[inline]
    const fn step(&mut self) {
        self.state = self.state.wrapping_mul(MULTIPLIER).wrapping_add(self.inc);
    }

    /// Next 32 uniformly distributed bits.
    #[inline]
    pub const fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.step();
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform integer in `[0, bound)` with no modulo bias (Lemire's
    /// multiply-and-reject method). Returns `0` when `bound == 0`.
    #[inline]
    pub const fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            return 0;
        }
        let mut m = self.next_u32() as u64 * bound as u64;
        let mut low = m as u32;
        if low < bound {
            // 2^32 mod bound: values of `low` below this are the biased tail.
            let threshold = bound.wrapping_neg() % bound;
            while low < threshold {
                m = self.next_u32() as u64 * bound as u64;
                low = m as u32;
            }
        }
        (m >> 32) as u32
    }

    /// Uniform integer in the half-open range `[lo, hi)`, unbiased. Returns
    /// `lo` without consuming randomness when the range is empty
    /// (`hi <= lo`), so a bad range never panics the sim.
    #[inline]
    pub const fn range(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        lo + self.below(hi - lo)
    }

    /// Uniform integer in the half-open range `[lo, hi)` for signed bounds.
    /// Same empty-range rule as [`Rng::range`].
    #[inline]
    pub const fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        let span = (hi as i64 - lo as i64) as u32;
        lo.wrapping_add(self.below(span) as i32)
    }

    /// `true` with probability `num / den` (exactly, via [`Rng::below`]).
    /// `den == 0` returns `false`; `num >= den` returns `true`. Always consumes
    /// one draw when `den != 0`, so call patterns stay in lockstep.
    #[inline]
    pub const fn chance(&mut self, num: u32, den: u32) -> bool {
        if den == 0 {
            return false;
        }
        self.below(den) < num
    }

    /// Uniform fixed-point value in `[0, 1)`: the top 16 bits of one draw.
    #[inline]
    pub const fn next_fx_unit(&mut self) -> Fx {
        Fx::from_raw((self.next_u32() >> 16) as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// First six outputs of the reference `pcg32-demo` (seed 42, stream 54).
    #[test]
    fn reference_vector() {
        let mut r = Rng::with_stream(42, 54);
        let got: Vec<u32> = (0..6).map(|_| r.next_u32()).collect();
        assert_eq!(
            got,
            [
                0xa15c02b7, 0x7b47f409, 0xba1d3330, 0x83d2f293, 0xbfa4784b, 0xcbed606e
            ]
        );
    }

    #[test]
    fn ranges() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let v = r.range(10, 13);
            assert!((10..13).contains(&v));
            let s = r.range_i32(-5, 5);
            assert!((-5..5).contains(&s));
            assert!(r.next_fx_unit() < Fx::ONE);
        }
        assert_eq!(r.range(5, 5), 5);
        assert!(r.range_i32(i32::MIN, i32::MAX) < i32::MAX);
        assert!(!r.chance(0, 10));
        assert!(r.chance(10, 10));
    }
}
