//! Per-movement-class terrain layers: a tier per attribute cell, folded
//! over the class footprint into a tier per footprint *anchor*, plus
//! connected components for cheap reachability. See `docs/pathfinding.md`.

use tak_math::{Fx, Vec2};

use crate::WorldPos;
use crate::map::{MapInfo, TerrainGrid, terrain};

/// Tier values (TA-derived, Nanolathe `04` §6.1): blocked, passable but
/// steep or next to an obstruction (costs [`super::STEEP_COST`] more), and
/// clear.
pub const BLOCKED: u8 = 0;
pub const STEEP: u8 = 1;
pub const CLEAR: u8 = 3;

/// World units per attribute cell.
const CELL: i64 = 16;
/// One cell in raw 16.16.
const CELL_RAW: i64 = CELL << 16;

/// A rectangle of attribute cells (16 world units each): top-left cell and
/// size. Used for dynamic obstacles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellRect {
    /// Left column.
    pub x: i32,
    /// Top row.
    pub z: i32,
    /// Width in cells.
    pub w: i32,
    /// Height in cells.
    pub h: i32,
}

impl CellRect {
    /// The whole `cols x rows` grid.
    pub fn whole(cols: u32, rows: u32) -> CellRect {
        CellRect {
            x: 0,
            z: 0,
            w: cols.min(i32::MAX as u32) as i32,
            h: rows.min(i32::MAX as u32) as i32,
        }
    }

    /// `(x0, x1, z0, z1)`, half-open, clipped to a `cols x rows` grid.
    pub fn clip(&self, cols: u32, rows: u32) -> (u32, u32, u32, u32) {
        let c = |lo: i32, len: i32, n: u32| {
            let lo64 = i64::from(lo).max(0);
            let hi64 = (i64::from(lo) + i64::from(len.max(0))).min(i64::from(n));
            (lo64.min(hi64.max(0)) as u32, hi64.max(0) as u32)
        };
        let (x0, x1) = c(self.x, self.w, cols);
        let (z0, z1) = c(self.z, self.h, rows);
        (x0, x1.max(x0), z0, z1.max(z0))
    }
}

/// The terrain limits that define a layer. Units whose [`super::Motion`]
/// reduces to the same key share one layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClassKey {
    pub fx: u8,
    pub fz: u8,
    pub max_slope: u8,
    pub bad_slope: u8,
    pub max_water_slope: u8,
    pub bad_water_slope: u8,
    pub max_water_depth: i32,
    pub min_water_depth: i32,
}

/// One movement class's layer over one map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassGrid {
    pub key: ClassKey,
    pub cols: u32,
    pub rows: u32,
    /// Tier of each attribute cell on its own (1 x 1).
    cell: Vec<u8>,
    /// Tier of each footprint anchor (top-left cell of the footprint).
    anchor: Vec<u8>,
    /// Connected component of each anchor (4-neighbour flood over
    /// non-blocked anchors); 0 for blocked anchors.
    comp: Vec<u32>,
}

/// The `GROUND2` limits the stored `PassGrid` flags were derived with;
/// a stored `STEEP`/`DEEP` flag is honoured only by classes at least that
/// strict (so hand-made synthetic flags still block ground units).
const STORED_MAX_SLOPE: u8 = 30;
const STORED_MAX_DEPTH: i32 = 20;

impl ClassGrid {
    /// Stamp the layer for `key` over `map` (whose terrain grid is `t`).
    pub fn build(key: ClassKey, map: &MapInfo, t: &TerrainGrid) -> ClassGrid {
        ClassGrid::build_with(key, map, t, &[])
    }

