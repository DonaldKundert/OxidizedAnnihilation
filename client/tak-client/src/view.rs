//! The input layer's view of the sim ([`SimView`], a `tak_input::WorldView`)
//! and the camera geometry shared by rendering and input ([`ViewGeom`]).
//! No Bevy types.

use tak_commands::{PlayerId, SpellId, UnitDefId, UnitId};
use tak_input::{BuilderKind, Camera, Category, UnitInfo, WorldView};
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
    /// Client definition info by `UnitDefId` (content categories for the
    /// `Keys.TDF` select-category keys), when known.
    pub defs: Option<&'a [crate::game::DefInfo]>,
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
                under_construction: !u.build.is_complete(),
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

    fn can_place(&self, def: UnitDefId, pos: tak_commands::WorldPos) -> bool {
        use tak_sim::construction::placement::{check_site, site_center};
        let Some(d) = self.world.defs().get(def) else {
            return false;
        };
        check_site(self.world, def, site_center(d, pos), None).is_ok()
    }

    fn spells(&self, unit: UnitId) -> Vec<SpellId> {
        unit_spells(self.world, unit)
            .into_iter()
            .map(|s| s.id)
            .collect()
    }

    fn can_cast(&self, unit: UnitId, spell: SpellId) -> bool {
        unit_spells(self.world, unit)
            .iter()
            .any(|s| s.id == spell && s.ready)
    }

    fn is_idle(&self, unit: UnitId) -> bool {
        self.world.unit(unit).is_some_and(|u| u.orders.is_empty())
    }

    fn monarch(&self) -> Option<UnitId> {
        self.world
            .players()
            .iter()
            .find(|p| p.id == self.local)
            .and_then(|p| p.monarch)
            .filter(|id| self.world.unit(*id).is_some())
    }

    fn in_category(&self, unit: UnitId, category: Category) -> bool {
        let Some(u) = self.world.unit(unit) else {
            return false;
        };
        let Some(d) = self.world.defs().get(u.def) else {
            return false;
        };
        let cats = self
            .defs
            .and_then(|d| d.get(u.def.0 as usize))
            .map_or(&[][..], |d| d.categories.as_slice());
        let has = |name: &str| cats.iter().any(|c| c.eq_ignore_ascii_case(name));
        let monarch = self.monarch() == Some(unit) || has("Monarch");
        let armed = !d.combat.weapons.is_empty();
        match category {
            Category::Builder => d.build.builder && d.build.mobile && !d.build.canbuild.is_empty(),
            Category::Factory => {
                has("FACTORY")
                    || (d.build.builder && !d.build.mobile && !d.build.canbuild.is_empty())
            }
            Category::Monarch => monarch,
            Category::Naval => has("BOAT") || has("NAVAL"),
            Category::Troops => {
                armed && d.build.mobile && !has("BOAT") && !has("NAVAL") && !monarch
            }
            Category::Armed => (has("ATTACK") || armed) && !monarch,
            Category::Magic => has("MAGIC") || d.combat.weapons.iter().any(|w| w.is_spell()),
            Category::Melee => has("MELEE"),
            Category::Ballistic => has("BALLISTIC"),
            Category::Fly => has("FLY"),
        }
    }
}

/// One spell of a unit: a weapon slot with `manapershot > 0`
/// (`docs/combat.md`), cast with `SimCommand::CastSpell { spell:
/// SpellId(slot) }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpellInfo {
    /// `SpellId(weapon slot)`, 0-based.
    pub id: SpellId,
    /// The weapon's FBI `name` (e.g. "Meteor").
    pub name: String,
    /// `manapershot`.
    pub cost: i32,
    /// The caster's own mana covers the cost now.
    pub ready: bool,
}

/// `unit`'s spells in weapon-slot order (the menu order `tak-input` uses).
pub fn unit_spells(world: &World, unit: UnitId) -> Vec<SpellInfo> {
    let Some(d) = world.unit(unit).and_then(|u| world.defs().get(u.def)) else {
        return Vec::new();
    };
    let mana = tak_sim::economy::unit_mana(world, unit).map_or(Fx::ZERO, |m| m.mana);
    d.combat
        .weapons
        .iter()
        .enumerate()
        .filter(|(_, w)| w.is_spell())
        .map(|(slot, w)| SpellInfo {
            id: SpellId(slot as u32),
            name: w.name.clone(),
            cost: w.mana_per_shot,
            ready: mana >= Fx::from_int(w.mana_per_shot),
        })
        .collect()
}

