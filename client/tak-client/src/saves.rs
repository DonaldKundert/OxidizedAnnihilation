//! Playable-client adapter for the canonical headless replay checkpoint format.
//!
//! The live client can mutate its `Sim`, so capture reconstructs the canonical
//! source and executed history off to the side and compares the complete live
//! hash before writing. This is O(elapsed ticks), not an instant snapshot.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tak_commands::{CommandLog, Tick};
use tak_headless::checkpoint::{Checkpoint, CheckpointSource, ReplayBudget, ReplaySession};

use crate::cli::Args;
use crate::game::{Session, SetupError};

pub mod browser;

static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn err(error: impl std::fmt::Display) -> SetupError {
    SetupError(error.to_string())
}

fn replay_budget(args: &Args) -> ReplayBudget {
    ReplayBudget {
        max_ticks: args.max_replay_ticks,
        max_commands: args.max_replay_commands,
    }
}

/// Client-local quick-save shortcut. The pad's West face button is reserved
/// for game saving only while the existing settings overlay owns device input.
/// This does not alter deterministic commands or the user's gameplay bindings.
pub fn quick_save_shortcut(f10: bool, overlay_open: bool, pad_west: bool) -> bool {
    f10 || (overlay_open && pad_west)
}

/// Immutable origin, never regenerated from a mutable client world.
pub(crate) struct SaveOrigin {
    checkpoint: Checkpoint,
    data: PathBuf,
    budget: ReplayBudget,
}

/// Start the live game from the same bounded source loader as checkpoints.
/// Identity is fixed before the caller can mutate the simulation.
pub(crate) fn start(args: &Args, log: CommandLog) -> Result<Session, SetupError> {
    let source = match &args.campaign {
        Some(map) => CheckpointSource::campaign(
            map,
            &tak_headless::campaign::parse_players(
                args.campaign_players
                    .as_deref()
                    .ok_or_else(|| err("missing campaign owners"))?,
            )
            .map_err(err)?,
            args.world_seed,
            true,
            args.campaign_objectives,
        ),
        None => CheckpointSource::map(&args.map, &args.players, args.world_seed, true),
    };
    let controllers = args
        .controller_config
        .as_deref()
        .map(crate::training::read)
        .transpose()?
        .unwrap_or_default();
    let replay =
        ReplaySession::new_with_controllers(source, Some(&args.data), controllers).map_err(err)?;
    tak_headless::checkpoint::driver::admit_driven(
        replay.sim().world(),
        replay.controllers(),
        replay.executed_log(),
        &log,
        0,
    )
    .map_err(err)?;
    let checkpoint = replay.checkpoint().map_err(err)?;
    from_replay(args, checkpoint, replay, log)
}

fn from_replay(
    args: &Args,
    checkpoint: Checkpoint,
    replay: ReplaySession,
    log: CommandLog,
) -> Result<Session, SetupError> {
    let map = match checkpoint.source() {
        CheckpointSource::Map { map, .. } | CheckpointSource::Campaign { map, .. } => map,
        CheckpointSource::Synthetic { .. } => {
            return Err(err("synthetic checkpoints have no renderable map"));
        }
    };
    let first_tick = replay.sim().tick();
    let executed = replay.executed_log().clone();
    // The canonical source's data path is authoritative for both simulation
    // and render metadata; an independent caller VFS must not redirect art/map.
    let vfs = crate::game::open_vfs(&args.data)?;
    let rules = tak_content::Ruleset::load(&vfs, tak_content::RulesetKind::Base);
    let loaded_map = tak_content::MapDef::load(&vfs, &rules, map).map_err(err)?;
    let report = replay.source_report().map(str::to_owned);
    let controllers = replay.controllers().clone();
    crate::training::validate_local(&controllers)?;
    let mut session = Session::from_sim(&vfs, map, loaded_map, replay.into_sim(), log, report)?;
    session.controllers = controllers;
    session.hashes_first_tick = first_tick;
    session.executed_log = executed;
    session.save_origin = Some(SaveOrigin {
        checkpoint,
        data: args.data.clone(),
        budget: replay_budget(args),
    });
    Ok(session)
}

/// Restore all gameplay/script state before exposing a playable session. A
/// synthetic checkpoint has no renderable real map and is explicitly refused.
pub(crate) fn restore(args: &Args, path: &Path, log: CommandLog) -> Result<Session, SetupError> {
    let checkpoint = Checkpoint::read(path).map_err(err)?;
    restore_checkpoint(args, checkpoint, log)
}