    /// [`ClassGrid::build`] with dynamic obstacles: a cell whose entry in
    /// `obstacles` (row-major, one per attribute cell; empty = none) is
    /// non-zero counts as holding a blocking feature (`docs/pathfinding.md`,
    /// "Dynamic obstacles").
    pub fn build_with(
        key: ClassKey,
        map: &MapInfo,
        t: &TerrainGrid,
        obstacles: &[u16],
    ) -> ClassGrid {
        let n = t.cols as usize * t.rows as usize;
        let mut g = ClassGrid {
            key,
            cols: t.cols,
            rows: t.rows,
            cell: vec![BLOCKED; n],
            anchor: vec![BLOCKED; n],
            comp: Vec::new(),
        };
        g.stamp(map, t, obstacles, CellRect::whole(t.cols, t.rows));
        g
    }

    /// Re-stamp after the obstacles changed inside `r`: its cells, every
    /// anchor whose footprint or one-cell ring touches it, then the
    /// connected components (whole map; a flood fill is cheap).
    pub fn restamp(&mut self, map: &MapInfo, t: &TerrainGrid, obstacles: &[u16], r: CellRect) {
        if t.cols == self.cols && t.rows == self.rows {
            self.stamp(map, t, obstacles, r);
        }
    }

    fn stamp(&mut self, map: &MapInfo, t: &TerrainGrid, obstacles: &[u16], r: CellRect) {
        let key = self.key;
        let (cols, rows) = (self.cols, self.rows);
        let n = cols as usize * rows as usize;
        let hg = &map.heights;
        let heights_match = hg.cols == cols && hg.rows == rows && hg.heights.len() == n;
        let h = |x: u32, z: u32| -> i32 {
            if heights_match {
                i32::from(hg.heights[z as usize * cols as usize + x as usize])
            } else {
                i32::from(hg.fill)
            }
        };
        let sea = map.sea_level;
        let (x0, x1, z0, z1) = r.clip(cols, rows);
        for z in z0..z1 {
            for x in x0..x1 {
                // Derived pair over the cell and its east, south and
                // south-east neighbours (the four corners), edge-guarded.
                let mut lo = h(x, z);
                let mut hi = lo;
                for (dx, dz) in [(1, 0), (0, 1), (1, 1)] {
                    let (nx, nz) = (x + dx, z + dz);
                    if nx < cols && nz < rows {
                        let v = h(nx, nz);
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                }
                let i = z as usize * cols as usize + x as usize;
                let mut flags = t.flags_of_cell(x, z);
                if obstacles.get(i).is_some_and(|&c| c > 0) {
                    flags |= terrain::FEATURE;
                }
                self.cell[i] = cell_tier(&key, flags, lo, hi, sea);
            }
        }
        let (fx, fz) = (i64::from(key.fx.max(1)), i64::from(key.fz.max(1)));
        // Anchors whose footprint (or its one-cell ring) reaches into `r`.
        let grown = CellRect {
            x: r.x.saturating_sub(fx as i32),
            z: r.z.saturating_sub(fz as i32),
            w: r.w.saturating_add(fx as i32 + 1),
            h: r.h.saturating_add(fz as i32 + 1),
        };
        let (ax0, ax1, az0, az1) = grown.clip(cols, rows);
        let cell = &self.cell;
        let at = |x: i64, z: i64| -> u8 {
            if x < 0 || z < 0 || x >= i64::from(cols) || z >= i64::from(rows) {
                BLOCKED
            } else {
                cell[z as usize * cols as usize + x as usize]
            }
        };
        let mut anchor = std::mem::take(&mut self.anchor);
        for z in i64::from(az0)..i64::from(az1) {
            for x in i64::from(ax0)..i64::from(ax1) {
                let mut m = CLEAR;
                'fp: for j in z..z + fz {
                    for i in x..x + fx {
                        m = m.min(at(i, j));
                        if m == BLOCKED {
                            break 'fp;
                        }
                    }
                }
                if m == CLEAR {
                    // Ring demotion: a clear footprint next to anything
                    // non-clear is steep.
                    let ring = (x - 1..=x + fx)
                        .any(|i| at(i, z - 1) < CLEAR || at(i, z + fz) < CLEAR)
                        || (z..z + fz).any(|j| at(x - 1, j) < CLEAR || at(x + fx, j) < CLEAR);
                    if ring {
                        m = STEEP;
                    }
                }
                anchor[z as usize * cols as usize + x as usize] = m;
            }
        }
        self.comp = components(&anchor, cols, rows);
        self.anchor = anchor;
    }

