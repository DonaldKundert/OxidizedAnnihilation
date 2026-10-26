//! Portable, versioned replay checkpoints, not Rust-memory snapshots.
//!
//! A checkpoint retains the initial setup and commands that actually executed.
//! Loading rebuilds a fresh simulation, verifies its external content identity,
//! replays the history, and returns it only after its state hash agrees. This
//! restores COB threads, RNG, arena generations and system state without making
//! their internal layouts a file format. Cost is linear in elapsed ticks.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bot::ControllerConfig;
use serde::{Deserialize, Serialize};
use tak_commands::{CommandLog, Tick};
use tak_content::{Ruleset, RulesetKind};
use tak_sim::{PlayerSetup, Scenario, Sim};
use tak_vfs::Vfs;
use xxhash_rust::xxh64::Xxh64;

pub mod driver;

/// Checkpoint wire format, independent of the command/scenario versions.
pub const CHECKPOINT_FORMAT_VERSION: u32 = 1;
/// Replay-rule compatibility gate. Bump when simulation changes invalidate saves.
// Rules 5 adds complete large-footprint separation candidate discovery.
// Reject older behavioral snapshots rather than silently changing their future.
pub const REPLAY_RULES_VERSION: u32 = 5;
/// Maximum serialized checkpoint size, checked before JSON deserialization.
pub const MAX_CHECKPOINT_BYTES: usize = 32 * 1024 * 1024;
/// Hard limit on executed commands stored in one checkpoint.
pub const MAX_CHECKPOINT_COMMANDS: usize = 1_000_000;
const FORMAT: &str = "tak-replay-checkpoint";
const MAX_CONTENT_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CONTENT_BYTES: u64 = 512 * 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Errors leave an existing session and existing checkpoint file untouched.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    /// File access, including atomic save preparation or rename failure.
    #[error("checkpoint {path}: {source}")]
    Io {
        /// File involved.
        path: PathBuf,
        /// Underlying failure.
        source: std::io::Error,
    },
    /// Invalid JSON or wire shape.
    #[error("checkpoint JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// This is not a checkpoint of the expected kind.
    #[error("unrecognized checkpoint format {0:?}")]
    Format(String),
    /// Unknown file version.
    #[error("unsupported checkpoint version {0} (expected {CHECKPOINT_FORMAT_VERSION})")]
    UnsupportedVersion(u32),
    /// Incompatible replay rules, even when the JSON version is readable.
    #[error("unsupported replay rules version {0} (expected {REPLAY_RULES_VERSION})")]
    UnsupportedRules(u32),
    /// Malformed data or a violated invariant.
    #[error("invalid checkpoint: {0}")]
    Invalid(String),
    /// An input or replay exceeds a configured/hard resource limit.
    #[error("checkpoint limit exceeded: {0}")]
    Limit(String),
    /// Accidental corruption detected by the canonical payload checksum.
    #[error("checkpoint checksum mismatch")]
    ChecksumMismatch,
    /// Installed gameplay content differs from the saved setup.
    #[error("checkpoint content mismatch: expected {expected}, found {actual}")]
    ContentMismatch {
        /// Saved identity.
        expected: String,
        /// Current identity.
        actual: String,
    },
    /// Deterministic replay did not reconstruct the saved state.
    #[error("checkpoint state mismatch at next tick {tick}: expected {expected}, found {actual}")]
    StateMismatch {
        /// Next tick, after replaying all executed ticks.
        tick: u32,
        /// Saved state digest.
        expected: String,
        /// Reconstructed state digest.
        actual: String,
    },
    /// Invalid command-log JSON or duplicate command keys.
    #[error("checkpoint command log: {0}")]
    Log(#[from] tak_commands::LogError),
    /// Invalid synthetic scenario.
    #[error("checkpoint scenario: {0}")]
    Scenario(#[from] tak_sim::ScenarioError),
    /// Install or map failure.
    #[error("checkpoint content: {0}")]
    Content(#[from] tak_sim::content::ContentError),
    /// External content read failure.
    #[error("checkpoint content: {0}")]
    Vfs(#[from] tak_vfs::VfsError),
}

/// A caller-controlled replay budget, checked before loading external content.
#[derive(Clone, Copy, Debug)]
pub struct ReplayBudget {
    /// Maximum elapsed ticks to replay (not additional ticks after resuming).
    pub max_ticks: u32,
    /// Maximum commands in the saved history.
    pub max_commands: usize,
}

impl Default for ReplayBudget {
    fn default() -> Self {
        Self {
            max_ticks: 30 * 60 * 60,
            max_commands: 250_000,
        }
    }
}

/// One explicit OTA-owner/side mapping, in simulation player order.
/// This descriptor contains no authored placements or other retail map data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointCampaignPlayer {
    /// One-based OTA owner slot (sparse slots are allowed without objectives).
    pub ota_slot: u8,
    /// Explicit side; never inferred from mission headers.
    pub side: String,
}

impl From<&tak_sim::campaign::CampaignPlayer> for CheckpointCampaignPlayer {
    fn from(player: &tak_sim::campaign::CampaignPlayer) -> Self {
        Self {
            ota_slot: player.ota_slot,
            side: player.side.clone(),
        }
    }
}

fn campaign_players(
    players: &[CheckpointCampaignPlayer],
) -> Vec<tak_sim::campaign::CampaignPlayer> {
    players
        .iter()
        .map(|player| tak_sim::campaign::CampaignPlayer {
            ota_slot: player.ota_slot,
            side: player.side.clone(),
        })
        .collect()
}

/// Reconstructible initial setup. Real maps reference the user's external
/// install; no archive, script, model or extracted map bytes are embedded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CheckpointSource {
    /// An explicitly supplied synthetic scenario, with no COB scripts.
    Synthetic {
        /// Initial conditions, not mutable world state.
        scenario: Box<Scenario>,
    },
    /// A base-ruleset skirmish loaded from an external owned install.
    Map {
        /// VFS map stem, e.g. `kmap/Adamantine Gate`.
        map: String,
        /// Sides in player-slot order.
        players: Vec<String>,
        /// Initial world RNG seed.
        world_seed: u64,
        /// Whether the loaded COB host is used.
        scripts: bool,
    },
    /// Authored campaign placements reloaded from an external owned install.
    /// This is an additive v1 source kind, not full campaign support.
    Campaign {
        /// VFS mission map stem, never an install-local filesystem path.
        map: String,
        /// Ordered, explicit OTA-owner/side mappings.
        players: Vec<CheckpointCampaignPlayer>,
        /// Initial world RNG seed.
        world_seed: u64,
        /// Whether the loaded COB host is used.
        scripts: bool,
        /// Opt in to strict, supported two-player objective evaluation.
        objectives: bool,
    },
}

impl CheckpointSource {
    /// Keep a synthetic scenario in the checkpoint.
    pub fn synthetic(scenario: Scenario) -> Self {
        Self::Synthetic {
            scenario: Box::new(scenario),
        }
    }

    /// Read a bounded synthetic scenario file.
    pub fn from_scenario_file(path: &Path) -> Result<Self, CheckpointError> {
        Ok(Self::synthetic(Scenario::from_json(&read_bounded(path)?)?))
    }

    /// Reference a real map. The install path is supplied only when opening it.
    pub fn map(map: &str, players: &str, world_seed: u64, scripts: bool) -> Self {
        Self::Map {
            map: tak_vfs::normalize(map),
            players: PlayerSetup::parse_list(players)
                .into_iter()
                .map(|p| p.side)
                .collect(),
            world_seed,
            scripts,
        }
    }

    /// Reference authored campaign placements without embedding their contents.
    /// Mappings remain ordered; validation occurs before external content reads.
    pub fn campaign(
        map: &str,
        players: &[tak_sim::campaign::CampaignPlayer],
        world_seed: u64,
        scripts: bool,
        objectives: bool,
    ) -> Self {
        Self::Campaign {
            map: tak_vfs::normalize(map),
            players: players.iter().map(CheckpointCampaignPlayer::from).collect(),
            world_seed,
            scripts,
            objectives,
        }
    }

    fn validate(&self) -> Result<(), CheckpointError> {
        match self {
            Self::Synthetic { scenario } => scenario.validate()?,
            Self::Map { map, players, .. } => {
                if map.is_empty() || map.len() > 1024 {
                    return Err(CheckpointError::Invalid("invalid map name length".into()));
                }
                if map.split(['/', '\\']).any(|s| s == "..") {
                    return Err(CheckpointError::Invalid(
                        "parent traversal in map name".into(),
                    ));
                }
                if players.is_empty() || players.len() > tak_sim::scenario::MAX_PLAYERS {
                    return Err(CheckpointError::Invalid("invalid player count".into()));
                }
                if players.iter().any(|p| p.trim().is_empty() || p.len() > 128) {
                    return Err(CheckpointError::Invalid("invalid side name length".into()));
                }
            }
            Self::Campaign {
                map,
                players,
                objectives,
                ..
            } => {
                // Validate descriptor shape and lengths before constructing a
                // mapping, opening archives, or parsing any external content.
                if map.is_empty()
                    || map.len() > 1024
                    || map.chars().any(char::is_control)
                    || map.split(['/', '\\']).any(|s| s == "..")
                {
                    return Err(CheckpointError::Invalid("invalid campaign map name".into()));
                }
                if players.len() > tak_sim::scenario::MAX_PLAYERS
                    || players.iter().any(|p| p.side.len() > 128)
                {
                    return Err(CheckpointError::Invalid(
                        "invalid campaign player mapping size".into(),
                    ));
                }
                crate::campaign::validate_players(&campaign_players(players), *objectives)
                    .map_err(campaign_setup_error)?;
            }
        }
        Ok(())
    }

    fn load(&self, data: Option<&Path>) -> Result<(Sim, String, Option<String>), CheckpointError> {
        self.validate()?;
        match self {
            Self::Synthetic { scenario } => {
                let identity = digest_json(scenario)?;
                Ok((Sim::new(scenario)?, identity, None))
            }
            Self::Map {
                map,
                players,
                world_seed,
                scripts,
            } => {
                let data = data.ok_or_else(|| {
                    CheckpointError::Invalid(
                        "real-map checkpoint needs --data or TAK_DATA_DIR".into(),
                    )
                })?;
                let vfs = open_checkpoint_install(data)?;
                let players: Vec<_> = players
                    .iter()
                    .map(|side| PlayerSetup { side: side.clone() })
                    .collect();
                let (sim, identity) = load_map_source(&vfs, map, &players, *world_seed, *scripts)?;
                Ok((sim, identity, None))
            }
            Self::Campaign {
                map,
                players,
                world_seed,
                scripts,
                objectives,
            } => {
                let data = data.ok_or_else(|| {
                    CheckpointError::Invalid(
                        "campaign checkpoint needs --data or TAK_DATA_DIR".into(),
                    )
                })?;
                let vfs = open_checkpoint_install(data)?;
                let players = campaign_players(players);
                let (sim, identity, report) =
                    load_campaign_source(&vfs, map, &players, *world_seed, *scripts, *objectives)?;
                Ok((sim, identity, Some(report)))
            }
        }
    }
}

// Apply the checkpoint per-file ceiling to all nested rules/map/script reads,
// not just fingerprinting. Aggregate unique-input preflight remains separate.
fn open_checkpoint_install(data: &Path) -> Result<Vfs, CheckpointError> {
    let mut vfs = Vfs::with_limits(tak_vfs::hapi::Limits {
        max_file_bytes: MAX_CONTENT_FILE_BYTES,
        ..Default::default()
    });
    for path in tak_vfs::kingdoms_mount_order(data) {
        vfs.mount_archive(&path)?;
    }
    Ok(vfs)
}

fn campaign_setup_error(error: crate::Error) -> CheckpointError {
    CheckpointError::Invalid(error.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    source: CheckpointSource,
    content_identity: String,
    /// Number of completed ticks; commands at this tick have not executed.
    next_tick: u32,
    state_hash: String,
    executed: CommandLog,
    #[serde(default, skip_serializing_if = "ControllerConfig::is_empty")]
    controllers: ControllerConfig,
}

/// An immutable validated wire checkpoint. The checksum is a corruption
/// detector, not a security signature: do not trust checkpoints from strangers.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    format: String,
    version: u32,
    replay_rules_version: u32,
    payload: Payload,
    checksum: String,
}

impl Checkpoint {
    /// Parse bounded JSON, validate versions, shape, history and checksum.
    pub fn from_json(text: &str) -> Result<Self, CheckpointError> {
        if text.len() > MAX_CHECKPOINT_BYTES {
            return Err(CheckpointError::Limit(
                "serialized file is too large".into(),
            ));
        }
        #[derive(Deserialize)]
        struct Header {
            format: String,
            version: u32,
            replay_rules_version: u32,
        }
        let header: Header = serde_json::from_str(text)?;
        check_versions(&header.format, header.version, header.replay_rules_version)?;
        let checkpoint: Self = serde_json::from_str(text)?;
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    /// Read at most the maximum file size plus one byte, even from a pipe or
    /// from a file that grows between metadata inspection and reading.
    pub fn read(path: &Path) -> Result<Self, CheckpointError> {
        Self::from_json(&read_bounded(path)?)
    }

    /// Serialize canonical field order with readable indentation.
    pub fn to_json_pretty(&self) -> Result<String, CheckpointError> {
        self.validate()?;
        let text = serde_json::to_string_pretty(self)?;
        if text.len() > MAX_CHECKPOINT_BYTES {
            return Err(CheckpointError::Limit(
                "serialized file is too large".into(),
            ));
        }
        Ok(text)
    }

    /// Sync a same-directory temporary file, then atomically rename it over
    /// the destination. Any failure before rename preserves the original.
    /// Directory sync after rename is best effort (not portable everywhere).
    pub fn write_atomic(&self, path: &Path) -> Result<(), CheckpointError> {
        let text = self.to_json_pretty()?;
        write_text_atomic(path, &text)
    }

    /// Number of completed ticks; this is the tick resumed commands execute on.
    pub fn next_tick(&self) -> Tick {
        Tick(self.payload.next_tick)
    }

    /// Number of executed commands saved (future commands are never included).
    pub fn command_count(&self) -> usize {
        self.payload.executed.len()
    }

    /// Digest of the state after all completed ticks, including zero ticks.
    pub fn state_hash(&self) -> &str {
        &self.payload.state_hash
    }

    /// Identity of gameplay inputs; no retail contents are stored in it.
    pub fn content_identity(&self) -> &str {
        &self.payload.content_identity
    }

    /// Initial setup descriptor.
    pub fn source(&self) -> &CheckpointSource {
        &self.payload.source
    }

    /// Persisted future controller identity; empty for legacy/default saves.
    pub fn controllers(&self) -> &ControllerConfig {
        &self.payload.controllers
    }

    /// Reconstruct off to the side and return only a fully verified session.
    /// This never mutates a caller's existing simulation.
    pub fn restore(
        &self,
        data: Option<&Path>,
        budget: ReplayBudget,
    ) -> Result<ReplaySession, CheckpointError> {
        self.validate()?;
        if self.payload.next_tick > budget.max_ticks {
            return Err(CheckpointError::Limit(format!(
                "{} saved ticks exceed replay budget {}",
                self.payload.next_tick, budget.max_ticks
            )));
        }
        if self.payload.executed.len() > budget.max_commands {
            return Err(CheckpointError::Limit(format!(
                "{} saved commands exceed replay budget {}",
                self.payload.executed.len(),
                budget.max_commands
            )));
        }
        let (mut sim, actual, source_report) = self.payload.source.load(data)?;
        if actual != self.payload.content_identity {
            return Err(CheckpointError::ContentMismatch {
                expected: self.payload.content_identity.clone(),
                actual,
            });
        }
        validate_controllers(&self.payload.source, &self.payload.controllers, sim.world())?;
        // Historical replay intentionally NEVER invokes a controller.
        // Do not allocate a per-tick hash history while replaying a save.
        for _ in 0..self.payload.next_tick {
            sim.step(self.payload.executed.for_tick(sim.tick()));
        }
        let actual = hex(sim.state_hash());
        if actual != self.payload.state_hash {
            return Err(CheckpointError::StateMismatch {
                tick: self.payload.next_tick,
                expected: self.payload.state_hash.clone(),
                actual,
            });
        }
        Ok(ReplaySession {
            source: self.payload.source.clone(),
            content_identity: self.payload.content_identity.clone(),
            executed: self.payload.executed.clone(),
            sim,
            source_report,
            controllers: self.payload.controllers.clone(),
        })
    }

    fn validate(&self) -> Result<(), CheckpointError> {
        check_versions(&self.format, self.version, self.replay_rules_version)?;
        if self.payload.next_tick > crate::MAX_TICKS {
            return Err(CheckpointError::Limit(
                "elapsed ticks exceed hard limit".into(),
            ));
        }
        if self.payload.executed.len() > MAX_CHECKPOINT_COMMANDS {
            return Err(CheckpointError::Limit("too many executed commands".into()));
        }
        for (field, value) in [
            ("content identity", &self.payload.content_identity),
            ("state hash", &self.payload.state_hash),
            ("checksum", &self.checksum),
        ] {
            if value.len() != 16
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(CheckpointError::Invalid(format!(
                    "{field} must be 16 lowercase hex digits"
                )));
            }
        }
        if self
            .payload
            .executed
            .last_tick()
            .is_some_and(|t| t.0 >= self.payload.next_tick)
        {
            return Err(CheckpointError::Invalid(
                "history contains unexecuted commands".into(),
            ));
        }
        if digest_json(&self.payload)? != self.checksum {
            return Err(CheckpointError::ChecksumMismatch);
        }
        self.payload.source.validate()?;
        self.payload
            .controllers
            .validate_shape()
            .map_err(bot_error)?;
        reject_campaign_controllers(&self.payload.source, &self.payload.controllers)?;
        Ok(())
    }
}

/// A saveable deterministic session. Mutable world access and out-of-band
/// spawns are intentionally unavailable: only recorded commands may advance it.
pub struct ReplaySession {
    source: CheckpointSource,
    content_identity: String,
    executed: CommandLog,
    sim: Sim,
    source_report: Option<String>,
    controllers: ControllerConfig,
}

impl ReplaySession {
    /// Start a session from a synthetic setup or an external real-map install.
    pub fn new(source: CheckpointSource, data: Option<&Path>) -> Result<Self, CheckpointError> {
        Self::new_with_controllers(source, data, ControllerConfig::default())
    }

    /// Explicitly opt in to future command generation. Initial source variants
    /// and defaults are unchanged, and campaign controllers fail closed.
    pub fn new_with_controllers(
        source: CheckpointSource,
        data: Option<&Path>,
        controllers: ControllerConfig,
    ) -> Result<Self, CheckpointError> {
        controllers.validate_shape().map_err(bot_error)?;
        reject_campaign_controllers(&source, &controllers)?;
        let (sim, content_identity, source_report) = source.load(data)?;
        validate_controllers(&source, &controllers, sim.world())?;
        Ok(Self {
            source,
            content_identity,
            executed: CommandLog::new(),
            sim,
            source_report,
            controllers,
        })
    }

    /// Persisted future controller settings, without mutable access.
    pub fn controllers(&self) -> &ControllerConfig {
        &self.controllers
    }

    /// Read-only simulation, for renderers, diagnostics and current state.
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// Consume this verified replay session and transfer its complete simulation
    /// (including the script host) to an interactive client. The caller must
    /// retain the source/history separately and verify replay against the live
    /// state before saving any subsequently mutable simulation. No mutable
    /// access is exposed while a `ReplaySession` remains intact.
    pub fn into_sim(self) -> Sim {
        self.sim
    }

    /// Startup fidelity diagnostics, present for authored campaigns.
    /// These are regenerated from the local install, never saved as retail text.
    pub fn source_report(&self) -> Option<&str> {
        self.source_report.as_deref()
    }

    /// Commands executed so far, with their original tick/player/sequence keys.
    pub fn executed_log(&self) -> &CommandLog {
        &self.executed
    }

    /// Advance from the current tick. Only commands in this tick range are
    /// recorded. Supplied earlier entries must match saved history; future
    /// orders are ignored. A future-only log is also valid.
    /// All resource checks happen before stepping any tick.
    pub fn advance(&mut self, log: &CommandLog, ticks: u32) -> Result<Vec<u64>, CheckpointError> {
        let end = self.validate_advance(log, ticks, 0)?;
        let start = self.sim.tick().0;
        // A valid CommandLog is sorted and unique. New tick keys cannot collide
        // with the strictly earlier history, so this copy cannot fail.
        for entry in log.iter().filter(|e| e.tick.0 >= start && e.tick.0 < end) {
            self.executed
                .insert(entry.clone())
                .map_err(|e| CheckpointError::Invalid(e.to_string()))?;
        }
        Ok(self.sim.run(log, ticks))
    }

    fn validate_advance(
        &self,
        log: &CommandLog,
        ticks: u32,
        additional: u64,
    ) -> Result<u32, CheckpointError> {
        driver::validate_advance(self.sim.tick().0, &self.executed, log, ticks, additional)
    }

    /// Generate and record commands only for FUTURE absolute ticks. Restore and
    /// plain `advance` remain replay-only. Competing input, changed history and
    /// conservative worst-case command limits are checked before any mutation.
    pub fn advance_driven(
        &mut self,
        input: &CommandLog,
        ticks: u32,
    ) -> Result<Vec<u64>, CheckpointError> {
        validate_controllers(&self.source, &self.controllers, self.sim.world())?;
        let start = self.sim.tick().0;
        let end = driver::admit_driven(
            self.sim.world(),
            &self.controllers,
            &self.executed,
            input,
            ticks,
        )?;
        let mut hashes = Vec::with_capacity(ticks as usize);
        for _ in start..end {
            let realized =
                driver::realize_admitted_tick(self.sim.world(), &self.controllers, input)?;
            for entry in &realized {
                // Preflight proves unique future keys; normal insertion cannot
                // fail. Retain a typed error instead of asserting/panicking.
                self.executed
                    .insert(entry.clone())
                    .map_err(|error| CheckpointError::Invalid(error.to_string()))?;
            }
            hashes.push(self.sim.step(&realized));
        }
        Ok(hashes)
    }

    /// Capture the exact current state as replay data, including tick zero.
    pub fn checkpoint(&self) -> Result<Checkpoint, CheckpointError> {
        let payload = Payload {
            source: self.source.clone(),
            content_identity: self.content_identity.clone(),
            next_tick: self.sim.tick().0,
            state_hash: hex(self.sim.state_hash()),
            executed: self.executed.clone(),
            controllers: self.controllers.clone(),
        };
        let checksum = digest_json(&payload)?;
        let checkpoint = Checkpoint {
            format: FORMAT.into(),
            version: CHECKPOINT_FORMAT_VERSION,
            replay_rules_version: REPLAY_RULES_VERSION,
            payload,
            checksum,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }
}

fn bot_error(error: crate::bot::BotError) -> CheckpointError {
    CheckpointError::Invalid(error.to_string())
}

fn reject_campaign_controllers(
    source: &CheckpointSource,
    controllers: &ControllerConfig,
) -> Result<(), CheckpointError> {
    if !controllers.controllers.is_empty() && matches!(source, CheckpointSource::Campaign { .. }) {
        return Err(CheckpointError::Invalid(
            "simple-skirmish-v1 does not support campaign sources".into(),
        ));
    }
    Ok(())
}

fn validate_controllers(
    source: &CheckpointSource,
    controllers: &ControllerConfig,
    world: &tak_sim::World,
) -> Result<(), CheckpointError> {
    reject_campaign_controllers(source, controllers)?;
    controllers.validate(world).map_err(bot_error)
}

fn check_versions(format: &str, version: u32, rules: u32) -> Result<(), CheckpointError> {
    if format != FORMAT {
        return Err(CheckpointError::Format(format.into()));
    }
    if version != CHECKPOINT_FORMAT_VERSION {
        return Err(CheckpointError::UnsupportedVersion(version));
    }
    if rules != REPLAY_RULES_VERSION {
        return Err(CheckpointError::UnsupportedRules(rules));
    }
    Ok(())
}

/// Load a bounded continuation/recording log. Future orders may be present,
/// but the complete input still obeys the hard command-count limit.
pub fn read_command_log(path: Option<&Path>) -> Result<CommandLog, CheckpointError> {
    let Some(path) = path else {
        return Ok(CommandLog::new());
    };
    let log = CommandLog::from_json(&read_bounded(path)?)?;
    if log.len() > MAX_CHECKPOINT_COMMANDS {
        return Err(CheckpointError::Limit(
            "input log has too many commands".into(),
        ));
    }
    Ok(log)
}

fn read_bounded(path: &Path) -> Result<String, CheckpointError> {
    let file = File::open(path).map_err(|source| io(path, source))?;
    let mut bytes = Vec::new();
    file.take(MAX_CHECKPOINT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io(path, source))?;
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(CheckpointError::Limit(
            "serialized file is too large".into(),
        ));
    }
    String::from_utf8(bytes).map_err(|_| CheckpointError::Invalid("file is not UTF-8".into()))
}

/// Write a text output by same-directory temporary file and atomic rename.
/// This never truncates the old destination inode, so other hard links to it
/// remain intact. Checkpoint serialization/size validation happens separately.
pub fn write_text_atomic(path: &Path, text: &str) -> Result<(), CheckpointError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| CheckpointError::Invalid("checkpoint output needs a file name".into()))?;
    let mut temporary = None;
    for _ in 0..16 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = name.to_os_string();
        temp_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
        let temp_path = parent.join(temp_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => {
                temporary = Some((temp_path, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io(&temp_path, e)),
        }
    }
    let (temp_path, mut file) = temporary.ok_or_else(|| {
        CheckpointError::Invalid("cannot allocate a unique temporary checkpoint file".into())
    })?;
    let result = (|| {
        file.write_all(text.as_bytes())
            .map_err(|e| io(&temp_path, e))?;
        file.sync_all().map_err(|e| io(&temp_path, e))?;
        drop(file);
        std::fs::rename(&temp_path, path).map_err(|e| io(path, e))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    } else if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
    result
}

fn io(path: &Path, source: std::io::Error) -> CheckpointError {
    CheckpointError::Io {
        path: path.into(),
        source,
    }
}

fn hex(hash: u64) -> String {
    format!("{hash:016x}")
}

fn digest_json(value: &impl Serialize) -> Result<String, CheckpointError> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_CHECKPOINT_BYTES {
        return Err(CheckpointError::Limit(
            "serialized payload is too large".into(),
        ));
    }
    Ok(hex(xxhash_rust::xxh64::xxh64(&bytes, 0)))
}