/// Restore a checkpoint already read and confirmed by the in-game browser.
/// Keep the verified object instead of reopening a pathname after confirmation.
pub(crate) fn restore_checkpoint(
    args: &Args,
    checkpoint: Checkpoint,
    log: CommandLog,
) -> Result<Session, SetupError> {
    crate::training::validate_local(checkpoint.controllers())?;
    if matches!(checkpoint.source(), CheckpointSource::Synthetic { .. }) {
        return Err(err(
            "synthetic checkpoints have no renderable map; use tak-headless checkpoint resume",
        ));
    }
    let mut replay = checkpoint
        .restore(Some(&args.data), replay_budget(args))
        .map_err(err)?;
    // Validates any supplied past entries without advancing or accepting changed
    // history. Only future commands are appended to the authoritative log below.
    replay.advance(&log, 0).map_err(err)?;
    tak_headless::checkpoint::driver::admit_driven(
        replay.sim().world(),
        replay.controllers(),
        replay.executed_log(),
        &log,
        0,
    )
    .map_err(err)?;
    let mut combined = replay.executed_log().clone();
    let first_tick = replay.sim().tick();
    for entry in log.iter().filter(|entry| entry.tick >= first_tick) {
        combined.insert(entry.clone()).map_err(err)?;
    }
    from_replay(args, checkpoint, replay, combined)
}

/// Replay and compare the complete live hash. Unlogged spawns, edits, changed
/// past history or changed gameplay content fail before a destination is touched.
pub fn capture(session: &Session) -> Result<Checkpoint, SetupError> {
    let origin = session.save_origin.as_ref().ok_or_else(|| err("quick-save unavailable for this source: demo, ghost and session-recording modes are not checkpoint sessions"))?;
    let now = session.tick().0;
    if now > origin.budget.max_ticks {
        return Err(err(format!(
            "quick-save needs {now} replay ticks; raise --max-replay-ticks intentionally"
        )));
    }
    // Count the authoritative restored prefix plus actual continuation before
    // opening content or replaying anything, regardless of public-log edits.
    if session.executed_log.len() > origin.budget.max_commands {
        return Err(err(
            "quick-save command replay budget exceeded; raise --max-replay-commands intentionally",
        ));
    }
    if session.history_error.is_some() || session.executed_through != session.tick() {
        return Err(err(
            "quick-save refused: simulation advanced outside the recorded session tick path",
        ));
    }
    if !session
        .log
        .iter()
        .filter(|entry| entry.tick.0 < now)
        .eq(session.executed_log.iter())
    {
        return Err(err(
            "quick-save refused: executed command history was edited; restore the original history before saving",
        ));
    }
    let mut replay = origin
        .checkpoint
        .restore(Some(&origin.data), origin.budget)
        .map_err(err)?;
    let ticks = now
        .checked_sub(replay.sim().tick().0)
        .ok_or_else(|| err("live session precedes its verified origin"))?;
    // Capture is historical replay only: retained bot decisions are ordinary
    // input, never regenerated from current planning/configuration.
    replay.advance(&session.executed_log, ticks).map_err(err)?;
    let actual = replay.sim().state_hash();
    let live = session.sim.state_hash();
    if actual != live {
        return Err(err(format!(
            "quick-save refused: logged replay hash {actual:016x} differs from live state {live:016x}; unlogged mutations cannot be saved"
        )));
    }
    let checkpoint = replay.checkpoint().map_err(err)?;
    // Defense in depth: never publish a file beyond the same restore budget.
    if checkpoint.command_count() > origin.budget.max_commands {
        return Err(err(
            "quick-save command replay budget exceeded; raise --max-replay-commands intentionally",
        ));
    }
    Ok(checkpoint)
}

/// Full hashes for fresh matches, absolute-labeled continuation hashes after load.
pub fn format_hashes(session: &Session) -> String {
    if session.hashes_first_tick == Tick(0) {
        return tak_headless::format_hashes(&session.hashes);
    }
    let mut text = String::from("# tak-headless continuation hashes v1\n");
    for (offset, hash) in session.hashes.iter().enumerate() {
        text.push_str(&format!(
            "{} {hash:016x}\n",
            u64::from(session.hashes_first_tick.0) + offset as u64
        ));
    }
    text
}

/// Per-user location only. No current-working-directory fallback.
pub fn default_save_dir() -> Result<PathBuf, SetupError> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    let base = base
        .filter(|p| p.is_absolute())
        .ok_or_else(|| err("no absolute per-user game data directory; configure --save-dir"))?;
    Ok(base.join("oxidized-annihilation/saves"))
}

/// Resolve before creating directories. Refuse writes to the install or any Git
/// worktree, including through symlinks. New components may not traverse parents.
pub fn save_directory(configured: Option<&Path>, data: &Path) -> Result<PathBuf, SetupError> {
    let path = match configured {
        Some(path) => path.to_path_buf(),
        None => default_save_dir()?,
    };
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().map_err(err)?.join(path)
    };
    let mut existing = absolute.as_path();
    let mut suffix = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| err("save directory has no existing ancestor"))?;
        suffix.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| err("save directory has no parent"))?;
    }
    let mut resolved = existing.canonicalize().map_err(err)?;
    for part in suffix.iter().rev() {
        let p = Path::new(part);
        if !matches!(p.components().next(), Some(Component::Normal(_))) {
            return Err(err("save directory contains unsupported path traversal"));
        }
        resolved.push(part);
    }
    let install = data.canonicalize().map_err(err)?;
    if resolved.starts_with(&install)
        || resolved
            .ancestors()
            .any(|p| p.join(".git").is_file() || p.join(".git/HEAD").is_file())
    {
        return Err(err(
            "save directory must be outside the owned install and Git repositories",
        ));
    }
    Ok(resolved)
}

