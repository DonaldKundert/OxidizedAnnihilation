//! Scenario: everything a match starts from besides the command log. Seed,
//! map, unit definitions and player slots, with a versioned JSON form.
//!
//! ```json
//! {
//!   "version": 1,
//!   "seed": 42,
//!   "map": {
//!     "width": 1024, "height": 1024,
//!     "heights": { "cols": 64, "rows": 64, "cell_size": 16 },
//!     "start_positions": [ { "x": 128, "y": 128 }, { "x": 896, "y": 896 } ]
//!   },
//!   "unit_defs": [
//!     { "name": "monarch", "maxvelocity": 98304, "acceleration": 8192,
//!       "brakerate": 16384, "turnrate": 728, "footprintx": 2, "footprintz": 2 }
//!   ],
//!   "players": [ { "start": 0, "monarch": 0 }, { "start": 1, "monarch": 0 } ]
//! }
//! ```
//!
//! `Fx` fields (`maxvelocity`, `acceleration`, `brakerate`) are raw 16.16
//! integers (`65536` = 1.0), exactly as in the command log. Map sizes and
//! start positions are whole world units. Player `i` in the list is
//! `PlayerId(i)`.

use serde::{Deserialize, Serialize};
use tak_commands::UnitDefId;

use crate::defs::{MAX_UNIT_DEFS, UnitDef};
use crate::map::MapInfo;

/// Version written into serialized scenarios.
pub const SCENARIO_FORMAT_VERSION: u32 = 1;

/// Most players in a match (TA's limit is 10; we allow a little headroom).
pub const MAX_PLAYERS: usize = 16;

/// Errors from loading or validating a scenario.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    /// Unknown `version`.
    #[error("unsupported scenario version {0} (expected {SCENARIO_FORMAT_VERSION})")]
    UnsupportedVersion(u32),
    /// Parse failure.
    #[error("scenario JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Semantically invalid content.
    #[error("invalid scenario: {0}")]
    Invalid(String),
}

/// One player slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSpec {
    /// Index into `map.start_positions`.
    pub start: u32,
    /// Definition of the unit spawned for this player on tick 0 (the
    /// monarch).
    pub monarch: UnitDefId,
}

/// A match setup. See the module docs for the JSON form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    /// Format version, [`SCENARIO_FORMAT_VERSION`].
    pub version: u32,
    /// Seed for the world RNG.
    pub seed: u64,
    /// The map.
    pub map: MapInfo,
    /// Unit definitions; `UnitDefId(i)` is entry `i`.
    pub unit_defs: Vec<UnitDef>,
    /// Player slots; entry `i` is `PlayerId(i)`.
    pub players: Vec<PlayerSpec>,
    /// Economy rules and sacred sites (WP-N). Optional in JSON.
    #[serde(default)]
    pub economy: Box<crate::economy::EconomySetup>,
    /// Feature definitions, placed map features and unit corpses (WP-P).
    /// Optional in JSON; empty for synthetic scenarios.
    #[serde(default, skip_serializing_if = "features_empty")]
    pub features: Box<crate::features::FeatureSetup>,
}

fn features_empty(f: &crate::features::FeatureSetup) -> bool {
    *f == crate::features::FeatureSetup::default()
}

impl Scenario {
    /// Parse and validate.
    pub fn from_json(s: &str) -> Result<Scenario, ScenarioError> {
        let sc: Scenario = serde_json::from_str(s)?;
        sc.validate()?;
        Ok(sc)
    }

    /// Indented JSON.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Check every invariant the sim relies on.
    pub fn validate(&self) -> Result<(), ScenarioError> {
        if self.version != SCENARIO_FORMAT_VERSION {
            return Err(ScenarioError::UnsupportedVersion(self.version));
        }
        self.map.validate()?;
        if self.unit_defs.len() > MAX_UNIT_DEFS {
            return Err(ScenarioError::Invalid("too many unit defs".into()));
        }
        for d in &self.unit_defs {
            d.validate()?;
        }
        self.economy.validate()?;
        self.features.validate(self.unit_defs.len())?;
        if self.players.len() > MAX_PLAYERS {
            return Err(ScenarioError::Invalid("too many players".into()));
        }
        for (i, p) in self.players.iter().enumerate() {
            if p.start as usize >= self.map.start_positions.len() {
                return Err(ScenarioError::Invalid(format!(
                    "player {i}: start {} has no start position",
                    p.start
                )));
            }
            if p.monarch.0 as usize >= self.unit_defs.len() {
                return Err(ScenarioError::Invalid(format!(
                    "player {i}: monarch def {} does not exist",
                    p.monarch.0
                )));
            }
        }
        Ok(())
    }
}
