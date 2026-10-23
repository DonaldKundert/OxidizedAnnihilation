//! Positioned byte sources, so archives can read file data lazily.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

/// Something an archive can read from at arbitrary offsets.
pub trait Source {
    fn len(&self) -> io::Result<u64>;
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
    fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }
}

impl Source for [u8] {
    fn len(&self) -> io::Result<u64> {
        Ok(<[u8]>::len(self) as u64)
    }
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| io::ErrorKind::UnexpectedEof)?;
        let src = start
            .checked_add(buf.len())
            .and_then(|end| self.get(start..end))
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(src);
        Ok(())
    }
}

impl Source for Vec<u8> {
    fn len(&self) -> io::Result<u64> {
        Ok(self.as_slice().len() as u64)
    }
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.as_slice().read_exact_at(offset, buf)
    }
}

impl<T: Source + ?Sized> Source for Arc<T> {
    fn len(&self) -> io::Result<u64> {
        (**self).len()
    }
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        (**self).read_exact_at(offset, buf)
    }
}

/// A file on disk, read with seek + read under a lock (portable across OSes).
pub struct FileSource {
    file: Mutex<std::fs::File>,
    len: u64,
}

impl FileSource {
    pub fn open(path: &std::path::Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(FileSource {
            file: Mutex::new(file),
            len,
        })
    }
}

impl Source for FileSource {
    fn len(&self) -> io::Result<u64> {
        Ok(self.len)
    }
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let mut f = self
            .file
            .lock()
            .map_err(|_| io::Error::other("file lock poisoned"))?;
        f.seek(SeekFrom::Start(offset))?;
        f.read_exact(buf)
    }
}

/// Reads at most `limit` bytes, then uses a one-byte stack probe to detect more.
/// Incremental reservations never deliberately grow the vector beyond the limit.
pub(crate) fn read_to_end_bounded<R: Read>(
    reader: &mut R,
    limit: u64,
) -> io::Result<(Vec<u8>, bool)> {
    let limit = usize::try_from(limit)
        .ok()
        .filter(|&n| n <= isize::MAX as usize)
        .ok_or_else(|| io::Error::other("read limit exceeds addressable memory"))?;
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let remaining = limit - out.len();
        if remaining == 0 {
            let mut probe = [0u8; 1];
            return Ok((out, read_retry(reader, &mut probe)? != 0));
        }
        let n = read_retry(reader, &mut buf[..remaining.min(8192)])?;
        if n == 0 {
            return Ok((out, false));
        }
        let want = out.len() + n;
        if want > out.capacity() {
            let capacity = out.capacity().saturating_mul(2).max(want).min(limit);
            out.try_reserve_exact(capacity - out.len())
                .map_err(io::Error::other)?;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn read_retry<R: Read>(reader: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match reader.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reader_checks_exact_and_zero_limits() {
        for (bytes, limit, exceeded) in [
            (&b""[..], 0, false),
            (&b"x"[..], 0, true),
            (&b"abc"[..], 3, false),
            (&b"abcd"[..], 3, true),
        ] {
            let (out, actual_exceeded) = read_to_end_bounded(&mut &bytes[..], limit).unwrap();
            assert_eq!(actual_exceeded, exceeded);
            assert_eq!(out, &bytes[..bytes.len().min(limit as usize)]);
            assert!(out.capacity() <= limit as usize);
        }
    }

    #[test]
    fn bounded_reader_does_not_double_past_limit() {
        let bytes = vec![0u8; 20_001];
        let (out, exceeded) = read_to_end_bounded(&mut &bytes[..], 20_000).unwrap();
        assert!(exceeded);
        assert_eq!(out.len(), 20_000);
        assert!(out.capacity() <= 20_000);
    }
}
