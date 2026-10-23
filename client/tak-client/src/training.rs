//! Explicit opt-in adapter for the shared command-only training controller.
//! This module contains no AI decisions or simulation mutations.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use tak_commands::PlayerId;
use tak_headless::bot::ControllerConfig;

use crate::game::SetupError;

/// Read bounded, strict configuration. Player zero is always the local human
/// in this client; a headless config controlling zero cannot be imported here.
pub(crate) fn read(path: &Path) -> Result<ControllerConfig, SetupError> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(65_537).read_to_end(&mut bytes))
        .map_err(|error| SetupError(format!("controller config {}: {error}", path.display())))?;
    if bytes.len() > 65_536 {
        return Err(SetupError("controller config exceeds 64 KiB".into()));
    }
    let config: ControllerConfig = serde_json::from_slice(&bytes)
        .map_err(|error| SetupError(format!("controller config: {error}")))?;
    if !config.human_players.contains(&PlayerId(0)) {
        return Err(SetupError(
            "playable controller config must explicitly declare local player 0 human".into(),
        ));
    }
    validate_local(&config)?;
    Ok(config)
}

/// Empty metadata preserves legacy no-bot behavior. Every explicit saved config
/// must protect player zero, even if it currently enables no opponents.
pub(crate) fn validate_local(config: &ControllerConfig) -> Result<(), SetupError> {
    config
        .validate_shape()
        .map_err(|error| SetupError(error.to_string()))?;
    if !config.is_empty()
        && (!config.human_players.contains(&PlayerId(0)) || config.controls(PlayerId(0)))
    {
        return Err(SetupError(
            "playable controller config must keep local player 0 human, never bot-controlled"
                .into(),
        ));
    }
    Ok(())
}

/// Visible fidelity disclosure, not a claim of original game AI parity.
pub(crate) fn report(config: &ControllerConfig) -> Option<String> {
    (!config.controllers.is_empty()).then(|| {
        let owners = config.controllers.iter().map(|controller| controller.player.0.to_string())
            .collect::<Vec<_>>().join(", ");
        format!("simple-skirmish-v1: full-map/omniscient training opponents for player(s) {owners}. Not original TA:K AI; no fog/scouting behavior.")
    })
}
