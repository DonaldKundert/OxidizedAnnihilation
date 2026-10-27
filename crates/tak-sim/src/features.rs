//! Features (WP-P): the map's placed features that matter to gameplay and
//! the corpses units leave when they die. Rules and confidence:
//! `docs/combat.md` ("Corpses and features").
//!
//! - **Static data** ([`FeatureSetup`], a field of the scenario): the
//!   feature definitions a match uses, the placed map features, and each
//!   unit definition's corpse (FBI `corpse`).
//! - **State** ([`FeatureState`], `World::features`): a generational arena
//!   of live features in slot order, hashed every tick.
//! - Blocking placed features and corpses own counted dynamic obstacles.
//!   Real-content scenarios transfer the placed stamps out of static
//!   terrain; unrelated authored terrain flags remain immutable.
//! - Corpses with a `decompose_time` vanish after that many seconds
//!   (**Believed**; only non-blocking infantry corpses carry the key).
//!
//! - Positive weapon impacts subtract health, then replace a destroyed
//!   feature with at most one `featuredead` successor. No mana is awarded.
//!
//! Not yet: builder Clear/reclaim (`reclaimable`, `energy`), burning,
//! resurrection or projectile/feature collision along the flight path.

use serde::{Deserialize, Serialize};
use tak_commands::UnitDefId;
use tak_math::{Angle, Fx};

use crate::WorldPos;
use crate::construction::placement::Rect;
use crate::hash::StateHasher;
use crate::pathfinding::{self, CellRect};
use crate::scenario::ScenarioError;
use crate::world::World;

/// Most feature definitions a scenario may carry.
pub const MAX_FEATURE_DEFS: usize = 8192;
/// Most placed map features a scenario may carry.
pub const MAX_PLACED_FEATURES: usize = 1 << 17;
/// Most live features at once; further corpses are not spawned.
pub const MAX_FEATURES: usize = 1 << 17;
/// How many `featuredead` steps a corpse type above 1 may follow.
pub const MAX_CORPSE_DEPTH: i32 = 4;

/// One feature definition (`features/**/*.tdf`), reduced to what the sim
/// uses. Names keep the TDF key names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureDef {
    /// Section name (as written).
    pub name: String,
    /// `footprintx` / `footprintz`, attribute cells (at least 1).
    pub footprint_x: u8,
    /// See `footprint_x`.
    pub footprint_z: u8,
    /// `blocking`: occupies its footprint for movement and building.
    pub blocking: bool,
    /// `damage`: hit points (0 = none given).
    pub damage: i32,
    /// `indestructible`.
    pub indestructible: bool,
    /// `reclaimable`.
    pub reclaimable: bool,
    /// `energy`: mana from reclaiming (Believed; not used yet).
    pub energy: i32,
    /// `sacredsite`: lodestone multiplier (0 = not a site).
    pub sacred_site: Fx,
    /// `decompose_time`, seconds (0 = stays).
    pub decompose_time: i32,
    /// `featuredead`: index of the feature this one turns into.
    pub feature_dead: Option<u32>,
    /// `object`: 3D model name for renderers (`objects3d/<object>.3do`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
}

impl FeatureDef {
    /// From the content definition. Values are clamped into sane ranges.
    pub fn from_content(f: &tak_content::FeatureDef) -> FeatureDef {
        FeatureDef {
            name: f.name.clone(),
            footprint_x: f.footprint_x.max(1),
            footprint_z: f.footprint_z.max(1),
            blocking: f.blocking,
            damage: f.damage.unwrap_or(0).clamp(0, 1_000_000),
            indestructible: f.indestructible,
            reclaimable: f.reclaimable,
            energy: f.energy.unwrap_or(0).clamp(0, 1_000_000),
            sacred_site: f.sacred_site.unwrap_or(Fx::ZERO).max(Fx::ZERO),
            decompose_time: f.decompose_time.unwrap_or(0).clamp(0, 100_000),
            feature_dead: None,
            object: f.object.clone(),
        }
    }

    /// Whether a placed map feature of this kind matters to gameplay
    /// (blocks, can be reclaimed or damaged, or is a sacred site). Scenery
    /// (sound emitters, decals) is left to renderers.
    pub fn matters(&self) -> bool {
        self.blocking
            || self.reclaimable
            || self.damage > 0
            || self.energy > 0
            || self.sacred_site > Fx::ZERO
    }

