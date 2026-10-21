//! Unit definitions stand-in.
//!
//! WP-H will fill these from `.fbi` files. Field names follow the FBI keys
//! (lowercased) so the mapping stays obvious, but the **units are the sim's**:
//! distances in world units, times in ticks. The FBI loader converts.

use serde::{Deserialize, Serialize};
use tak_commands::UnitDefId;
use tak_math::Fx;

use crate::hash::StateHasher;
use crate::scenario::ScenarioError;

/// Upper bound on `maxvelocity`, in world units per tick (sanity cap).
pub const MAX_VELOCITY_CAP: Fx = Fx::from_int(64);

/// Most definitions a scenario may carry.
pub const MAX_UNIT_DEFS: usize = 4096;

/// Movement-relevant subset of a unit definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitDef {
    /// FBI `UnitName` (internal name), e.g. `"aramon_monarch"`.
    pub name: String,
    /// FBI `MaxVelocity`: top speed, world units per tick (raw 16.16).
    pub maxvelocity: Fx,
    /// FBI `Acceleration`: speed gained per tick (raw 16.16).
    pub acceleration: Fx,
    /// FBI `BrakeRate`: speed lost per tick when braking (raw 16.16).
    pub brakerate: Fx,
    /// FBI `TurnRate`: binary angle units (65536 per turn) per tick.
    pub turnrate: u16,
    /// FBI `FootprintX`, in footprint squares.
    pub footprintx: u8,
    /// FBI `FootprintZ`, in footprint squares.
    pub footprintz: u8,
    /// Economy keys (WP-N).
    #[serde(default)]
    pub economy: crate::economy::EconomyDef,
}

impl UnitDef {
    pub(crate) fn validate(&self) -> Result<(), ScenarioError> {
        let bad = |what: &str| {
            Err(ScenarioError::Invalid(format!(
                "unit def {:?}: {what}",
                self.name
            )))
        };
        if self.maxvelocity < Fx::ZERO || self.maxvelocity > MAX_VELOCITY_CAP {
            return bad("maxvelocity out of range");
        }
        if self.acceleration <= Fx::ZERO || self.acceleration > MAX_VELOCITY_CAP {
            return bad("acceleration must be positive");
        }
        if self.brakerate <= Fx::ZERO || self.brakerate > MAX_VELOCITY_CAP {
            return bad("brakerate must be positive");
        }
        if self.footprintx == 0 || self.footprintz == 0 {
            return bad("footprint must be at least 1x1");
        }
        if self.name.len() > 256 {
            return bad("name too long");
        }
        self.economy.validate(&self.name)?;
        Ok(())
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(self.name.as_bytes());
        h.fx(self.maxvelocity);
        h.fx(self.acceleration);
        h.fx(self.brakerate);
        h.u16(self.turnrate);
        h.u8(self.footprintx);
        h.u8(self.footprintz);
        self.economy.hash(h);
    }
}

/// All definitions of a match, indexed by [`UnitDefId`] (position in the
/// list).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct UnitDefs(Vec<UnitDef>);

impl UnitDefs {
    /// Wrap a list; `UnitDefId(i)` is `defs[i]`.
    pub fn new(defs: Vec<UnitDef>) -> UnitDefs {
        UnitDefs(defs)
    }

    /// Look up a definition.
    pub fn get(&self, id: UnitDefId) -> Option<&UnitDef> {
        self.0.get(id.0 as usize)
    }

    /// Number of definitions.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` if empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// All definitions in id order.
    pub fn as_slice(&self) -> &[UnitDef] {
        &self.0
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.len(self.0.len());
        for d in &self.0 {
            d.hash(h);
        }
    }
}
