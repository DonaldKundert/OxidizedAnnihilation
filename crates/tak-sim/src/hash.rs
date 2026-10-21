//! State hashing: xxHash64 (seed 0) over a canonical little-endian byte
//! stream.
//!
//! Every value is written explicitly with a fixed width and byte order, so the
//! hash never depends on `std::hash::Hash`, pointer width, struct layout or
//! container capacity. Variable-length data (byte strings, lists) is always
//! preceded by its length as a `u32`, so two different sequences can never
//! produce the same stream.

use tak_commands::{PlayerId, Tick, UnitDefId, UnitId};
use tak_math::{Angle, Fx, Vec2};
use xxhash_rust::xxh64::Xxh64;

/// Streaming hasher for sim state. See the module docs.
pub struct StateHasher {
    inner: Xxh64,
}

impl Default for StateHasher {
    fn default() -> Self {
        StateHasher::new()
    }
}

impl StateHasher {
    /// A fresh hasher (xxHash64, seed 0).
    pub fn new() -> StateHasher {
        StateHasher {
            inner: Xxh64::new(0),
        }
    }

    /// One byte.
    pub fn u8(&mut self, v: u8) {
        self.inner.update(&[v]);
    }

    /// `false` = 0, `true` = 1.
    pub fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }

    /// Little-endian `u16`.
    pub fn u16(&mut self, v: u16) {
        self.inner.update(&v.to_le_bytes());
    }

    /// Little-endian `u32`.
    pub fn u32(&mut self, v: u32) {
        self.inner.update(&v.to_le_bytes());
    }

    /// Little-endian `u64`.
    pub fn u64(&mut self, v: u64) {
        self.inner.update(&v.to_le_bytes());
    }

    /// Little-endian `i32`.
    pub fn i32(&mut self, v: i32) {
        self.inner.update(&v.to_le_bytes());
    }

    /// A length or count, as a `u32` (saturating; sim containers are capped
    /// far below `u32::MAX`).
    pub fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }

    /// Length-prefixed bytes.
    pub fn bytes(&mut self, b: &[u8]) {
        self.len(b.len());
        self.inner.update(b);
    }

    /// Fixed point, as its raw `i32`.
    pub fn fx(&mut self, v: Fx) {
        self.i32(v.raw());
    }

    /// `x` then `y`.
    pub fn vec2(&mut self, v: Vec2) {
        self.fx(v.x);
        self.fx(v.y);
    }

    /// Raw `u16`.
    pub fn angle(&mut self, a: Angle) {
        self.u16(a.raw());
    }

    /// Raw `u32`.
    pub fn tick(&mut self, t: Tick) {
        self.u32(t.0);
    }

    /// Raw `u8`.
    pub fn player(&mut self, p: PlayerId) {
        self.u8(p.0);
    }

    /// `index` then `generation`.
    pub fn unit_id(&mut self, id: UnitId) {
        self.u32(id.index);
        self.u32(id.generation);
    }

    /// Raw `u32`.
    pub fn unit_def(&mut self, d: UnitDefId) {
        self.u32(d.0);
    }

    /// Tag byte 0 for `None`, 1 then the value for `Some`.
    pub fn option<T>(&mut self, v: Option<T>, f: impl FnOnce(&mut StateHasher, T)) {
        match v {
            None => self.u8(0),
            Some(x) => {
                self.u8(1);
                f(self, x);
            }
        }
    }

    /// The digest of everything written so far.
    pub fn finish(&self) -> u64 {
        self.inner.digest()
    }
}

/// Adapter for types whose fields are private but which derive `Hash` over
/// fixed-width integers only (the world RNG). Every integer write is routed
/// to the explicit little-endian methods above; `usize`/`isize` go out as 64
/// bits. Only use this for types made of fixed-width integers: a derived
/// `Hash` over slices or strings would add length prefixes we do not control.
pub(crate) struct LeHasher<'a>(pub(crate) &'a mut StateHasher);

impl std::hash::Hasher for LeHasher<'_> {
    fn finish(&self) -> u64 {
        0
    }
    fn write(&mut self, bytes: &[u8]) {
        self.0.inner.update(bytes);
    }
    fn write_u8(&mut self, i: u8) {
        self.0.u8(i);
    }
    fn write_u16(&mut self, i: u16) {
        self.0.u16(i);
    }
    fn write_u32(&mut self, i: u32) {
        self.0.u32(i);
    }
    fn write_u64(&mut self, i: u64) {
        self.0.u64(i);
    }
    fn write_u128(&mut self, i: u128) {
        self.0.inner.update(&i.to_le_bytes());
    }
    fn write_usize(&mut self, i: usize) {
        self.0.u64(i as u64);
    }
    fn write_i8(&mut self, i: i8) {
        self.0.u8(i as u8);
    }
    fn write_i16(&mut self, i: i16) {
        self.0.u16(i as u16);
    }
    fn write_i32(&mut self, i: i32) {
        self.0.i32(i);
    }
    fn write_i64(&mut self, i: i64) {
        self.0.u64(i as u64);
    }
    fn write_i128(&mut self, i: i128) {
        self.0.inner.update(&i.to_le_bytes());
    }
    fn write_isize(&mut self, i: isize) {
        self.0.u64(i as i64 as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // xxh64("", seed 0) reference value.
        assert_eq!(StateHasher::new().finish(), 0xEF46_DB37_51D8_E999);
    }

    #[test]
    fn length_prefix_separates() {
        let mut a = StateHasher::new();
        a.bytes(b"ab");
        a.bytes(b"c");
        let mut b = StateHasher::new();
        b.bytes(b"a");
        b.bytes(b"bc");
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn rng_state_feeds_hash() {
        use std::hash::Hash;
        let mut r1 = tak_math::Rng::new(7);
        let r0 = r1.clone();
        r1.next_u32();
        let digest = |r: &tak_math::Rng| {
            let mut h = StateHasher::new();
            r.hash(&mut LeHasher(&mut h));
            h.finish()
        };
        assert_eq!(digest(&r0), digest(&r0.clone()));
        assert_ne!(digest(&r0), digest(&r1));
    }
}
