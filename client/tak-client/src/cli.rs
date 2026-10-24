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
    /// Skirmish map: VFS path without extension.
    #[arg(long, default_value = "kmap/Adamantine Gate")]
    pub map: String,
    /// Authored mission placements, not campaign progression. Requires an
    /// explicit --campaign-players mapping; player 0 is local.
    #[arg(
        long,
        requires = "campaign_players",
        conflicts_with_all = ["map", "players", "demo", "ghost", "record_session", "synth_events"]
    )]
    pub campaign: Option<String>,
    /// Campaign OTA owners and sides, in simulation player order, e.g.
    /// 1:ARA,2:TAR,10:ARA. No sides or alliances are inferred.
    #[arg(long, requires = "campaign")]
    pub campaign_players: Option<String>,
    /// Opt into strict supported objectives (exactly OTA owners 1,2 in
    /// that order). No mission AI or scenario trigger interpreter.
    #[arg(long, requires = "campaign")]
    pub campaign_objectives: bool,
    /// Skirmish sides, comma-separated (one monarch each); player 0 is local.
    #[arg(long, default_value = "ARA,ZON")]
    pub players: String,
    /// World RNG seed.
    #[arg(long, default_value_t = 1)]
    pub world_seed: u64,
    /// Resume a verified real-map or explicit campaign replay checkpoint.
    /// Its source/seed/scripts override startup defaults; --data may relocate.
    #[arg(long, conflicts_with_all = ["map", "players", "world_seed", "campaign", "campaign_players", "campaign_objectives", "demo", "ghost", "record_session", "synth_events"])]
    pub load_checkpoint: Option<PathBuf>,
    /// Skirmish only: explicit ControllerConfig JSON (64 KiB maximum).
    /// Enables simple-skirmish-v1 full-map/omniscient training, not retail AI.
    /// Player 0 must be declared human. Saved configuration resumes unchanged.
    #[arg(long, conflicts_with_all = ["load_checkpoint", "campaign", "demo", "ghost", "record_session", "synth_events"])]
    pub controller_config: Option<PathBuf>,
    /// Quick-save directory. Each F10 / overlay pad X save creates a new file,
    /// never overwrites another save. Default: the per-user game data directory.
    /// Repositories and the owned install are refused as save destinations.
    #[arg(long)]
    pub save_dir: Option<PathBuf>,
    /// Maximum absolute elapsed ticks to replay when loading or quick-saving.
    #[arg(long, default_value_t = 108_000)]
    pub max_replay_ticks: u32,
    /// Maximum executed commands to replay when loading or quick-saving.
    #[arg(long, default_value_t = 250_000)]
    pub max_replay_commands: usize,
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
    /// Screenshot mode: click the HUD at these screen points (`x,y,x,y`)
    /// before each capture, through the real hit test and key path.
    #[arg(long, value_delimiter = ',')]
    pub click: Vec<f32>,
    /// Input tuning preset: console (default), assisted, precise, wp_j.
    /// Ignored when `--input-profile` names an existing file.
    #[arg(long, default_value = "console")]
    pub input_preset: String,
    /// Control scheme: kingdoms (Kingdoms Console pad + Classic TA:K keys,
    /// the default), original (left-click contextual orders), wp_s or wp_j.
    /// Ignored when `--input-profile` names an existing file.
    #[arg(long, default_value = "kingdoms")]
    pub controls: String,
    /// Import this `Keys.TDF` (TA:K's key bindings file) into the keyboard
    /// bindings. Default: the install's own `Keys.TDF` when present.
    #[arg(long)]
    pub keys_tdf: Option<PathBuf>,
    /// Do not import the install's `Keys.TDF`.
    #[arg(long)]
    pub no_keys_tdf: bool,
    /// Turn on the optional Grid build hotkeys (Q-R, A-F, Z-V with a builder
    /// selected).
    #[arg(long)]
    pub grid_keys: bool,
    /// HUD layout: auto (collapse the side panel while the pad is in use),
    /// console (always collapsed, wheels and button prompts) or classic.
    #[arg(long, default_value = "auto")]
    pub hud: String,
    /// Input profile JSON (bindings + tuning). Watched while playing: edits
    /// apply on save. The tuning overlay saves here. Created from the preset
    /// if missing.
    #[arg(long)]
    pub input_profile: Option<PathBuf>,
    /// Skirmish only: record device events, cursor, camera, selections, orders,
    /// plus the match and command log to this file on exit. Send
    /// it back for analysis or watch it with `--ghost`.
    #[arg(long)]
    pub record_session: Option<PathBuf>,
    /// Skirmish only: replay a recorded session's map, players, seed and orders,
    /// with the player's cursor drawn as a ghost (live input is off).
    #[arg(long, conflicts_with_all = ["log", "demo"])]
    pub ghost: Option<PathBuf>,
    /// Ghost playback: follow the recorded camera.
    #[arg(long)]
    pub ghost_camera: bool,
    /// Skirmish only: drive the match headless with scripted gamepad events in
    /// this file (`tak-input` event JSON) through the real input path, write
    /// a session recording to `--record-session` and exit.
    #[arg(long, requires = "record_session")]
    pub synth_events: Option<PathBuf>,
    /// Ticks to run for `--synth-events`.
    #[arg(long, default_value_t = 600)]
    pub synth_ticks: u32,
    /// Controller button glyphs: auto (detect), xbox or playstation.
    #[arg(long, default_value = "auto")]
    pub glyphs: String,
    /// Screenshot mode: feed these gamepad events (`tak-input` event JSON)
    /// to the mapper before each capture, so wheels, cursors and hints show.
    #[arg(long)]
    pub pad_events: Option<PathBuf>,
    /// Start with the tuning overlay open.
    #[arg(long)]
    pub overlay: bool,
}