    /// Cells in the layer.
    pub fn len(&self) -> usize {
        self.anchor.len()
    }

    /// `true` for an empty layer (never, for a valid map).
    pub fn is_empty(&self) -> bool {
        self.anchor.is_empty()
    }

    fn idx(&self, a: (i32, i32)) -> Option<usize> {
        if a.0 < 0 || a.1 < 0 || a.0 as u32 >= self.cols || a.1 as u32 >= self.rows {
            None
        } else {
            Some(a.1 as usize * self.cols as usize + a.0 as usize)
        }
    }

    /// Anchor index for in-range anchors.
    pub fn index(&self, a: (i32, i32)) -> Option<u32> {
        self.idx(a).map(|i| i as u32)
    }

    /// Anchor of an index.
    pub fn coords(&self, i: u32) -> (i32, i32) {
        ((i % self.cols) as i32, (i / self.cols) as i32)
    }

    /// Anchor tier; out of range is blocked.
    pub fn tier(&self, a: (i32, i32)) -> u8 {
        self.idx(a).map_or(BLOCKED, |i| self.anchor[i])
    }

    /// Component of an anchor (0: blocked or out of range).
    pub fn comp(&self, a: (i32, i32)) -> u32 {
        self.idx(a).map_or(0, |i| self.comp[i])
    }

    /// The single-cell tier at a ground point (the cell containing it).
    pub fn cell_tier_at(&self, p: WorldPos) -> u8 {
        let c = (
            (i64::from(p.x.raw()).div_euclid(CELL_RAW)) as i32,
            (i64::from(p.y.raw()).div_euclid(CELL_RAW)) as i32,
        );
        self.idx(c).map_or(BLOCKED, |i| self.cell[i])
    }

    /// Raw offset from a unit's position to its footprint's top-left
    /// corner: `(f - 1) * 8` world units per axis, so the footprint is
    /// centred on the unit (TA's half-footprint bias).
    fn bias(&self) -> (i64, i64) {
        (
            (i64::from(self.key.fx.max(1)) - 1) * (8 << 16),
            (i64::from(self.key.fz.max(1)) - 1) * (8 << 16),
        )
    }

    /// The footprint anchor of a unit standing at `p` (may be out of range).
    pub fn anchor_of(&self, p: WorldPos) -> (i32, i32) {
        let (bx, bz) = self.bias();
        (
            (i64::from(p.x.raw()) - bx).div_euclid(CELL_RAW) as i32,
            (i64::from(p.y.raw()) - bz).div_euclid(CELL_RAW) as i32,
        )
    }

    /// [`Self::anchor_of`] clamped into the grid.
    pub fn anchor_clamped(&self, p: WorldPos) -> (i32, i32) {
        let a = self.anchor_of(p);
        (
            a.0.clamp(0, self.cols as i32 - 1),
            a.1.clamp(0, self.rows as i32 - 1),
        )
    }

    /// Where a unit stands when its footprint sits on anchor `a`: the
    /// footprint centre, `(2a + f) * 8` (TA-derived).
    pub fn anchor_world(&self, a: (i32, i32)) -> WorldPos {
        let f = |c: i32, s: u8| {
            Fx::from_int(c.saturating_mul(2).saturating_add(i32::from(s.max(1))) * 8)
        };
        Vec2::new(f(a.0, self.key.fx), f(a.1, self.key.fz))
    }

