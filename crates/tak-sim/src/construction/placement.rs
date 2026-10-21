//! Where things may be built: footprint snapping, the site check and the
//! builder range test. Footprint squares are 16 world units (one TNT
//! attribute cell). See `docs/construction.md`.

use tak_math::{Fx, Vec2};

use crate::defs::UnitDef;
use crate::map::{MapInfo, terrain};
use crate::world::World;
use crate::{UnitDefId, UnitId, WorldPos};

/// World units per footprint square. **Known** for TA (Nanolathe: "half of
/// the sixteen world units per footprint cell"); Kingdoms footprints line
/// up with its 16-unit attribute cells.
pub const SQUARE: i32 = 16;

/// Footprint rectangle in map cells: anchor (top-left) cell and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Anchor column.
    pub x: i32,
    /// Anchor row.
    pub z: i32,
    /// Width in cells.
    pub w: i32,
    /// Height in cells.
    pub h: i32,
}

impl Rect {
    /// The footprint of a `fx x fz` definition centred near `p`: the
    /// anchor is the nearest cell to `p - (fx, fz) * 8` (**Believed**, TA
    /// snaps the position "to a cell index biased by half its extent").
    pub fn around(p: WorldPos, fx: u8, fz: u8) -> Rect {
        let (w, h) = (i32::from(fx.max(1)), i32::from(fz.max(1)));
        let snap = |v: Fx, n: i32| {
            v.to_int_floor()
                .saturating_sub(n * SQUARE / 2)
                .saturating_add(SQUARE / 2)
                .div_euclid(SQUARE)
        };
        Rect {
            x: snap(p.x, w),
            z: snap(p.y, h),
            w,
            h,
        }
    }

    /// The footprint of a unit standing at `p`, without snapping (cells it
    /// overlaps). Used for occupancy of mobile units.
    pub fn covering(p: WorldPos, fx: u8, fz: u8) -> Rect {
        Rect::around(p, fx, fz)
    }

    /// Centre in world units.
    pub fn center(&self) -> WorldPos {
        Vec2::from_ints(
            self.x * SQUARE + self.w * SQUARE / 2,
            self.z * SQUARE + self.h * SQUARE / 2,
        )
    }

    /// Cells this rectangle shares with `o`, as absolute cells.
    fn overlap(&self, o: &Rect) -> Option<Rect> {
        let x0 = self.x.max(o.x);
        let z0 = self.z.max(o.z);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let z1 = (self.z + self.h).min(o.z + o.h);
        (x0 < x1 && z0 < z1).then_some(Rect {
            x: x0,
            z: z0,
            w: x1 - x0,
            h: z1 - z0,
        })
    }
}

/// Map size in cells.
fn map_cells(map: &MapInfo) -> (i32, i32) {
    match &map.terrain {
        Some(t) => (t.cols as i32, t.rows as i32),
        None => (map.width.div_euclid(SQUARE), map.height.div_euclid(SQUARE)),
    }
}

fn terrain_of_cell(map: &MapInfo, cx: i32, cz: i32) -> u8 {
    match &map.terrain {
        Some(t) if cx >= 0 && cz >= 0 => t.flags_of_cell(cx as u32, cz as u32),
        Some(_) => terrain::BLOCKS,
        None => 0,
    }
}

/// `true` if the yard character occupies its cell. `.` leaves the cell
/// free (no checks, no occupancy); everything else occupies it.
fn occupies(c: u8) -> bool {
    c != b'.'
}

/// Snapped site centre for building `def` at `p`.
pub fn site_center(def: &UnitDef, p: WorldPos) -> WorldPos {
    Rect::around(p, def.footprintx, def.footprintz).center()
}

/// Why a site was refused (for tests and tools).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteError {
    /// Touches the map's outermost row or column, or leaves the map
    /// (**Known** TA bounds: anchor `>= 1`, far edge `< map size`).
    Bounds,
    /// A covered cell is too steep, too deep, or holds a blocking feature.
    Terrain,
    /// A structure or a frame already occupies a covered cell.
    Occupied,
}

