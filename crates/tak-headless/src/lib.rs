//! Runs a sim from a scenario (synthetic JSON, or a real map loaded from an
//! install with COB scripts running) and a command log, and prints per-tick
//! hashes. Renders nothing.
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

pub mod campaign;
pub mod checkpoint;
pub mod economy_report;

use tak_commands::{CommandLog, LogError};
use tak_sim::content::ContentError;
use tak_sim::{
    Match, PlayerSetup, Scenario, ScenarioError, ScriptStats, Sim, load_match, run_scenario,
};

/// Header written at the top of hash files.
pub const HASH_FILE_HEADER: &str = "# tak-headless hashes v1";

/// Most ticks a hash file may describe (about 9 days of game time).
pub const MAX_TICKS: u32 = 1 << 24;

/// Errors from the runner.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Replay checkpoint validation or restore failure.
    #[error("{0}")]
    Checkpoint(#[from] checkpoint::CheckpointError),
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
    /// Real map or install could not be loaded.
    #[error("{0}")]
    Content(#[from] ContentError),
    /// Inconsistent arguments.
    #[error("{0}")]
    Setup(String),
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

/// [`run`], also returning the match outcome line ([`outcome`]).
pub fn run_with_outcome(
    scenario: &Scenario,
    log: &CommandLog,
    ticks: u32,
) -> Result<(Vec<u64>, String), Error> {
    let mut sim = Sim::new(scenario)?;
    let hashes = sim.run(log, ticks);
    Ok((hashes, outcome(&sim)))
}

/// `match: running` or `match: over at tick T: winners [..]` (WP-U).
pub fn outcome(sim: &Sim) -> String {
    format!("match: {}", sim.world().victory().summary())
}

/// Load a real-map match from an install: `map` is a VFS path without
/// extension (`kmap/Adamantine Gate`), `players` a comma-separated side
/// list (`ARA,ZON`).
pub fn load_map(data: &Path, map: &str, players: &str, seed: u64) -> Result<Match, Error> {
    let players = PlayerSetup::parse_list(players);
    if players.is_empty() {
        return Err(Error::Setup("--players names no side".into()));
    }
    let vfs = tak_vfs::open_kingdoms(data).map_err(ContentError::from)?;
    Ok(load_match(&vfs, map, &players, seed)?)
}

/// What a real-map run produced besides its hashes.
pub struct MatchRun {
    /// Per-tick hashes.
    pub hashes: Vec<u64>,
    /// Script diagnostics (not part of the hashes).
    pub stats: ScriptStats,
    /// Wall-clock milliseconds spent stepping (excludes loading).
    pub millis: u128,
    /// Final position of each player's monarch, whole world units.
    pub monarchs: Vec<Option<(i32, i32)>>,
    /// The match outcome line ([`outcome`]).
    pub outcome: String,
}

impl MatchRun {
    /// Ticks per wall-clock second (integer).
    pub fn ticks_per_second(&self) -> u128 {
        (self.hashes.len() as u128).saturating_mul(1000) / self.millis.max(1)
    }

    /// Human-readable summary for stderr.
    pub fn report(&self) -> String {
        let mut s = format!(
            "ran {} ticks in {} ms ({} ticks/s)\n",
            self.hashes.len(),
            self.millis,
            self.ticks_per_second()
        );
        for (i, p) in self.monarchs.iter().enumerate() {
            match p {
                Some((x, y)) => s.push_str(&format!("player {i} monarch at ({x}, {y})\n")),
                None => s.push_str(&format!("player {i} has no monarch\n")),
            }
        }
        s.push_str(&self.outcome);
        s.push('\n');
        s.push_str(&self.stats.report());
        s
    }
}

/// Run a loaded match with its COB scripts (or none) for `ticks` ticks.
pub fn run_match(m: Match, log: &CommandLog, ticks: u32, scripts: bool) -> Result<MatchRun, Error> {
    let mut sim = if scripts {
        Sim::with_scripts(&m.scenario, Box::new(m.scripts))?
    } else {
        Sim::new(&m.scenario)?
    };
    let start = std::time::Instant::now();
    let hashes = sim.run(log, ticks);
    let millis = start.elapsed().as_millis();
    let monarchs = sim
        .world()
        .players()
        .iter()
        .map(|p| {
            let u = sim.world().unit(p.monarch?)?;
            Some((u.pos.x.to_int_floor(), u.pos.y.to_int_floor()))
        })
        .collect();
    Ok(MatchRun {
        hashes,
        stats: sim.script_stats(),
        millis,
        monarchs,
        outcome: outcome(&sim),
    })
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
