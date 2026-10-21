//! Inspection tools for Total Annihilation: Kingdoms data. Not shipped in the
//! game binary.

mod archive_cmds;
mod disasm_cmd;
mod map_cmd;
mod model_cmd;
mod report;
mod sprite_cmd;
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
    /// Export a 3DO model as OBJ + MTL + JSON piece tree. Keep the output outside the repo.
    DumpModel {
        /// VFS path (`objects3d/araknigh.3do`) or bare model name (`araknigh`).
        path: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Inventory archive contents by extension and magic.
    Census {
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Disassemble a COB script; with --stats, opcode counts under a prefix.
    Disasm {
        path: String,
        #[arg(long)]
        stats: bool,
    },
    /// Write every frame of a GAF/TAF/GAO or PCX as PNG. Keep the output outside the repo.
    DumpSprite {
        path: String,
        #[arg(long)]
        out: PathBuf,
        /// Palette for indexed art (.pal or .pcx path in the archives).
        #[arg(long)]
        palette: Option<String>,
    },
    /// Write PNGs of a map's heights, cells, minimaps and terrain mosaic.
    DumpMap {
        /// VFS path to a .tnt, e.g. "kmap/Adamantine Gate.tnt".
        path: String,
        /// Output folder. Keep it outside the repo.
        #[arg(long)]
        out: PathBuf,
        /// VFS path of a .pcx/.pal for the minimaps (default: from the .ota kingdom).
        #[arg(long)]
        palette: Option<String>,
        /// Mosaic pixels per tile cell (0 skips the mosaic).
        #[arg(long, default_value_t = 16)]
        piece: u32,
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
            Cmd::Disasm { path, stats } => disasm_cmd::run(vfs, &path, stats),
            Cmd::DumpModel { path, out } => model_cmd::dump_model(vfs, &path, &out),
            Cmd::DumpSprite { path, out, palette } => {
                sprite_cmd::dump_sprite(vfs, &path, &out, palette.as_deref())
            }
            Cmd::DumpMap {
                path,
                out,
                palette,
                piece,
            } => map_cmd::dump_map(vfs, &path, &out, palette.as_deref(), piece),
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
