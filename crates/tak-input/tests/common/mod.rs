//! A plain-struct `WorldView` for tests, loadable from readable JSON with
//! integer world coordinates.

#![allow(dead_code)]

use serde::Deserialize;
use tak_commands::{PlayerId, SpellId, Tick, UnitDefId, UnitId};
use tak_input::{BuilderKind, Category, InputEvent, Mapper, Output, UnitInfo, WorldView};
use tak_math::{Fx, Vec2};

fn twelve() -> i32 {
    12
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitSpec {
    pub index: u32,
    #[serde(default)]
    pub generation: u32,
    pub owner: u8,
    pub def: u32,
    pub x: i32,
    pub y: i32,
    #[serde(default = "twelve")]
    pub radius: i32,
    #[serde(default)]
    pub builder: Option<BuilderKind>,
    #[serde(default)]
    pub build_options: Vec<u32>,
    #[serde(default)]
    pub spells: Vec<u32>,
    /// Spells this unit cannot pay for now.
    #[serde(default)]
    pub no_mana: Vec<u32>,
    #[serde(default)]
    pub idle: bool,
    #[serde(default = "yes")]
    pub selectable: bool,
    /// A construction frame rather than a completed unit.
    #[serde(default)]
    pub under_construction: bool,
    /// The local player's monarch.
    #[serde(default)]
    pub monarch: bool,
    /// TA select-categories (`Keys.TDF` names, e.g. "MELEE").
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSpec {
    pub local_player: u8,
    pub enemies: Vec<u8>,
    pub units: Vec<UnitSpec>,
}

#[derive(Clone, Debug)]
pub struct TestWorld {
    pub spec: WorldSpec,
}

impl UnitSpec {
    pub fn id(&self) -> UnitId {
        UnitId::new(self.index, self.generation)
    }
    pub fn info(&self) -> UnitInfo {
        UnitInfo {
            id: self.id(),
            pos: Vec2::from_ints(self.x, self.y),
            radius: Fx::from_int(self.radius),
            owner: PlayerId(self.owner),
            def: UnitDefId(self.def),
            selectable: self.selectable,
            builder: self.builder,
            under_construction: self.under_construction,
        }
    }
}

impl TestWorld {
    pub fn new(spec: WorldSpec) -> TestWorld {
        TestWorld { spec }
    }
    fn find(&self, id: UnitId) -> Option<&UnitSpec> {
        self.spec.units.iter().find(|u| u.id() == id)
    }
}

impl WorldView for TestWorld {
    fn local_player(&self) -> PlayerId {
        PlayerId(self.spec.local_player)
    }
    fn for_each_unit(&self, f: &mut dyn FnMut(&UnitInfo)) {
        for u in &self.spec.units {
            f(&u.info());
        }
    }
    fn is_enemy(&self, owner: PlayerId) -> bool {
        self.spec.enemies.contains(&owner.0)
    }
    fn build_options(&self, unit: UnitId) -> Vec<UnitDefId> {
        self.find(unit)
            .map(|u| u.build_options.iter().map(|&d| UnitDefId(d)).collect())
            .unwrap_or_default()
    }
    fn spells(&self, unit: UnitId) -> Vec<SpellId> {
        self.find(unit)
            .map(|u| u.spells.iter().map(|&s| SpellId(s)).collect())
            .unwrap_or_default()
    }
    fn can_cast(&self, unit: UnitId, spell: SpellId) -> bool {
        self.find(unit)
            .is_some_and(|u| !u.no_mana.contains(&spell.0))
    }
    fn is_idle(&self, unit: UnitId) -> bool {
        self.find(unit).is_some_and(|u| u.idle)
    }
    fn monarch(&self) -> Option<UnitId> {
        self.spec
            .units
            .iter()
            .find(|u| u.monarch && u.owner == self.spec.local_player)
            .map(|u| u.id())
    }
    fn in_category(&self, unit: UnitId, category: Category) -> bool {
        self.find(unit).is_some_and(|u| {
            u.categories
                .iter()
                .any(|c| Category::from_tdf(c) == Some(category))
        })
    }
}

/// Tick for a millisecond timestamp at 30 ticks per second.
pub fn tick_at(t: u32) -> Tick {
    Tick(t * 30 / 1000)
}

/// Run events through a mapper, stamping each with the tick of its time.
pub fn run(mapper: &mut Mapper, world: &TestWorld, events: &[InputEvent]) -> Vec<Output> {
    events
        .iter()
        .flat_map(|e| mapper.handle(e, world, tick_at(e.t)))
        .collect()
}

pub fn uid(i: u32) -> UnitId {
    UnitId::new(i, 0)
}

pub fn pos(x: i32, y: i32) -> Vec2 {
    Vec2::from_ints(x, y)
}

/// The default mouse/keyboard profile with the WP-J tuning (the WP-J tests
/// pin the original feel values).
pub fn wp_j_mk() -> tak_input::Profile {
    let mut p = tak_input::Profile::wp_s_mouse_keyboard();
    p.tuning = tak_input::Tuning::wp_j();
    p
}

/// A mapper plus the camera a client keeps: after every event the
/// requested pan and zoom (about the screen centre) are applied and handed
/// back, so glides and the reticle move over the world.
pub struct CamRig {
    pub m: Mapper,
    pub cam: tak_input::Camera,
}

impl CamRig {
    pub fn new(p: tak_input::Profile) -> CamRig {
        let cam = tak_input::Camera::default();
        let mut m = Mapper::new(p);
        m.set_camera(cam);
        CamRig { m, cam }
    }

    pub fn run(&mut self, world: &TestWorld, events: &[InputEvent]) -> Vec<Output> {
        let mut out = Vec::new();
        for e in events {
            out.extend(self.m.handle(e, world, tick_at(e.t)));
            let pan = self.m.take_camera_pan();
            self.cam.origin += pan;
            if let Some(z) = self.m.take_camera_zoom() {
                let c = self.cam.screen_to_world(self.cam.center_screen());
                self.cam.world_per_px = z;
                let half = self.cam.center_screen().scale(z);
                self.cam.origin = c - half;
            }
            self.m.set_camera(self.cam);
        }
        out
    }
}
