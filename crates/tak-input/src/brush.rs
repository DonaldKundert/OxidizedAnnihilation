//! Selection brush and drag-box geometry.

use tak_commands::{UnitSet, WorldPos};
use tak_math::Fx;

use crate::cursor::per_second;
use crate::tuning::Tuning;
use crate::world::{WorldView, fx_int};

/// Brush radius after it has been active for `active_ms`:
/// `brush_min + brush_growth_per_s * active_ms / 1000`, capped at `brush_max`.
pub fn brush_radius(active_ms: u32, t: &Tuning) -> Fx {
    let max = fx_int(t.brush_max);
    // Cap the time so the product cannot overflow before the clamp.
    let grown = fx_int(t.brush_min)
        .saturating_add(per_second(t.brush_growth_per_s, active_ms.min(600_000)));
    grown.min(max)
}

/// Own, selectable units whose centre lies within `radius` of `center`.
pub fn units_in_circle<W: WorldView + ?Sized>(world: &W, center: WorldPos, radius: Fx) -> UnitSet {
    let me = world.local_player();
    let mut v = Vec::new();
    world.for_each_unit(&mut |u| {
        if u.owner == me && u.selectable && u.pos.distance(center) <= radius {
            v.push(u.id);
        }
    });
    UnitSet::from(v)
}

/// A mouse drag from `a` to `b` as a circle: centre at the midpoint, radius
/// half the drag length.
pub fn drag_circle(a: WorldPos, b: WorldPos) -> (WorldPos, Fx) {
    let half = |p: Fx, q: Fx| p + (q - p).checked_div_int(2).unwrap_or(Fx::ZERO);
    let center = WorldPos::new(half(a.x, b.x), half(a.y, b.y));
    let radius = a.distance(b).checked_div_int(2).unwrap_or(Fx::ZERO);
    (center, radius)
}
