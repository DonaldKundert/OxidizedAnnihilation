use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use tak_commands::CommandLog;
use tak_headless::{
    Error, MAX_TICKS, first_mismatch, format_hashes, load_log, load_map, load_scenario,
    parse_hashes, run, run_match, write,
};
use tak_sim::Match;
use tak_sim::testing::{example_scenario, random_log, walk_log};

/// Headless Oxidized Annihilation sim: runs scenarios and replays, prints
/// per-tick state hashes.
#[derive(Parser)]
#[command(name = "tak-headless", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// Where the match comes from: a synthetic scenario JSON (`--scenario`), or
/// a real map from an install (`--map`, with `--data` or `$TAK_DATA_DIR`).
#[derive(Args)]
struct Source {
    /// Scenario JSON (synthetic match, no scripts).
    #[arg(long, conflicts_with = "map")]
    scenario: Option<PathBuf>,
    /// Real map: VFS path without extension, e.g. "kmap/Adamantine Gate".
    #[arg(long)]
    map: Option<String>,
    /// Kingdoms install directory (for --map).
    #[arg(long, env = "TAK_DATA_DIR")]
    data: Option<PathBuf>,
    /// Sides for --map, comma-separated, by prefix or name (one monarch each).
    #[arg(long, default_value = "ARA,ZON")]
    players: String,
    /// World RNG seed for --map.
    #[arg(long, default_value_t = 1)]
    world_seed: u64,
    /// Run a --map match without its COB scripts.
    #[arg(long)]
    no_scripts: bool,
}

enum Loaded {
    Synthetic(tak_sim::Scenario),
    Map(Box<Match>, bool),
}

impl Loaded {
    fn scenario(&self) -> &tak_sim::Scenario {
        match self {
            Loaded::Synthetic(s) => s,
            Loaded::Map(m, _) => &m.scenario,
        }
    }

    /// Run and return hashes; real-map runs report timing and script
    /// stats on stderr.
    fn run(self, log: &CommandLog, ticks: u32) -> Result<Vec<u64>, Error> {
        match self {
            Loaded::Synthetic(s) => run(&s, log, ticks),
            Loaded::Map(m, scripts) => {
                let r = run_match(*m, log, ticks, scripts)?;
                eprint!("{}", r.report());
                Ok(r.hashes)
            }
        }
    }
}

impl Source {
    fn load(&self) -> Result<Loaded, Error> {
        match (&self.scenario, &self.map) {
            (Some(p), _) => Ok(Loaded::Synthetic(load_scenario(p)?)),
            (None, Some(map)) => {
                let data = self
                    .data
                    .as_ref()
                    .ok_or_else(|| Error::Setup("--map needs --data or $TAK_DATA_DIR".into()))?;
                let m = load_map(data, map, &self.players, self.world_seed)?;
                Ok(Loaded::Map(Box::new(m), !self.no_scripts))
            }
            (None, None) => Err(Error::Setup("pass --scenario or --map".into())),
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum LogKind {
    /// Fuzz: valid and invalid orders mixed (soak tests).
    Random,
    /// Every player's monarch walks to a random point every `--every` ticks.
    Walk,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a scenario and print (or write) the per-tick hashes.
    Run {
        #[command(flatten)]
        source: Source,
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
        #[command(flatten)]
        source: Source,
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
    /// Write a command log for a scenario or real map (fuzz or walks).
    GenLog {
        #[command(flatten)]
        source: Source,
        /// Ticks the log should span.
        #[arg(long)]
        ticks: u32,
        /// Generator seed.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Kind of log.
        #[arg(long, value_enum, default_value_t = LogKind::Random)]
        kind: LogKind,
        /// Ticks between walk orders (`--kind walk`).
        #[arg(long, default_value_t = 600)]
        every: u32,
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
            source,
            log,
            ticks,
            hashes,
        } => {
            let loaded = source.load()?;
            let log = load_log(log.as_deref())?;
            let text = format_hashes(&loaded.run(&log, ticks)?);
            match hashes {
                Some(p) => write(&p, &text)?,
                None => print!("{text}"),
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Verify {
            source,
            log,
            hashes,
        } => {
            let loaded = source.load()?;
            let log = load_log(log.as_deref())?;
            let text = std::fs::read_to_string(&hashes).map_err(|source| Error::Io {
                path: hashes.display().to_string(),
                source,
            })?;
            let expected = parse_hashes(&text)?;
            let actual = loaded.run(&log, expected.len() as u32)?;
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
            source,
            ticks,
            seed,
            kind,
            every,
            out,
        } => {
            let loaded = source.load()?;
            let sc = loaded.scenario();
            let log = match kind {
                LogKind::Random => random_log(sc, ticks, seed),
                LogKind::Walk => walk_log(sc, ticks, every, seed),
            };
            let json = log.to_json_pretty();
            match out {
                Some(p) => write(&p, &json)?,
                None => println!("{json}"),
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