/// The TA building validator, reduced to what the sim knows: every cell
/// the yard map occupies must be open terrain (not `STEEP`/`DEEP`, and not
/// `FEATURE` unless the yard cell is `S`, a lodestone site that sits on its
/// own sacred-site feature), and no structure or frame may occupy it.
/// Mobile units do not block placement (**Chosen**: TA's ground word also
/// holds mobile units, but players expect builders to start next to their
/// army; units standing on a new frame keep their place). Whether an `S`
/// cell actually holds a sacred site is not checked (**Unknown** until the
/// map carries feature identities).
pub fn check_site(
    world: &World,
    def_id: UnitDefId,
    center: WorldPos,
    ignore: Option<UnitId>,
) -> Result<(), SiteError> {
    let Some(def) = world.defs().get(def_id) else {
        return Err(SiteError::Bounds);
    };
    let r = Rect::around(center, def.footprintx, def.footprintz);
    let (cols, rows) = map_cells(world.map());
    if r.x < 1 || r.z < 1 || r.x + r.w >= cols || r.z + r.h >= rows {
        return Err(SiteError::Bounds);
    }
    for dz in 0..r.h {
        for dx in 0..r.w {
            let c = def.build.yard_cell(dx as u32, dz as u32, def.footprintx);
            if !occupies(c) {
                continue;
            }
            let mut blocks = terrain::DEEP | terrain::STEEP;
            if c != b'S' {
                blocks |= terrain::FEATURE;
            }
            if terrain_of_cell(world.map(), r.x + dx, r.z + dz) & blocks != 0 {
                return Err(SiteError::Terrain);
            }
        }
    }
    for (id, u) in world.units() {
        if Some(id) == ignore {
            continue;
        }
        let Some(od) = world.defs().get(u.def) else {
            continue;
        };
        let structure = od.maxvelocity == Fx::ZERO || !u.build.is_complete();
        if !structure {
            continue;
        }
        let or = Rect::around(u.pos, od.footprintx, od.footprintz);
        let Some(ov) = r.overlap(&or) else {
            continue;
        };
        for z in ov.z..ov.z + ov.h {
            for x in ov.x..ov.x + ov.w {
                let mine = def
                    .build
                    .yard_cell((x - r.x) as u32, (z - r.z) as u32, def.footprintx);
                let theirs =
                    od.build
                        .yard_cell((x - or.x) as u32, (z - or.z) as u32, od.footprintx);
                if occupies(mine) && occupies(theirs) {
                    return Err(SiteError::Occupied);
                }
            }
        }
    }
    Ok(())
}

/// A factory's exit check: no unit other than the factory may overlap the
/// product's snapped footprint at `at` (**Known** TA: the exit spot is
/// area-validated against the ground word, which holds every unit,
/// including the previous product still on the pad; a refusal retries
/// silently every 15 ticks). Map edges do not refuse here: the spot lies
/// inside the factory.
pub fn exit_clear(world: &World, def_id: UnitDefId, at: WorldPos, factory: UnitId) -> bool {
    let Some(def) = world.defs().get(def_id) else {
        return false;
    };
    let r = Rect::around(at, def.footprintx, def.footprintz);
    world.units().all(|(id, u)| {
        if id == factory {
            return true;
        }
        let Some(od) = world.defs().get(u.def) else {
            return true;
        };
        r.overlap(&Rect::covering(u.pos, od.footprintx, od.footprintz))
            .is_none()
    })
}

/// Half-diagonal pad of a footprint in whole world units:
/// `trunc(8 * hypot(fx, fz))`, computed exactly as `isqrt(64 (fx² + fz²))`.
/// TA's floating-point routine can come out one lower when the hypotenuse
/// is a whole number (Nanolathe: `(20, 99)` gives 807, exact is 808); no
/// Kingdoms footprint hits that case closely enough to matter.
pub fn pad(fx: u8, fz: u8) -> i32 {
    let (x, z) = (u64::from(fx.max(1)), u64::from(fz.max(1)));
    (64 * (x * x + z * z)).isqrt() as i32
}

/// TA's build range test (**Known** for TA, Nanolathe R-WORK-01 §2):
/// centre distance (whole units, truncated) minus both footprint pads is
/// at most `builddistance`, inclusive.
pub fn in_range(
    builder_pos: WorldPos,
    builder: &UnitDef,
    site: WorldPos,
    target: &UnitDef,
) -> bool {
    let d = builder_pos.distance(site).to_int_floor();
    let gap = d
        .saturating_sub(pad(builder.footprintx, builder.footprintz))
        .saturating_sub(pad(target.footprintx, target.footprintz));
    gap <= builder.build.builddistance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapping_and_pads() {
        let r = Rect::around(Vec2::from_ints(100, 100), 2, 2);
        assert_eq!((r.x, r.z), (5, 5));
        assert_eq!(r.center(), Vec2::from_ints(96, 96));
        let r = Rect::around(Vec2::from_ints(100, 100), 3, 1);
        assert_eq!((r.x, r.z, r.w, r.h), (5, 6, 3, 1));
        assert_eq!(r.center(), Vec2::from_ints(104, 104));
        assert_eq!(pad(1, 1), 11);
        assert_eq!(pad(7, 12), 111);
        assert_eq!(pad(20, 99), 808, "exact; TA's float routine gives 807");
    }

    #[test]
    fn overlap() {
        let a = Rect {
            x: 0,
            z: 0,
            w: 4,
            h: 4,
        };
        let b = Rect {
            x: 3,
            z: 2,
            w: 4,
            h: 4,
        };
        assert_eq!(
            a.overlap(&b),
            Some(Rect {
                x: 3,
                z: 2,
                w: 1,
                h: 2
            })
        );
        let c = Rect {
            x: 4,
            z: 0,
            w: 1,
            h: 1,
        };
        assert_eq!(a.overlap(&c), None);
    }
}
