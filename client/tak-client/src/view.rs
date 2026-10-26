//! The input layer's view of the sim ([`SimView`], a `tak_input::WorldView`)
//! and the camera geometry shared by rendering and input ([`ViewGeom`]).
//! No Bevy types.

use tak_commands::{PlayerId, SpellId, UnitDefId, UnitId};
use tak_input::{BuilderKind, Camera, Category, PlacementCandidate, UnitInfo, WorldView};
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
    /// The local player's fog of war: enemy units outside current sight
    /// are not part of the view at all, so picking, aim assist, the
    /// reticle snap, brushes, same-type select and context orders never see
    /// them (an order onto ground where a hidden enemy stands is a move).
    /// `None`: everything is visible (no fog).
    pub fog: Option<&'a crate::fog::FogGrid>,
}

impl SimView<'_> {
    /// `true` if the local player can currently see a unit of `owner` at
    /// `pos`: own and allied units always, enemies only inside current
    /// sight.
    pub fn sees(&self, owner: PlayerId, pos: tak_commands::WorldPos) -> bool {
        if !self.is_enemy(owner) {
            return true;
        }
        self.fog
            .is_none_or(|g| g.at(f(pos.x), f(pos.y)) == crate::fog::FogCell::Seen)
    }

    /// Shared visibility and metadata for enumeration and exact-ID lookup.
    fn unit_info(&self, id: UnitId, u: &tak_sim::world::Unit) -> Option<UnitInfo> {
        let d = self.world.defs().get(u.def)?;
        if !self.sees(u.owner, u.pos) {
            return None;
        }
        let builder = d.build.builder.then_some(if d.build.mobile {
            BuilderKind::Mobile
        } else {
            BuilderKind::Factory
        });
        let r = pick_radius(d.footprintx, d.footprintz);
        Some(UnitInfo {
            id,
            pos: u.pos,
            radius: Fx::from_raw((r * 65536.0) as i32),
            owner: u.owner,
            def: u.def,
            selectable: !u.combat.is_dying(),
            builder,
            under_construction: !u.build.is_complete(),
        })
    }
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
        for (id, u) in self.world.units() {
            if let Some(info) = self.unit_info(id, u) {
                f(&info);
            }
        }
    }

    fn unit(&self, id: UnitId) -> Option<UnitInfo> {
        // Avoid a full-world scan per queued target while retaining the
        // exact same generation, fog and selectability rules as enumeration.
        self.world.unit(id).and_then(|u| self.unit_info(id, u))
    }

    fn is_enemy(&self, owner: PlayerId) -> bool {
        self.world.are_hostile(self.local, owner)
    }

    fn can_heal(&self, unit: UnitId) -> bool {
        self.world.unit(unit).is_some_and(|u| u.owner == self.local)
            && tak_sim::construction::can_heal(self.world, unit)
    }

    fn build_options(&self, unit: UnitId) -> Vec<UnitDefId> {
        self.world
            .unit(unit)
            .and_then(|u| self.world.defs().get(u.def))
            .map(|d| d.build.canbuild.clone())
            .unwrap_or_default()
    }

    fn can_factory_control(&self, unit: UnitId) -> bool {
        self.world.unit(unit).is_some_and(|u| u.owner == self.local)
            && tak_sim::construction::can_factory_control(self.world, unit)
    }

    fn can_factory_guard_target(&self, target: UnitId) -> bool {
        self.world.unit(target).is_some_and(|u| {
            !u.combat.is_dying()
                && (u.combat.max_health == 0 || u.combat.health > 0)
                && !self.world.are_hostile(self.local, u.owner)
        })
    }

    fn factory_repeat(&self, unit: UnitId) -> Option<UnitDefId> {
        self.world
            .unit(unit)
            .filter(|u| u.owner == self.local)
            .and_then(|_| tak_sim::construction::factory_repeat(self.world, unit))
    }

    fn production_count(&self, unit: UnitId, def: UnitDefId) -> u32 {
        if self.world.unit(unit).is_some_and(|u| u.owner == self.local) {
            tak_sim::construction::production_count(self.world, unit, def)
        } else {
            0
        }
    }

    fn can_place(&self, def: UnitDefId, pos: tak_commands::WorldPos) -> bool {
        use tak_sim::construction::placement::{check_site, site_center};
        let Some(d) = self.world.defs().get(def) else {
            return false;
        };
        check_site(self.world, def, site_center(d, pos), None).is_ok()
    }

    fn placement_candidate(
        &self,
        def: UnitDefId,
        pos: tak_commands::WorldPos,
    ) -> Option<PlacementCandidate> {
        use tak_sim::construction::placement::{Rect, SQUARE, check_site, site_center};
        let d = self.world.defs().get(def)?;
        let center = site_center(d, pos);
        check_site(self.world, def, center, None).ok()?;
        let r = Rect::around(center, d.footprintx, d.footprintz);
        // Reserve the whole footprint for drag spacing, including open yard
        // cells. The half-open bounds let adjacent footprints touch.
        Some(PlacementCandidate {
            center,
            min: Vec2::from_ints(r.x * SQUARE, r.z * SQUARE),
            max: Vec2::from_ints((r.x + r.w) * SQUARE, (r.z + r.h) * SQUARE),
        })
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
        self.world.unit(unit).is_some_and(|u| {
            u.build.is_complete()
                && !u.combat.is_dying()
                && u.orders.is_empty()
                && u.build.factory_repeat.is_none()
                && u.build.factory.product.is_none()
        })
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
            Category::Builder => d.build.builder && d.build.mobile,
            Category::Factory => has("FACTORY") || (d.build.builder && !d.build.mobile),
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
    /// Mana in this caster's own pool, not the player's build resource.
    pub mana: Fx,
    /// The caster's own mana covers the cost now.
    pub ready: bool,
}