    fn hash(&self, h: &mut StateHasher) {
        h.bytes(self.name.as_bytes());
        h.u8(self.footprint_x);
        h.u8(self.footprint_z);
        h.bool(self.blocking);
        h.i32(self.damage);
        h.bool(self.indestructible);
        h.bool(self.reclaimable);
        h.i32(self.energy);
        h.fx(self.sacred_site);
        h.i32(self.decompose_time);
        h.option(self.feature_dead, StateHasher::u32);
        h.option(self.object.as_deref(), |h, o| h.bytes(o.as_bytes()));
    }
}

/// A feature placed on the map (a TNT feature-grid entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedFeature {
    /// Index into [`FeatureSetup::defs`].
    pub def: u32,
    /// Anchor (top-left) attribute cell.
    pub x: u32,
    /// See `x`.
    pub z: u32,
}

/// The feature part of a scenario. Empty by default (synthetic scenarios).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureSetup {
    /// Definitions used by `placed` and `corpses`.
    pub defs: Vec<FeatureDef>,
    /// Gameplay-relevant placed features, in map order.
    pub placed: Vec<PlacedFeature>,
    /// Corpse feature of unit definition `i` (FBI `corpse`), if any.
    pub corpses: Vec<Option<u32>>,
    /// Whether placed features block. Preserves the content passability
    /// rule; corpses are unaffected. Defaults to true for authored scenarios.
    pub placed_features_block: bool,
}

impl Default for FeatureSetup {
    fn default() -> Self {
        Self {
            defs: Vec::new(),
            placed: Vec::new(),
            corpses: Vec::new(),
            placed_features_block: true,
        }
    }
}

impl FeatureSetup {
    /// The gameplay-relevant placed features of a real map, plus the
    /// corpses of `units` (see [`FeatureSetup::resolve_corpses`]).
    pub fn from_content(
        map: &tak_content::MapDef,
        rs: &tak_content::Ruleset,
        units: &[crate::UnitDef],
    ) -> Result<FeatureSetup, ScenarioError> {
        let mut s = FeatureSetup {
            placed_features_block: map.passability.rule.features_block,
            ..FeatureSetup::default()
        };
        for p in &map.features {
            let Some(f) = p.def.as_deref().and_then(|d| rs.feature(d)) else {
                continue;
            };
            if !FeatureDef::from_content(f).matters() {
                continue;
            }
            if s.placed.len() >= MAX_PLACED_FEATURES {
                return Err(ScenarioError::Invalid(
                    "features: too many placed features".into(),
                ));
            }
            let def = s.intern(rs, f).ok_or_else(|| {
                ScenarioError::Invalid("features: too many feature definitions".into())
            })?;
            let (x, z) = match map.passability.rule.anchor {
                tak_content::FeatureAnchor::TopLeft => (p.x, p.z),
                tak_content::FeatureAnchor::Center => (
                    p.x.saturating_sub(u32::from(f.footprint_x).saturating_sub(1) / 2),
                    p.z.saturating_sub(u32::from(f.footprint_z).saturating_sub(1) / 2),
                ),
            };
            s.placed.push(PlacedFeature { def, x, z });
        }
        s.resolve_corpses(rs, units);
        Ok(s)
    }

    /// Fill `corpses` for every unit definition from its FBI `corpse`
    /// (and the `featuredead` chain behind it), adding definitions as
    /// needed. Call again after adding unit definitions.
    pub fn resolve_corpses(&mut self, rs: &tak_content::Ruleset, units: &[crate::UnitDef]) {
        self.corpses = units
            .iter()
            .map(|u| {
                let fbi = rs.unit(&u.name)?;
                let f = rs.feature(fbi.corpse.as_deref()?)?;
                self.intern(rs, f)
            })
            .collect();
    }

