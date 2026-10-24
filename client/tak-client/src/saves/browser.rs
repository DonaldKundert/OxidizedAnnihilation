//! Renderer-independent checkpoint browser and bounded filesystem operations.
//!
//! These are replay checkpoints, not instantaneous/full presentation savestates.
//! Browsing checks wire integrity; load rechecks the selected file and verifies
//! installed gameplay content plus the complete reconstructed simulation hash.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tak_commands::CommandLog;
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, MAX_CHECKPOINT_BYTES};

use super::{err, save_directory};
use crate::cli::Args;
use crate::game::{Session, SetupError};

/// Bound directory scans and total checkpoint parsing work per refresh.
pub const MAX_BROWSER_ENTRIES: usize = 128;
const MAX_SCAN_ENTRIES: usize = 4096;
const MAX_CATALOG_BYTES: u64 = 64 * 1024 * 1024;
/// Rows per page in the keyboard, mouse and controller browser.
pub const PAGE_ROWS: usize = 5;

/// Metadata read from a validated checkpoint, without replaying it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub map: String,
    pub factions: String,
    pub next_tick: u32,
    pub commands: usize,
    pub content_identity: String,
    pub state_hash: String,
    pub campaign: bool,
    pub playable: bool,
    /// In-process change detector of the entire canonical checkpoint, not a
    /// security signature. The canonical format separately checks its checksum.
    fingerprint: u64,
}

impl Metadata {
    fn from_checkpoint(checkpoint: &Checkpoint) -> Result<Self, SetupError> {
        let (map, factions, campaign, playable) = match checkpoint.source() {
            CheckpointSource::Map { map, players, .. } => {
                (map.clone(), players.join(" / "), false, true)
            }
            CheckpointSource::Campaign { map, players, .. } => (
                map.clone(),
                players
                    .iter()
                    .map(|p| format!("{}:{}", p.ota_slot, p.side))
                    .collect::<Vec<_>>()
                    .join(" / "),
                true,
                true,
            ),
            CheckpointSource::Synthetic { .. } => (
                "Synthetic (headless only)".into(),
                "Synthetic".into(),
                false,
                false,
            ),
        };
        let mut hasher = DefaultHasher::new();
        checkpoint.to_json_pretty().map_err(err)?.hash(&mut hasher);
        Ok(Self {
            map,
            factions,
            next_tick: checkpoint.next_tick().0,
            commands: checkpoint.command_count(),
            content_identity: checkpoint.content_identity().into(),
            state_hash: checkpoint.state_hash().into(),
            campaign,
            playable,
            fingerprint: hasher.finish(),
        })
    }

    /// Match time is exact 30 Hz simulation time, independent of speed/pause.
    pub fn match_time(&self) -> String {
        let seconds = self.next_tick / 30;
        format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub modified: Option<SystemTime>,
    /// Broken/incompatible files stay visible with an actionable error.
    pub metadata: Result<Metadata, String>,
}

impl Entry {
    pub fn saved_time(&self) -> String {
        match self.modified.and_then(|time| time.elapsed().ok()) {
            Some(age) if age.as_secs() < 60 => "saved just now".into(),
            Some(age) if age.as_secs() < 3600 => format!("saved {}m ago", age.as_secs() / 60),
            Some(age) if age.as_secs() < 86400 => format!("saved {}h ago", age.as_secs() / 3600),
            Some(age) => format!("saved {}d ago", age.as_secs() / 86400),
            None => "saved time unavailable".into(),
        }
    }
}

#[derive(Default, Debug)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

/// Never create a directory while merely opening the browser. Symlinks and
/// special files are ignored, and malformed checkpoint files remain visible.
pub fn catalog(directory: &Path, data: &Path) -> Result<Catalog, SetupError> {
    let directory = save_directory(Some(directory), data)?;
    let read = match std::fs::read_dir(&directory) {
        Ok(read) => read,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Catalog::default()),
        Err(error) => return Err(err(format!("cannot list {}: {error}", directory.display()))),
    };
    let mut candidates = Vec::new();
    let mut truncated = false;
    for (index, entry) in read.enumerate() {
        if index >= MAX_SCAN_ENTRIES {
            truncated = true;
            break;
        }
        let entry = entry.map_err(err)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.ends_with(".save.json") || !entry.file_type().map_err(err)?.is_file() {
            continue;
        }
        let metadata = entry.metadata().map_err(err)?;
        candidates.push((entry.path(), name, metadata.modified().ok(), metadata.len()));
    }
    candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
    truncated |= candidates.len() > MAX_BROWSER_ENTRIES;
    candidates.truncate(MAX_BROWSER_ENTRIES);
    let mut bytes = 0;
    let entries = candidates
        .into_iter()
        .map(|(path, name, modified, size)| {
            let metadata = if size > MAX_CHECKPOINT_BYTES as u64 {
                Err("Checkpoint exceeds the 32 MiB file limit".into())
            } else if bytes + size > MAX_CATALOG_BYTES {
                truncated = true;
                Err("Refresh parse budget exhausted; move older saves out of this folder".into())
            } else {
                bytes += size;
                Checkpoint::read(&path)
                    .map_err(err)
                    .and_then(|checkpoint| Metadata::from_checkpoint(&checkpoint))
                    .map_err(|error| error.to_string())
            };
            Entry {
                path,
                name,
                modified,
                metadata,
            }
        })
        .collect();
    Ok(Catalog { entries, truncated })
}

/// Future commands cannot be represented by the canonical checkpoint format.
/// Browser saving refuses them rather than silently discarding queued orders.
pub fn pending_orders(session: &Session) -> usize {
    session
        .log
        .iter()
        .filter(|entry| entry.tick >= session.tick())
        .count()
}