impl SpellInfo {
    /// The whole mana still needed, rounded up so an almost-ready spell
    /// never misleadingly says "short 0 mana".
    pub fn shortfall(&self) -> i32 {
        Fx::from_int(self.cost)
            .saturating_sub(self.mana)
            .max(Fx::ZERO)
            .to_int_ceil()
    }

    /// Shared detail for mouse help and the controller wheel.
    pub fn mana_text(&self) -> String {
        let have = self.mana.max(Fx::ZERO).to_int_floor();
        format!("{} mana (have {have})", self.cost)
    }

    /// Shared shortage text; a ready spell has no warning.
    pub fn shortfall_text(&self) -> Option<String> {
        (!self.ready).then(|| format!("short {} mana", self.shortfall()))
    }
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
            mana,
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

    /// [`MinimapGeom::fit`] above a bottom-right panel `reserved` px tall
    /// (the Classic TA:K side panel): the box is the same share of the
    /// window but no taller than the space between the top bar (34 px) and
    /// the panel, and it sits right above the panel.
    pub fn fit_above(view: [f32; 2], image: (u32, u32), map: [f32; 2], reserved: f32) -> Self {
        let avail = (view[1] - reserved - 34.0 - 2.0 * MINIMAP_MARGIN).max(40.0);
        let bw = (view[0] * MINIMAP_SHARE[0]).clamp(MINIMAP_MIN[0], MINIMAP_MAX[0]);
        let bh = (view[1] * MINIMAP_SHARE[1])
            .clamp(MINIMAP_MIN[1], MINIMAP_MAX[1])
            .min(avail);
        let (iw, ih) = (image.0.max(1) as f32, image.1.max(1) as f32);
        let sc = ((bw - 2.0 * MINIMAP_PAD) / iw).min((bh - 2.0 * MINIMAP_PAD) / ih);
        let (w, h) = ((iw * sc).round().max(1.0), (ih * sc).round().max(1.0));
        let (pw, ph) = (w + 2.0 * MINIMAP_PAD, h + 2.0 * MINIMAP_PAD);
        let (pl, pt) = (
            view[0] - pw - MINIMAP_MARGIN,
            view[1] - reserved - ph - MINIMAP_MARGIN,
        );
        Self {
            rect: [pl + MINIMAP_PAD, pt + MINIMAP_PAD, w, h],
            panel: [pl, pt, pw, ph],
            map,
        }
    }

