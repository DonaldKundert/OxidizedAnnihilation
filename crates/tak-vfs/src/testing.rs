//! Gating for tests that need the retail game data.
//!
//! Game data is never committed. Tests that need it call [`data_dir`] and
//! return early when it yields `None`, so CI stays green without the game.

use std::path::PathBuf;

/// Name of the environment variable that points at a Kingdoms install.
pub const DATA_DIR_VAR: &str = "TAK_DATA_DIR";

/// The Kingdoms install directory from `TAK_DATA_DIR`, if it is set, non-empty
/// and points at an existing directory.
pub fn data_dir() -> Option<PathBuf> {
    from_value(std::env::var_os(DATA_DIR_VAR))
}

fn from_value(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let path = PathBuf::from(value.filter(|v| !v.is_empty())?);
    path.is_dir().then_some(path)
}

/// Returns the data dir, or prints a skip notice and returns `None`.
///
/// ```ignore
/// let Some(dir) = tak_vfs::testing::data_dir_or_skip("archives_mount") else { return };
/// ```
pub fn data_dir_or_skip(test: &str) -> Option<PathBuf> {
    let dir = data_dir();
    if dir.is_none() {
        eprintln!("skipping {test}: {DATA_DIR_VAR} not set");
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_empty_is_none() {
        assert_eq!(from_value(None), None);
        assert_eq!(from_value(Some("".into())), None);
    }

    #[test]
    fn missing_dir_is_none() {
        assert_eq!(
            from_value(Some("/definitely/not/a/tak/install".into())),
            None
        );
    }

    #[test]
    fn existing_dir_is_some() {
        let tmp = std::env::temp_dir();
        assert_eq!(from_value(Some(tmp.clone().into())), Some(tmp));
    }
}
