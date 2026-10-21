//! Runs a sim from a scenario and a command log and prints per-tick hashes.
//! Renders nothing.
//!
//! # Hash file format
//!
//! Plain text, one line per tick, tick numbers contiguous from 0:
//!
//! ```text
//! # tak-headless hashes v1
//! 0 9b5e1c7a33d0f2e4
//! 1 04a1f8c2d6e7b390
//! ```
//!
//! Each line is the tick number in decimal, one space, and the xxHash64 state
//! hash after that tick as 16 lowercase hex digits. Lines starting with `#`
//! and blank lines are ignored.

use std::path::Path;

use tak_commands::{CommandLog, LogError};
use tak_sim::{Scenario, ScenarioError, run_scenario};

/// Header written at the top of hash files.
pub const HASH_FILE_HEADER: &str = "# tak-headless hashes v1";

/// Most ticks a hash file may describe (about 9 days of game time).
pub const MAX_TICKS: u32 = 1 << 24;

/// Errors from the runner.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// File I/O.
    #[error("{path}: {source}")]
    Io {
        /// Path involved.
        path: String,
        /// Underlying error.
        source: std::io::Error,
    },
    /// Bad scenario.
    #[error("scenario: {0}")]
    Scenario(#[from] ScenarioError),
    /// Bad command log.
    #[error("command log: {0}")]
    Log(#[from] LogError),
    /// Bad hash file.
    #[error("hash file line {line}: {msg}")]
    HashFile {
        /// 1-based line number.
        line: usize,
        /// What is wrong.
        msg: String,
    },
}

fn read(path: &Path) -> Result<String, Error> {
    std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Write a file, mapping errors.
pub fn write(path: &Path, contents: &str) -> Result<(), Error> {
    std::fs::write(path, contents).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Load and validate a scenario file.
pub fn load_scenario(path: &Path) -> Result<Scenario, Error> {
    Ok(Scenario::from_json(&read(path)?)?)
}

/// Load a command log, or an empty log for `None`.
pub fn load_log(path: Option<&Path>) -> Result<CommandLog, Error> {
    match path {
        None => Ok(CommandLog::new()),
        Some(p) => Ok(CommandLog::from_json(&read(p)?)?),
    }
}

/// Run a scenario with a log for `ticks` ticks.
pub fn run(scenario: &Scenario, log: &CommandLog, ticks: u32) -> Result<Vec<u64>, Error> {
    Ok(run_scenario(scenario, log, ticks)?)
}

/// Render hashes in the hash file format.
pub fn format_hashes(hashes: &[u64]) -> String {
    let mut s = String::with_capacity(hashes.len().saturating_mul(24).saturating_add(32));
    s.push_str(HASH_FILE_HEADER);
    s.push('\n');
    for (t, h) in hashes.iter().enumerate() {
        s.push_str(&format!("{t} {h:016x}\n"));
    }
    s
}

/// Parse the hash file format.
pub fn parse_hashes(text: &str) -> Result<Vec<u64>, Error> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = |msg: String| Error::HashFile { line: i + 1, msg };
        let (tick, hash) = line
            .split_once(' ')
            .ok_or_else(|| bad("expected `<tick> <hash>`".into()))?;
        let tick: u32 = tick
            .parse()
            .map_err(|_| bad(format!("bad tick {tick:?}")))?;
        if tick as usize != out.len() {
            return Err(bad(format!("expected tick {}, found {tick}", out.len())));
        }
        if out.len() >= MAX_TICKS as usize {
            return Err(bad("too many ticks".into()));
        }
        let hash = hash.trim();
        if hash.len() != 16 {
            return Err(bad(format!("hash {hash:?} is not 16 hex digits")));
        }
        let hash = u64::from_str_radix(hash, 16).map_err(|_| bad(format!("bad hash {hash:?}")))?;
        out.push(hash);
    }
    Ok(out)
}

/// The first tick whose hash differs, as `(tick, expected, actual)`.
pub fn first_mismatch(expected: &[u64], actual: &[u64]) -> Option<(usize, u64, u64)> {
    expected
        .iter()
        .zip(actual)
        .enumerate()
        .find(|(_, (e, a))| e != a)
        .map(|(t, (e, a))| (t, *e, *a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_file_round_trip() {
        let h = vec![0, u64::MAX, 0x0123_4567_89ab_cdef];
        let text = format_hashes(&h);
        assert!(text.starts_with(HASH_FILE_HEADER));
        assert_eq!(parse_hashes(&text).unwrap(), h);
    }

    #[test]
    fn hash_file_rejects_garbage() {
        assert!(parse_hashes("1 0000000000000000\n").is_err());
        assert!(parse_hashes("0 00\n").is_err());
        assert!(parse_hashes("0 zzzzzzzzzzzzzzzz\n").is_err());
        assert!(parse_hashes("nonsense\n").is_err());
        assert_eq!(parse_hashes("# c\n\n").unwrap(), Vec::<u64>::new());
    }

    #[test]
    fn mismatch_reports_first_tick() {
        assert_eq!(first_mismatch(&[1, 2, 3], &[1, 2, 3]), None);
        assert_eq!(first_mismatch(&[1, 2, 3], &[1, 5, 6]), Some((1, 2, 5)));
    }
}