    /// The full-screen Crystal Ball (TA Tab): the same minimap, centred
    /// and as large as fits in 88 % of the window width and 80 % of its
    /// height, keeping its aspect.
    pub fn full(view: [f32; 2], image: (u32, u32), map: [f32; 2]) -> Self {
        let (bw, bh) = (view[0] * 0.88, view[1] * 0.80);
        let (iw, ih) = (image.0.max(1) as f32, image.1.max(1) as f32);
        let s = ((bw - 2.0 * MINIMAP_PAD) / iw).min((bh - 2.0 * MINIMAP_PAD) / ih);
        let (w, h) = ((iw * s).round().max(1.0), (ih * s).round().max(1.0));
        let (pw, ph) = (w + 2.0 * MINIMAP_PAD, h + 2.0 * MINIMAP_PAD);
        let (pl, pt) = ((view[0] - pw) / 2.0, (view[1] - ph) / 2.0);
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

    fn placement_view(world: &World) -> SimView<'_> {
        SimView {
            world,
            local: PlayerId(0),
            defs: None,
            fog: None,
        }
    }

    fn world_hash(world: &World) -> u64 {
        let mut hash = tak_sim::hash::StateHasher::new();
        world.hash_into(&mut hash);
        hash.finish()
    }

    #[test]
    fn placement_candidate_snaps_even_and_odd_whole_footprints() {
        for (footprint, request, center, min, max) in [
            ((2, 4), (103, 103), (96, 96), (80, 64), (112, 128)),
            ((2, 4), (104, 104), (112, 112), (96, 80), (128, 144)),
            ((3, 1), (111, 111), (104, 104), (80, 96), (128, 112)),
            ((3, 1), (112, 112), (120, 120), (96, 112), (144, 128)),
        ] {
            let mut scenario = tak_sim::testing::example_scenario(1, 1);
            scenario.unit_defs[0].footprintx = footprint.0;
            scenario.unit_defs[0].footprintz = footprint.1;
            // Sparse yard maps still reserve the entire rectangular footprint.
            scenario.unit_defs[0].build.yardmap = "o".to_owned()
                + &".".repeat(usize::from(footprint.0) * usize::from(footprint.1) - 1);
            let world = World::new(&scenario).unwrap();
            let before = world_hash(&world);
            let view = placement_view(&world);
            let pos = Vec2::new(
                Fx::from_raw((request.0 << 16) + 65_535),
                Fx::from_raw((request.1 << 16) + 32_768),
            );
            let candidate = view.placement_candidate(UnitDefId(0), pos).unwrap();
            assert_eq!(candidate.center, Vec2::from_ints(center.0, center.1));
            assert_eq!(candidate.min, Vec2::from_ints(min.0, min.1));
            assert_eq!(candidate.max, Vec2::from_ints(max.0, max.1));
            assert_eq!(
                candidate.max.x - candidate.min.x,
                Fx::from_int(i32::from(footprint.0) * 16)
            );
            assert_eq!(
                candidate.max.y - candidate.min.y,
                Fx::from_int(i32::from(footprint.1) * 16)
            );
            assert!(view.can_place(UnitDefId(0), pos));
            assert_eq!(world_hash(&world), before);
        }
    }

