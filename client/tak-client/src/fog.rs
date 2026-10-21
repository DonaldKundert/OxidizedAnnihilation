//! Visual fog of war (client side only; gameplay line of sight is the
//! sim's business). No Bevy types.
//!
//! Per attribute cell: unexplored (never seen), explored (seen before),
//! seen (inside a friendly unit's `sightdistance` now). Recomputed from
//! sim positions, so it never feeds back into the sim.

/// Fog state of one cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FogCell {
    /// Never seen: black.
    Unexplored,
    /// Seen before, not now: dimmed.
    Explored,
    /// In sight now: clear.
    Seen,
}

/// Opacity of the fog overlay over an explored cell (**Chosen**; the
/// blend is in linear light, so 0.6 darkens to roughly half brightness on
/// screen).
pub const EXPLORED_ALPHA: f32 = 0.6;

/// The fog grid of one player.
#[derive(Clone, Debug)]
pub struct FogGrid {
    /// Cells (w, h).
    pub size: (u32, u32),
    explored: Vec<bool>,
    seen: Vec<bool>,
}

/// World units per attribute cell.
const CELL: f32 = 16.0;

impl FogGrid {
    /// An all-unexplored grid.
    pub fn new(w: u32, h: u32) -> FogGrid {
        let n = w as usize * h as usize;
        FogGrid {
            size: (w, h),
            explored: vec![false; n],
            seen: vec![false; n],
        }
    }

    /// Recompute what is seen from `(x, z, radius)` circles (world units);
    /// everything seen becomes explored. Returns whether any cell changed.
    pub fn update(&mut self, eyes: &[(f32, f32, f32)]) -> bool {
        let (w, h) = (self.size.0 as i64, self.size.1 as i64);
        let mut seen = vec![false; self.seen.len()];
        for &(x, z, r) in eyes {
            if r <= 0.0 {
                continue;
            }
            let (cx0, cx1) = (
                ((x - r) / CELL).floor() as i64,
                ((x + r) / CELL).ceil() as i64,
            );
            let (cz0, cz1) = (
                ((z - r) / CELL).floor() as i64,
                ((z + r) / CELL).ceil() as i64,
            );
            let r2 = r * r;
            for cz in cz0.max(0)..cz1.min(h) {
                for cx in cx0.max(0)..cx1.min(w) {
                    let (px, pz) = ((cx as f32 + 0.5) * CELL, (cz as f32 + 0.5) * CELL);
                    if (px - x) * (px - x) + (pz - z) * (pz - z) <= r2 {
                        seen[(cz * w + cx) as usize] = true;
                    }
                }
            }
        }
        let mut changed = seen != self.seen;
        for (e, &s) in self.explored.iter_mut().zip(&seen) {
            if s && !*e {
                *e = true;
                changed = true;
            }
        }
        self.seen = seen;
        changed
    }

    /// State of a cell (outside the map: unexplored).
    pub fn cell(&self, cx: i64, cz: i64) -> FogCell {
        if cx < 0 || cz < 0 || cx >= i64::from(self.size.0) || cz >= i64::from(self.size.1) {
            return FogCell::Unexplored;
        }
        let i = (cz * i64::from(self.size.0) + cx) as usize;
        if self.seen[i] {
            FogCell::Seen
        } else if self.explored[i] {
            FogCell::Explored
        } else {
            FogCell::Unexplored
        }
    }

    /// State at a world point.
    pub fn at(&self, x: f32, z: f32) -> FogCell {
        self.cell((x / CELL).floor() as i64, (z / CELL).floor() as i64)
    }

    /// Overlay opacity of a cell.
    pub fn alpha(&self, cx: i64, cz: i64) -> f32 {
        match self.cell(cx, cz) {
            FogCell::Unexplored => 1.0,
            FogCell::Explored => EXPLORED_ALPHA,
            FogCell::Seen => 0.0,
        }
    }

    /// Overlay opacity at cell corner `(cx, cz)`: the mean of the four
    /// cells touching it, for soft edges.
    pub fn corner_alpha(&self, cx: i64, cz: i64) -> f32 {
        (self.alpha(cx - 1, cz - 1)
            + self.alpha(cx, cz - 1)
            + self.alpha(cx - 1, cz)
            + self.alpha(cx, cz))
            / 4.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sight_reveals_then_remembers() {
        let mut g = FogGrid::new(20, 20);
        assert_eq!(g.at(100.0, 100.0), FogCell::Unexplored);
        assert!(g.update(&[(100.0, 100.0, 40.0)]));
        assert_eq!(g.at(100.0, 100.0), FogCell::Seen);
        assert_eq!(g.at(300.0, 300.0), FogCell::Unexplored);
        assert!(!g.update(&[(100.0, 100.0, 40.0)]), "no change, no update");
        assert!(g.update(&[(250.0, 250.0, 20.0)]));
        assert_eq!(g.at(100.0, 100.0), FogCell::Explored);
        assert_eq!(g.at(250.0, 250.0), FogCell::Seen);
        assert!((g.corner_alpha(0, 0) - 1.0).abs() < 1e-6);
        // Out-of-map eyes are clipped, not a panic.
        g.update(&[(-500.0, 9000.0, 4000.0)]);
    }
}
