//! Mounts archives and loose folders, applies precedence, case-insensitive
//! paths, lazy decompression. Does not know what any file means.
//!
//! ```no_run
//! let vfs = tak_vfs::open_kingdoms(std::path::Path::new("/path/to/kingdoms"))?;
//! let bytes = vfs.read("units/ARAMON_Monarch.fbi")?; // case-insensitive
//! # Ok::<(), tak_vfs::VfsError>(())
//! ```

pub mod hapi;
pub mod source;
pub mod testing;
mod vfs;

pub use vfs::{
    FileInfo, Origin, Result, Vfs, VfsError, kingdoms_mount_order, normalize, open_kingdoms,
};