/// The caster the mapper's cast actions use: the first own, live unit of
/// the selection (id order) that has spells, with its spells.
pub fn caster_spells(
    world: &World,
    local: PlayerId,
    selection: &tak_commands::UnitSet,
) -> Option<(UnitId, Vec<SpellInfo>)> {
    selection.iter().copied().find_map(|id| {
        world.unit(id).filter(|u| u.owner == local)?;
        let s = unit_spells(world, id);
        (!s.is_empty()).then_some((id, s))
    })
}

/// Radius for a radial wheel whose slot boxes (`sizes`, logical px) sit at
/// `dirs` (unit vectors, screen `+y` down) around the cursor: the smallest
/// (from `min`, in 2-px steps) at which no two boxes come closer than
/// `gap` and every box stays `clear` px away from the centre on at least
/// one axis, so the cursor and the boxes never overlap. Long labels and
/// many slots push it out.
pub fn wheel_radius(dirs: &[[f32; 2]], sizes: &[[f32; 2]], min: f32, gap: f32, clear: f32) -> f32 {
    let n = dirs.len().min(sizes.len());
    let fits = |r: f32| {
        let c: Vec<[f32; 2]> = dirs[..n].iter().map(|d| [d[0] * r, d[1] * r]).collect();
        for i in 0..n {
            let [w, h] = sizes[i];
            if c[i][0].abs() - w / 2.0 < clear && c[i][1].abs() - h / 2.0 < clear {
                return false;
            }
            for j in i + 1..n {
                let [w2, h2] = sizes[j];
                if (c[i][0] - c[j][0]).abs() < (w + w2) / 2.0 + gap
                    && (c[i][1] - c[j][1]).abs() < (h + h2) / 2.0 + gap
                {
                    return false;
                }
            }
        }
        true
    };
    let mut r = min;
    while r < 900.0 && !fits(r) {
        r += 2.0;
    }
    r
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
        self.virtual_px_at(x, z, 0.0)
    }

    /// [`ViewGeom::input_camera`] for the plane at height `h` instead of 0:
    /// with `h` the terrain height under the screen centre, the centre
    /// pixel maps exactly to the ground point drawn there, so a reticle
    /// fixed at the centre picks what it covers (the oblique view shifts
    /// higher ground up the screen).
    pub fn input_camera_at(&self, h: f32) -> Camera {
        let o = self.ground_at(0.0, 0.0, h);
        let fx = |v: f32| Fx::from_raw((v * 65536.0) as i32);
        Camera {
            origin: Vec2::new(fx(o[0]), fx(o[1])),
            world_per_px: fx(self.zoom),
            width_px: self.size[0] as i32,
            height_px: self.size[1] as i32,
        }
    }

    /// Input pixel of a world ground point for [`ViewGeom::input_camera_at`].
    pub fn virtual_px_at(&self, x: f32, z: f32, h: f32) -> (i32, i32) {
        let o = self.ground_at(0.0, 0.0, h);
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

/// The minimap's place on screen and the map it shows. The minimap is
/// screen-space art like the terrain mosaic, so it maps linearly to
/// *projected* world coordinates `(x, z - SHEAR * height)`, which are also
/// what a camera target means ([`ViewGeom::target`] is on the height-0
/// plane).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapGeom {
    /// The image: `(left, top, width, height)` in logical pixels.
    pub rect: [f32; 4],
    /// The panel around it (frame included), same units. Clicks anywhere
    /// on it belong to the minimap.
    pub panel: [f32; 4],
    /// Map size in world units (cells × 16).
    pub map: [f32; 2],
}

/// Largest minimap panel as a share of the viewport (width, height)
/// (**Chosen**: about TA:K's sidebar minimap at 640×480, a quarter of the
/// width; a third of the height keeps it clear of the HUD panel).
pub const MINIMAP_SHARE: [f32; 2] = [0.24, 0.36];
/// Panel size limits, logical pixels: never smaller than this box ...
pub const MINIMAP_MIN: [f32; 2] = [200.0, 160.0];
/// ... nor larger than this one.
pub const MINIMAP_MAX: [f32; 2] = [560.0, 560.0];
/// Gap between the panel and the window edge.
pub const MINIMAP_MARGIN: f32 = 8.0;
/// Frame between the panel edge and the image.
pub const MINIMAP_PAD: f32 = 3.0;

