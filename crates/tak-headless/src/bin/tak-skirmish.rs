//! Dedicated opt-in training controller runner. Ordinary tak-headless defaults
//! and source constructors remain replay-only and unchanged.
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use tak_headless::bot::{ControllerConfig, demo};
use tak_headless::checkpoint::{
    Checkpoint, CheckpointSource, ReplayBudget, ReplaySession, read_command_log, write_text_atomic,
};
use tak_headless::{Error, MAX_TICKS};

#[derive(Parser)]
#[command(
    name = "tak-skirmish",
    about = "Explicit simple-skirmish-v1 omniscient training opponent (not retail AI)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Output {
    /// Additional ticks (absolute cadence remains anchored to original tick 1).
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=MAX_TICKS as i64))]
    ticks: u32,
    /// Optional future HUMAN commands; controlled-owner future input is rejected.
    #[arg(long)]
    log: Option<PathBuf>,
    /// Write complete realized command history (including controller decisions).
    #[arg(long)]
    record: Option<PathBuf>,
    /// Write per-tick hashes, labelled with absolute original ticks.
    #[arg(long)]
    hashes: Option<PathBuf>,
    /// Atomically save controller identity and realized history.
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the hand-authored income -> production -> attack acceptance demo.
    Demo {
        /// Construct troops directly with a mobile builder, not a factory.
        #[arg(long)]
        direct_builder: bool,
        #[command(flatten)]
        output: Output,
    },
    /// Run explicit recipes against a synthetic scenario or external owned map.
    Run {
        #[arg(long, required_unless_present = "map", conflicts_with = "map")]
        scenario: Option<PathBuf>,
        #[arg(long, required_unless_present = "scenario")]
        map: Option<String>,
        #[arg(long, env = "TAK_DATA_DIR")]
        data: Option<PathBuf>,
        #[arg(long, default_value = "ARA,ZON")]
        players: String,
        #[arg(long, default_value_t = 1)]
        world_seed: u64,
        #[arg(long)]
        no_scripts: bool,
        /// Explicit ControllerConfig JSON, at most 64 KiB.
        #[arg(long)]
        config: PathBuf,
        #[command(flatten)]
        output: Output,
    },
    /// Replay historical commands only, verify state, then resume FUTURE planning.
    Resume {
        #[arg(long)]
        checkpoint: PathBuf,
        #[arg(long, env = "TAK_DATA_DIR")]
        data: Option<PathBuf>,
        #[command(flatten)]
        output: Output,
    },
}

fn io(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.display().to_string(),
        source,
    }
}
fn config(path: &Path) -> Result<ControllerConfig, Error> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| io(path, error))?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|error| io(path, error))?;
    if bytes.len() > 65_536 {
        return Err(Error::Setup("controller config exceeds 64 KiB".into()));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| Error::Setup(format!("controller config: {error}")))
}
fn paths_distinct(inputs: &[&Path], output: &Output) -> Result<(), Error> {
    let outputs: Vec<_> = [
        output.record.as_deref(),
        output.hashes.as_deref(),
        output.out.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let normalize = |path: &Path| -> Result<PathBuf, Error> {
        // Resolve parent aliases even for an output that does not exist yet.
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|error| io(path, error))?
                .join(path)
        };
        let parent = absolute
            .parent()
            .ok_or_else(|| Error::Setup("output has no parent".into()))?;
        let parent = parent.canonicalize().map_err(|error| io(parent, error))?;
        Ok(parent.join(
            absolute
                .file_name()
                .ok_or_else(|| Error::Setup("output needs a file name".into()))?,
        ))
    };
    for (index, path) in outputs.iter().enumerate() {
        let normalized = normalize(path)?;
        for other in outputs
            .iter()
            .take(index)
            .copied()
            .chain(inputs.iter().copied())
        {
            let other_normalized = normalize(other)?;
            // Conservatively reject case-only variants even on case-sensitive
            // hosts, so new output pairs remain portable to typical macOS and
            // Windows volumes. Existing aliases also get inode checks below.
            let same = normalized == other_normalized
                || normalized.to_string_lossy().to_lowercase()
                    == other_normalized.to_string_lossy().to_lowercase();
            #[cfg(unix)]
            let same = {
                use std::os::unix::fs::MetadataExt;
                same || match (path.metadata(), other.metadata()) {
                    (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
                    _ => false,
                }
            };
            if same {
                return Err(Error::Setup(
                    "outputs must not alias inputs or each other".into(),
                ));
            }
        }
    }
    Ok(())
}
fn finish(mut session: ReplaySession, output: Output, inputs: &[&Path]) -> Result<(), Error> {
    let mut inputs = inputs.to_vec();
    if let Some(log) = output.log.as_deref() {
        inputs.push(log);
    }
    paths_distinct(&inputs, &output)?;
    let input = read_command_log(output.log.as_deref())?;
    let start = session.sim().tick().0;
    let hashes = session.advance_driven(&input, output.ticks)?;
    if let Some(path) = output.hashes {
        let mut text = String::from("# tak-skirmish absolute hashes v1\n");
        for (offset, hash) in hashes.iter().enumerate() {
            text.push_str(&format!("{} {hash:016x}\n", start + offset as u32));
        }
        write_text_atomic(&path, &text)?;
    }
    if let Some(path) = output.record {
        write_text_atomic(&path, &session.executed_log().to_json_pretty())?;
    }
    if let Some(path) = output.out {
        session.checkpoint()?.write_atomic(&path)?;
    }
    println!(
        "simple-skirmish-v1 omniscient-training-v1: next tick {}, {} realized commands, state {:016x}; {}",
        session.sim().tick().0,
        session.executed_log().len(),
        session.sim().state_hash(),
        tak_headless::outcome(session.sim())
    );
    Ok(())
}
fn run() -> Result<(), Error> {
    match Cli::parse().command {
        Command::Demo {
            direct_builder,
            output,
        } => {
            let source = CheckpointSource::synthetic(demo::scenario(direct_builder));
            let plain = ReplaySession::new(source.clone(), None)?;
            let config = demo::config(plain.sim().world(), direct_builder)
                .map_err(|error| Error::Setup(error.to_string()))?;
            finish(
                ReplaySession::new_with_controllers(source, None, config)?,
                output,
                &[],
            )
        }
        Command::Run {
            scenario,
            map,
            data,
            players,
            world_seed,
            no_scripts,
            config: path,
            output,
        } => {
            let source = if let Some(scenario) = &scenario {
                CheckpointSource::from_scenario_file(scenario)?
            } else {
                CheckpointSource::map(
                    map.as_deref().unwrap_or_default(),
                    &players,
                    world_seed,
                    !no_scripts,
                )
            };
            let controllers = config(&path)?;
            let session =
                ReplaySession::new_with_controllers(source, data.as_deref(), controllers)?;
            let mut inputs = vec![path.as_path()];
            inputs.extend(scenario.as_deref());
            finish(session, output, &inputs)
        }
        Command::Resume {
            checkpoint,
            data,
            output,
        } => {
            let session =
                Checkpoint::read(&checkpoint)?.restore(data.as_deref(), ReplayBudget::default())?;
            finish(session, output, &[&checkpoint])
        }
    }
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tak-skirmish: {error}");
            ExitCode::from(2)
        }
    }
}
