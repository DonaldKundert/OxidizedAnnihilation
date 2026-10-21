//! Map description the sim runs on: dimensions, a height grid, optional
//! per-cell terrain flags (water, blockers, paving) and start positions.
//!
//! Synthetic scenarios fill this struct by hand; real maps are converted
//! from `tak_content::MapDef` by [`crate::content::map_from_content`].
//!
//! # Coordinates
//!
//! The ground plane is `(x, y)` in world units, one world unit per map pixel
//! (**Believed**, TA convention: 32 px per tile, 16 px per height cell). The
//! origin is the map's top-left corner, `+x` points east (right on the
//! minimap) and `+y` points south (down on the minimap), matching the TNT's
//! row-major layout. The playable area is `[0, width] x [0, height]`. A TNT
//! attribute cell `(cx, cz)` covers world `[16 cx, 16 cx + 16) x [16 cz,
//! 16 cz + 16)`; TNT/OTA `z` is the sim's `y`.

use serde::{Deserialize, Serialize};
use tak_math::{Fx, Vec2};

use crate::WorldPos;
use crate::hash::StateHasher;
use crate::scenario::ScenarioError;

/// Largest accepted map side, in world units. Keeps every in-map distance
/// (diagonal of 16384 x 16384 is about 23170) inside the 16.16 range.
pub const MAX_MAP_SIDE: i32 = 16384;

/// Largest accepted height grid (cells).
pub const MAX_HEIGHT_CELLS: u32 = 1 << 22;

/// Most start positions a map may declare.
pub const MAX_START_POSITIONS: usize = 16;

/// A height sample per `cell_size x cell_size` square, row-major from the
/// top-left. Heights are raw TNT-style bytes (one world unit per step,
/// **Believed**).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeightGrid {
    /// Cells per row.
    pub cols: u32,
    /// Rows.
    pub rows: u32,
    /// World units per cell side (TA: 16).
    pub cell_size: u32,
    /// `cols * rows` samples, row-major. Empty means flat at `fill`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heights: Vec<u8>,
    /// Height used when `heights` is empty.
    #[serde(default)]
    pub fill: u8,
}

impl HeightGrid {
    /// A flat grid.
    pub fn flat(cols: u32, rows: u32, cell_size: u32, h: u8) -> HeightGrid {
        HeightGrid {
            cols,
            rows,
            cell_size,
            heights: Vec::new(),
            fill: h,
        }
    }

    fn validate(&self) -> Result<(), ScenarioError> {
        let cells = self.cols.checked_mul(self.rows);
        match cells {
            Some(n) if n > 0 && n <= MAX_HEIGHT_CELLS && self.cell_size > 0 => {}
            _ => return Err(ScenarioError::Invalid("height grid size".into())),
        }
        if !self.heights.is_empty() && self.heights.len() as u64 != u64::from(cells.unwrap_or(0)) {
            return Err(ScenarioError::Invalid(
                "height grid: heights.len() != cols * rows".into(),
            ));
        }
        Ok(())
    }

    /// Height at a ground point: the containing cell's sample (no
    /// interpolation yet), clamped to the grid.
    pub fn height_at(&self, p: WorldPos) -> Fx {
        if self.heights.is_empty() {
            return Fx::from_int(i32::from(self.fill));
        }
        let (c, r) = cell_of(p, self.cols, self.rows, self.cell_size);
        let idx = (r as usize)
            .saturating_mul(self.cols as usize)
            .saturating_add(c as usize)
            .min(self.heights.len().saturating_sub(1));
        Fx::from_int(i32::from(self.heights[idx]))
    }

    fn hash(&self, h: &mut StateHasher) {
        h.u32(self.cols);
        h.u32(self.rows);
        h.u32(self.cell_size);
        h.u8(self.fill);
        h.bytes(&self.heights);
    }
}

/// The `(col, row)` of the cell containing `p`, clamped to the grid.
fn cell_of(p: WorldPos, cols: u32, rows: u32, cell_size: u32) -> (u32, u32) {
    let cs = i32::try_from(cell_size).unwrap_or(i32::MAX).max(1);
    let cell = |v: Fx, n: u32| -> u32 {
        let max = i32::try_from(n).unwrap_or(i32::MAX).saturating_sub(1);
        v.to_int_floor().div_euclid(cs).clamp(0, max.max(0)) as u32
    };
    (cell(p.x, cols), cell(p.y, rows))
}