/// Publish a complete verified save without replacing any existing pathname.
/// The canonical writer syncs a staging file, then a same-filesystem hard link
/// commits it atomically with no-clobber semantics. Unsupported filesystems fail
/// closed rather than falling back to overwrite/truncation.
pub fn quick_save(session: &Session, directory: &Path) -> Result<PathBuf, SetupError> {
    let checkpoint = capture(session)?;
    let origin = session
        .save_origin
        .as_ref()
        .ok_or_else(|| err("missing save origin"))?;
    let directory = save_directory(Some(directory), &origin.data)?;
    std::fs::create_dir_all(&directory).map_err(err)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(err)?
        .as_nanos();
    let seq = SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let stem = format!(
        "quick-tick-{}-{nonce}-{}-{seq}",
        session.tick().0,
        std::process::id()
    );
    publish_new(&checkpoint, &directory, &stem)
}

fn publish_new(
    checkpoint: &Checkpoint,
    directory: &Path,
    stem: &str,
) -> Result<PathBuf, SetupError> {
    let staging = directory.join(format!(".{stem}.pending"));
    std::fs::create_dir(&staging).map_err(err)?;
    let temporary = staging.join("checkpoint.json");
    let destination = directory.join(format!("{stem}.save.json"));
    let result = (|| {
        checkpoint.write_atomic(&temporary).map_err(err)?;
        std::fs::hard_link(&temporary, &destination).map_err(err)?;
        if let Ok(dir) = std::fs::File::open(directory) {
            let _ = dir.sync_all();
        }
        Ok(destination)
    })();
    let _ = std::fs::remove_file(&temporary);
    let _ = std::fs::remove_dir(&staging);
    result
}

/// Prevent exit/profile/screenshot outputs from overwriting a loaded save or
/// explicit controller config, including symlink and Unix device/inode aliases.
pub fn protect_checkpoint_outputs(args: &Args) -> Result<(), SetupError> {
    let inputs: Vec<_> = [
        ("--load-checkpoint", args.load_checkpoint.as_ref()),
        ("--controller-config", args.controller_config.as_ref()),
    ]
    .into_iter()
    .filter_map(|(flag, path)| path.map(|path| (flag, path)))
    .collect();
    if inputs.is_empty() {
        return Ok(());
    }
    let mut outputs: Vec<PathBuf> = [&args.record, &args.hashes, &args.record_session]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    outputs.push(
        args.input_profile
            .clone()
            .unwrap_or_else(crate::feel::default_profile_path),
    );
    if let Some(pattern) = &args.screenshot {
        for tick in &args.at_tick {
            let output = PathBuf::from(pattern.replace("{tick}", &tick.to_string()));
            check_screenshot_destination(args.load_checkpoint.is_some(), &output)?;
            outputs.push(output);
        }
    }
    for output in outputs {
        for (flag, input) in &inputs {
            if same_file(input, &output) {
                return Err(err(format!(
                    "output {} aliases {flag} {}; choose a different output",
                    output.display(),
                    input.display()
                )));
            }
        }
    }
    Ok(())
}

/// Portable fail-closed protection for loaded-session screenshots. Rechecked at
/// capture dispatch; this is not atomic against hostile concurrent path changes.
/// Fresh screenshots and non-loaded legacy overwrite behavior remain supported.
pub fn check_screenshot_destination(loaded: bool, path: &Path) -> Result<(), SetupError> {
    if !loaded {
        return Ok(());
    }
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(err(format!(
            "loaded-checkpoint screenshots require a fresh output path; {} already exists (choose a new --screenshot path)",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(err(format!(
            "cannot verify fresh screenshot destination {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn same_file(a: &Path, b: &Path) -> bool {
    if let (Ok(a), Ok(b)) = (a.canonicalize(), b.canonicalize())
        && a == b
    {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return a.dev() == b.dev() && a.ino() == b.ino();
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_publication_never_replaces_an_existing_file_and_cleans_staging() {
        let root = std::env::temp_dir().join(format!(
            "tak-client-publish-{}-{}",
            std::process::id(),
            SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let checkpoint = ReplaySession::new(
            CheckpointSource::synthetic(tak_sim::testing::example_scenario(42, 2)),
            None,
        )
        .unwrap()
        .checkpoint()
        .unwrap();
        let destination = root.join("existing.save.json");
        std::fs::write(&destination, b"unrelated original bytes").unwrap();
        assert!(publish_new(&checkpoint, &root, "existing").is_err());
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"unrelated original bytes"
        );
        assert!(!root.join(".existing.pending").exists());
        let published = publish_new(&checkpoint, &root, "new").unwrap();
        assert_eq!(
            Checkpoint::read(&published).unwrap().state_hash(),
            checkpoint.state_hash()
        );
        assert!(!root.join(".new.pending").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
#[path = "saves/training_tests.rs"]
mod training_tests;