pub fn ensure_no_pending_orders(session: &Session) -> Result<(), SetupError> {
    let pending = pending_orders(session);
    if pending != 0 {
        return Err(err(format!(
            "{pending} queued/future orders cannot be checkpointed. Close the browser, resume until they execute, then save again. F10 still saves executed history only."
        )));
    }
    Ok(())
}

fn checked_entry(directory: &Path, data: &Path, entry: &Entry) -> Result<Checkpoint, SetupError> {
    let directory = save_directory(Some(directory), data)?;
    if entry.path.parent() != Some(directory.as_path())
        || !entry.name.ends_with(".save.json")
        || entry.path.file_name().and_then(|name| name.to_str()) != Some(entry.name.as_str())
    {
        return Err(err(
            "checkpoint selection is not a direct save-directory entry; refresh the browser",
        ));
    }
    let file = std::fs::symlink_metadata(&entry.path).map_err(err)?;
    if !file.file_type().is_file() {
        return Err(err(
            "checkpoint selection is no longer a regular file; refresh the browser",
        ));
    }
    let expected = entry.metadata.as_ref().map_err(err)?;
    let checkpoint = Checkpoint::read(&entry.path).map_err(err)?;
    if Metadata::from_checkpoint(&checkpoint)? != *expected {
        return Err(err(
            "checkpoint changed since it was listed; refresh and confirm it again",
        ));
    }
    Ok(checkpoint)
}

pub fn save_new(session: &Session, directory: &Path) -> Result<PathBuf, SetupError> {
    ensure_no_pending_orders(session)?;
    super::quick_save(session, directory)
}

/// Only call after the browser's explicit overwrite confirmation. Capture and
/// destination revalidation complete before the atomic writer touches the old
/// pathname. As elsewhere, this is not protection against hostile path races.
pub fn overwrite(
    session: &Session,
    directory: &Path,
    entry: &Entry,
) -> Result<PathBuf, SetupError> {
    ensure_no_pending_orders(session)?;
    let origin = session
        .save_origin
        .as_ref()
        .ok_or_else(|| err("this session cannot be checkpointed"))?;
    checked_entry(directory, &origin.data, entry)?;
    let checkpoint = super::capture(session)?;
    // A long verification replay must not mask an intervening file edit.
    checked_entry(directory, &origin.data, entry)?;
    checkpoint.write_atomic(&entry.path).map_err(err)?;
    Ok(entry.path.clone())
}

/// Prepare a load off to the side. No current session, queued order, or renderer
/// is modified. The caller still validates presentation assets before swapping.
pub fn prepare_load(args: &Args, directory: &Path, entry: &Entry) -> Result<Session, SetupError> {
    if args.demo
        || args.ghost.is_some()
        || args.record_session.is_some()
        || args.synth_events.is_some()
        || args.screenshot.is_some()
    {
        return Err(err(
            "In-game loading is unavailable in demo, ghost, session-recording and screenshot modes. Start a normal match or use --load-checkpoint.",
        ));
    }
    let checkpoint = checked_entry(directory, &args.data, entry)?;
    let mut protected = args.clone();
    protected.load_checkpoint = Some(entry.path.clone());
    // Preserve controller-config as a protected input even though the saved
    // controllers, not that startup file, drive the restored session.
    super::protect_checkpoint_outputs(&protected)?;
    super::restore_checkpoint(args, checkpoint, CommandLog::new())
}

#[derive(Clone, Debug)]
pub enum Operation {
    SaveNew,
    Overwrite(Entry),
    Load(Entry),
}

#[derive(Clone, Debug)]
pub struct Confirmation {
    pub operation: Operation,
    /// Destructive confirmations always start with Cancel focused.
    pub accept: bool,
}

#[derive(Default, Debug)]
pub struct Browser {
    pub catalog: Catalog,
    pub selected: usize,
    pub confirmation: Option<Confirmation>,
    pub message: String,
}

impl Browser {
    pub fn refresh(&mut self, directory: &Path, data: &Path) -> Result<(), SetupError> {
        let selected = self.selected().map(|entry| entry.path.clone());
        self.catalog = catalog(directory, data)?;
        self.selected = selected
            .and_then(|path| self.catalog.entries.iter().position(|e| e.path == path))
            .unwrap_or(self.selected)
            .min(self.catalog.entries.len().saturating_sub(1));
        self.confirmation = None;
        Ok(())
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.catalog.entries.get(self.selected)
    }
    pub fn first_visible(&self) -> usize {
        self.selected / PAGE_ROWS * PAGE_ROWS
    }

    pub fn navigate(&mut self, delta: isize) {
        if self.confirmation.is_some() {
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.catalog.entries.len().saturating_sub(1));
    }

    pub fn request_selected(&mut self, overwrite: bool) -> Result<(), SetupError> {
        let entry = self
            .selected()
            .ok_or_else(|| err("No checkpoint selected"))?;
        let metadata = entry.metadata.as_ref().map_err(err)?;
        if !overwrite && !metadata.playable {
            return Err(err(
                "Synthetic checkpoints are headless only; use tak-headless checkpoint resume",
            ));
        }
        self.confirmation = Some(Confirmation {
            operation: if overwrite {
                Operation::Overwrite(entry.clone())
            } else {
                Operation::Load(entry.clone())
            },
            accept: false,
        });
        Ok(())
    }

    /// Cancel/decline cannot return an operation; selection was captured when
    /// asking, so later list movement cannot redirect the confirmed operation.
    pub fn resolve_confirmation(&mut self, accept: bool) -> Option<Operation> {
        self.confirmation
            .take()
            .and_then(|confirmation| accept.then_some(confirmation.operation))
    }
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
