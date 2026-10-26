//! Immobile-unit yard occupancy. TA research supplies the yard selection
//! table; its Kingdoms use is Believed, except `S` (Chosen always blocked).
use super::{CellRect, PathState};
use crate::construction::placement::Rect;
use crate::hash::StateHasher;
use crate::script::ScriptHost;
use crate::world::World;
use tak_commands::UnitId;
use tak_math::Fx;

const YARD_OPEN: i32 = 18;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Stamp {
    id: UnitId,
    open: bool,
    cells: Vec<CellRect>,
}

impl Stamp {
    pub(super) fn hash(&self, h: &mut StateHasher) {
        h.unit_id(self.id);
        h.bool(self.open);
        h.len(self.cells.len());
        for r in &self.cells {
            h.i32(r.x);
            h.i32(r.z);
            h.i32(r.w);
            h.i32(r.h);
        }
    }
}

/// TA yard bytes: permanent body, closed-only doors, open-only body,
/// and never occupied. `S` is the Kingdoms sacred-site extension.
fn blocks(c: u8, open: bool) -> bool {
    match c {
        b'.' | b'Y' | b'y' => false,
        b'c' | b'C' => !open,
        b'O' => open,
        _ => true,
    }
}

fn closing_occupied(world: &World, id: UnitId, old: bool, new: bool, r: Rect) -> bool {
    let Some(u) = world.unit(id) else {
        return false;
    };
    let Some(d) = world.defs().get(u.def) else {
        return false;
    };
    for (_, other) in world.units().filter(|(other, _)| *other != id) {
        let Some(od) = world.defs().get(other.def) else {
            continue;
        };
        if od.motion.canfly {
            continue;
        }
        let o = Rect::covering(other.pos, od.footprintx, od.footprintz);
        for z in r.z.max(o.z)..(r.z + r.h).min(o.z + o.h) {
            for x in r.x.max(o.x)..(r.x + r.w).min(o.x + o.w) {
                let c = d
                    .build
                    .yard_cell((x - r.x) as u32, (z - r.z) as u32, d.footprintx);
                if blocks(c, new) && !blocks(c, old) {
                    return true;
                }
            }
        }
    }
    false
}

/// Script callbacks need synchronous port readback: a COB may set the yard
/// and immediately branch on `get YARD_OPEN` before the next pathing tick.
/// Bit 0 denies closed, bit 1 denies open. Only transition-selected cells
/// matter; permanently occupied body cells are already accounted for.
pub(super) fn blocked_states(world: &World, id: UnitId) -> u8 {
    if world.map().terrain.is_none() {
        return 0;
    }
    let Some(u) = world.unit(id) else { return 0 };
    let Some(d) = world.defs().get(u.def) else {
        return 0;
    };
    if d.maxvelocity != Fx::ZERO
        || d.motion.canfly
        || !d
            .build
            .yardmap
            .bytes()
            .any(|c| matches!(c, b'c' | b'C' | b'O'))
    {
        return 0;
    }
    let r = Rect::around(u.pos, d.footprintx, d.footprintz);
    u8::from(closing_occupied(world, id, true, false, r))
        | (u8::from(closing_occupied(world, id, false, true, r)) << 1)
}

pub(super) fn sync(world: &mut World, scripts: &mut dyn ScriptHost, ps: &mut PathState) {
    let Some(t) = world.map().terrain.as_ref() else {
        return;
    };
    let (cols, rows) = (t.cols, t.rows);
    let mut stamps = Vec::new();
    for (id, u) in world.units() {
        let Some(d) = world.defs().get(u.def) else {
            continue;
        };
        if d.maxvelocity != Fx::ZERO || d.motion.canfly {
            continue;
        }
        let r = Rect::around(u.pos, d.footprintx, d.footprintz);
        let old = ps
            .structure_stamps
            .binary_search_by_key(&id, |s| s.id)
            .ok()
            .map(|i| ps.structure_stamps[i].open)
            .unwrap_or(false);
        let requested = u.build.is_complete()
            && scripts
                .port_value(id, YARD_OPEN)
                .map_or(u.build.factory.active, |v| v != 0);
        let open = if requested != old && closing_occupied(world, id, old, requested, r) {
            // A script polls the resulting port to retry its transition. No-script
            // factories use the same effective gate until the product clears.
            scripts.set_port(id, YARD_OPEN, i32::from(old));
            old
        } else {
            requested
        };
        let mut cells = Vec::new();
        for z in 0..r.h {
            let mut x = 0;
            while x < r.w {
                if !blocks(d.build.yard_cell(x as u32, z as u32, d.footprintx), open) {
                    x += 1;
                    continue;
                }
                let first = x;
                x += 1;
                while x < r.w && blocks(d.build.yard_cell(x as u32, z as u32, d.footprintx), open) {
                    x += 1;
                }
                cells.push(CellRect {
                    x: r.x + first,
                    z: r.z + z,
                    w: x - first,
                    h: 1,
                });
            }
        }
        stamps.push(Stamp { id, open, cells });
    }
    if stamps == ps.structure_stamps {
        return;
    }
    let mut dirty: Option<CellRect> = None;
    ps.structures.resize(cols as usize * rows as usize, 0);
    for (list, other, add) in [
        (&ps.structure_stamps, &stamps, false),
        (&stamps, &ps.structure_stamps, true),
    ] {
        for s in list {
            if other
                .binary_search_by_key(&s.id, |s| s.id)
                .ok()
                .is_some_and(|i| other[i] == *s)
            {
                continue;
            }
            for &r in &s.cells {
                let (x0, x1, z0, z1) = r.clip(cols, rows);
                if x0 >= x1 || z0 >= z1 {
                    continue;
                }
                for z in z0..z1 {
                    for x in x0..x1 {
                        let c = &mut ps.structures[(z * cols + x) as usize];
                        *c = if add {
                            c.saturating_add(1)
                        } else {
                            c.saturating_sub(1)
                        };
                    }
                }
                dirty = Some(match dirty {
                    None => r,
                    Some(d) => {
                        let x = d.x.min(r.x);
                        let z = d.z.min(r.z);
                        CellRect {
                            x,
                            z,
                            w: (d.x + d.w).max(r.x + r.w) - x,
                            h: (d.z + d.h).max(r.z + r.h) - z,
                        }
                    }
                });
            }
        }
    }
    ps.structure_stamps = stamps;
    if let Some(r) = dirty {
        let blockers = ps.combined_obstacles();
        let t = world.map().terrain.as_ref().expect("terrain checked");
        for g in &mut ps.grids {
            g.restamp(world.map(), t, &blockers, r);
        }
        ps.invalidate_search(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn yard_selection_table() {
        for c in b"ofwGS" {
            assert!(blocks(*c, false) && blocks(*c, true));
        }
        for c in b"cC" {
            assert!(blocks(*c, false) && !blocks(*c, true));
        }
        for c in b".Yy" {
            assert!(!blocks(*c, false) && !blocks(*c, true));
        }
        assert!(!blocks(b'O', false) && blocks(b'O', true));
    }
}