    #[test]
    fn placement_candidate_rejects_unknown_defs_negative_and_map_edge_sites() {
        let scenario = tak_sim::testing::example_scenario(1, 1);
        let world = World::new(&scenario).unwrap();
        let before = world_hash(&world);
        let view = placement_view(&world);
        assert!(
            view.placement_candidate(UnitDefId(u32::MAX), Vec2::from_ints(96, 96))
                .is_none()
        );
        for (x, y) in [
            (-16, 96),
            (96, -16),
            (16, 96),
            (96, 16),
            (2032, 96),
            (96, 2032),
        ] {
            let pos = Vec2::from_ints(x, y);
            assert!(
                view.placement_candidate(UnitDefId(0), pos).is_none(),
                "{pos:?}"
            );
            assert!(!view.can_place(UnitDefId(0), pos));
        }
        for (x, y) in [(32, 32), (2016, 2016)] {
            assert!(
                view.placement_candidate(UnitDefId(0), Vec2::from_ints(x, y))
                    .is_some()
            );
        }
        assert_eq!(world_hash(&world), before);
    }

    #[test]
    fn placement_candidate_checks_every_occupied_terrain_cell() {
        use tak_sim::map::{TerrainGrid, terrain};
        for blocked in [terrain::DEEP, terrain::STEEP, terrain::FEATURE] {
            let mut scenario = tak_sim::testing::example_scenario(1, 1);
            let mut flags = vec![0; 128 * 128];
            // At (96, 96), a 2x2 footprint covers cells [5, 7) on each axis.
            flags[6 * 128 + 6] = blocked;
            scenario.map.terrain = Some(TerrainGrid {
                cols: 128,
                rows: 128,
                cell_size: 16,
                flags,
            });
            let world = World::new(&scenario).unwrap();
            let before = world_hash(&world);
            let view = placement_view(&world);
            assert!(
                view.placement_candidate(UnitDefId(0), Vec2::from_ints(96, 96))
                    .is_none()
            );
            assert!(
                view.placement_candidate(UnitDefId(0), Vec2::from_ints(128, 96))
                    .is_some(),
                "a footprint adjacent to the blocked one is still legal"
            );
            assert_eq!(world_hash(&world), before);
        }
    }

    #[test]
    fn placement_candidate_obeys_structure_and_frame_occupancy_without_mutation() {
        for (stationary, frame) in [(true, false), (false, true), (false, false)] {
            let mut scenario = tak_sim::testing::example_scenario(1, 1);
            if stationary {
                scenario.unit_defs[0].maxvelocity = Fx::ZERO;
            }
            let mut world = World::new(&scenario).unwrap();
            let pos = Vec2::from_ints(96, 96);
            let unit = world
                .spawn(UnitDefId(0), PlayerId(0), pos, tak_math::Angle::ZERO)
                .unwrap();
            if frame {
                world.unit_mut(unit).unwrap().build.frame =
                    Some(tak_sim::construction::Frame::new(10));
            }
            let before = world_hash(&world);
            let view = placement_view(&world);
            assert_eq!(
                view.placement_candidate(UnitDefId(0), pos).is_some(),
                !stationary && !frame,
                "completed mobile units do not block placement; structures and owned frames do"
            );
            assert!(
                view.placement_candidate(UnitDefId(0), Vec2::from_ints(128, 96))
                    .is_some(),
                "touching the occupied footprint's exclusive edge is legal"
            );
            assert_eq!(world_hash(&world), before);
            assert_eq!(world.unit_count(), 1);
        }
    }