    /// The point closest to `p` among the positions whose anchor is `a`
    /// (one world unit inside the anchor's square).
    pub fn clamp_into(&self, p: WorldPos, a: (i32, i32)) -> WorldPos {
        let (bx, bz) = self.bias();
        let lo = |c: i32, b: i64| i64::from(c) * CELL_RAW + b + (1 << 16);
        let hi = |c: i32, b: i64| i64::from(c) * CELL_RAW + b + CELL_RAW - (1 << 16);
        let cl = |v: Fx, c: i32, b: i64| {
            Fx::from_raw(
                i64::from(v.raw())
                    .clamp(lo(c, b), hi(c, b))
                    .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            )
        };
        Vec2::new(cl(p.x, a.0, bx), cl(p.y, a.1, bz))
    }

    /// Straight-line walkability from `p` to `q` for this class: every
    /// anchor the unit's anchor passes through is passable, and a crossing
    /// exactly through an anchor corner needs both side anchors passable
    /// (no squeezing between diagonal blockers). Exact integer traversal.
    /// Returns the answer and the number of anchors visited (the work
    /// charged to the path budget).
    pub fn line_of_sight(&self, p: WorldPos, q: WorldPos) -> (bool, u32) {
        let (bx, bz) = self.bias();
        let (x0, z0) = (i64::from(p.x.raw()) - bx, i64::from(p.y.raw()) - bz);
        let (x1, z1) = (i64::from(q.x.raw()) - bx, i64::from(q.y.raw()) - bz);
        let mut c = (
            x0.div_euclid(CELL_RAW) as i32,
            z0.div_euclid(CELL_RAW) as i32,
        );
        let end = (
            x1.div_euclid(CELL_RAW) as i32,
            z1.div_euclid(CELL_RAW) as i32,
        );
        if self.tier(c) == BLOCKED {
            return (false, 1);
        }
        let (dx, dz) = (x1 - x0, z1 - z0);
        let (sx, sz) = (dx.signum() as i32, dz.signum() as i32);
        let limit = (end.0 - c.0).unsigned_abs() + (end.1 - c.1).unsigned_abs() + 2;
        let mut visited = 1u32;
        while visited <= limit {
            if c == end {
                return (true, visited);
            }
            let bxn = if sx > 0 {
                i64::from(c.0) + 1
            } else {
                i64::from(c.0)
            } * CELL_RAW;
            let bzn = if sz > 0 {
                i64::from(c.1) + 1
            } else {
                i64::from(c.1)
            } * CELL_RAW;
            // Compare the segment parameter of the next x and z boundary:
            // |bx - x0| / |dx| against |bz - z0| / |dz|.
            let ord = match (sx, sz) {
                (0, 0) => return (true, visited),
                (0, _) => core::cmp::Ordering::Greater,
                (_, 0) => core::cmp::Ordering::Less,
                _ => {
                    let tx = i128::from((bxn - x0).abs()) * i128::from(dz.abs());
                    let tz = i128::from((bzn - z0).abs()) * i128::from(dx.abs());
                    tx.cmp(&tz)
                }
            };
            match ord {
                core::cmp::Ordering::Less => c.0 += sx,
                core::cmp::Ordering::Greater => c.1 += sz,
                core::cmp::Ordering::Equal => {
                    if self.tier((c.0 + sx, c.1)) == BLOCKED
                        || self.tier((c.0, c.1 + sz)) == BLOCKED
                    {
                        return (false, visited);
                    }
                    c = (c.0 + sx, c.1 + sz);
                }
            }
            visited += 1;
            if self.tier(c) == BLOCKED {
                return (false, visited);
            }
        }
        (false, visited)
    }