/// Terrain flag bits, identical to `tak_content::PassGrid`'s.
pub mod terrain {
    /// Below sea level.
    pub const WATER: u8 = 1;
    /// Too deep for ground units.
    pub const DEEP: u8 = 2;
    /// Too steep for ground units.
    pub const STEEP: u8 = 4;
    /// Covered by a blocking feature.
    pub const FEATURE: u8 = 8;
    /// Paved (TNT `0xFFFB`, Believed roads).
    pub const PAVED: u8 = 16;
    /// TNT `0xFFFC` (Unknown).
    pub const MARKER: u8 = 32;
    /// Flags that stop a ground unit.
    pub const BLOCKS: u8 = DEEP | STEEP | FEATURE;
}

/// One flag byte (see [`terrain`]) per `cell_size x cell_size` square,
/// row-major from the top-left, laid out like [`HeightGrid`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainGrid {
    /// Cells per row.
    pub cols: u32,
    /// Rows.
    pub rows: u32,
    /// World units per cell side (16 on real maps).
    pub cell_size: u32,
    /// `cols * rows` flag bytes.
    pub flags: Vec<u8>,
}

impl TerrainGrid {
    fn validate(&self) -> Result<(), ScenarioError> {
        let cells = self.cols.checked_mul(self.rows);
        match cells {
            Some(n) if n > 0 && n <= MAX_HEIGHT_CELLS && self.cell_size > 0 => {}
            _ => return Err(ScenarioError::Invalid("terrain grid size".into())),
        }
        if self.flags.len() as u64 != u64::from(cells.unwrap_or(0)) {
            return Err(ScenarioError::Invalid(
                "terrain grid: flags.len() != cols * rows".into(),
            ));
        }
        Ok(())
    }

    /// The cell containing a ground point, clamped to the grid.
    pub fn cell_of(&self, p: WorldPos) -> (u32, u32) {
        cell_of(p, self.cols, self.rows, self.cell_size)
    }

    /// Flags of a cell; out-of-range cells read as [`terrain::BLOCKS`].
    pub fn flags_of_cell(&self, c: u32, r: u32) -> u8 {
        if c >= self.cols || r >= self.rows {
            return terrain::BLOCKS;
        }
        self.flags
            .get(r as usize * self.cols as usize + c as usize)
            .copied()
            .unwrap_or(terrain::BLOCKS)
    }

    /// Flags at a ground point (containing cell, clamped to the grid).
    pub fn flags_at(&self, p: WorldPos) -> u8 {
        let (c, r) = self.cell_of(p);
        self.flags_of_cell(c, r)
    }

    fn hash(&self, h: &mut StateHasher) {
        h.u32(self.cols);
        h.u32(self.rows);
        h.u32(self.cell_size);
        h.bytes(&self.flags);
    }
}

/// A start position in whole world units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartPos {
    /// East.
    pub x: i32,
    /// South.
    pub y: i32,
}

impl StartPos {
    /// As a ground point.
    pub fn to_world(self) -> WorldPos {
        Vec2::from_ints(self.x, self.y)
    }
}

/// Static map description. See the module docs for coordinates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapInfo {
    /// Width in world units.
    pub width: i32,
    /// Height (north-south extent) in world units.
    pub height: i32,
    /// Terrain heights.
    pub heights: HeightGrid,
    /// Start positions, indexed by scenario player `start`.
    pub start_positions: Vec<StartPos>,
    /// Per-cell terrain flags; `None` (synthetic maps) means open ground
    /// everywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain: Option<TerrainGrid>,
    /// Sea level in height units (TNT header). Informational: the sim reads
    /// the [`terrain::WATER`] flag.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sea_level: i32,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

