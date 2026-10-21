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
