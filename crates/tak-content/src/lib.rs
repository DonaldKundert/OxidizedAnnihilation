//! Turns parsed files into game-ready definitions (units, weapons, features,
//! build menus, sides, movement classes, maps). Holds no mutable game state.
//!
//! Work package WP-H (M3b). Every number is an integer or a 16.16
//! [`tak_math::Fx`]; maps are `BTreeMap`s; there are no floats. Each def
//! keeps the TDF entries it does not map in an `extra` map. Field-to-key
//! mapping, confidence tags and corpus coverage: `docs/content.md`.
//!
//! ```ignore
//! let vfs = tak_vfs::open_kingdoms(&dir)?;
//! let rs = tak_content::Ruleset::load(&vfs, tak_content::RulesetKind::Base);
//! let map = tak_content::MapDef::load(&vfs, &rs, "kmap/Adamantine Gate")?;
//! ```

pub mod feature;
pub mod fields;
pub mod gamedata;
pub mod map;
pub mod ota;
pub mod ruleset;
pub mod unit;

pub use feature::FeatureDef;
pub use fields::{Coverage, Issue, SectionCoverage, WideFx, parse_fx};
pub use gamedata::{MoveClassDef, SideDef};
pub use map::{
    CrtSummary, FeatureAnchor, MapDef, MapError, PassGrid, PassRule, PlacedFeature, StartPosition,
    StartSource, playable_maps,
};
pub use ota::{OtaDef, PlacedUnit, Special};
pub use ruleset::{BuildOption, DanglingRef, RefKind, Ruleset, RulesetKind};
pub use unit::{AuraDef, DamageTable, UnitDef, WeaponDef};
