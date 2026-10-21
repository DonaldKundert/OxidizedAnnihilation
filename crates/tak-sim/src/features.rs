//! Features (WP-P): the map's placed features that matter to gameplay and
//! the corpses units leave when they die. Rules and confidence:
//! `docs/combat.md` ("Corpses and features").
//!
//! - **Static data** ([`FeatureSetup`], a field of the scenario): the
//!   feature definitions a match uses, the placed map features, and each
//!   unit definition's corpse (FBI `corpse`).
//! - **State** ([`FeatureState`], `World::features`): a generational arena
//!   of live features in slot order, hashed every tick.
//! - Placed features block through the map's terrain grid (their `FEATURE`
//!   flag is already in it). Corpses with `blocking = 1` add a dynamic
//!   obstacle through [`crate::pathfinding::add_obstacle`] while they lie
//!   there, so movement layers route around them and placement refuses the
//!   cells.
//! - Corpses with a `decompose_time` vanish after that many seconds
//!   (**Believed**; only non-blocking infantry corpses carry the key).
//!
//! Not yet: reclaim (`reclaimable`, `energy`), feature damage and burning,
//! resurrection, removing a placed feature (its block lives in the static
//! terrain grid).

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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureSetup {
    /// Definitions used by `placed` and `corpses`.
    pub defs: Vec<FeatureDef>,
    /// Gameplay-relevant placed features, in map order.
    pub placed: Vec<PlacedFeature>,
    /// Corpse feature of unit definition `i` (FBI `corpse`), if any.
    pub corpses: Vec<Option<u32>>,
}

impl FeatureSetup {
    /// The gameplay-relevant placed features of a real map, plus the
    /// corpses of `units` (see [`FeatureSetup::resolve_corpses`]).
    pub fn from_content(
        map: &tak_content::MapDef,
        rs: &tak_content::Ruleset,
        units: &[crate::UnitDef],
    ) -> FeatureSetup {
        let mut s = FeatureSetup::default();
        for p in &map.features {
            if s.placed.len() >= MAX_PLACED_FEATURES {
                break;
            }
            let Some(f) = p.def.as_deref().and_then(|d| rs.feature(d)) else {
                continue;
            };
            if !FeatureDef::from_content(f).matters() {
                continue;
            }
            let Some(def) = s.intern(rs, f) else { break };
            s.placed.push(PlacedFeature {
                def,
                x: p.x,
                z: p.z,
            });
        }
        s.resolve_corpses(rs, units);
        s
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
                None => break,
            };
            self.defs[cur as usize].feature_dead = Some(j);
            cur = j;
            next = nf.feature_dead.as_deref();
        }
        Some(me)
    }

    pub(crate) fn validate(&self, unit_defs: usize) -> Result<(), ScenarioError> {
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
        let side = crate::map::MAX_MAP_SIDE as u32;
        if self
            .placed
            .iter()
            .any(|p| p.def >= n || p.x > side || p.z > side)
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
        h.finish()
    }
}

/// Where a live feature came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Placed on the map; blocks through the map's terrain grid.
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
    /// Facing (corpses keep the unit's).
    pub heading: Angle,
    /// Hit points (the definition's `damage`).
    pub health: i32,
    /// Placed or corpse.
    pub origin: Origin,
    /// Tick it appeared.
    pub born: u32,
    /// It holds a dynamic obstacle (blocking corpse).
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

    fn hash(&self, h: &mut StateHasher) {
        h.u32(self.def);
        h.i32(self.rect.x);
        h.i32(self.rect.z);
        h.i32(self.rect.w);
        h.i32(self.rect.h);
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
        for p in &setup.placed {
            let Some(d) = setup.defs.get(p.def as usize) else {
                continue;
            };
            let rect = CellRect {
                x: p.x as i32,
                z: p.z as i32,
                w: i32::from(d.footprint_x),
                h: i32::from(d.footprint_z),
            };
            s.insert(Feature {
                def: p.def,
                rect,
                heading: Angle::ZERO,
                health: d.damage,
                origin: Origin::Map,
                born: 0,
                obstacle: false,
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
/// (placed features through the terrain grid are not included; see
/// [`crate::map::terrain::FEATURE`]).
pub fn dynamic_block_at(world: &World, cx: i32, cz: i32) -> bool {
    pathfinding::is_obstacle(world, cx, cz)
}