    /// Index of `f` in `defs`, adding it (and its `featuredead` chain).
    fn intern(&mut self, rs: &tak_content::Ruleset, f: &tak_content::FeatureDef) -> Option<u32> {
        let find = |defs: &[FeatureDef], n: &str| {
            defs.iter()
                .position(|d| d.name.eq_ignore_ascii_case(n))
                .map(|i| i as u32)
        };
        if let Some(i) = find(&self.defs, &f.name) {
            return Some(i);
        }
        if self.defs.len() >= MAX_FEATURE_DEFS {
            return None;
        }
        self.defs.push(FeatureDef::from_content(f));
        let me = (self.defs.len() - 1) as u32;
        // The chain is short in the data; the depth cap guards cycles.
        let mut cur = me;
        let mut next = f.feature_dead.as_deref();
        for _ in 0..MAX_CORPSE_DEPTH {
            let Some(nf) = next.and_then(|n| rs.feature(n)) else {
                break;
            };
            let j = match find(&self.defs, &nf.name) {
                Some(j) => j,
                None if self.defs.len() < MAX_FEATURE_DEFS => {
                    self.defs.push(FeatureDef::from_content(nf));
                    (self.defs.len() - 1) as u32
                }
                None => return None,
            };
            self.defs[cur as usize].feature_dead = Some(j);
            cur = j;
            next = nf.feature_dead.as_deref();
        }
        Some(me)
    }

    pub(crate) fn validate(
        &self,
        unit_defs: usize,
        map: &crate::map::MapInfo,
    ) -> Result<(), ScenarioError> {
        let bad = |m: &str| Err(ScenarioError::Invalid(format!("features: {m}")));
        if self.defs.len() > MAX_FEATURE_DEFS || self.placed.len() > MAX_PLACED_FEATURES {
            return bad("too many");
        }
        if self.corpses.len() > unit_defs {
            return bad("more corpse entries than unit defs");
        }
        let n = self.defs.len() as u32;
        for d in &self.defs {
            if d.footprint_x == 0
                || d.footprint_z == 0
                || d.name.len() > 256
                || d.feature_dead.is_some_and(|j| j >= n)
                || !(0..=1_000_000).contains(&d.damage)
                || !(0..=100_000).contains(&d.decompose_time)
                || d.sacred_site < Fx::ZERO
            {
                return bad("bad definition");
            }
        }
        let cols = (map.width as u32).div_ceil(16);
        let rows = (map.height as u32).div_ceil(16);
        if self
            .placed
            .iter()
            .any(|p| p.def >= n || p.x >= cols || p.z >= rows)
            || self.corpses.iter().flatten().any(|&c| c >= n)
        {
            return bad("index out of range");
        }
        Ok(())
    }

    fn digest(&self) -> u64 {
        let mut h = StateHasher::new();
        h.len(self.defs.len());
        for d in &self.defs {
            d.hash(&mut h);
        }
        h.len(self.placed.len());
        for p in &self.placed {
            h.u32(p.def);
            h.u32(p.x);
            h.u32(p.z);
        }
        h.len(self.corpses.len());
        for c in &self.corpses {
            h.option(*c, StateHasher::u32);
        }
        if !self.placed_features_block {
            h.bytes(b"PLACED_NONBLOCKING");
        }
        h.finish()
    }
}

/// Where a live feature came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Placed on the map.
    Map,
    /// Left by a dead unit of this definition.
    Corpse(UnitDefId),
}

/// A live feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feature {
    /// Index into the setup's definitions ([`FeatureState::def`]).
    pub def: u32,
    /// Footprint in attribute cells (anchor top-left).
    pub rect: CellRect,
    /// Stable world centre used when snapping successor footprints. Kept
    /// across generations so odd/even footprint cycles cannot drift.
    pub replacement_center: WorldPos,
    /// Facing (corpses keep the unit's).
    pub heading: Angle,
    /// Hit points (the definition's `damage`).
    pub health: i32,
    /// Placed or corpse.
    pub origin: Origin,
    /// Tick it appeared.
    pub born: u32,
    /// It owns a counted dynamic obstacle.
    pub obstacle: bool,
}

impl Feature {
    /// Centre in world units.
    pub fn center(&self) -> WorldPos {
        let c = crate::construction::placement::SQUARE;
        tak_math::Vec2::from_ints(
            self.rect.x * c + self.rect.w * c / 2,
            self.rect.z * c + self.rect.h * c / 2,
        )
    }

    /// Half-open footprint contains `point` (the deterministic direct-hit
    /// shape; Chosen, not a model collision volume).
    pub fn contains(&self, point: WorldPos) -> bool {
        let x = point.x.to_int_floor().div_euclid(16);
        let z = point.y.to_int_floor().div_euclid(16);
        self.covers_cell(x, z)
    }

