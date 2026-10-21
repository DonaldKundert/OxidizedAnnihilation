//! Inspection tools for Total Annihilation: Kingdoms data. Not shipped in the
//! game binary.

mod report;
mod survey;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "tak-cli",
    version,
    about = "Inspect Total Annihilation: Kingdoms data"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Inventory an install: sizes, first 64 bytes, files grouped by magic.
    Survey {
        /// The Kingdoms install directory.
        dir: PathBuf,
        /// Output folder for survey.json and survey.md. Keep it outside the repo.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Survey { dir, out } => {
            let out = out.unwrap_or_else(|| std::env::temp_dir().join("tak-survey"));
            match run_survey(&dir, &out) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("survey failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn run_survey(dir: &std::path::Path, out: &std::path::Path) -> std::io::Result<()> {
    let s = survey::run(dir)?;
    std::fs::create_dir_all(out)?;
    let json = serde_json::to_string_pretty(&s).map_err(std::io::Error::other)?;
    std::fs::write(out.join("survey.json"), json)?;
    std::fs::write(out.join("survey.md"), report::markdown(&s))?;
    eprintln!(
        "surveyed {} files ({} signatures, {} errors) -> {}",
        s.files.len(),
        s.signatures.len(),
        s.errors.len(),
        out.display()
    );
    Ok(())
}