fn hash_part(hash: &mut Xxh64, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

// Use exactly the public conversions of tak_sim::load_match, but stage declared
// size preflight before each loader and do not load ignored COB/model data.
fn load_map_source(
    vfs: &Vfs,
    map: &str,
    players: &[PlayerSetup],
    seed: u64,
    scripts: bool,
) -> Result<(Sim, String), CheckpointError> {
    let mut paths = base_content_paths(vfs, map);
    preflight_content(vfs, &paths)?;
    let rules = Ruleset::load(vfs, RulesetKind::Base);
    let loaded_map = tak_content::MapDef::load(vfs, &rules, map)
        .map_err(tak_sim::content::ContentError::from)?;
    let mut scenario =
        Scenario::from_content(&loaded_map, &rules, players)?.with_build_trees(&rules);
    scenario.seed = seed;
    scenario.validate()?;
    if scripts {
        add_script_paths(&mut paths, &rules, &scenario);
        preflight_content(vfs, &paths)?;
    }
    let identity = identity_for_paths(vfs, &paths, &scenario, scripts)?;
    let sim = if scripts {
        Sim::with_scripts(
            &scenario,
            Box::new(tak_sim::content::load_scripts(vfs, &rules, &scenario)),
        )?
    } else {
        Sim::new(&scenario)?
    };
    Ok((sim, identity))
}

// Do not call load_campaign directly: it eagerly reads COB/model data before
// the second size preflight and even when the descriptor disables scripts.
fn load_campaign_source(
    vfs: &Vfs,
    map: &str,
    players: &[tak_sim::campaign::CampaignPlayer],
    seed: u64,
    scripts: bool,
    objectives: bool,
) -> Result<(Sim, String, String), CheckpointError> {
    crate::campaign::validate_players(players, objectives).map_err(campaign_setup_error)?;
    let mut paths = base_content_paths(vfs, map);
    preflight_content(vfs, &paths)?;
    let rules = Ruleset::load(vfs, RulesetKind::Base);
    let loaded_map = tak_content::MapDef::load(vfs, &rules, map)
        .map_err(tak_sim::content::ContentError::from)?;
    let (mut scenario, unsupported) =
        tak_sim::campaign::scenario_from_campaign(&loaded_map, &rules, players)?;
    scenario.seed = seed;
    if objectives {
        crate::campaign::enable_strict_objectives(&mut scenario, &loaded_map, players)
            .map_err(campaign_setup_error)?;
    }
    scenario.validate()?;
    if scripts {
        add_script_paths(&mut paths, &rules, &scenario);
        preflight_content(vfs, &paths)?;
    }
    // Include the descriptor as well as content. Distinct owner ordering or
    // objective/script modes can convert to the same Scenario on empty maps.
    let content = identity_for_paths(vfs, &paths, &scenario, scripts)?;
    let source = CheckpointSource::campaign(map, players, seed, scripts, objectives);
    let identity = digest_json(&("tak-checkpoint-campaign-v1", source, content))?;
    let report = crate::campaign::report_setup(&scenario, &unsupported, players, objectives);
    let sim = if scripts {
        Sim::with_scripts(
            &scenario,
            Box::new(tak_sim::content::load_scripts(vfs, &rules, &scenario)),
        )?
    } else {
        Sim::new(&scenario)?
    };
    Ok((sim, identity, report))
}

fn base_content_paths(vfs: &Vfs, map: &str) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for (directory, extension) in [
        ("units", ".fbi"),
        ("features", ".tdf"),
        ("canbuild", ".tdf"),
    ] {
        for entry in vfs.list(directory) {
            if entry.path.ends_with(extension) {
                paths.insert(entry.path);
            }
        }
    }
    paths.insert("gamedata/sidedata.tdf".into());
    paths.insert("gamedata/moveinfo.tdf".into());
    let map = tak_vfs::normalize(map);
    let map = map.strip_suffix(".tnt").unwrap_or(&map);
    for extension in ["tnt", "ota", "crt"] {
        paths.insert(format!("{map}.{extension}"));
    }
    paths
}