    fn covers_cell(&self, x: i32, z: i32) -> bool {
        x >= self.rect.x
            && x < self.rect.x + self.rect.w
            && z >= self.rect.z
            && z < self.rect.z + self.rect.h
    }

    /// Distance to the nearest point of the footprint, zero inside.
    /// Chosen blast shape, centralized here for later fidelity corrections.
    pub fn distance_to(&self, point: WorldPos) -> Fx {
        let nearest = tak_math::Vec2::new(
            point.x.clamp(
                Fx::from_int(self.rect.x * 16),
                Fx::from_int((self.rect.x + self.rect.w) * 16),
            ),
            point.y.clamp(
                Fx::from_int(self.rect.z * 16),
                Fx::from_int((self.rect.z + self.rect.h) * 16),
            ),
        );
        point.distance(nearest)
    }

    fn hash(&self, h: &mut StateHasher) {
        h.u32(self.def);
        h.i32(self.rect.x);
        h.i32(self.rect.z);
        h.i32(self.rect.w);
        h.i32(self.rect.h);
        h.vec2(self.replacement_center);
        h.angle(self.heading);
        h.i32(self.health);
        match self.origin {
            Origin::Map => h.u8(0),
            Origin::Corpse(d) => {
                h.u8(1);
                h.unit_def(d);
            }
        }
        h.u32(self.born);
        h.bool(self.obstacle);
    }
}

/// Handle to a live feature (slot plus generation, like `UnitId`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureId {
    /// Slot.
    pub index: u32,
    /// Bumped whenever the slot is freed.
    pub generation: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Slot {
    generation: u32,
    feature: Option<Feature>,
}

/// World-level feature state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeatureState {
    setup: FeatureSetup,
    digest: u64,
    slots: Vec<Slot>,
    /// Corpses spawned so far.
    pub corpses_spawned: u64,
}

impl FeatureState {
    /// The match's placed features, live from the start.
    pub fn new(setup: &FeatureSetup) -> FeatureState {
        let mut s = FeatureState {
            setup: setup.clone(),
            digest: setup.digest(),
            slots: Vec::new(),
            corpses_spawned: 0,
        };
        for p in setup.placed.iter().take(MAX_FEATURES) {
            let Some(d) = setup.defs.get(p.def as usize) else {
                continue;
            };
            let rect = CellRect {
                x: p.x as i32,
                z: p.z as i32,
                w: i32::from(d.footprint_x),
                h: i32::from(d.footprint_z),
            };
            // Fresh map slots need no free-slot scan (linear initialization).
            s.slots.push(Slot {
                generation: 0,
                feature: Some(Feature {
                    def: p.def,
                    rect,
                    replacement_center: tak_math::Vec2::from_ints(
                        rect.x * 16 + rect.w * 8,
                        rect.z * 16 + rect.h * 8,
                    ),
                    heading: Angle::ZERO,
                    health: d.damage,
                    origin: Origin::Map,
                    born: 0,
                    obstacle: d.blocking && setup.placed_features_block,
                }),
            });
        }
        s
    }

    /// The static setup.
    pub fn setup(&self) -> &FeatureSetup {
        &self.setup
    }

    /// A feature's definition.
    pub fn def(&self, f: &Feature) -> Option<&FeatureDef> {
        self.setup.defs.get(f.def as usize)
    }

    /// A live feature.
    pub fn get(&self, id: FeatureId) -> Option<&Feature> {
        let s = self.slots.get(id.index as usize)?;
        (s.generation == id.generation)
            .then_some(s.feature.as_ref())
            .flatten()
    }

