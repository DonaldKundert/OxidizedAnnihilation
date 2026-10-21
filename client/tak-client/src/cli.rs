//! Command line.

use std::path::PathBuf;

use clap::Parser;

/// Oxidized Annihilation: play a Total Annihilation: Kingdoms map on the
/// deterministic core.
#[derive(Parser, Debug, Clone)]
#[command(name = "tak-client", version)]
pub struct Args {
    /// Kingdoms install directory.
    #[arg(long, env = "TAK_DATA_DIR")]
    pub data: PathBuf,
    /// Map: VFS path without extension.
    #[arg(long, default_value = "kmap/Adamantine Gate")]
    pub map: String,
    /// Sides, comma-separated (one monarch each); player 0 is local.
    #[arg(long, default_value = "ARA,ZON")]
    pub players: String,
    /// World RNG seed.
    #[arg(long, default_value_t = 1)]
    pub world_seed: u64,
    /// Command log to play (JSON, `tak-headless` format). Orders you give
    /// are added to it.
    #[arg(long)]
    pub log: Option<PathBuf>,
    /// Play the scripted demo session (a wall, then the monarchs fight)
    /// instead of `--log`.
    #[arg(long, conflicts_with = "log")]
    pub demo: bool,
    /// On exit, write the full command log (fixture plus your orders) here.
    #[arg(long)]
    pub record: Option<PathBuf>,
    /// On exit, write one hash per tick here (`tak-headless verify` format).
    #[arg(long)]
    pub hashes: Option<PathBuf>,
    /// Verification mode: run to each `--at-tick`, save a PNG and exit.
    /// `{tick}` in the path is replaced by the tick.
    #[arg(long)]
    pub screenshot: Option<String>,
    /// Ticks for `--screenshot`, comma-separated, ascending.
    #[arg(long, value_delimiter = ',', default_value = "0")]
    pub at_tick: Vec<u32>,
    /// Camera focus for screenshots: `monarch:N`, `unit:SLOT` or `X,Y`
    /// (world units). Default: the local monarch.
    #[arg(long, default_value = "monarch:0")]
    pub focus: String,
    /// World units per screen pixel (1 = TA's native scale).
    #[arg(long, default_value_t = 0.5)]
    pub zoom: f32,
    /// Window size, `WxH`.
    #[arg(long, default_value = "1280x720")]
    pub window: String,
    /// Select these unit slots before the screenshot (comma-separated), to
    /// show selection rings and the HUD panel.
    #[arg(long, value_delimiter = ',')]
    pub select: Vec<u32>,
    /// Render these 3DO models (names under `objects3d/`, comma-separated)
    /// in a row next to the focus, at rest pose. Render only: for checking
    /// textures and art by eye.
    #[arg(long, value_delimiter = ',')]
    pub showcase: Vec<String>,
    /// Start with the visual fog of war off (F9 toggles it in play).
    #[arg(long)]
    pub no_fog: bool,
}

impl Args {
    /// Parsed window size.
    pub fn window_size(&self) -> (u32, u32) {
        self.window
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
            .unwrap_or((1280, 720))
    }
}

/// Where the camera looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Focus {
    /// Player N's monarch.
    Monarch(u8),
    /// A unit slot.
    Unit(u32),
    /// A world point.
    Point(f32, f32),
}

/// Parse `--focus`.
pub fn parse_focus(s: &str) -> Option<Focus> {
    if let Some(n) = s.strip_prefix("monarch:") {
        return n.parse().ok().map(Focus::Monarch);
    }
    if let Some(n) = s.strip_prefix("unit:") {
        return n.parse().ok().map(Focus::Unit);
    }
    let (x, y) = s.split_once(',')?;
    Some(Focus::Point(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_forms() {
        assert_eq!(parse_focus("monarch:1"), Some(Focus::Monarch(1)));
        assert_eq!(parse_focus("unit:7"), Some(Focus::Unit(7)));
        assert_eq!(parse_focus("100, 200.5"), Some(Focus::Point(100.0, 200.5)));
        assert_eq!(parse_focus("nope"), None);
    }
}
