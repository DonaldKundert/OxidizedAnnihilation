//! FNV-1a 64 as a [`Hasher`] whose output does not depend on the platform:
//! integers are fed little-endian and `usize`/`isize` (which derived `Hash`
//! uses for lengths and enum discriminants) are widened to 64 bits.

use std::hash::Hasher;

#[derive(Debug, Clone, Copy)]
pub struct Fnv64(u64);

impl Fnv64 {
    pub const fn new() -> Fnv64 {
        Fnv64(0xcbf2_9ce4_8422_2325)
    }
}

impl Default for Fnv64 {
    fn default() -> Self {
        Fnv64::new()
    }
}

impl Hasher for Fnv64 {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn write_u16(&mut self, i: u16) {
        self.write(&i.to_le_bytes());
    }
    fn write_u32(&mut self, i: u32) {
        self.write(&i.to_le_bytes());
    }
    fn write_u64(&mut self, i: u64) {
        self.write(&i.to_le_bytes());
    }
    fn write_i16(&mut self, i: i16) {
        self.write(&i.to_le_bytes());
    }
    fn write_i32(&mut self, i: i32) {
        self.write(&i.to_le_bytes());
    }
    fn write_i64(&mut self, i: i64) {
        self.write(&i.to_le_bytes());
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
    fn write_isize(&mut self, i: isize) {
        self.write_i64(i as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        let mut h = Fnv64::new();
        h.write(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
    }
}
