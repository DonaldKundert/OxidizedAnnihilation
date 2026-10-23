//! CLI surface for replay checkpoints, isolated from ordinary run/map sources.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use tak_headless::checkpoint::{
    Checkpoint, CheckpointSource, ReplayBudget, ReplaySession, read_command_log, write_text_atomic,
};
use tak_headless::{Error, MAX_TICKS};

#[derive(Args)]
pub(crate) struct CheckpointArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Start {
    /// Synthetic scenario JSON (saved inline, without scripts).
    #[arg(long, required_unless_present = "map", conflicts_with = "map")]
    scenario: Option<PathBuf>,
    /// Real-map VFS stem; retail content stays in the external install.
    #[arg(long, required_unless_present = "scenario")]
    map: Option<String>,
    /// External Kingdoms install (for --map only).
    #[arg(long, env = "TAK_DATA_DIR")]
    data: Option<PathBuf>,
    /// Sides for a real map, comma separated, in player-slot order.
    #[arg(long, default_value = "ARA,ZON")]
    players: String,
    /// Initial real-map world RNG seed.
    #[arg(long, default_value_t = 1)]
    world_seed: u64,
    /// Disable COB scripts for a real-map match.
    #[arg(long)]
    no_scripts: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Run from the beginning and atomically save the reached state as replay data.
    Save {
        #[command(flatten)]
        start: Start,
        /// Commands to execute; orders at or after --ticks are not saved.
        #[arg(long)]
        log: Option<PathBuf>,
        /// Ticks to execute from the initial setup (zero is valid).
        #[arg(long, value_parser = clap::value_parser!(u32).range(0..=MAX_TICKS as i64))]
        ticks: u32,
        /// Checkpoint destination, atomically replaced only after a complete write.
        #[arg(long)]
        out: PathBuf,
    },
    /// Verify/replay a checkpoint, then optionally execute new commands and save.
    Resume {
        /// Existing replay checkpoint.
        #[arg(long)]
        checkpoint: PathBuf,
        /// External install; required only for a real-map checkpoint.
        #[arg(long, env = "TAK_DATA_DIR")]
        data: Option<PathBuf>,
        /// Continuation log using absolute original-match ticks.
        #[arg(long)]
        log: Option<PathBuf>,
        /// Additional ticks after restoration; zero only verifies/restores.
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u32).range(0..=MAX_TICKS as i64))]
        ticks: u32,
        /// Maximum saved elapsed ticks allowed to replay (default: one game hour).
        #[arg(long, default_value_t = 108_000, value_parser = clap::value_parser!(u32).range(0..=MAX_TICKS as i64))]
        max_replay_ticks: u32,
        /// Maximum saved commands allowed to replay.
        #[arg(long, default_value_t = 250_000)]
        max_replay_commands: usize,
        /// Optional updated checkpoint (may be the original checkpoint path).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Optional continuation hashes with absolute tick labels (not a full
        /// standalone `verify` hash file when resuming after tick zero).
        #[arg(long)]
        hashes: Option<PathBuf>,
    },
    /// Inspect metadata and validate checksum, without loading or replaying content.
    Info {
        /// Existing replay checkpoint.
        #[arg(long)]
        checkpoint: PathBuf,
    },
}

impl CheckpointArgs {
    pub(crate) fn run(self) -> Result<ExitCode, Error> {
        match self.command {
            Command::Save {
                start,
                log,
                ticks,
                out,
            } => {
                let source = match (&start.scenario, &start.map) {
                    (Some(path), None) => CheckpointSource::from_scenario_file(path)?,
                    (None, Some(map)) => CheckpointSource::map(
                        map,
                        &start.players,
                        start.world_seed,
                        !start.no_scripts,
                    ),
                    _ => return Err(Error::Setup("pass --scenario or --map".into())),
                };
                let log = read_command_log(log.as_deref())?;
                let mut session = ReplaySession::new(source, start.data.as_deref())?;
                session.advance(&log, ticks)?;
                let saved = session.checkpoint()?;
                saved.write_atomic(&out)?;
                println!(
                    "saved: next tick {}, {} commands, state {}",
                    saved.next_tick().0,
                    saved.command_count(),
                    saved.state_hash()
                );
                eprintln!("{}", tak_headless::outcome(session.sim()));
            }
            Command::Resume {
                checkpoint,
                data,
                log,
                ticks,
                max_replay_ticks,
                max_replay_commands,
                out,
                hashes,
            } => {
                if hashes.as_ref().is_some_and(|p| {
                    same_path(p, &checkpoint) || out.as_ref().is_some_and(|out| same_path(p, out))
                }) {
                    return Err(Error::Setup(
                        "hash output must differ from checkpoint input/output".into(),
                    ));
                }
                let saved = Checkpoint::read(&checkpoint)?;
                let mut session = saved.restore(
                    data.as_deref(),
                    ReplayBudget {
                        max_ticks: max_replay_ticks,
                        max_commands: max_replay_commands,
                    },
                )?;
                let first_tick = session.sim().tick().0;
                let log = read_command_log(log.as_deref())?;
                let continuation = session.advance(&log, ticks)?;
                let updated = session.checkpoint()?;
                // All validation and replay complete before touching output paths.
                if let Some(path) = hashes {
                    let mut text = String::from("# tak-headless continuation hashes v1\n");
                    for (offset, hash) in continuation.iter().enumerate() {
                        text.push_str(&format!("{} {hash:016x}\n", first_tick + offset as u32));
                    }
                    write_text_atomic(&path, &text)?;
                }
                if let Some(path) = out {
                    updated.write_atomic(&path)?;
                }
                println!(
                    "resumed: verified {} ticks, next tick {}, {} commands, state {}",
                    first_tick,
                    updated.next_tick().0,
                    updated.command_count(),
                    updated.state_hash()
                );
                eprintln!("{}", tak_headless::outcome(session.sim()));
            }
            Command::Info { checkpoint } => {
                let saved = Checkpoint::read(&checkpoint)?;
                let kind = match saved.source() {
                    CheckpointSource::Synthetic { .. } => "synthetic",
                    CheckpointSource::Map { .. } => "map (external install required)",
                };
                println!(
                    "checkpoint: {kind}\nnext tick: {}\nexecuted commands: {}\nstate: {}\ncontent: {}\nchecksum: valid (not authenticated)\nreplay: not checked",
                    saved.next_tick().0,
                    saved.command_count(),
                    saved.state_hash(),
                    saved.content_identity()
                );
            }
        }
        Ok(ExitCode::SUCCESS)
    }
}

// Detect existing aliases and compare canonical parents for not-yet-created
// outputs. Atomic replacement is a further safeguard against alias races.
fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(unix)]
    if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
        use std::os::unix::fs::MetadataExt;
        if a.dev() == b.dev() && a.ino() == b.ino() {
            return true;
        }
    }
    match (resolved_output(a), resolved_output(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn resolved_output(path: &std::path::Path) -> Option<PathBuf> {
    if let Ok(path) = std::fs::canonicalize(path) {
        return Some(path);
    }
    let name = path.file_name()?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    Some(std::fs::canonicalize(parent).ok()?.join(name))
}