impl MinimapGeom {
    /// Lay the minimap out in the bottom-right corner of a `view`-sized
    /// window: the image (`image` pixels, any aspect: retail maps run from
    /// 512×107 to 129×431) is scaled to fit the largest box the window
    /// allows ([`MINIMAP_SHARE`], clamped to [`MINIMAP_MIN`] /
    /// [`MINIMAP_MAX`]) keeping its aspect, and the panel is the image plus
    /// a [`MINIMAP_PAD`] frame.
    pub fn fit(view: [f32; 2], image: (u32, u32), map: [f32; 2]) -> Self {
        let bw = (view[0] * MINIMAP_SHARE[0]).clamp(MINIMAP_MIN[0], MINIMAP_MAX[0]);
        let bh = (view[1] * MINIMAP_SHARE[1]).clamp(MINIMAP_MIN[1], MINIMAP_MAX[1]);
        let (iw, ih) = (image.0.max(1) as f32, image.1.max(1) as f32);
        let s = ((bw - 2.0 * MINIMAP_PAD) / iw).min((bh - 2.0 * MINIMAP_PAD) / ih);
        let (w, h) = ((iw * s).round().max(1.0), (ih * s).round().max(1.0));
        let (pw, ph) = (w + 2.0 * MINIMAP_PAD, h + 2.0 * MINIMAP_PAD);
        let (pl, pt) = (view[0] - pw - MINIMAP_MARGIN, view[1] - ph - MINIMAP_MARGIN);
        Self {
            rect: [pl + MINIMAP_PAD, pt + MINIMAP_PAD, w, h],
            panel: [pl, pt, pw, ph],
            map,
        }
    }

    /// Whether a screen pixel is on the minimap panel.
    pub fn contains(&self, sx: f32, sy: f32) -> bool {
        let [l, t, w, h] = self.panel;
        sx >= l && sy >= t && sx < l + w && sy < t + h
    }

    /// Minimap pixel (relative to its top-left) of a projected world point.
    pub fn to_mini(&self, x: f32, projected_z: f32) -> [f32; 2] {
        [
            x / self.map[0] * self.rect[2],
            projected_z / self.map[1] * self.rect[3],
        ]
    }

    /// Projected world point (a camera target) under a screen pixel on the
    /// minimap, clamped to the map.
    pub fn target_at(&self, sx: f32, sy: f32) -> [f32; 2] {
        let u = ((sx - self.rect[0]) / self.rect[2]).clamp(0.0, 1.0);
        let v = ((sy - self.rect[1]) / self.rect[3]).clamp(0.0, 1.0);
        [u * self.map[0], v * self.map[1]]
    }