impl Args {
    /// Check source-specific runtime restrictions as well as clap's conflicts.
    /// This also protects callers constructing or modifying Args directly.
    pub fn validate_source(&self) -> Result<(), String> {
        if self.controller_config.is_some()
            && (self.load_checkpoint.is_some()
                || self.campaign.is_some()
                || self.campaign_players.is_some()
                || self.campaign_objectives
                || self.demo
                || self.ghost.is_some()
                || self.record_session.is_some()
                || self.synth_events.is_some())
        {
            return Err("--controller-config is skirmish only and cannot combine with --load-checkpoint, campaign, --demo, --ghost, --record-session or --synth-events; recorded and saved orders/configuration are authoritative".into());
        }
        if self.load_checkpoint.is_some()
            && (self.campaign.is_some()
                || self.campaign_players.is_some()
                || self.campaign_objectives
                || self.demo
                || self.ghost.is_some()
                || self.record_session.is_some()
                || self.synth_events.is_some())
        {
            return Err("--load-checkpoint cannot combine with campaign setup, --demo, --ghost, --record-session or --synth-events; the saved source and history are authoritative".into());
        }
        if self.save_dir.is_some()
            && (self.demo
                || self.ghost.is_some()
                || self.record_session.is_some()
                || self.synth_events.is_some())
        {
            return Err(
                "quick-saving does not support --demo, --ghost, --record-session or --synth-events"
                    .into(),
            );
        }
        if self.campaign.is_some() {
            if self.campaign_players.is_none() {
                return Err(
                    "--campaign requires an explicit --campaign-players OTA_SLOT:SIDE mapping"
                        .into(),
                );
            }
            if self.demo {
                return Err(
                    "--demo is a skirmish fixture and cannot be used with --campaign".into(),
                );
            }
            if self.ghost.is_some() || self.record_session.is_some() || self.synth_events.is_some()
            {
                return Err("--campaign does not support --ghost, --record-session or --synth-events: session recordings currently describe skirmishes only; use --log, --record and --hashes with the same explicit campaign setup instead".into());
            }
        } else if self.campaign_players.is_some() || self.campaign_objectives {
            return Err("--campaign-players and --campaign-objectives require --campaign".into());
        }
        Ok(())
    }

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
