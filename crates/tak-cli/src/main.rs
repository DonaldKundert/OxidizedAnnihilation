//! Inspection tools for Total Annihilation: Kingdoms data. Not shipped in the
//! game binary.

mod archive_cmds;
mod report;
mod survey;

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "tak-cli",
    version,
    about = "Inspect Total Annihilation: Kingdoms data"
)]
struct Cli {
    /// Kingdoms install directory. Defaults to $TAK_DATA_DIR.
    #[arg(long, global = true, env = "TAK_DATA_DIR")]
    data: Option<PathBuf>,
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
    /// List files in the mounted archives (case-insensitive prefix).
    Ls {
        #[arg(default_value = "")]
        prefix: String,
        /// Show size, source archive and shadowing.
        #[arg(short, long)]
        long: bool,
    },
    /// Write one file's contents to stdout.
    Cat { path: String },
    /// Extract files under a prefix. Keep the output outside the repo.
    Extract {
        #[arg(default_value = "")]
        prefix: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// List paths provided by more than one archive.
    Shadowed {
        #[arg(default_value = "")]
        prefix: String,
    },
    /// Inventory archive contents by extension and magic.
    Census {
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Survey { dir, out } => {
            let out = out.unwrap_or_else(|| std::env::temp_dir().join("tak-survey"));
            run_survey(&dir, &out).map_err(Into::into)
        }
        cmd => with_vfs(cli.data.as_deref(), |vfs| match cmd {
            Cmd::Ls { prefix, long } => archive_cmds::ls(vfs, &prefix, long),
            Cmd::Cat { path } => archive_cmds::cat(vfs, &path),
            Cmd::Extract { prefix, out } => archive_cmds::extract(vfs, &prefix, &out),
            Cmd::Shadowed { prefix } => archive_cmds::shadowed(vfs, &prefix),
            Cmd::Census { out } => {
                let out = out.unwrap_or_else(|| std::env::temp_dir().join("tak-census"));
                archive_cmds::census(vfs, &out)
            }
            Cmd::Survey { .. } => unreachable!("handled above"),
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        // Piping into `head` closes stdout early; that is not a failure.
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe) =>
        {
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn with_vfs(
    data: Option<&Path>,
    f: impl FnOnce(&tak_vfs::Vfs) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let data = data.ok_or("no install given: pass --data or set TAK_DATA_DIR")?;
    let vfs = tak_vfs::open_kingdoms(data)?;
    f(&vfs)
}

fn run_survey(dir: &Path, out: &Path) -> std::io::Result<()> {
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
