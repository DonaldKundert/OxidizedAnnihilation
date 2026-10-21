//! The input layer's view of the sim ([`SimView`], a `tak_input::WorldView`)
//! and the camera geometry shared by rendering and input ([`ViewGeom`]).
//! No Bevy types.

use tak_commands::{PlayerId, UnitDefId, UnitId};
use tak_input::{BuilderKind, Camera, UnitInfo, WorldView};
use tak_math::{Fx, Vec2};
use tak_sim::World;

/// Camera pitch below the horizon: `atan(2)`, so a unit of height rises
/// half as far on screen as a unit of ground recedes, as in TA's oblique
/// view (**Chosen**: the camera is a tilted orthographic one; the ground
/// is foreshortened by `sin(pitch)` = 0.894 on screen).
pub fn pitch() -> f32 {
    2.0f32.atan()
}

/// Read-only adapter from the sim world to `tak_input::WorldView`.
pub struct SimView<'a> {
    /// The world.
    pub world: &'a World,
    /// Local player.
    pub local: PlayerId,
}

/// Pick radius for a unit: half its footprint's larger side plus a margin.
pub fn pick_radius(fx: u8, fz: u8) -> f32 {
    f32::from(fx.max(fz).max(1)) * 8.0 + 6.0
}

impl WorldView for SimView<'_> {
    fn local_player(&self) -> PlayerId {
        self.local
    }

    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo)) {
        let defs = self.world.defs();
        for (id, u) in self.world.units() {
            let Some(d) = defs.get(u.def) else { continue };
            let builder =
                (d.build.builder && !d.build.canbuild.is_empty()).then_some(if d.build.mobile {
                    BuilderKind::Mobile
                } else {
                    BuilderKind::Factory
                });
            let r = pick_radius(d.footprintx, d.footprintz);
            f(&UnitInfo {
                id,
                pos: u.pos,
                radius: Fx::from_raw((r * 65536.0) as i32),
                owner: u.owner,
                def: u.def,
                selectable: !u.combat.is_dying(),
                builder,
            });
        }
    }

    fn is_enemy(&self, owner: PlayerId) -> bool {
        owner != self.local
    }

    fn build_options(&self, unit: UnitId) -> Vec<UnitDefId> {
        self.world
            .unit(unit)
            .and_then(|u| self.world.defs().get(u.def))
            .map(|d| d.build.canbuild.clone())
            .unwrap_or_default()
    }

    fn is_idle(&self, unit: UnitId) -> bool {
        self.world.unit(unit).is_some_and(|u| u.orders.is_empty())
    }
}

/// The camera, in world terms: the ground point at the screen centre, the
/// zoom and the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewGeom {
    /// World x/y (east/south) the screen centre looks at, on the height-0
    /// plane.
    pub target: [f32; 2],
    /// World units per logical pixel.
    pub zoom: f32,
    /// Viewport in logical pixels.
    pub size: [f32; 2],
}

impl ViewGeom {
    /// Unit vector from the target toward the camera (south and up), in
    /// render space (`x` east, `y` up, `z` south).
    pub fn back(&self) -> [f32; 3] {
        let p = pitch();
        [0.0, p.sin(), p.cos()]
    }

    /// Ray through a screen pixel: origin on the view plane through the
    /// target, and the shared forward direction.
    pub fn ray(&self, sx: f32, sy: f32) -> ([f32; 3], [f32; 3]) {
        let p = pitch();
        let (s, c) = (p.sin(), p.cos());
        let fwd = [0.0, -s, -c];
        let up = [0.0, c, -s];
        let a = (sx - self.size[0] / 2.0) * self.zoom;
        let b = (self.size[1] / 2.0 - sy) * self.zoom;
        let o = [self.target[0] + a, up[1] * b, self.target[1] + up[2] * b];
        (o, fwd)
    }

    /// Where a screen pixel's ray meets the plane `y = h`.
    pub fn ground_at(&self, sx: f32, sy: f32, h: f32) -> [f32; 2] {
        let (o, d) = self.ray(sx, sy);
        let t = (h - o[1]) / d[1];
        [o[0] + d[0] * t, o[2] + d[2] * t]
    }

    /// Screen pixel of a render-space point.
    pub fn to_screen(&self, p: [f32; 3]) -> [f32; 2] {
        let pt = pitch();
        let (s, c) = (pt.sin(), pt.cos());
        let rel = [p[0] - self.target[0], p[1], p[2] - self.target[1]];
        let a = rel[0];
        let b = rel[1] * c - rel[2] * s;
        [
            self.size[0] / 2.0 + a / self.zoom,
            self.size[1] / 2.0 - b / self.zoom,
        ]
    }

    /// `tak-input`'s orthographic ground mapping. The input layer works in
    /// "virtual" pixels where both axes have the same world scale: the
    /// height-0 ground point of screen pixel `(sx, sy)` is virtual pixel
    /// `(sx, sy / sin(pitch))`. The client feeds pointer positions as the
    /// virtual pixel of the terrain point under the mouse.
    pub fn input_camera(&self) -> Camera {
        let o = self.ground_at(0.0, 0.0, 0.0);
        let fx = |v: f32| Fx::from_raw((v * 65536.0) as i32);
        Camera {
            origin: Vec2::new(fx(o[0]), fx(o[1])),
            world_per_px: fx(self.zoom),
            width_px: self.size[0] as i32,
            height_px: (self.size[1] / pitch().sin()) as i32,
        }
    }

    /// Virtual input pixel of a world ground point (see
    /// [`ViewGeom::input_camera`]).
    pub fn virtual_px(&self, x: f32, z: f32) -> (i32, i32) {
        let o = self.ground_at(0.0, 0.0, 0.0);
        (
            ((x - o[0]) / self.zoom).round() as i32,
            ((z - o[1]) / self.zoom).round() as i32,
        )
    }
}

/// `Fx` to f32.
pub fn f(v: Fx) -> f32 {
    v.raw() as f32 / 65536.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom() -> ViewGeom {
        ViewGeom {
            target: [1000.0, 800.0],
            zoom: 0.5,
            size: [1280.0, 720.0],
        }
    }

    #[test]
    fn centre_looks_at_target() {
        let g = geom();
        let p = g.ground_at(640.0, 360.0, 0.0);
        assert!(
            (p[0] - 1000.0).abs() < 1e-3 && (p[1] - 800.0).abs() < 1e-3,
            "{p:?}"
        );
        let s = g.to_screen([1000.0, 0.0, 800.0]);
        assert!((s[0] - 640.0).abs() < 1e-3 && (s[1] - 360.0).abs() < 1e-3);
    }

    #[test]
    fn screen_round_trip_at_height() {
        let g = geom();
        let p = g.ground_at(100.0, 50.0, 40.0);
        let s = g.to_screen([p[0], 40.0, p[1]]);
        assert!(
            (s[0] - 100.0).abs() < 1e-2 && (s[1] - 50.0).abs() < 1e-2,
            "{s:?}"
        );
    }

    #[test]
    fn virtual_pixels_match_input_camera() {
        let g = geom();
        let cam = g.input_camera();
        let p = g.ground_at(300.0, 200.0, 0.0);
        let (vx, vy) = g.virtual_px(p[0], p[1]);
        let w = cam.screen_to_world(Vec2::from_ints(vx, vy));
        assert!((f(w.x) - p[0]).abs() < 1.0 && (f(w.y) - p[1]).abs() < 1.0);
        assert_eq!(vx, 300);
        // Ground recedes 1/sin(pitch) per screen pixel.
        assert!((vy as f32 - 200.0 / pitch().sin()).abs() <= 1.0);
    }
}
