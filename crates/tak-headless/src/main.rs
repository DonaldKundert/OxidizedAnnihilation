use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tak_headless::{
    Error, MAX_TICKS, first_mismatch, format_hashes, load_log, load_scenario, parse_hashes, run,
    write,
};
use tak_sim::testing::{example_scenario, random_log};

/// Headless Oxidized Annihilation sim: runs scenarios and replays, prints
/// per-tick state hashes.
#[derive(Parser)]
#[command(name = "tak-headless", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a scenario and print (or write) the per-tick hashes.
    Run {
        /// Scenario JSON.
        #[arg(long)]
        scenario: PathBuf,
        /// Command log JSON (omit for no commands).
        #[arg(long)]
        log: Option<PathBuf>,
        /// Number of ticks to run.
        #[arg(long, value_parser = clap::value_parser!(u32).range(0..=MAX_TICKS as i64))]
        ticks: u32,
        /// Write hashes here instead of stdout.
        #[arg(long)]
        hashes: Option<PathBuf>,
    },
    /// Re-run a scenario and log and compare against a hash file. Exits 1 on
    /// the first mismatch.
    Verify {
        /// Scenario JSON.
        #[arg(long)]
        scenario: PathBuf,
        /// Command log JSON (omit for no commands).
        #[arg(long)]
        log: Option<PathBuf>,
        /// Hash file to check against (its length sets the tick count).
        #[arg(long)]
        hashes: PathBuf,
    },
    /// Print a synthetic example scenario (flat map, monarchs in corners).
    ExampleScenario {
        /// World RNG seed.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Players, 1 to 4.
        #[arg(long, default_value_t = 2)]
        players: usize,
    },
    /// Write a random fuzz command log for a scenario (for soak tests).
    GenLog {
        /// Scenario JSON.
        #[arg(long)]
        scenario: PathBuf,
        /// Ticks the log should span.
        #[arg(long)]
        ticks: u32,
        /// Generator seed.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Output path (stdout if omitted).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("tak-headless: {e}");
            ExitCode::from(2)
        }
    }
}

fn real_main() -> Result<ExitCode, Error> {
    match Cli::parse().cmd {
        Cmd::Run {
            scenario,
            log,
            ticks,
            hashes,
        } => {
            let sc = load_scenario(&scenario)?;
            let log = load_log(log.as_deref())?;
            let text = format_hashes(&run(&sc, &log, ticks)?);
            match hashes {
                Some(p) => write(&p, &text)?,
                None => print!("{text}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Verify {
            scenario,
            log,
            hashes,
        } => {
            let sc = load_scenario(&scenario)?;
            let log = load_log(log.as_deref())?;
            let text = std::fs::read_to_string(&hashes).map_err(|source| Error::Io {
                path: hashes.display().to_string(),
                source,
            })?;
            let expected = parse_hashes(&text)?;
            let actual = run(&sc, &log, expected.len() as u32)?;
            match first_mismatch(&expected, &actual) {
                None => {
                    let last = actual.last().map(|h| format!(" (final {h:016x})"));
                    println!(
                        "ok: {} ticks match{}",
                        actual.len(),
                        last.unwrap_or_default()
                    );
                    Ok(ExitCode::SUCCESS)
                }
                Some((t, e, a)) => {
                    println!("mismatch at tick {t}: expected {e:016x}, got {a:016x}");
                    Ok(ExitCode::from(1))
                }
            }
        }
        Cmd::ExampleScenario { seed, players } => {
            println!("{}", example_scenario(seed, players).to_json_pretty());
            Ok(ExitCode::SUCCESS)
        }
        Cmd::GenLog {
            scenario,
            ticks,
            seed,
            out,
        } => {
            let sc = load_scenario(&scenario)?;
            let json = random_log(&sc, ticks, seed).to_json_pretty();
            match out {
                Some(p) => write(&p, &json)?,
                None => println!("{json}"),
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
