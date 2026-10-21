//! The input layer's view of the sim ([`SimView`], a `tak_input::WorldView`)
//! and the camera geometry shared by rendering and input ([`ViewGeom`]).
//! No Bevy types.

use tak_commands::{PlayerId, UnitDefId, UnitId};
use tak_input::{BuilderKind, Camera, UnitInfo, WorldView};
use tak_math::{Fx, Vec2};
use tak_sim::World;

/// TA's oblique projection: one screen pixel per world unit on both ground
/// axes (at zoom 1), and height shifts the image up the screen by half its
/// value: `screen_y = z - SHEAR * height` (**Believed**: Nanolathe's TA
/// research gives `screenY = worldZ - (worldY >> 1)`, and Kingdoms' terrain
/// art lines up with its height map under exactly this shift, see
/// `docs/client.md`).
pub const SHEAR: f32 = 0.5;

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
///
/// Projection (render space `x` east, `y` up, `z` south): screen x =
/// `(x - target.x) / zoom`, screen y (down) = `(z - SHEAR * y - target.z) /
/// zoom`, both from the screen centre. Rays run along `(0, -1, -SHEAR)`:
/// down and north. A point is nearer the viewer the larger `y + SHEAR * z`.
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
    /// Unit vector toward the viewer (up and south), in render space. Moving
    /// along it does not move a point on screen.
    pub fn back() -> [f32; 3] {
        let l = (1.0 + SHEAR * SHEAR).sqrt();
        [0.0, 1.0 / l, SHEAR / l]
    }

    /// The world direction that moves a point straight up the screen by one
    /// pixel at zoom 1 without changing its depth: `(0, k, -1) / (1 + k²)`.
    pub fn screen_up() -> [f32; 3] {
        let d = 1.0 + SHEAR * SHEAR;
        [0.0, SHEAR / d, -1.0 / d]
    }

    /// Ray through a screen pixel: a point on it (at height 0) and the
    /// shared unit direction.
    pub fn ray(&self, sx: f32, sy: f32) -> ([f32; 3], [f32; 3]) {
        let g = self.ground_at(sx, sy, 0.0);
        let b = Self::back();
        ([g[0], 0.0, g[1]], [-b[0], -b[1], -b[2]])
    }

    /// The world point at height `h` that projects to a screen pixel.
    pub fn ground_at(&self, sx: f32, sy: f32, h: f32) -> [f32; 2] {
        [
            self.target[0] + (sx - self.size[0] / 2.0) * self.zoom,
            self.target[1] + (sy - self.size[1] / 2.0) * self.zoom + SHEAR * h,
        ]
    }

    /// Screen pixel of a render-space point.
    pub fn to_screen(&self, p: [f32; 3]) -> [f32; 2] {
        [
            self.size[0] / 2.0 + (p[0] - self.target[0]) / self.zoom,
            self.size[1] / 2.0 + (p[2] - SHEAR * p[1] - self.target[1]) / self.zoom,
        ]
    }

    /// `tak-input`'s orthographic ground mapping. The ground is not
    /// foreshortened, so input pixels are screen pixels of the height-0
    /// plane; the client feeds pointer positions as the pixel of the
    /// terrain point under the mouse ([`ViewGeom::virtual_px`]).
    pub fn input_camera(&self) -> Camera {
        let o = self.ground_at(0.0, 0.0, 0.0);
        let fx = |v: f32| Fx::from_raw((v * 65536.0) as i32);
        Camera {
            origin: Vec2::new(fx(o[0]), fx(o[1])),
            world_per_px: fx(self.zoom),
            width_px: self.size[0] as i32,
            height_px: self.size[1] as i32,
        }
    }

    /// Input pixel of a world ground point (see [`ViewGeom::input_camera`]).
    pub fn virtual_px(&self, x: f32, z: f32) -> (i32, i32) {
        let o = self.ground_at(0.0, 0.0, 0.0);
        (
            ((x - o[0]) / self.zoom).round() as i32,
            ((z - o[1]) / self.zoom).round() as i32,
        )
    }

    /// The camera's clip-from-view matrix (column-major, glam layout) for a
    /// camera placed at `(target.x, 0, target.z)` with no rotation: the
    /// oblique projection above, with reverse depth (`1` near, `0` far)
    /// spanning `±depth` of `y + SHEAR * z` around the camera.
    pub fn clip_from_view(&self, depth: f32) -> [[f32; 4]; 4] {
        let (w, h) = (self.size[0] * self.zoom, self.size[1] * self.zoom);
        let dz = 1.0 / (2.0 * depth);
        [
            [2.0 / w, 0.0, 0.0, 0.0],
            [0.0, 2.0 * SHEAR / h, dz, 0.0],
            [0.0, -2.0 / h, SHEAR * dz, 0.0],
            [0.0, 0.0, 0.5, 1.0],
        ]
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
    fn height_shifts_up_by_half() {
        let g = geom();
        let a = g.to_screen([1000.0, 0.0, 800.0]);
        let b = g.to_screen([1000.0, 40.0, 800.0]);
        assert!((a[1] - b[1] - 20.0 / g.zoom).abs() < 1e-3);
        // Moving along the view ray keeps the pixel; screen_up moves it up.
        let d = ViewGeom::back();
        let c = g.to_screen([1000.0 + d[0] * 50.0, d[1] * 50.0, 800.0 + d[2] * 50.0]);
        assert!((c[0] - a[0]).abs() < 1e-3 && (c[1] - a[1]).abs() < 1e-3);
        let u = ViewGeom::screen_up();
        let e = g.to_screen([1000.0, u[1] * 10.0, 800.0 + u[2] * 10.0]);
        assert!((a[1] - e[1] - 10.0 / g.zoom).abs() < 1e-3, "{a:?} {e:?}");
    }

    #[test]
    fn clip_matrix_matches_to_screen() {
        let g = geom();
        let m = g.clip_from_view(4000.0);
        let p = [1100.0f32, 30.0, 700.0];
        let v = [p[0] - g.target[0], p[1], p[2] - g.target[1]];
        let clip = |r: usize| m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2] + m[3][r];
        let s = g.to_screen(p);
        let ndc = [s[0] / g.size[0] * 2.0 - 1.0, 1.0 - s[1] / g.size[1] * 2.0];
        assert!((clip(0) - ndc[0]).abs() < 1e-4 && (clip(1) - ndc[1]).abs() < 1e-4);
        assert!((clip(3) - 1.0).abs() < 1e-6);
        // Nearer (higher) points get larger depth, inside 0..1.
        assert!(m[1][2] > 0.0 && clip(2) > 0.0 && clip(2) < 1.0);
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
        // The ground is not foreshortened: input pixels are screen pixels.
        assert_eq!(vy, 200);
    }
}