    /// Live features in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (FeatureId, &Feature)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.feature.as_ref().map(|f| {
                (
                    FeatureId {
                        index: i as u32,
                        generation: s.generation,
                    },
                    f,
                )
            })
        })
    }

    /// Number of live features.
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.feature.is_some()).count()
    }

    /// `true` if no feature is live.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Live corpses (features of [`Origin::Corpse`]).
    pub fn corpses(&self) -> impl Iterator<Item = (FeatureId, &Feature)> {
        self.iter()
            .filter(|(_, f)| matches!(f.origin, Origin::Corpse(_)))
    }

    /// Insert in the lowest free slot.
    fn insert(&mut self, f: Feature) -> Option<FeatureId> {
        if let Some(i) = self.slots.iter().position(|s| s.feature.is_none()) {
            let s = &mut self.slots[i];
            s.feature = Some(f);
            return Some(FeatureId {
                index: i as u32,
                generation: s.generation,
            });
        }
        if self.slots.len() >= MAX_FEATURES {
            return None;
        }
        self.slots.push(Slot {
            generation: 0,
            feature: Some(f),
        });
        Some(FeatureId {
            index: (self.slots.len() - 1) as u32,
            generation: 0,
        })
    }

    fn take(&mut self, id: FeatureId) -> Option<Feature> {
        let s = self.slots.get_mut(id.index as usize)?;
        if s.generation != id.generation {
            return None;
        }
        let f = s.feature.take()?;
        s.generation = s.generation.wrapping_add(1);
        Some(f)
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"FEAT");
        h.u64(self.digest);
        h.u64(self.corpses_spawned);
        h.len(self.slots.len());
        for s in &self.slots {
            h.u32(s.generation);
            h.option(s.feature.as_ref(), |h, f| f.hash(h));
        }
    }
}

/// The feature a dead unit of `def` leaves for `Killed`'s corpse answer
/// `kind`: none for 0 (or no FBI `corpse`), the corpse for 1, and for
/// higher values `kind − 1` steps along `featuredead` where the chain
/// allows (**Believed**: TA's corpse depth).
pub fn corpse_def(world: &World, def: UnitDefId, kind: i32) -> Option<u32> {
    if kind <= 0 {
        return None;
    }
    let s = &world.features.setup;
    let mut f = (*s.corpses.get(def.0 as usize)?)?;
    for _ in 1..kind.min(MAX_CORPSE_DEPTH + 1) {
        match s.defs.get(f as usize).and_then(|d| d.feature_dead) {
            Some(n) => f = n,
            None => break,
        }
    }
    Some(f)
}

/// Spawn the corpse of a unit of `def` that died at `pos` (see
/// [`corpse_def`]). The footprint is centred on the unit, snapped like a
/// structure's (**Chosen**: FBI `corpseadjustx/z` are not applied). A
/// blocking corpse becomes a dynamic obstacle at once.
pub fn spawn_corpse(
    world: &mut World,
    def: UnitDefId,
    pos: WorldPos,
    heading: Angle,
    kind: i32,
) -> Option<FeatureId> {
    let fdef = corpse_def(world, def, kind)?;
    let d = world.features.setup.defs.get(fdef as usize)?;
    let r = Rect::around(pos, d.footprint_x, d.footprint_z);
    let rect = CellRect {
        x: r.x,
        z: r.z,
        w: r.w,
        h: r.h,
    };
    let f = Feature {
        def: fdef,
        rect,
        replacement_center: r.center(),
        heading,
        health: d.damage,
        origin: Origin::Corpse(def),
        born: world.tick().0,
        obstacle: d.blocking,
    };
    let id = world.features.insert(f)?;
    world.features.corpses_spawned += 1;
    if f.obstacle {
        pathfinding::add_obstacle(world, rect);
    }
    Some(id)
}

/// Remove a feature (and its obstacle). Returns it.
pub fn remove(world: &mut World, id: FeatureId) -> Option<Feature> {
    let f = world.features.take(id)?;
    if f.obstacle {
        pathfinding::remove_obstacle(world, f.rect);
    }
    Some(f)
}

/// Stamp all initial placed blockers before movement layers exist. Calls
/// do not rebuild grids or allocate combined planes during initialization.
pub(crate) fn initialize_obstacles(world: &mut World) {
    let rects: Vec<_> = world
        .features
        .iter()
        .filter(|(_, f)| f.obstacle)
        .map(|(_, f)| f.rect)
        .collect();
    for rect in rects {
        pathfinding::add_obstacle(world, rect);
    }
}

