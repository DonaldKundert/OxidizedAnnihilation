//! The tick-stamped command log: the complete input of a match, and so the
//! basis of replays, lockstep networking and determinism tests.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::{PlayerId, SimCommand, Tick};

/// Version written into serialized logs. Bump on incompatible changes.
pub const LOG_FORMAT_VERSION: u32 = 1;

/// One command as issued: which tick it executes on, who issued it, and a
/// per-player sequence number that orders a player's commands within a tick.
///
/// `Ord` compares `(tick, player, seq)` first; since a log never holds two
/// entries with the same key, that is the execution order.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct LoggedCommand {
    /// Tick on which the sim applies this command (before stepping it).
    pub tick: Tick,
    /// Issuing player.
    pub player: PlayerId,
    /// Order among this player's commands on this tick.
    pub seq: u32,
    /// The command.
    pub command: SimCommand,
}

impl LoggedCommand {
    /// The ordering key `(tick, player, seq)`.
    #[inline]
    pub fn key(&self) -> (Tick, PlayerId, u32) {
        (self.tick, self.player, self.seq)
    }
}

/// Errors from building or loading a [`CommandLog`].
#[derive(Debug)]
pub enum LogError {
    /// Two entries share `(tick, player, seq)`.
    Duplicate {
        /// Tick of the clash.
        tick: Tick,
        /// Player of the clash.
        player: PlayerId,
        /// Sequence number of the clash.
        seq: u32,
    },
    /// The serialized log has a version this build does not read.
    UnsupportedVersion(u32),
    /// The JSON could not be parsed (includes the errors above when they are
    /// found during deserialization).
    Json(serde_json::Error),
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogError::Duplicate { tick, player, seq } => write!(
                f,
                "duplicate command at tick {}, player {}, seq {}",
                tick.0, player.0, seq
            ),
            LogError::UnsupportedVersion(v) => write!(
                f,
                "unsupported command log version {v} (expected {LOG_FORMAT_VERSION})"
            ),
            LogError::Json(e) => write!(f, "command log JSON: {e}"),
        }
    }
}

impl std::error::Error for LogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LogError::Json(e) => Some(e),
            _ => None,
        }
    }
}

/// Serialized shape: `{"version": 1, "entries": [...]}`.
#[derive(Serialize, Deserialize)]
struct LogFile {
    version: u32,
    entries: Vec<LoggedCommand>,
}

/// All commands of a match, kept sorted by `(tick, player, seq)` with unique
/// keys. That order is canonical: every peer that holds the same commands
/// holds the same log, byte for byte when serialized.
///
/// Serializes as `{"version": 1, "entries": [...]}`. Loading accepts entries
/// in any order (they are sorted) but rejects duplicate keys and unknown
/// versions.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(try_from = "LogFile", into = "LogFile")]
pub struct CommandLog {
    entries: Vec<LoggedCommand>,
}

impl CommandLog {
    /// An empty log.
    pub const fn new() -> CommandLog {
        CommandLog {
            entries: Vec::new(),
        }
    }

    /// Insert an entry at its sorted position. Fails if the key is taken.
    pub fn insert(&mut self, entry: LoggedCommand) -> Result<(), LogError> {
        let key = entry.key();
        match self.entries.binary_search_by_key(&key, LoggedCommand::key) {
            Ok(_) => Err(LogError::Duplicate {
                tick: key.0,
                player: key.1,
                seq: key.2,
            }),
            Err(at) => {
                self.entries.insert(at, entry);
                Ok(())
            }
        }
    }

    /// Append a command for `player` on `tick`, numbering it after that
    /// player's existing commands on that tick. Returns the assigned `seq`.
    pub fn record(&mut self, tick: Tick, player: PlayerId, command: SimCommand) -> u32 {
        // Index just past the last entry with key <= (tick, player, MAX).
        let end = self
            .entries
            .partition_point(|e| (e.tick, e.player) <= (tick, player));
        let seq = match end.checked_sub(1).map(|i| &self.entries[i]) {
            Some(prev) if prev.tick == tick && prev.player == player => prev.seq.saturating_add(1),
            _ => 0,
        };
        self.entries.insert(
            end,
            LoggedCommand {
                tick,
                player,
                seq,
                command,
            },
        );
        seq
    }

    /// The commands to apply on `tick`, in execution order.
    pub fn for_tick(&self, tick: Tick) -> &[LoggedCommand] {
        let start = self.entries.partition_point(|e| e.tick < tick);
        let end = self.entries.partition_point(|e| e.tick <= tick);
        &self.entries[start..end]
    }

    /// All entries in execution order.
    pub fn entries(&self) -> &[LoggedCommand] {
        &self.entries
    }

    /// Iterate in execution order.
    pub fn iter(&self) -> core::slice::Iter<'_, LoggedCommand> {
        self.entries.iter()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if the log is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Tick of the last command, if any.
    pub fn last_tick(&self) -> Option<Tick> {
        self.entries.last().map(|e| e.tick)
    }

    /// Compact JSON.
    pub fn to_json(&self) -> String {
        // Serializing plain data with string keys cannot fail.
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Indented JSON, for humans and diffs.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Parse JSON produced by [`CommandLog::to_json`] (or written by hand).
    pub fn from_json(s: &str) -> Result<CommandLog, LogError> {
        let file: LogFile = serde_json::from_str(s).map_err(LogError::Json)?;
        CommandLog::try_from(file)
    }
}

impl TryFrom<LogFile> for CommandLog {
    type Error = LogError;
    fn try_from(file: LogFile) -> Result<Self, Self::Error> {
        if file.version != LOG_FORMAT_VERSION {
            return Err(LogError::UnsupportedVersion(file.version));
        }
        let mut entries = file.entries;
        entries.sort_by_key(LoggedCommand::key);
        if let Some(w) = entries.windows(2).find(|w| w[0].key() == w[1].key()) {
            let (tick, player, seq) = w[0].key();
            return Err(LogError::Duplicate { tick, player, seq });
        }
        Ok(CommandLog { entries })
    }
}

impl From<CommandLog> for LogFile {
    fn from(log: CommandLog) -> LogFile {
        LogFile {
            version: LOG_FORMAT_VERSION,
            entries: log.entries,
        }
    }
}

impl<'a> IntoIterator for &'a CommandLog {
    type Item = &'a LoggedCommand;
    type IntoIter = core::slice::Iter<'a, LoggedCommand>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}