    /// The anchor nearest `from` (by square rings, then octile distance,
    /// then row, then column) that satisfies `want`. Bounded by the grid.
    pub fn nearest(
        &self,
        from: (i32, i32),
        want: impl Fn(&ClassGrid, (i32, i32)) -> bool,
    ) -> Option<(i32, i32)> {
        let from = (
            from.0.clamp(0, self.cols as i32 - 1),
            from.1.clamp(0, self.rows as i32 - 1),
        );
        let max_r = self.cols.max(self.rows) as i32;
        for r in 0..=max_r {
            let mut best: Option<(u32, i32, i32)> = None;
            let mut consider = |x: i32, z: i32| {
                if self.idx((x, z)).is_none() || !want(self, (x, z)) {
                    return;
                }
                let (ax, az) = ((x - from.0).unsigned_abs(), (z - from.1).unsigned_abs());
                let d = 10 * ax.max(az) + 4 * ax.min(az);
                let cand = (d, z, x);
                if best.is_none_or(|b| cand < b) {
                    best = Some(cand);
                }
            };
            if r == 0 {
                consider(from.0, from.1);
            } else {
                for x in from.0 - r..=from.0 + r {
                    consider(x, from.1 - r);
                    consider(x, from.1 + r);
                }
                for z in from.1 - r + 1..from.1 + r {
                    consider(from.0 - r, z);
                    consider(from.0 + r, z);
                }
            }
            if let Some((_, z, x)) = best {
                return Some((x, z));
            }
        }
        None
    }

    /// May a unit of this class step from `from` to `to` (a move of less
    /// than one cell)? Staying on the same anchor is always allowed. A
    /// blocked destination anchor refuses the step, except that a unit
    /// standing on a blocked anchor may move onto any point whose own
    /// cell is passable (so units placed in an obstruction's margin can
    /// walk out, but never onto a cliff). A diagonal anchor change between
    /// passable anchors needs both side anchors passable.
    pub fn step_allowed(&self, from: WorldPos, to: WorldPos) -> bool {
        let (a, b) = (self.anchor_of(from), self.anchor_of(to));
        if a == b {
            return true;
        }
        let ta = self.tier(a);
        if self.tier(b) == BLOCKED {
            return ta == BLOCKED && self.cell_tier_at(to) != BLOCKED;
        }
        if ta != BLOCKED && a.0 != b.0 && a.1 != b.1 {
            // Same rule as the route's line of sight, so a smoothed leg is
            // always walkable.
            return self.line_of_sight(from, to).0;
        }
        true
    }
}

/// One cell's tier for a class (TA-derived classifier, Nanolathe `04`
/// §6.1 R-DOC04-B): feature → deep → shallow → slope.
fn cell_tier(key: &ClassKey, flags: u8, lo: i32, hi: i32, sea: i32) -> u8 {
    if flags & terrain::FEATURE != 0 {
        return BLOCKED;
    }
    let water = lo < sea || flags & terrain::WATER != 0;
    let deep = lo < sea.saturating_sub(key.max_water_depth)
        || (flags & terrain::DEEP != 0 && key.max_water_depth <= STORED_MAX_DEPTH);
    let shallow = hi > sea.saturating_sub(key.min_water_depth);
    if deep || shallow {
        return BLOCKED;
    }
    let slope = (hi - lo).clamp(0, 255) as u8;
    let (max, bad) = if water {
        (key.max_water_slope, key.bad_water_slope)
    } else {
        (key.max_slope, key.bad_slope)
    };
    if slope > max || (flags & terrain::STEEP != 0 && max <= STORED_MAX_SLOPE) {
        BLOCKED
    } else if slope <= bad {
        CLEAR
    } else {
        STEEP
    }
}

/// 4-neighbour connected components over non-blocked anchors. Diagonal
/// steps need both side anchors passable, so 4-connectivity is exactly
/// the search's connectivity.
fn components(anchor: &[u8], cols: u32, rows: u32) -> Vec<u32> {
    let mut comp = vec![0u32; anchor.len()];
    let mut next = 0u32;
    let mut stack = Vec::new();
    for start in 0..anchor.len() {
        if anchor[start] == BLOCKED || comp[start] != 0 {
            continue;
        }
        next += 1;
        comp[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, z) = ((i % cols as usize) as u32, (i / cols as usize) as u32);
            let mut visit = |j: usize| {
                if anchor[j] != BLOCKED && comp[j] == 0 {
                    comp[j] = next;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < cols {
                visit(i + 1);
            }
            if z > 0 {
                visit(i - cols as usize);
            }
            if z + 1 < rows {
                visit(i + cols as usize);
            }
        }
    }
    comp
}