fn add_script_paths(paths: &mut BTreeSet<String>, rules: &Ruleset, scenario: &Scenario) {
    for def in &scenario.unit_defs {
        if let Some(unit) = rules.unit(&def.name) {
            paths.insert(tak_vfs::normalize(&unit.script));
            if let Some(object) = &unit.objectname {
                paths.insert(tak_vfs::normalize(&format!("objects3d/{object}.3do")));
            }
        }
    }
}

// Stage the aggregate unique-input guard before parsers. The VFS also bounds
// each nested read and checks SQSH/directory agreement before decoding.
fn preflight_content(vfs: &Vfs, paths: &BTreeSet<String>) -> Result<(), CheckpointError> {
    let mut total = 0u64;
    for path in paths {
        if let Some(info) = vfs.stat(path) {
            total = total.saturating_add(info.size);
            if info.size > MAX_CONTENT_FILE_BYTES || total > MAX_CONTENT_BYTES {
                return Err(CheckpointError::Limit(
                    "gameplay content exceeds declared-size budget".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Deliberately conservative: raw map/rules bytes also detect edits that happen
/// to convert to the same current Scenario. Renderer-only textures, audio and
/// UI assets are not relevant. Models matter to script piece/weapon origins.
fn identity_for_paths(
    vfs: &Vfs,
    paths: &BTreeSet<String>,
    scenario: &Scenario,
    scripts: bool,
) -> Result<String, CheckpointError> {
    // Recheck immediately before reads, including callers of this helper.
    preflight_content(vfs, paths)?;
    let mut hash = Xxh64::new(0);
    hash_part(&mut hash, b"tak-checkpoint-content-v1");
    hash_part(&mut hash, &serde_json::to_vec(scenario)?);
    hash_part(&mut hash, &[u8::from(scripts)]);
    let mut total = 0u64;
    for path in paths {
        hash_part(&mut hash, path.as_bytes());
        if vfs.stat(path).is_none() {
            hash_part(&mut hash, b"missing");
            continue;
        }
        hash_part(&mut hash, b"present");
        let remaining = MAX_CONTENT_BYTES.saturating_sub(total);
        let bytes = vfs.read_bounded(path, remaining.min(MAX_CONTENT_FILE_BYTES))?;
        total += bytes.len() as u64;
        hash_part(&mut hash, &bytes);
    }
    Ok(hex(hash.digest()))
}

#[cfg(test)]
fn map_identity(
    vfs: &Vfs,
    map: &str,
    scenario: &Scenario,
    scripts: bool,
) -> Result<String, CheckpointError> {
    let mut paths = base_content_paths(vfs, map);
    preflight_content(vfs, &paths)?;
    if scripts {
        let rules = Ruleset::load(vfs, RulesetKind::Base);
        add_script_paths(&mut paths, &rules, scenario);
    }
    identity_for_paths(vfs, &paths, scenario, scripts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_commands::{PlayerId, SimCommand, UnitId, UnitSet};
    use tak_sim::testing::{example_scenario, random_log, walk_log};

    fn session() -> ReplaySession {
        ReplaySession::new(CheckpointSource::synthetic(example_scenario(42, 2)), None).unwrap()
    }

    fn reseal(checkpoint: &mut Checkpoint) {
        checkpoint.checksum = digest_json(&checkpoint.payload).unwrap();
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(label: &str) -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "tak-checkpoint-{label}-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, path: &str, bytes: &[u8]) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn additive_campaign_kind_preserves_wire_payload_and_checks_replay_rules() {
        let source = CheckpointSource::map("KMAP/Example", "ARA,ZON", 7, false);
        let payload = Payload {
            source,
            content_identity: "0000000000000000".into(),
            next_tick: 0,
            state_hash: "1111111111111111".into(),
            executed: CommandLog::new(),
            controllers: ControllerConfig::default(),
        };
        // Exact pre-campaign v1 field order, not a Value round trip (which
        // would sort object keys and fail to pin the canonical checksum).
        let legacy = r#"{"source":{"kind":"map","map":"kmap/example","players":["ARA","ZON"],"world_seed":7,"scripts":false},"content_identity":"0000000000000000","next_tick":0,"state_hash":"1111111111111111","executed":{"version":1,"entries":[]}}"#;
        assert_eq!(serde_json::to_string(&payload).unwrap(), legacy);
        let checksum = hex(xxhash_rust::xxh64::xxh64(legacy.as_bytes(), 0));
        assert_eq!(digest_json(&payload).unwrap(), checksum);
        let wire = format!(
            r#"{{"format":"tak-replay-checkpoint","version":1,"replay_rules_version":1,"payload":{legacy},"checksum":"{checksum}"}}"#
        );
        assert!(matches!(
            Checkpoint::from_json(&wire),
            Err(CheckpointError::UnsupportedRules(1))
        ));
        let previous = wire.replace("\"replay_rules_version\":1", "\"replay_rules_version\":2");
        assert!(matches!(
            Checkpoint::from_json(&previous),
            Err(CheckpointError::UnsupportedRules(2))
        ));
        let previous = wire.replace("\"replay_rules_version\":1", "\"replay_rules_version\":3");
        assert!(matches!(
            Checkpoint::from_json(&previous),
            Err(CheckpointError::UnsupportedRules(3))
        ));
        let previous = wire.replace("\"replay_rules_version\":1", "\"replay_rules_version\":4");
        assert!(matches!(
            Checkpoint::from_json(&previous),
            Err(CheckpointError::UnsupportedRules(4))
        ));
        let current = wire.replace(
            "\"replay_rules_version\":1",
            &format!("\"replay_rules_version\":{REPLAY_RULES_VERSION}"),
        );
        let parsed = Checkpoint::from_json(&current).unwrap();
        assert_eq!(parsed.source(), &payload.source);
        assert_eq!(parsed.checksum, checksum);
        assert_eq!(CHECKPOINT_FORMAT_VERSION, 1);
        assert_eq!(REPLAY_RULES_VERSION, 5);
    }

    #[test]
    fn consuming_verified_session_transfers_tick_and_complete_state() {
        let mut running = session();
        running.advance(&CommandLog::new(), 9).unwrap();
        let saved = running.checkpoint().unwrap();
        let restored = saved.restore(None, ReplayBudget::default()).unwrap();
        let mut sim = restored.into_sim();
        assert_eq!(sim.tick(), saved.next_tick());
        assert_eq!(hex(sim.state_hash()), saved.state_hash());
        assert_eq!(
            sim.step(&[]),
            running.advance(&CommandLog::new(), 1).unwrap()[0]
        );
    }

    #[test]
    fn split_restore_continuation_matches_uninterrupted_for_every_split() {
        let scenario = example_scenario(0xfeed, 4);
        let log = random_log(&scenario, 600, 77);
        let expected = tak_sim::run_scenario(&scenario, &log, 600).unwrap();
        for split in [0, 1, 2, 29, 300, 599, 600] {
            let mut first =
                ReplaySession::new(CheckpointSource::synthetic(scenario.clone()), None).unwrap();
            assert_eq!(
                first.advance(&log, split).unwrap(),
                expected[..split as usize]
            );
            let checkpoint = first.checkpoint().unwrap();
            assert_eq!(
                checkpoint.command_count(),
                log.iter().filter(|e| e.tick.0 < split).count()
            );
            let json = checkpoint.to_json_pretty().unwrap();
            let checkpoint = Checkpoint::from_json(&json).unwrap();
            let mut resumed = checkpoint.restore(None, ReplayBudget::default()).unwrap();
            assert_eq!(resumed.sim().tick(), Tick(split));
            assert_eq!(resumed.sim().state_hash(), first.sim().state_hash());
            assert_eq!(
                resumed.advance(&log, 600 - split).unwrap(),
                expected[split as usize..]
            );
            assert_eq!(resumed.sim().state_hash(), *expected.last().unwrap());
        }
    }

    #[test]
    fn future_only_commands_and_sequence_keys_survive_multiple_resumes() {
        let mut running = session();
        let mut original = CommandLog::new();
        original.record(
            Tick(2),
            PlayerId(0),
            SimCommand::Stop {
                units: UnitSet::single(UnitId::new(0, 0)),
            },
        );
        original.record(
            Tick(2),
            PlayerId(0),
            SimCommand::Stop {
                units: UnitSet::single(UnitId::new(0, 0)),
            },
        );
        original.record(
            Tick(30),
            PlayerId(0),
            SimCommand::Stop {
                units: UnitSet::single(UnitId::new(0, 0)),
            },
        );
        running.advance(&original, 10).unwrap();
        assert_eq!(running.executed_log().len(), 2);
        let mut resumed = running
            .checkpoint()
            .unwrap()
            .restore(None, ReplayBudget::default())
            .unwrap();
        let mut later = CommandLog::new();
        later.record(
            Tick(10),
            PlayerId(0),
            SimCommand::Stop {
                units: UnitSet::single(UnitId::new(0, 0)),
            },
        );
        resumed.advance(&later, 10).unwrap();
        assert_eq!(resumed.executed_log().len(), 3);
        assert_eq!(resumed.executed_log().entries()[1].seq, 1);
        let second = resumed.checkpoint().unwrap();
        let resumed_again = second.restore(None, ReplayBudget::default()).unwrap();
        assert_eq!(resumed_again.sim().state_hash(), resumed.sim().state_hash());
        assert_eq!(resumed_again.executed_log(), resumed.executed_log());
    }

    #[test]
    fn rejects_changed_past_history_before_mutating_session() {
        let mut running = session();
        let sc = example_scenario(42, 2);
        let log = walk_log(&sc, 40, 4, 2);
        running.advance(&log, 20).unwrap();
        let before = running.checkpoint().unwrap().to_json_pretty().unwrap();
        let mut changed = CommandLog::new();
        changed.record(
            Tick(1),
            PlayerId(0),
            SimCommand::Stop {
                units: UnitSet::single(UnitId::new(0, 0)),
            },
        );
        assert!(matches!(
            running.advance(&changed, 10),
            Err(CheckpointError::Invalid(_))
        ));
        assert_eq!(
            running.checkpoint().unwrap().to_json_pretty().unwrap(),
            before
        );
    }

    #[test]
    fn malformed_programmatic_log_cannot_partially_mutate_history() {
        let mut running = session();
        let before = running.checkpoint().unwrap().to_json_pretty().unwrap();
        let mut log = CommandLog::new();
        let command = SimCommand::Stop {
            units: UnitSet::new(),
        };
        log.insert(tak_commands::LoggedCommand {
            tick: Tick(0),
            player: PlayerId(0),
            seq: u32::MAX,
            command: command.clone(),
        })
        .unwrap();
        // CommandLog::record saturates the existing sequence at MAX, which can
        // produce a duplicate even though JSON loading correctly rejects it.
        log.record(Tick(0), PlayerId(0), command);
        assert!(matches!(
            running.advance(&log, 1),
            Err(CheckpointError::Invalid(_))
        ));
        assert_eq!(
            running.checkpoint().unwrap().to_json_pretty().unwrap(),
            before
        );
    }

    #[test]
    fn zero_tick_hash_is_verified_not_an_empty_hash_sentinel() {
        let running = session();
        let mut checkpoint = running.checkpoint().unwrap();
        assert_eq!(checkpoint.next_tick(), Tick(0));
        assert_eq!(checkpoint.state_hash(), hex(running.sim().state_hash()));
        checkpoint.payload.state_hash = "0000000000000000".into();
        reseal(&mut checkpoint);
        assert!(matches!(
            checkpoint.restore(None, ReplayBudget::default()),
            Err(CheckpointError::StateMismatch { tick: 0, .. })
        ));
    }

    #[test]
    fn versions_corruption_unknown_fields_and_duplicate_commands_rejected() {
        let checkpoint = session().checkpoint().unwrap();
        let mut value = serde_json::to_value(&checkpoint).unwrap();
        value["version"] = 2.into();
        assert!(matches!(
            Checkpoint::from_json(&value.to_string()),
            Err(CheckpointError::UnsupportedVersion(2))
        ));
        value["version"] = 1.into();
        value["replay_rules_version"] = 999.into();
        assert!(matches!(
            Checkpoint::from_json(&value.to_string()),
            Err(CheckpointError::UnsupportedRules(999))
        ));
        value["replay_rules_version"] = REPLAY_RULES_VERSION.into();
        value["surprise"] = true.into();
        assert!(matches!(
            Checkpoint::from_json(&value.to_string()),
            Err(CheckpointError::Json(_))
        ));
        value.as_object_mut().unwrap().remove("surprise");
        value["payload"]["next_tick"] = 1.into();
        assert!(matches!(
            Checkpoint::from_json(&value.to_string()),
            Err(CheckpointError::ChecksumMismatch)
        ));
        assert!(Checkpoint::from_json("{not json}").is_err());
        assert!(
            Checkpoint::from_json(
                "{\"format\":\"other\",\"version\":1,\"replay_rules_version\":1}"
            )
            .is_err()
        );
        let entry = serde_json::json!({"tick": 0, "player": 0, "seq": 0, "command": {"type": "stop", "units": []}});
        value["payload"]["executed"]["entries"] = serde_json::json!([entry.clone(), entry]);
        assert!(matches!(
            Checkpoint::from_json(&value.to_string()),
            Err(CheckpointError::Json(_))
        ));
    }

    #[test]
    fn content_identity_state_and_history_validation_fail_closed() {
        let mut checkpoint = session().checkpoint().unwrap();
        checkpoint.payload.content_identity = "0000000000000000".into();
        reseal(&mut checkpoint);
        assert!(matches!(
            checkpoint.restore(None, ReplayBudget::default()),
            Err(CheckpointError::ContentMismatch { .. })
        ));
        let mut checkpoint = session().checkpoint().unwrap();
        checkpoint.payload.executed.record(
            Tick(0),
            PlayerId(0),
            SimCommand::Stop {
                units: UnitSet::new(),
            },
        );
        reseal(&mut checkpoint);
        assert!(matches!(
            checkpoint.restore(None, ReplayBudget::default()),
            Err(CheckpointError::Invalid(_))
        ));
        let mut checkpoint = session().checkpoint().unwrap();
        if let CheckpointSource::Synthetic { scenario } = &mut checkpoint.payload.source {
            scenario.map.width = 0;
        }
        reseal(&mut checkpoint);
        assert!(matches!(
            checkpoint.restore(None, ReplayBudget::default()),
            Err(CheckpointError::Scenario(_))
        ));
    }

    #[test]
    fn replay_and_file_limits_are_checked_before_loading_or_mutating() {
        let mut running = session();
        let before = running.sim().state_hash();
        assert!(matches!(
            running.advance(&CommandLog::new(), crate::MAX_TICKS + 1),
            Err(CheckpointError::Limit(_))
        ));
        assert_eq!(running.sim().state_hash(), before);
        let log = walk_log(&example_scenario(42, 2), 20, 2, 3);
        running.advance(&log, 20).unwrap();
        let checkpoint = running.checkpoint().unwrap();
        assert!(matches!(
            checkpoint.restore(
                None,
                ReplayBudget {
                    max_ticks: 19,
                    max_commands: usize::MAX
                }
            ),
            Err(CheckpointError::Limit(_))
        ));
        assert!(matches!(
            checkpoint.restore(
                None,
                ReplayBudget {
                    max_ticks: 20,
                    max_commands: 0
                }
            ),
            Err(CheckpointError::Limit(_))
        ));
        let mut checkpoint = checkpoint.clone();
        checkpoint.payload.next_tick = u32::MAX;
        reseal(&mut checkpoint);
        assert!(matches!(
            checkpoint.restore(
                None,
                ReplayBudget {
                    max_ticks: u32::MAX,
                    max_commands: usize::MAX
                }
            ),
            Err(CheckpointError::Limit(_))
        ));
        let text = " ".repeat(MAX_CHECKPOINT_BYTES + 1);
        assert!(matches!(
            Checkpoint::from_json(&text),
            Err(CheckpointError::Limit(_))
        ));
        let scratch = Scratch::new("oversize");
        let path = scratch.0.join("sparse.json");
        File::create(&path)
            .unwrap()
            .set_len(MAX_CHECKPOINT_BYTES as u64 + 1)
            .unwrap();
        assert!(matches!(
            Checkpoint::read(&path),
            Err(CheckpointError::Limit(_))
        ));
        assert!(matches!(
            read_command_log(Some(&path)),
            Err(CheckpointError::Limit(_))
        ));
        assert!(matches!(
            CheckpointSource::from_scenario_file(&path),
            Err(CheckpointError::Limit(_))
        ));
        scratch.write("non-utf8.json", &[0xff]);
        assert!(matches!(
            Checkpoint::read(&scratch.0.join("non-utf8.json")),
            Err(CheckpointError::Invalid(_))
        ));
    }

    #[test]
    fn atomic_replace_round_trips_and_failed_save_preserves_original() {
        let scratch = Scratch::new("atomic");
        let path = scratch.0.join("save.json");
        scratch.write("save.json", b"old contents");
        let mut running = session();
        running.checkpoint().unwrap().write_atomic(&path).unwrap();
        assert_eq!(Checkpoint::read(&path).unwrap().next_tick(), Tick(0));
        running.advance(&CommandLog::new(), 10).unwrap();
        let saved = running.checkpoint().unwrap();
        saved.write_atomic(&path).unwrap();
        assert_eq!(Checkpoint::read(&path).unwrap().next_tick(), Tick(10));
        let original = std::fs::read(&path).unwrap();
        let mut bad = saved.clone();
        bad.payload.state_hash = "not a hash".into();
        assert!(bad.write_atomic(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        // Rename over a directory fails after writing/syncing the temporary file.
        scratch.write("directory/keep", b"unchanged");
        assert!(saved.write_atomic(&scratch.0.join("directory")).is_err());
        assert_eq!(
            std::fs::read(scratch.0.join("directory/keep")).unwrap(),
            b"unchanged"
        );
        assert!(
            !std::fs::read_dir(&scratch.0).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-"))
        );
    }

    #[test]
    fn identity_covers_raw_rules_map_used_scripts_models_and_missing_inputs() {
        let scratch = Scratch::new("identity");
        scratch.write(
            "units/monarch.fbi",
            b"[UNITINFO]\n{\nunitname=monarch;\nobjectname=monarch;\n}\n",
        );
        scratch.write("kmap/synthetic.tnt", b"synthetic map bytes only");
        scratch.write("scripts/monarch.cob", b"synthetic script bytes only");
        scratch.write("objects3d/monarch.3do", b"synthetic model bytes only");
        scratch.write("features/test.tdf", b"[feature]\n{\nblocking=1;\n}\n");
        let mut vfs = Vfs::new();
        vfs.mount_dir(&scratch.0).unwrap();
        let scenario = example_scenario(1, 1);
        let base = map_identity(&vfs, "kmap/synthetic", &scenario, true).unwrap();
        for path in [
            "units/monarch.fbi",
            "kmap/synthetic.tnt",
            "scripts/monarch.cob",
            "objects3d/monarch.3do",
            "features/test.tdf",
        ] {
            let original = std::fs::read(scratch.0.join(path)).unwrap();
            let mut changed = original.clone();
            changed.push(b' ');
            scratch.write(path, &changed);
            assert_ne!(
                map_identity(&vfs, "kmap/synthetic", &scenario, true).unwrap(),
                base,
                "{path}"
            );
            scratch.write(path, &original);
        }
        let no_scripts = map_identity(&vfs, "kmap/synthetic", &scenario, false).unwrap();
        scratch.write("scripts/monarch.cob", b"other script");
        assert_eq!(
            map_identity(&vfs, "kmap/synthetic", &scenario, false).unwrap(),
            no_scripts
        );
        scratch.write("scripts/monarch.cob", b"synthetic script bytes only");
        // Adding a previously missing optional OTA is a content change.
        scratch.write("kmap/synthetic.ota", b"synthetic ota");
        scratch.write("sounds/cosmetic.wav", b"not a real audio file");
        let mut refreshed = Vfs::new();
        refreshed.mount_dir(&scratch.0).unwrap();
        assert_ne!(
            map_identity(&refreshed, "kmap/synthetic", &scenario, true).unwrap(),
            base
        );
        std::fs::remove_file(scratch.0.join("kmap/synthetic.ota")).unwrap();
        let mut refreshed = Vfs::new();
        refreshed.mount_dir(&scratch.0).unwrap();
        assert_eq!(
            map_identity(&refreshed, "kmap/synthetic", &scenario, true).unwrap(),
            base
        );
    }

    // Hand-built 4x4 cell TNT with flat height and no features/art/minimap.
    fn tiny_map_bytes() -> Vec<u8> {
        let header: [u32; 13] = [0x4000, 4, 4, 0, 52, 68, 0, 0, 100, 116, 120, 0, 0];
        let mut bytes: Vec<_> = header.into_iter().flat_map(u32::to_le_bytes).collect();
        bytes.extend([0u8; 16]);
        for _ in 0..16 {
            bytes.extend(0xffffu16.to_le_bytes());
        }
        bytes.extend([0u8; 24]);
        bytes
    }

    #[test]
    fn declared_content_limits_are_preflighted_before_any_map_or_rules_decode() {
        for kind in ["map", "rules", "total"] {
            let scratch = Scratch::new(kind);
            scratch.write("kmap/test.tnt", b"intentionally invalid small map");
            let paths = match kind {
                "map" => vec![("kmap/test.tnt".to_string(), MAX_CONTENT_FILE_BYTES + 1)],
                "rules" => vec![("units/oversize.fbi".to_string(), MAX_CONTENT_FILE_BYTES + 1)],
                _ => (0..5)
                    .map(|i| (format!("features/{i}.tdf"), MAX_CONTENT_FILE_BYTES))
                    .collect(),
            };
            for (path, length) in paths {
                scratch.write(&path, &[]);
                File::options()
                    .write(true)
                    .open(scratch.0.join(path))
                    .unwrap()
                    .set_len(length)
                    .unwrap();
            }
            let mut vfs = Vfs::new();
            vfs.mount_dir(&scratch.0).unwrap();
            // The invalid TNT would give Content(Map(Tnt)) if any decode ran.
            assert!(
                matches!(
                    load_map_source(&vfs, "kmap/test", &PlayerSetup::parse_list("TST"), 1, false),
                    Err(CheckpointError::Limit(_))
                ),
                "{kind}"
            );
        }
    }

    #[test]
    fn declared_script_model_limits_precede_loading_and_no_scripts_skips_them() {
        let scratch = Scratch::new("no-scripts");
        scratch.write("kmap/test.tnt", &tiny_map_bytes());
        scratch.write("kmap/test.ota", b"[GlobalHeader]\n{\nnumplayers=1;\n[Map Data]\n{\n[specials]\n{\n[special0]\n{\nspecialwhat=StartPos1;\nXPos=1;\nZPos=1;\n}\n}\n}\n}\n");
        scratch.write(
            "units/monarch.fbi",
            b"[UNITINFO]\n{\nunitname=monarch;\nobjectname=monarch;\n}\n",
        );
        scratch.write(
            "gamedata/sidedata.tdf",
            b"[SIDE0]\n{\nnameprefix=TST;\ncommander=monarch;\n}\n",
        );
        for path in ["scripts/monarch.cob", "objects3d/monarch.3do"] {
            scratch.write(path, &[]);
            File::options()
                .write(true)
                .open(scratch.0.join(path))
                .unwrap()
                .set_len(MAX_CONTENT_FILE_BYTES + 1)
                .unwrap();
        }
        let mut vfs = Vfs::new();
        vfs.mount_dir(&scratch.0).unwrap();
        let players = PlayerSetup::parse_list("TST");
        assert!(matches!(
            load_map_source(&vfs, "kmap/test", &players, 1, true),
            Err(CheckpointError::Limit(_))
        ));
        let (mut sim, _) = load_map_source(&vfs, "kmap/test", &players, 1, false).unwrap();
        assert!(sim.cob().is_none());
        sim.run(&CommandLog::new(), 1);
        assert!(sim.world().unit(UnitId::new(0, 0)).is_some());
    }

    fn campaign_install(label: &str, units: &str, condition: &str) -> Scratch {
        let scratch = Scratch::new(label);
        let ota = format!(
            "[GLOBALHEADER]\n{{\n{condition}\n[MAP DATA]\n{{\n[UNITS]\n{{\n{units}\n}}\n}}\n}}\n"
        );
        let archive = tak_vfs::testing::HapiBuilder::new()
            .file("missions/test.tnt", &tiny_map_bytes(), false)
            .file("missions/test.ota", ota.as_bytes(), false)
            .file(
                "units/monarch.fbi",
                b"[UNITINFO]\n{\nunitname=monarch;\nmaxdamage=200;\nobjectname=monarch;\n}\n",
                false,
            )
            .file(
                "gamedata/sidedata.tdf",
                b"[SIDE0]\n{\nnameprefix=TST;\ncommander=monarch;\n}\n",
                false,
            )
            .build();
        scratch.write("synthetic.hpi", &archive);
        scratch
    }

    fn authored_units() -> &'static str {
        "[UNIT0]\n{\nunitname=monarch;\nplayer=2;\nxpos=2;\nzpos=2;\ninitialmission=patrol;\naiignore=1;\n}\n[UNIT1]\n{\nunitname=monarch;\nplayer=1;\nxpos=1;\nzpos=1;\n}\n"
    }

    #[test]
    fn campaign_identity_binds_ordered_mapping_seed_scripts_and_objective_mode() {
        // Empty startup deliberately makes mapping changes observationally
        // equivalent to the current Scenario. The descriptor must still bind.
        let scratch = campaign_install("campaign-identity", "", "destroyallunits=1;");
        let players = crate::campaign::parse_players("1:TST,2:TST").unwrap();
        let source = CheckpointSource::campaign("missions/test", &players, 9, false, false);
        let checkpoint = ReplaySession::new(source, Some(&scratch.0))
            .unwrap()
            .checkpoint()
            .unwrap();
        let text = checkpoint.to_json_pretty().unwrap();
        assert!(!text.contains("unit_defs"));
        assert!(!text.contains("initial_units"));
        assert!(!text.contains(scratch.0.to_str().unwrap()));
        let parsed = Checkpoint::from_json(&text).unwrap();
        assert_eq!(parsed.source(), checkpoint.source());
        parsed
            .restore(Some(&scratch.0), ReplayBudget::default())
            .unwrap();
        for variant in [
            "mapping-order",
            "mapping-slot",
            "side",
            "seed",
            "scripts",
            "objectives",
        ] {
            let mut changed = checkpoint.clone();
            if let CheckpointSource::Campaign {
                players,
                world_seed,
                scripts,
                objectives,
                ..
            } = &mut changed.payload.source
            {
                match variant {
                    "mapping-order" => players.swap(0, 1),
                    "mapping-slot" => players[1].ota_slot = 10,
                    "side" => players[0].side = "tst".into(),
                    "seed" => *world_seed += 1,
                    "scripts" => *scripts = true,
                    "objectives" => *objectives = true,
                    _ => unreachable!(),
                }
            }
            reseal(&mut changed);
            assert!(
                matches!(
                    changed.restore(Some(&scratch.0), ReplayBudget::default()),
                    Err(CheckpointError::ContentMismatch { .. })
                ),
                "{variant}"
            );
        }
    }

    #[test]
    fn campaign_zero_tick_and_repeated_resume_match_authored_startup() {
        let scratch = campaign_install("campaign-resume", authored_units(), "");
        let players = crate::campaign::parse_players("1:TST,2:TST").unwrap();
        let source = CheckpointSource::campaign("missions/test", &players, 4, false, false);
        let mut whole = ReplaySession::new(source.clone(), Some(&scratch.0)).unwrap();
        assert!(
            whole
                .source_report()
                .unwrap()
                .contains("ignores initialmission")
        );
        let mut log = CommandLog::new();
        for tick in [0, 0, 1, 29, 60, 119] {
            log.record(
                Tick(tick),
                PlayerId(1),
                SimCommand::Stop {
                    units: UnitSet::single(UnitId::new(0, 0)),
                },
            );
        }
        let expected = whole.advance(&log, 120).unwrap();
        for split in [0, 1, 29, 60, 119, 120] {
            let mut first = ReplaySession::new(source.clone(), Some(&scratch.0)).unwrap();
            first.advance(&log, split).unwrap();
            let checkpoint = first.checkpoint().unwrap();
            let mut resumed = checkpoint
                .restore(Some(&scratch.0), ReplayBudget::default())
                .unwrap();
            assert_eq!(first.sim().state_hash(), resumed.sim().state_hash());
            assert_eq!(
                resumed.advance(&log, 120 - split).unwrap(),
                expected[split as usize..]
            );
            let again = resumed
                .checkpoint()
                .unwrap()
                .restore(Some(&scratch.0), ReplayBudget::default())
                .unwrap();
            assert_eq!(again.sim().state_hash(), whole.sim().state_hash());
            assert_eq!(again.executed_log(), whole.executed_log());
        }
    }

    #[test]
    fn campaign_descriptor_and_replay_budgets_fail_before_install_access() {
        let players = crate::campaign::parse_players("1:TST,2:TST").unwrap();
        let source = CheckpointSource::campaign("missions/test", &players, 1, false, false);
        for variant in [
            "map",
            "map-control",
            "count",
            "owner",
            "duplicate",
            "side",
            "objectives",
        ] {
            let mut changed = source.clone();
            if let CheckpointSource::Campaign {
                map,
                players,
                objectives,
                ..
            } = &mut changed
            {
                match variant {
                    "map" => *map = "../missions/test".into(),
                    "map-control" => *map = "missions/test\u{001b}".into(),
                    "count" => players.clear(),
                    "owner" => players[0].ota_slot = 0,
                    "duplicate" => players[1].ota_slot = 1,
                    "side" => players[0].side = "bad\nside".into(),
                    "objectives" => {
                        players.swap(0, 1);
                        *objectives = true;
                    }
                    _ => unreachable!(),
                }
            }
            assert!(
                matches!(
                    ReplaySession::new(changed, None),
                    Err(CheckpointError::Invalid(_))
                ),
                "{variant}"
            );
        }
        let scratch = campaign_install("campaign-budget", authored_units(), "");
        let mut running = ReplaySession::new(source, Some(&scratch.0)).unwrap();
        running.advance(&CommandLog::new(), 2).unwrap();
        let checkpoint = running.checkpoint().unwrap();
        assert!(matches!(
            checkpoint.restore(
                None,
                ReplayBudget {
                    max_ticks: 1,
                    max_commands: 0
                }
            ),
            Err(CheckpointError::Limit(_))
        ));
        let mut value = serde_json::to_value(&checkpoint).unwrap();
        value["payload"]["source"]["players"][0]["surprise"] = true.into();
        assert!(matches!(
            Checkpoint::from_json(&value.to_string()),
            Err(CheckpointError::Json(_))
        ));
        value["payload"]["source"]["players"][0]
            .as_object_mut()
            .unwrap()
            .remove("surprise");
        value["payload"]["source"]
            .as_object_mut()
            .unwrap()
            .remove("objectives");
        assert!(matches!(
            Checkpoint::from_json(&value.to_string()),
            Err(CheckpointError::Json(_))
        ));
    }

    #[test]
    fn campaign_strict_objectives_reuse_cli_rejection_and_initial_order_warnings() {
        let players = crate::campaign::parse_players("1:TST,2:TST").unwrap();
        for (label, condition) in [
            ("unsupported", "useonlyunits=monarch;"),
            ("unresolved", "killalloftype=NOTLOADED;"),
            ("malformed", "victorytimerrunsout=not-a-number;"),
        ] {
            let scratch = campaign_install(label, authored_units(), condition);
            let source = CheckpointSource::campaign("missions/test", &players, 1, false, true);
            assert!(
                ReplaySession::new(source, Some(&scratch.0)).is_err(),
                "{label}"
            );
            let source = CheckpointSource::campaign("missions/test", &players, 1, false, false);
            let session = ReplaySession::new(source, Some(&scratch.0)).unwrap();
            let report = session.source_report().unwrap();
            assert!(report.contains("initialmission"));
            assert!(report.contains("mission AI, alliances and neutrality are not implemented"));
        }
    }

    #[test]
    fn campaign_staged_preflight_rejects_oversize_before_parsing_or_script_loading() {
        let scratch = Scratch::new("campaign-staged-preflight");
        scratch.write("missions/test.tnt", b"invalid small TNT");
        scratch.write("units/huge.fbi", &[]);
        File::options()
            .write(true)
            .open(scratch.0.join("units/huge.fbi"))
            .unwrap()
            .set_len(MAX_CONTENT_FILE_BYTES + 1)
            .unwrap();
        let mut vfs = Vfs::new();
        vfs.mount_dir(&scratch.0).unwrap();
        let players = crate::campaign::parse_players("1:TST,2:TST").unwrap();
        assert!(matches!(
            load_campaign_source(&vfs, "missions/test", &players, 1, false, false),
            Err(CheckpointError::Limit(_))
        ));
        std::fs::remove_file(scratch.0.join("units/huge.fbi")).unwrap();
        let ota = format!(
            "[GLOBALHEADER]\n{{\n[MAP DATA]\n{{\n[UNITS]\n{{\n{}\n}}\n}}\n}}\n",
            authored_units()
        );
        scratch.write("missions/test.tnt", &tiny_map_bytes());
        scratch.write("missions/test.ota", ota.as_bytes());
        scratch.write(
            "units/monarch.fbi",
            b"[UNITINFO]\n{\nunitname=monarch;\nobjectname=monarch;\n}\n",
        );
        scratch.write(
            "gamedata/sidedata.tdf",
            b"[SIDE0]\n{\nnameprefix=TST;\ncommander=monarch;\n}\n",
        );
        for path in ["scripts/monarch.cob", "objects3d/monarch.3do"] {
            scratch.write(path, &[]);
            File::options()
                .write(true)
                .open(scratch.0.join(path))
                .unwrap()
                .set_len(MAX_CONTENT_FILE_BYTES + 1)
                .unwrap();
        }
        let mut vfs = Vfs::new();
        vfs.mount_dir(&scratch.0).unwrap();
        assert!(matches!(
            load_campaign_source(&vfs, "missions/test", &players, 1, true, false),
            Err(CheckpointError::Limit(_))
        ));
        let (mut sim, _, _) =
            load_campaign_source(&vfs, "missions/test", &players, 1, false, false).unwrap();
        assert!(sim.cob().is_none());
        sim.run(&CommandLog::new(), 1);
        assert_eq!(sim.world().units().count(), 2);
    }

    #[test]
    fn atomic_text_replacement_does_not_truncate_other_hardlinks() {
        let scratch = Scratch::new("atomic-hardlink");
        scratch.write("input.json", b"original checkpoint bytes");
        let alias = scratch.0.join("alias.txt");
        std::fs::hard_link(scratch.0.join("input.json"), &alias).unwrap();
        write_text_atomic(&alias, "new hash bytes").unwrap();
        assert_eq!(
            std::fs::read(scratch.0.join("input.json")).unwrap(),
            b"original checkpoint bytes"
        );
        assert_eq!(std::fs::read(alias).unwrap(), b"new hash bytes");
    }

    #[test]
    fn map_source_refuses_missing_install_invalid_slots_and_traversal() {
        assert!(
            ReplaySession::new(CheckpointSource::map("kmap/test", "ARA,ZON", 1, true), None)
                .is_err()
        );
        for source in [
            CheckpointSource::map("../test", "ARA", 1, true),
            CheckpointSource::map("", "ARA", 1, true),
            CheckpointSource::map("kmap/test", "", 1, true),
        ] {
            assert!(source.validate().is_err());
        }
    }
}