    #[test]
    fn factory_guard_view_uses_teams_and_rejects_dead_or_stale_targets() {
        let mut scenario = tak_sim::testing::example_scenario(1, 3);
        scenario.victory.teams = vec![Some(0), Some(0), Some(1)];
        let mut world = World::new(&scenario).unwrap();
        let target = world
            .spawn(
                UnitDefId(0),
                PlayerId(1),
                Vec2::from_ints(300, 300),
                tak_math::Angle::ZERO,
            )
            .unwrap();
        let allowed = |world: &World, target| {
            let view = SimView {
                world,
                local: PlayerId(0),
                defs: None,
                fog: None,
            };
            assert!(!view.is_enemy(PlayerId(1)));
            assert!(view.is_enemy(PlayerId(2)));
            view.can_factory_guard_target(target)
        };
        assert!(allowed(&world, target));
        world.unit_mut(target).unwrap().owner = PlayerId(2);
        assert!(!allowed(&world, target));
        world.unit_mut(target).unwrap().owner = PlayerId(1);
        world.unit_mut(target).unwrap().combat.max_health = 100;
        world.unit_mut(target).unwrap().combat.health = 0;
        assert!(!allowed(&world, target));
        world.unit_mut(target).unwrap().combat.health = 100;
        assert!(allowed(&world, target));
        world.despawn(target).unwrap();
        let replacement = world
            .spawn(
                UnitDefId(0),
                PlayerId(1),
                Vec2::from_ints(300, 300),
                tak_math::Angle::ZERO,
            )
            .unwrap();
        assert_eq!(replacement.index, target.index);
        assert_ne!(replacement, target);
        assert!(!allowed(&world, target));
        assert!(allowed(&world, replacement));
    }

