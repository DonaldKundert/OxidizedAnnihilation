//! Identifiers shared by commands and the sim.

use serde::{Deserialize, Serialize};

/// Simulation tick number. 30 ticks per second; tick 0 is the first tick of
/// the match. `u32` lasts about 4.5 years of game time.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(pub u32);

impl Tick {
    /// The tick after this one (wrapping, which a real match never reaches).
    #[inline]
    pub const fn next(self) -> Tick {
        Tick(self.0.wrapping_add(1))
    }
}

/// A player slot in the match (not a network or account identity).
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PlayerId(pub u8);

/// Generational unit handle: an arena slot `index` plus the `generation` of
/// the unit occupying it. When a unit dies the sim bumps the slot's generation,
/// so stale IDs held by commands or the UI stop matching instead of pointing at
/// whichever unit reused the slot. Orders as `(index, generation)`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct UnitId {
    /// Arena slot.
    pub index: u32,
    /// Slot generation at the time this ID was issued.
    pub generation: u32,
}

impl UnitId {
    /// Construct from slot index and generation.
    #[inline]
    pub const fn new(index: u32, generation: u32) -> UnitId {
        UnitId { index, generation }
    }
}

/// Generational feature handle. Removing or replacing a tree/wreck retires its
/// lifetime, so queued Clear commands cannot target a successor in the same slot.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct FeatureId {
    /// Arena slot.
    pub index: u32,
    /// Slot generation at the time this ID was issued.
    pub generation: u32,
}

impl FeatureId {
    /// Construct from slot index and generation.
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }
}

/// Opaque reference to a unit definition (a unit type). Its meaning is
/// assigned by the content layer (`tak-content`) when definitions load; this
/// crate only carries it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnitDefId(pub u32);

/// Opaque reference to a spell definition, assigned by the content layer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpellId(pub u32);

/// Control-group number, `0..=9` (the number-row keys).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct ControlGroup(u8);

impl ControlGroup {
    /// Number of control groups.
    pub const COUNT: u8 = 10;

    /// `Some` for `0..=9`, `None` otherwise.
    #[inline]
    pub const fn new(n: u8) -> Option<ControlGroup> {
        if n < Self::COUNT {
            Some(ControlGroup(n))
        } else {
            None
        }
    }

    /// The group number, `0..=9`.
    #[inline]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for ControlGroup {
    type Error = String;
    fn try_from(n: u8) -> Result<Self, Self::Error> {
        ControlGroup::new(n).ok_or_else(|| format!("control group {n} out of range 0..=9"))
    }
}

impl From<ControlGroup> for u8 {
    fn from(g: ControlGroup) -> u8 {
        g.0
    }
}