/// Subtract positive weapon damage from a destructible feature. Returns
/// whether health changed. Destruction retires the old generation and
/// creates at most one `featuredead` successor; excess damage is discarded.
/// Reclaimability is independent of damage and no economy credit is made.
pub fn apply_damage(world: &mut World, id: FeatureId, amount: i32) -> bool {
    let Some(f) = world.features.get(id).copied() else {
        return false;
    };
    let Some(d) = world.features.def(&f) else {
        return false;
    };
    if amount <= 0 || f.health <= 0 || d.indestructible {
        return false;
    }
    if amount < f.health {
        world.features.slots[id.index as usize]
            .feature
            .as_mut()
            .expect("live")
            .health -= amount;
        return true;
    }
    let next = d.feature_dead;
    remove(world, id);
    let Some((def, d)) =
        next.and_then(|n| world.features.setup.defs.get(n as usize).map(|d| (n, d)))
    else {
        return true;
    };
    // Keep the same world centre, snapped to the successor footprint like
    // corpses. A replacement is a new lifetime even in a cyclic definition.
    let r = Rect::around(f.replacement_center, d.footprint_x, d.footprint_z);
    let successor = Feature {
        def,
        rect: CellRect {
            x: r.x,
            z: r.z,
            w: r.w,
            h: r.h,
        },
        replacement_center: f.replacement_center,
        heading: f.heading,
        health: d.damage,
        origin: f.origin,
        born: world.tick().0,
        obstacle: d.blocking
            && (matches!(f.origin, Origin::Corpse(_))
                || world.features.setup.placed_features_block),
    };
    if world.features.insert(successor).is_some() && successor.obstacle {
        pathfinding::add_obstacle(world, successor.rect);
    }
    true
}

/// Snapshot this impact's feature hits before any lifetime changes.
/// Chosen: small impacts hit the first damage-eligible footprint in slot order, unless a
/// unit already took the direct hit; blasts use footprint-edge distance
/// and the existing unit falloff. Feature category modifiers are unknown,
/// so only the weapon's authored default damage is used.
pub(crate) fn impact_hits(
    world: &World,
    point: WorldPos,
    aoe: i32,
    damage: i32,
    edge: Fx,
    direct_unit: bool,
) -> Vec<(FeatureId, i32)> {
    if damage <= 0 {
        return Vec::new();
    }
    let damageable =
        |f: &Feature| f.health > 0 && world.features.def(f).is_some_and(|d| !d.indestructible);
    if aoe <= 16 {
        return if direct_unit {
            Vec::new()
        } else {
            world
                .features
                .iter()
                .find(|(_, f)| damageable(f) && f.contains(point))
                .map(|(id, _)| vec![(id, damage)])
                .unwrap_or_default()
        };
    }
    let radius = aoe.max(0) / 2;
    world
        .features
        .iter()
        .filter(|(_, f)| damageable(f))
        .filter_map(|(id, f)| {
            let distance = f.distance_to(point).to_int_floor();
            (distance < radius).then(|| {
                (
                    id,
                    crate::combat::area_damage(damage, distance, radius, edge),
                )
            })
        })
        .collect()
}

/// Placement's legacy `S` yard cell permits placed map-feature stamps, but
/// not corpses or externally added obstacles. Count rather than exempting
/// the whole cell so overlapping blockers remain independent.
pub(crate) fn blocks_placement(world: &World, cx: i32, cz: i32, sacred_yard: bool) -> bool {
    let cols = world.map().terrain.as_ref().map_or(0, |t| t.cols);
    let count = world.pathing.obstacles_at(cx, cz, cols);
    if count == 0 || !sacred_yard {
        return count > 0;
    }
    let placed = world
        .features
        .iter()
        .filter(|(_, f)| f.obstacle && matches!(f.origin, Origin::Map) && f.covers_cell(cx, cz))
        .count() as u32;
    count > placed
}

/// Per tick (called at the end of the combat lane): corpses whose
/// `decompose_time` has run out vanish, in slot order.
pub fn tick(world: &mut World) {
    let now = world.tick().0;
    let expired: Vec<FeatureId> = world
        .features
        .iter()
        .filter(|(_, f)| {
            matches!(f.origin, Origin::Corpse(_))
                && world.features.def(f).is_some_and(|d| {
                    d.decompose_time > 0
                        && now.saturating_sub(f.born)
                            >= (d.decompose_time as u32).saturating_mul(crate::TICKS_PER_SECOND)
                })
        })
        .map(|(id, _)| id)
        .collect();
    for id in expired {
        remove(world, id);
    }
}

/// `true` if a live blocking feature covers attribute cell `(cx, cz)`
/// (placed map features and corpses).
pub fn dynamic_block_at(world: &World, cx: i32, cz: i32) -> bool {
    pathfinding::is_obstacle(world, cx, cz)
}