    #[test]
    fn factory_context_cursor_matches_sim_view_on_terrain_and_minimap() {
        use tak_commands::{Tick, UnitSet};
        use tak_input::{Action, CursorKind, EventKind, InputEvent, Key, Mapper, Output, Profile};

        let mut scenario = tak_sim::testing::example_scenario(1, 2);
        scenario.unit_defs[0].build = tak_sim::construction::BuildDef {
            builder: true,
            mobile: false,
            canbuild: vec![UnitDefId(1)],
            workertime: 10,
            ..Default::default()
        };
        let mut world = World::new(&scenario).unwrap();
        let mut spawn = |def, owner, x| {
            world
                .spawn(
                    UnitDefId(def),
                    PlayerId(owner),
                    Vec2::from_ints(x, 100),
                    tak_math::Angle::ZERO,
                )
                .unwrap()
        };
        let factory = spawn(0, 0, 100);
        let mobile = spawn(1, 0, 200);
        let friendly = spawn(1, 0, 300);
        let enemy = spawn(1, 1, 400);
        let view = SimView {
            world: &world,
            local: PlayerId(0),
            defs: None,
            fog: None,
        };
        assert!(view.can_factory_control(factory));
        assert_eq!(
            view.unit(factory).unwrap().builder,
            Some(BuilderKind::Factory)
        );
        assert_eq!(view.unit(mobile).unwrap().builder, None);
        let event = |m: &mut Mapper, kind| {
            m.handle(&InputEvent::new(m.now() + 1, kind), &view, Tick(1));
        };

        for minimap in [false, true] {
            for selected in [&[factory][..], &[mobile][..], &[factory, mobile][..]] {
                let mut m = Mapper::new(Profile::default_combined());
                event(
                    &mut m,
                    EventKind::KeyDown {
                        key: Key::new("shift"),
                    },
                );
                for &id in selected {
                    event(
                        &mut m,
                        EventKind::PointerMove {
                            x: view.unit(id).unwrap().pos.x.raw() / 65536,
                            y: 100,
                        },
                    );
                    m.perform(Action::Select, &view, Tick(1));
                }
                event(
                    &mut m,
                    EventKind::KeyUp {
                        key: Key::new("shift"),
                    },
                );
                assert_eq!(m.selection(), &UnitSet::from(selected.to_vec()));
                for (target, expected) in
                    [(friendly, CursorKind::Guard), (enemy, CursorKind::Attack)]
                {
                    let to = view.unit(target).unwrap().pos;
                    event(
                        &mut m,
                        if minimap {
                            EventKind::MapPointer {
                                x: 1200,
                                y: 50,
                                world: to,
                            }
                        } else {
                            EventKind::PointerMove {
                                x: to.x.raw() / 65536,
                                y: 100,
                            }
                        },
                    );
                    let has_mobile = selected.contains(&mobile);
                    let has_factory = selected.contains(&factory);
                    assert_eq!(
                        m.cursor_kind(&view),
                        if has_mobile || target == friendly {
                            expected
                        } else {
                            CursorKind::Invalid
                        }
                    );
                    let commands: Vec<_> = m
                        .perform(Action::Context { append: false }, &view, Tick(1))
                        .into_iter()
                        .filter_map(|out| match out {
                            Output::Sim { command, .. } => Some(command),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        commands.len(),
                        usize::from(has_mobile) + usize::from(has_factory && target == friendly)
                    );
                    for command in commands {
                        if matches!(command, tak_commands::SimCommand::SetFactoryRally { .. }) {
                            assert_eq!(command.units(), &UnitSet::single(factory));
                            assert!(matches!(command,
                                tak_commands::SimCommand::SetFactoryRally {
                                    order: Some(tak_commands::FactoryRallyOrder::Guard { target }), ..
                                } if target == friendly
                            ));
                        } else {
                            assert_eq!(command.units(), &UnitSet::single(mobile));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn spell_menu_keeps_weapon_slots_and_the_actual_casters_mana() {
        use tak_sim::combat::WeaponStats;
        let mut scenario = tak_sim::testing::example_scenario(1, 2);
        scenario.players[0].monarch = UnitDefId(1);
        scenario.unit_defs[0].economy.maxmana = 100;
        let spell = |name: &str, cost| WeaponStats {
            name: name.into(),
            mana_per_shot: cost,
            ..Default::default()
        };
        scenario.unit_defs[0].combat.weapons = vec![
            spell("ordinary", 0),
            spell("first spell", 50),
            spell("another ordinary", 0),
            spell("second spell", 100),
        ];
        let mut world = World::new(&scenario).unwrap();
        tak_sim::systems::spawn_start_units(&mut world);
        let noncaster = world.players()[0].monarch.unwrap();
        let enemy = world.players()[1].monarch.unwrap();
        let caster = world
            .spawn(
                UnitDefId(0),
                PlayerId(0),
                Vec2::from_ints(300, 300),
                tak_math::Angle::ZERO,
            )
            .unwrap();
        tak_sim::economy::set_unit_mana_percent(&mut world, caster, 100);
        assert!(tak_sim::economy::spend_unit_mana(
            &mut world,
            caster,
            Fx::from_ratio(1, 2).unwrap()
        ));
        let selection = tak_commands::UnitSet::from(vec![noncaster, enemy, caster]);
        let (id, spells) = caster_spells(&world, PlayerId(0), &selection).unwrap();
        assert_eq!(id, caster);
        assert_eq!(
            spells.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![SpellId(1), SpellId(3)]
        );
        assert!(spells[0].ready);
        assert_eq!(spells[0].shortfall(), 0);
        assert_eq!(spells[0].shortfall_text(), None);
        assert!(!spells[1].ready);
        assert_eq!(spells[1].shortfall(), 1, "fractional shortage rounds up");
        assert_eq!(spells[1].mana_text(), "100 mana (have 99)");
        assert_eq!(spells[1].shortfall_text().as_deref(), Some("short 1 mana"));
        assert!(
            caster_spells(&world, PlayerId(0), &tak_commands::UnitSet::single(enemy)).is_none()
        );
        assert!(unit_spells(&world, noncaster).is_empty());
        tak_sim::economy::set_unit_mana_percent(&mut world, caster, 100);
        assert!(unit_spells(&world, caster).iter().all(|s| s.ready));
    }

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
    fn minimap_sits_above_the_side_panel() {
        for view in [[1280.0, 720.0], [1920.0, 1080.0], [1024.0, 768.0]] {
            for image in [(256, 256), (512, 107), (129, 431)] {
                let g = MinimapGeom::fit_above(view, image, [4096.0, 4096.0], 352.0);
                let [l, t, w, h] = g.panel;
                assert!(t >= 34.0 - 1e-3, "{view:?} {image:?} {:?}", g.panel);
                assert!(t + h <= view[1] - 352.0 + 1e-3, "{view:?} {image:?}");
                assert!(l + w <= view[0] + 1e-3 && l >= 0.0);
            }
        }
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
