//! Oxidized Annihilation's first Bevy client.
//!
//! The deterministic core lives in the main workspace; this crate only
//! renders it, adapts input to `tak-input`, and feeds orders back through
//! the sim's command log. See `docs/client.md`.
//!
//! - [`game`]: the match (sim + log), snapshots for interpolation. No Bevy.
//! - [`terrain`], [`models`]: install data to mesh/texture arrays. No Bevy.
//! - [`app`]: the Bevy app (rendering, camera, input, HUD, screenshots).

pub mod app;
pub mod cli;
pub mod feel;
pub mod fog;
pub mod game;
pub mod models;
pub mod sprites;
pub mod terrain;
pub mod view;