    /// The camera's view as a minimap rectangle `(left, top, w, h)`,
    /// relative to the minimap's top-left.
    pub fn view_rect(&self, g: &ViewGeom) -> [f32; 4] {
        let (hw, hh) = (g.size[0] * g.zoom / 2.0, g.size[1] * g.zoom / 2.0);
        let a = self.to_mini(g.target[0] - hw, g.target[1] - hh);
        let b = self.to_mini(g.target[0] + hw, g.target[1] + hh);
        [a[0], a[1], b[0] - a[0], b[1] - a[1]]
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
    fn minimap_round_trip_and_view_rect() {
        let m = MinimapGeom {
            rect: [1000.0, 500.0, 200.0, 100.0],
            panel: [1000.0, 500.0, 200.0, 100.0],
            map: [3000.0, 1500.0],
        };
        assert!(m.contains(1100.0, 550.0) && !m.contains(990.0, 550.0));
        let t = m.target_at(1050.0, 525.0);
        assert!((t[0] - 750.0).abs() < 1e-3 && (t[1] - 375.0).abs() < 1e-3);
        let p = m.to_mini(t[0], t[1]);
        assert!((p[0] - 50.0).abs() < 1e-3 && (p[1] - 25.0).abs() < 1e-3);
        // Clicking puts that point at the screen centre: the view rectangle
        // is centred on it.
        let g = ViewGeom {
            target: t,
            zoom: 1.0,
            size: [300.0, 150.0],
        };
        let r = m.view_rect(&g);
        assert!((r[0] + r[2] / 2.0 - 50.0).abs() < 1e-3);
        assert!((r[1] + r[3] / 2.0 - 25.0).abs() < 1e-3);
        assert!((r[2] - 20.0).abs() < 1e-3 && (r[3] - 10.0).abs() < 1e-3);
    }

    #[test]
    fn minimap_fits_every_aspect() {
        // Square, the widest and the tallest retail minimaps, at three
        // window sizes: the image keeps its aspect, stays inside the box
        // and the window, and one side fills the box.
        for view in [[1280.0, 720.0], [1920.0, 1080.0], [3456.0, 2160.0]] {
            for image in [(446, 431), (512, 107), (129, 431), (432, 432)] {
                let m = MinimapGeom::fit(view, image, [3072.0, 3072.0]);
                let [l, t, w, h] = m.rect;
                let a = image.0 as f32 / image.1 as f32;
                assert!((w / h - a).abs() / a < 0.03, "{view:?} {image:?} {w}x{h}");
                let bw = (view[0] * MINIMAP_SHARE[0]).clamp(MINIMAP_MIN[0], MINIMAP_MAX[0]);
                let bh = (view[1] * MINIMAP_SHARE[1]).clamp(MINIMAP_MIN[1], MINIMAP_MAX[1]);
                let inner = [bw - 2.0 * MINIMAP_PAD, bh - 2.0 * MINIMAP_PAD];
                assert!(w <= inner[0] + 0.5 && h <= inner[1] + 0.5);
                assert!(w >= inner[0] - 1.0 || h >= inner[1] - 1.0);
                assert!(l > 0.0 && t > 0.0);
                assert!((l + w + MINIMAP_PAD + MINIMAP_MARGIN - view[0]).abs() < 1e-3);
                assert!((t + h + MINIMAP_PAD + MINIMAP_MARGIN - view[1]).abs() < 1e-3);
                assert!(m.contains(m.panel[0] + 1.0, m.panel[1] + 1.0));
                // The map's corners land on the image's corners.
                let c = m.to_mini(m.map[0], m.map[1]);
                assert!((c[0] - w).abs() < 1e-3 && (c[1] - h).abs() < 1e-3);
            }
        }
        // 1280x720, square map: about 260 px (was a fixed 230-px width).
        let m = MinimapGeom::fit([1280.0, 720.0], (446, 431), [3072.0, 3072.0]);
        assert!(m.rect[3] > 240.0, "{:?}", m.rect);
    }

    #[test]
    fn wheel_radius_separates_slots_and_clears_the_cursor() {
        let dirs = |n: usize| -> Vec<[f32; 2]> {
            (0..n)
                .map(|k| {
                    let a =
                        std::f32::consts::FRAC_PI_2 - k as f32 * std::f32::consts::TAU / n as f32;
                    [a.cos(), -a.sin()]
                })
                .collect()
        };
        // "Return fire" beside "Hold fire": the case that crowded.
        let wide = [[110.0, 26.0]; 8];
        let r8 = wheel_radius(&dirs(8), &wide, 60.0, 6.0, 24.0);
        let c: Vec<[f32; 2]> = dirs(8).iter().map(|d| [d[0] * r8, d[1] * r8]).collect();
        for i in 0..8 {
            assert!(c[i][0].abs() - 55.0 >= 24.0 || c[i][1].abs() - 13.0 >= 24.0);
            for j in i + 1..8 {
                assert!(
                    (c[i][0] - c[j][0]).abs() >= 116.0 || (c[i][1] - c[j][1]).abs() >= 32.0,
                    "{i} {j} at r {r8}"
                );
            }
        }
        // More slots or wider labels need a larger radius.
        let r12 = wheel_radius(&dirs(12), &[[110.0, 26.0]; 12], 60.0, 6.0, 24.0);
        let r8n = wheel_radius(&dirs(8), &[[50.0, 26.0]; 8], 60.0, 6.0, 24.0);
        assert!(r12 > r8 && r8 > r8n, "{r12} {r8} {r8n}");
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