impl MapInfo {
    /// A flat map with TA-sized height cells (16 world units).
    pub fn flat(width: i32, height: i32, start_positions: Vec<StartPos>) -> MapInfo {
        let cols = (width.max(1) as u32).div_ceil(16);
        let rows = (height.max(1) as u32).div_ceil(16);
        MapInfo {
            width,
            height,
            heights: HeightGrid::flat(cols, rows, 16, 0),
            start_positions,
            terrain: None,
            sea_level: 0,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ScenarioError> {
        if !(1..=MAX_MAP_SIDE).contains(&self.width) || !(1..=MAX_MAP_SIDE).contains(&self.height) {
            return Err(ScenarioError::Invalid(format!(
                "map size {}x{} outside 1..={MAX_MAP_SIDE}",
                self.width, self.height
            )));
        }
        self.heights.validate()?;
        if let Some(t) = &self.terrain {
            t.validate()?;
        }
        if self.start_positions.len() > MAX_START_POSITIONS {
            return Err(ScenarioError::Invalid("too many start positions".into()));
        }
        for s in &self.start_positions {
            if !(0..=self.width).contains(&s.x) || !(0..=self.height).contains(&s.y) {
                return Err(ScenarioError::Invalid(format!(
                    "start position ({}, {}) outside the map",
                    s.x, s.y
                )));
            }
        }
        Ok(())
    }

    /// Clamp a ground point into the playable area.
    pub fn clamp(&self, p: WorldPos) -> WorldPos {
        Vec2::new(
            p.x.clamp(Fx::ZERO, Fx::from_int(self.width)),
            p.y.clamp(Fx::ZERO, Fx::from_int(self.height)),
        )
    }

    /// Terrain height at a ground point.
    pub fn height_at(&self, p: WorldPos) -> Fx {
        self.heights.height_at(p)
    }

    /// Terrain flags at a ground point (0, open ground, on maps without a
    /// terrain grid).
    pub fn terrain_at(&self, p: WorldPos) -> u8 {
        self.terrain.as_ref().map_or(0, |t| t.flags_at(p))
    }

    /// `true` if a ground unit at `from` may not step to `to`: the
    /// destination cell blocks ([`terrain::BLOCKS`]) and is not the cell the
    /// unit stands in. Leaving a blocked cell is always allowed, so a unit
    /// placed on a blocker can walk off it (**Chosen**, `docs/sim.md`). Only
    /// the unit's centre point is tested, not its footprint.
    pub fn blocks_step(&self, from: WorldPos, to: WorldPos) -> bool {
        let Some(t) = &self.terrain else {
            return false;
        };
        let (a, b) = (t.cell_of(from), t.cell_of(to));
        a != b && t.flags_of_cell(b.0, b.1) & terrain::BLOCKS != 0
    }

    /// Centre of the map.
    pub fn center(&self) -> WorldPos {
        Vec2::from_ints(self.width / 2, self.height / 2)
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.i32(self.width);
        h.i32(self.height);
        self.heights.hash(h);
        h.len(self.start_positions.len());
        for s in &self.start_positions {
            h.i32(s.x);
            h.i32(s.y);
        }
        h.option(self.terrain.as_ref(), |h, t| t.hash(h));
        h.i32(self.sea_level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_lookup_clamps() {
        let g = HeightGrid {
            cols: 2,
            rows: 2,
            cell_size: 16,
            heights: vec![1, 2, 3, 4],
            fill: 0,
        };
        assert_eq!(g.height_at(Vec2::from_ints(0, 0)), Fx::from_int(1));
        assert_eq!(g.height_at(Vec2::from_ints(17, 0)), Fx::from_int(2));
        assert_eq!(g.height_at(Vec2::from_ints(0, 20)), Fx::from_int(3));
        assert_eq!(g.height_at(Vec2::from_ints(999, 999)), Fx::from_int(4));
        assert_eq!(g.height_at(Vec2::from_ints(-5, -5)), Fx::from_int(1));
    }

    #[test]
    fn rejects_bad_maps() {
        let mut m = MapInfo::flat(256, 256, vec![StartPos { x: 10, y: 10 }]);
        assert!(m.validate().is_ok());
        m.start_positions.push(StartPos { x: 300, y: 0 });
        assert!(m.validate().is_err());
        let mut m = MapInfo::flat(256, 256, vec![]);
        m.heights.heights = vec![0; 3];
        assert!(m.validate().is_err());
        let m = MapInfo::flat(MAX_MAP_SIDE + 1, 16, vec![]);
        assert!(m.validate().is_err());
        let mut m = MapInfo::flat(64, 64, vec![]);
        m.terrain = Some(TerrainGrid {
            cols: 4,
            rows: 4,
            cell_size: 16,
            flags: vec![0; 15],
        });
        assert!(m.validate().is_err());
    }

    #[test]
    fn blocked_cells_stop_steps_but_can_be_left() {
        let mut m = MapInfo::flat(64, 64, vec![]);
        let mut flags = vec![0; 16];
        flags[1] = terrain::STEEP; // cell (1, 0)
        flags[4] = terrain::WATER; // cell (0, 1): wet but passable
        m.terrain = Some(TerrainGrid {
            cols: 4,
            rows: 4,
            cell_size: 16,
            flags,
        });
        assert!(m.validate().is_ok());
        let p = Vec2::from_ints;
        assert!(m.blocks_step(p(15, 5), p(16, 5)));
        assert!(!m.blocks_step(p(16, 5), p(17, 5)), "inside the blocker");
        assert!(!m.blocks_step(p(17, 5), p(33, 5)), "leaving it");
        assert!(!m.blocks_step(p(5, 15), p(5, 16)), "water is passable");
        assert_eq!(m.terrain_at(p(5, 20)), terrain::WATER);
        let open = MapInfo::flat(64, 64, vec![]);
        assert!(!open.blocks_step(p(15, 5), p(16, 5)));
        assert_eq!(open.terrain_at(p(15, 5)), 0);
    }
}
