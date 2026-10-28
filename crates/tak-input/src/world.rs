//! What the input layer may know about the world: a read-only [`WorldView`]
//! and a screen/world [`Camera`]. Nothing here can change sim state.

use serde::{Deserialize, Serialize};
use tak_commands::{FeatureId, PlayerId, SpellId, UnitDefId, UnitId, WorldPos};
use tak_math::{Fx, Vec2};

use crate::binding::Category;

/// Whether a unit builds by placing structures or by producing units.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuilderKind {
    /// Walks to a site and builds there (`Build { at: Some(..) }`).
    Mobile,
    /// Produces into its own queue (`Build { at: None }`).
    Factory,
}

/// A snapshot of one unit, as the input layer sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct UnitInfo {
    /// Handle.
    pub id: UnitId,
    /// Ground position.
    pub pos: WorldPos,
    /// Pick radius in world units: the cursor is "on" the unit within it.
    pub radius: Fx,
    /// Owner.
    pub owner: PlayerId,
    /// Unit type.
    pub def: UnitDefId,
    /// Visible and pickable by the cursor (own or not). Selection commands
    /// further restrict to the local player's units.
    pub selectable: bool,
    /// Builder kind, if it builds.
    pub builder: Option<BuilderKind>,
    /// A frame still under construction (an aim-assist target in build
    /// mode).
    #[serde(default)]
    pub under_construction: bool,
}

/// A snapped mobile-construction site supplied by the world adapter.
///
/// Bounds are a half-open world-space rectangle `[min, max)`. They describe
/// the actual footprint, not a screen-pixel radius. Adjacent edges may touch;
/// overlapping footprints are not distinct sites within one input stroke.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlacementCandidate {
    /// Centre used by the existing `Build { at: Some(..) }` command.
    pub center: WorldPos,
    /// Inclusive footprint minimum.
    pub min: WorldPos,
    /// Exclusive footprint maximum.
    pub max: WorldPos,
}

impl PlacementCandidate {
    pub(crate) fn valid(self) -> bool {
        self.min.x < self.max.x
            && self.min.y < self.max.y
            && self.center.x >= self.min.x
            && self.center.x < self.max.x
            && self.center.y >= self.min.y
            && self.center.y < self.max.y
    }

    pub(crate) fn overlaps(self, other: Self) -> bool {
        self.min.x < other.max.x
            && other.min.x < self.max.x
            && self.min.y < other.max.y
            && other.min.y < self.max.y
    }
}

/// Read-only access to the world for the local player. Implemented by the
/// sim adapter in the client, and by plain structs in tests.
///
/// Iteration order does not matter: the input layer sorts by [`UnitId`]
/// wherever order could affect the result.
pub trait WorldView {
    /// The player this input belongs to.
    fn local_player(&self) -> PlayerId;

