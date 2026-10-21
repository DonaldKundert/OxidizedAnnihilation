//! A bounds-checked little-endian cursor over a byte slice.
//!
//! Every parser in this crate reads through [`Reader`] so that malformed input
//! becomes an error instead of a panic, and so that allocation sizes can be
//! checked against the bytes actually present.

/// Why a read failed. Parsers wrap this in their own error type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("unexpected end of data reading {what} at offset {offset} (need {need}, have {have})")]
    Eof {
        what: &'static str,
        offset: usize,
        need: usize,
        have: usize,
    },
    #[error("offset {offset} for {what} is outside the data ({len} bytes)")]
    BadOffset {
        what: &'static str,
        offset: u64,
        len: usize,
    },
    #[error("{what}: count {count} × {elem_size} bytes exceeds the {remaining} bytes remaining")]
    CountTooLarge {
        what: &'static str,
        count: u64,
        elem_size: usize,
        remaining: usize,
    },
}

pub type ReadResult<T> = Result<T, ReadError>;

#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    /// A reader positioned at an absolute offset (e.g. a pointer read from a header).
    pub fn at(data: &'a [u8], offset: u64, what: &'static str) -> ReadResult<Self> {
        let pos = usize::try_from(offset)
            .ok()
            .filter(|&p| p <= data.len())
            .ok_or(ReadError::BadOffset {
                what,
                offset,
                len: data.len(),
            })?;
        Ok(Reader { data, pos })
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }
    pub fn pos(&self) -> usize {
        self.pos
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn seek(&mut self, offset: u64, what: &'static str) -> ReadResult<()> {
        *self = Reader::at(self.data, offset, what)?;
        Ok(())
    }

    pub fn bytes(&mut self, n: usize, what: &'static str) -> ReadResult<&'a [u8]> {
        let have = self.remaining();
        if n > have {
            return Err(ReadError::Eof {
                what,
                offset: self.pos,
                need: n,
                have,
            });
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize, what: &'static str) -> ReadResult<()> {
        self.bytes(n, what).map(|_| ())
    }

    pub fn array<const N: usize>(&mut self, what: &'static str) -> ReadResult<[u8; N]> {
        Ok(self.bytes(N, what)?.try_into().expect("length checked"))
    }

    pub fn u8(&mut self, what: &'static str) -> ReadResult<u8> {
        Ok(self.array::<1>(what)?[0])
    }
    pub fn u16(&mut self, what: &'static str) -> ReadResult<u16> {
        Ok(u16::from_le_bytes(self.array(what)?))
    }
    pub fn i16(&mut self, what: &'static str) -> ReadResult<i16> {
        Ok(i16::from_le_bytes(self.array(what)?))
    }
    pub fn u32(&mut self, what: &'static str) -> ReadResult<u32> {
        Ok(u32::from_le_bytes(self.array(what)?))
    }
    pub fn i32(&mut self, what: &'static str) -> ReadResult<i32> {
        Ok(i32::from_le_bytes(self.array(what)?))
    }
    /// IEEE f32, for file formats that store floats. Never put the result
    /// into simulation state; convert to fixed point at the boundary.
    pub fn f32(&mut self, what: &'static str) -> ReadResult<f32> {
        Ok(f32::from_le_bytes(self.array(what)?))
    }

    /// A fixed-size, NUL-padded Latin-1 string field (e.g. a 32-byte name).
    pub fn fixed_str(&mut self, n: usize, what: &'static str) -> ReadResult<String> {
        let b = self.bytes(n, what)?;
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        Ok(latin1(&b[..end]))
    }

    /// A NUL-terminated Latin-1 string at the current position.
    pub fn cstr(&mut self, what: &'static str) -> ReadResult<String> {
        let rest = &self.data[self.pos..];
        let end = rest.iter().position(|&c| c == 0).ok_or(ReadError::Eof {
            what,
            offset: self.pos,
            need: rest.len() + 1,
            have: rest.len(),
        })?;
        self.pos += end + 1;
        Ok(latin1(&rest[..end]))
    }

    /// Validates that `count` elements of `elem_size` bytes can still fit in
    /// the remaining data. Call before `Vec::with_capacity(count)`.
    pub fn check_count(
        &self,
        count: u64,
        elem_size: usize,
        what: &'static str,
    ) -> ReadResult<usize> {
        let remaining = self.remaining();
        let err = ReadError::CountTooLarge {
            what,
            count,
            elem_size,
            remaining,
        };
        let n = usize::try_from(count).map_err(|_| err.clone())?;
        match n.checked_mul(elem_size.max(1)) {
            Some(total) if total <= remaining => Ok(n),
            _ => Err(err),
        }
    }
}

/// Decodes Latin-1 bytes (each byte is one char).
pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_and_tracks_position() {
        let d = [1, 0, 2, 0, 0, 0, 0xff, 0xff, b'a', b'b', 0, b'c', 0];
        let mut r = Reader::new(&d);
        assert_eq!(r.u16("a").unwrap(), 1);
        assert_eq!(r.u32("b").unwrap(), 2);
        assert_eq!(r.i16("c").unwrap(), -1);
        assert_eq!(r.cstr("s").unwrap(), "ab");
        assert_eq!(r.cstr("s").unwrap(), "c");
        assert!(r.is_empty());
    }

    #[test]
    fn errors_instead_of_panicking() {
        let d = [1, 2, 3];
        let mut r = Reader::new(&d);
        assert!(matches!(
            r.u32("x"),
            Err(ReadError::Eof {
                need: 4,
                have: 3,
                ..
            })
        ));
        assert_eq!(r.pos(), 0, "failed reads do not advance");
        assert!(Reader::at(&d, 4, "p").is_err());
        assert!(Reader::at(&d, u64::MAX, "p").is_err());
        assert!(r.check_count(2, 2, "n").is_err());
        assert_eq!(r.check_count(3, 1, "n").unwrap(), 3);
        assert!(r.check_count(u64::MAX, 8, "n").is_err());
        assert!(Reader::new(b"ab").cstr("s").is_err());
    }

    #[test]
    fn fixed_strings_stop_at_nul() {
        let mut d = b"MoveButton".to_vec();
        d.resize(32, 0);
        assert_eq!(Reader::new(&d).fixed_str(32, "name").unwrap(), "MoveButton");
    }
}