    /// Call `f` for every unit the local player knows about.
    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo));

    /// One unit, or `None` if it no longer exists.
    fn unit(&self, id: UnitId) -> Option<UnitInfo> {
        let mut found = None;
        self.for_each_unit(&mut |u| {
            if u.id == id {
                found = Some(*u);
            }
        });
        found
    }

    /// `true` if `owner` is hostile to the local player.
    fn is_enemy(&self, owner: PlayerId) -> bool;

    /// `true` if `unit` is an operational mobile builder that can heal or
    /// assist construction. The default uses the visible snapshot: an owned,
    /// selectable, complete mobile builder. Sim adapters should additionally
    /// reject dead/dying units and builders with no positive work rate.
    ///
    /// The input layer still enforces ownership, completion and builder kind
    /// when an adapter overrides this capability check.
    fn can_heal(&self, unit: UnitId) -> bool {
        self.unit(unit).is_some_and(|u| {
            u.owner == self.local_player()
                && u.selectable
                && !u.under_construction
                && u.builder == Some(BuilderKind::Mobile)
        })
    }

    /// Explicit Clear capability. Defaults to false: builder status alone
    /// must not invent an authored `canreclaim` flag. Nonbuilders may have it. Adapters additionally
    /// check the live unit's completion, work rate, range and lifecycle.
    fn can_clear(&self, unit: UnitId) -> bool {
        let _ = unit;
        false
    }

    /// A currently visible, reclaimable feature whose footprint contains
    /// `pos`. Resolve live generational handles and break overlap ties by ID.
    /// Unit picking does not override this explicit feature-targeting mode.
    fn clear_target_at(&self, pos: WorldPos) -> Option<FeatureId> {
        let _ = pos;
        None
    }

    /// A Clear target's currently visible exact-generation centre, for order
    /// lines. Never return a replacement feature occupying the same slot.
    fn clear_target_pos(&self, target: FeatureId) -> Option<WorldPos> {
        let _ = target;
        None
    }

    /// Whether a unit may receive factory rally/production controls. Adapters
    /// should additionally reject dying units and factories unable to build.
    fn can_factory_control(&self, unit: UnitId) -> bool {
        self.unit(unit).is_some_and(|u| {
            u.owner == self.local_player()
                && u.selectable
                && !u.under_construction
                && u.builder == Some(BuilderKind::Factory)
        })
    }

    /// Whether `target` may be stored as a factory's standing Guard target.
    /// The default requires a current, selectable, non-hostile unit. Sim
    /// adapters should additionally reject dead/dying targets. This capability
    /// applies only to factory standing orders, not ordinary mobile Guard.
    ///
    /// The mapper still checks the handle, visibility and hostility when an
    /// adapter overrides this capability.
    fn can_factory_guard_target(&self, target: UnitId) -> bool {
        self.unit(target)
            .is_some_and(|u| u.id == target && u.selectable && !self.is_enemy(u.owner))
    }

    /// Active plus pending production items of `def` at this factory.
    /// Used for presentation and explicit cancellation before a new Stop
    /// gesture. Adapters with active production must report accurate counts;
    /// the sim remains authoritative and the default preserves legacy Stop.
    fn production_count(&self, unit: UnitId, def: UnitDefId) -> u32 {
        let _ = (unit, def);
        0
    }

    /// The continuous-production recipe currently enabled at this factory.
    /// Used only for presentation; the sim remains authoritative.
    fn factory_repeat(&self, unit: UnitId) -> Option<UnitDefId> {
        let _ = unit;
        None
    }

    /// Whether `unit` can receive construction orders now. Construction
    /// frames remain selectable and valid assistance targets, but cannot
    /// queue their own work until complete. Menus describe definitions;
    /// use this capability separately when choosing an actionable builder.
    fn can_build(&self, unit: UnitId) -> bool {
        self.unit(unit).is_some_and(|u| {
            u.owner == self.local_player()
                && u.selectable
                && !u.under_construction
                && u.builder.is_some()
                && (u.builder != Some(BuilderKind::Factory) || self.can_factory_control(unit))
        })
    }

    /// What `unit` can build, in menu order. Empty for non-builders.
    fn build_options(&self, unit: UnitId) -> Vec<UnitDefId> {
        let _ = unit;
        Vec::new()
    }

    /// Spells `unit` can cast, in menu order.
    fn spells(&self, unit: UnitId) -> Vec<SpellId> {
        let _ = unit;
        Vec::new()
    }

    /// `true` if `unit` can cast `spell` now (enough mana). Default: always.
    fn can_cast(&self, unit: UnitId, spell: SpellId) -> bool {
        let _ = (unit, spell);
        true
    }

    /// `true` if a building of type `def` could be placed with its centre at
    /// `pos` (for the build cursor). Default: always.
    fn can_place(&self, def: UnitDefId, pos: WorldPos) -> bool {
        let _ = (def, pos);
        true
    }

    /// A legal, snapped candidate for mouse construction painting, including
    /// its actual footprint. `None` means unsupported geometry or no legal
    /// candidate here. The mapper never guesses geometry from screen pixels.
    /// Snapping must be idempotent: querying the returned centre yields the
    /// same candidate while its geometry and legality remain unchanged.
    ///
    /// This is only a preview filter for the new drag gesture. Ordinary click
    /// dispatch never depends on it, and the simulation validates every Build.
    fn placement_candidate(&self, def: UnitDefId, pos: WorldPos) -> Option<PlacementCandidate> {
        let _ = (def, pos);
        None
    }

    /// `true` if `unit` has no orders (for idle-builder cycling).
    fn is_idle(&self, unit: UnitId) -> bool {
        let _ = unit;
        false
    }

    /// The local player's monarch, if alive.
    fn monarch(&self) -> Option<UnitId> {
        None
    }

    /// `true` if `unit` belongs to a TA select-category (Ctrl+letter keys).
    fn in_category(&self, unit: UnitId, category: Category) -> bool {
        let _ = (unit, category);
        false
    }
}

/// Every unit, sorted by id.
pub fn units_sorted<W: WorldView + ?Sized>(world: &W) -> Vec<UnitInfo> {
    let mut v = Vec::new();
    world.for_each_unit(&mut |u| v.push(*u));
    v.sort_by_key(|u| u.id);
    v
}

/// An orthographic ground-plane mapping between screen pixels and world
/// units, supplied by the client every frame (or whenever it changes).
///
/// `world = origin + screen * world_per_px`. Screen origin is the top-left
/// pixel, `+y` down; the world's `+y` is taken to run the same way.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Camera {
    /// World position of screen pixel `(0, 0)`.
    pub origin: WorldPos,
    /// World units per screen pixel (zoom). Must be positive.
    pub world_per_px: Fx,
    /// Viewport width in pixels.
    pub width_px: i32,
    /// Viewport height in pixels.
    pub height_px: i32,
}

impl Default for Camera {
    fn default() -> Camera {
        Camera {
            origin: Vec2::ZERO,
            world_per_px: Fx::ONE,
            width_px: 1280,
            height_px: 960,
        }
    }
}

impl Camera {
    /// Screen pixels to world.
    pub fn screen_to_world(&self, s: Vec2) -> WorldPos {
        self.origin + s.scale(self.world_per_px)
    }

    /// World to screen pixels (saturating if zoom is degenerate).
    pub fn world_to_screen(&self, w: WorldPos) -> Vec2 {
        let d = w - self.origin;
        Vec2::new(
            d.x.saturating_div(self.world_per_px),
            d.y.saturating_div(self.world_per_px),
        )
    }

    /// Clamp a screen position into the viewport.
    pub fn clamp_screen(&self, s: Vec2) -> Vec2 {
        Vec2::new(
            s.x.clamp(Fx::ZERO, fx_int(self.width_px)),
            s.y.clamp(Fx::ZERO, fx_int(self.height_px)),
        )
    }

    /// `true` if the world point is inside the viewport.
    pub fn contains_world(&self, w: WorldPos) -> bool {
        let s = self.world_to_screen(w);
        s.x >= Fx::ZERO
            && s.y >= Fx::ZERO
            && s.x < fx_int(self.width_px)
            && s.y < fx_int(self.height_px)
    }

    /// Viewport centre in screen pixels.
    pub fn center_screen(&self) -> Vec2 {
        Vec2::new(fx_int(self.width_px / 2), fx_int(self.height_px / 2))
    }
}

/// Integer to `Fx`, saturating to the representable range.
pub(crate) fn fx_int(n: i32) -> Fx {
    Fx::from_int(n.clamp(-32768, 32767))
}
