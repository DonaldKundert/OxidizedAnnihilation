//! World state: players, the unit arena, the RNG and the static match data.
//!
//! Containers are `Vec`s (iterated by index) and a `VecDeque` per unit for
//! its order queue; nothing is hashed or iterated in an unspecified order.

use std::collections::VecDeque;
use std::hash::Hash;

use tak_commands::{FireState, PlayerId, Target, Tick, UnitDefId, UnitId};
use tak_math::{Angle, Fx, Rng};

use crate::WorldPos;
use crate::defs::{UnitDef, UnitDefs};
use crate::hash::{LeHasher, StateHasher};
use crate::map::MapInfo;
use crate::scenario::{Scenario, ScenarioError};

/// Most queued orders a unit keeps; further appends are dropped.
pub const MAX_ORDERS: usize = 256;

/// Most units alive at once; spawns beyond this fail.
pub const MAX_UNITS: usize = 1 << 16;

/// A queued order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Drive to a ground point (already clamped to the map).
    Move {
        /// Destination.
        to: WorldPos,
    },
    /// Attack a unit or a ground point (combat, WP-M).
    Attack {
        /// What to attack.
        target: Target,
    },
    /// Follow and protect a unit (combat, WP-M).
    Guard {
        /// Unit to guard.
        target: UnitId,
    },
    /// Patrol leg toward a point; re-queued at the back when reached
    /// (pathfinding, WP-L).
    Patrol {
        /// Leg end.
        to: WorldPos,
    },
    /// Build a unit: at a ground point for mobile builders, or in place
    /// (`at: None`) for factories (construction, WP-O).
    Build {
        /// What to build.
        def: UnitDefId,
        /// Where, for builders that place structures.
        at: Option<WorldPos>,
    },
}

impl Order {
    fn hash(&self, h: &mut StateHasher) {
        match *self {
            Order::Move { to } => {
                h.u8(0);
                h.vec2(to);
            }
            Order::Attack { target } => {
                h.u8(1);
                match target {
                    Target::Ground(p) => {
                        h.u8(0);
                        h.vec2(p);
                    }
                    Target::Unit(id) => {
                        h.u8(1);
                        h.unit_id(id);
                    }
                }
            }
            Order::Guard { target } => {
                h.u8(2);
                h.unit_id(target);
            }
            Order::Patrol { to } => {
                h.u8(3);
                h.vec2(to);
            }
            Order::Build { def, at } => {
                h.u8(4);
                h.unit_def(def);
                h.option(at, StateHasher::vec2);
            }
        }
    }
}

/// One unit's simulation state.
///
/// `heading` is a binary angle on the ground plane: [`Angle::ZERO`] faces
/// `+x` (east), [`Angle::QUARTER`] faces `+y` (south); the unit's forward
/// vector is `Vec2::from_angle(heading)`. A renderer maps this to the 3DO
/// model's forward axis (`-Z`, Y up) with a fixed rotation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// Definition.
    pub def: UnitDefId,
    /// Owning player.
    pub owner: PlayerId,
    /// Ground position.
    pub pos: WorldPos,
    /// Facing.
    pub heading: Angle,
    /// Forward speed, world units per tick, never negative.
    pub speed: Fx,
    /// Fire state toggle.
    pub fire_state: FireState,
    /// Order queue; the front is the active order.
    pub orders: VecDeque<Order>,
    /// Terrain height under `pos` (height units, Believed 1 world unit per
    /// step), refreshed whenever the unit moves.
    pub ground: Fx,
    /// Terrain flags under `pos` ([`crate::map::terrain`]), refreshed with
    /// `ground`.
    pub terrain: u8,
    /// Pathfinding state (WP-L).
    pub path: crate::pathfinding::UnitPath,
    /// Combat state (WP-M).
    pub combat: crate::combat::UnitCombat,
    /// Economy state (WP-N).
    pub economy: crate::economy::UnitEconomy,
    /// Construction state (WP-O).
    pub build: crate::construction::UnitBuild,
}

impl Unit {
    /// Refresh `ground` and `terrain` from the map.
    pub(crate) fn sample_terrain(&mut self, map: &MapInfo) {
        self.ground = map.height_at(self.pos);
        self.terrain = map.terrain_at(self.pos);
    }

    fn hash(&self, h: &mut StateHasher) {
        h.unit_def(self.def);
        h.player(self.owner);
        h.vec2(self.pos);
        h.fx(self.ground);
        h.u8(self.terrain);
        h.angle(self.heading);
        h.fx(self.speed);
        h.u8(match self.fire_state {
            FireState::HoldFire => 0,
            FireState::ReturnFire => 1,
            FireState::FireAtWill => 2,
        });
        h.len(self.orders.len());
        for o in &self.orders {
            o.hash(h);
        }
        self.path.hash(h);
        self.combat.hash(h);
        self.economy.hash(h);
        self.build.hash(h);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot {
    generation: u32,
    unit: Option<Unit>,
}

/// A player slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    /// Slot id.
    pub id: PlayerId,
    /// Start position.
    pub start: WorldPos,
    /// Definition spawned on tick 0.
    pub monarch_def: UnitDefId,
    /// The monarch, once spawned.
    pub monarch: Option<UnitId>,
}

/// The whole simulation state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct World {
    pub(crate) tick: Tick,
    pub(crate) rng: Rng,
    map: MapInfo,
    defs: UnitDefs,
    pub(crate) players: Vec<Player>,
    slots: Vec<Slot>,
    /// Digest of the immutable match data (map, defs), folded into every
    /// state hash.
    static_digest: u64,
    /// Pathfinding state (WP-L).
    pub pathing: crate::pathfinding::PathState,
    /// Combat state (WP-M).
    pub combat: crate::combat::CombatState,
    /// Economy state (WP-N).
    pub economy: crate::economy::EconomyState,
    /// Construction state (WP-O).
    pub construction: crate::construction::ConstructionState,
}

impl World {
    /// Build the pre-tick-0 world from a scenario (validated first). No units
    /// exist until tick 0 runs.
    pub fn new(scenario: &Scenario) -> Result<World, ScenarioError> {
        scenario.validate()?;
        let map = scenario.map.clone();
        let defs = UnitDefs::new(scenario.unit_defs.clone());
        let players = scenario
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| Player {
                id: PlayerId(i as u8),
                start: map.start_positions[p.start as usize].to_world(),
                monarch_def: p.monarch,
                monarch: None,
            })
            .collect();
        let mut h = StateHasher::new();
        map.hash(&mut h);
        defs.hash(&mut h);
        let static_digest = h.finish();
        Ok(World {
            tick: Tick(0),
            rng: Rng::new(scenario.seed),
            map,
            defs,
            players,
            slots: Vec::new(),
            static_digest,
            pathing: Default::default(),
            combat: Default::default(),
            economy: crate::economy::EconomyState::new(&scenario.economy, scenario.players.len()),
            construction: Default::default(),
        })
    }

    /// The next tick to run (the number of ticks run so far).
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// The map.
    pub fn map(&self) -> &MapInfo {
        &self.map
    }

    /// Unit definitions.
    pub fn defs(&self) -> &UnitDefs {
        &self.defs
    }

    /// Players in id order.
    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// The world RNG (the only randomness source in the sim).
    pub fn rng_mut(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// `true` if `p` is a player in this match.
    pub fn has_player(&self, p: PlayerId) -> bool {
        (p.0 as usize) < self.players.len()
    }

    /// A live unit, or `None` if the id is stale or out of range.
    pub fn unit(&self, id: UnitId) -> Option<&Unit> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.unit.as_ref()
    }

    /// Mutable access to a live unit.
    pub fn unit_mut(&mut self, id: UnitId) -> Option<&mut Unit> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.unit.as_mut()
    }

    /// A live unit owned by `player`; `None` if stale or someone else's.
    pub fn unit_mut_owned(&mut self, id: UnitId, player: PlayerId) -> Option<&mut Unit> {
        self.unit_mut(id).filter(|u| u.owner == player)
    }

    /// Live units in ascending slot order (the canonical iteration order).
    pub fn units(&self) -> impl Iterator<Item = (UnitId, &Unit)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.unit
                .as_ref()
                .map(|u| (UnitId::new(i as u32, s.generation), u))
        })
    }

    /// Number of live units.
    pub fn unit_count(&self) -> usize {
        self.slots.iter().filter(|s| s.unit.is_some()).count()
    }

    /// Ids of live units in slot order.
    pub(crate) fn unit_ids(&self) -> Vec<UnitId> {
        self.units().map(|(id, _)| id).collect()
    }

    /// The unit and its definition, for systems.
    pub(crate) fn unit_and_def_mut(
        &mut self,
        id: UnitId,
    ) -> Option<(&mut Unit, &UnitDef, &MapInfo)> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let unit = slot.unit.as_mut()?;
        let def = self.defs.get(unit.def)?;
        Some((unit, def, &self.map))
    }

    /// Spawn a unit in the lowest free slot (so slot reuse is a pure function
    /// of the live set, with no free-list state). `None` if the arena is full
    /// or `def` does not exist.
    pub fn spawn(
        &mut self,
        def: UnitDefId,
        owner: PlayerId,
        pos: WorldPos,
        heading: Angle,
    ) -> Option<UnitId> {
        self.defs.get(def)?;
        let mut unit = Unit {
            def,
            owner,
            pos: self.map.clamp(pos),
            heading,
            speed: Fx::ZERO,
            fire_state: FireState::default(),
            orders: VecDeque::new(),
            ground: Fx::ZERO,
            terrain: 0,
            path: Default::default(),
            combat: Default::default(),
            economy: Default::default(),
            build: Default::default(),
        };
        unit.sample_terrain(&self.map);
        if let Some(i) = self.slots.iter().position(|s| s.unit.is_none()) {
            let slot = &mut self.slots[i];
            slot.unit = Some(unit);
            return Some(UnitId::new(i as u32, slot.generation));
        }
        if self.slots.len() >= MAX_UNITS {
            return None;
        }
        self.slots.push(Slot {
            generation: 0,
            unit: Some(unit),
        });
        Some(UnitId::new((self.slots.len() - 1) as u32, 0))
    }

    /// Remove a unit, bumping its slot's generation so `id` goes stale.
    /// Returns the removed unit.
    pub fn despawn(&mut self, id: UnitId) -> Option<Unit> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let u = slot.unit.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        Some(u)
    }

    /// Feed the canonical serialization of the world into `h`.
    ///
    /// Layout (all little-endian): magic `b"OXASIM"`, format `u32 = 2`,
    /// tick `u32`, RNG state (two `u64`: state, increment), static digest
    /// `u64`, player count `u32` then per player `id u8, start (x, y) i32
    /// raw, monarch_def u32, monarch option`, slot count `u32` then per slot
    /// `generation u32, occupied u8` and, if occupied, `def u32, owner u8,
    /// pos x,y i32 raw, ground i32 raw, terrain u8, heading u16, speed i32 raw, fire_state u8, order
    /// count u32, orders (tag u8 + payload)`.
    pub fn hash_into(&self, h: &mut StateHasher) {
        h.bytes(b"OXASIM");
        h.u32(3);
        h.tick(self.tick);
        self.rng.hash(&mut LeHasher(h));
        h.u64(self.static_digest);
        h.len(self.players.len());
        for p in &self.players {
            h.player(p.id);
            h.vec2(p.start);
            h.unit_def(p.monarch_def);
            h.option(p.monarch, StateHasher::unit_id);
        }
        h.len(self.slots.len());
        for s in &self.slots {
            h.u32(s.generation);
            match &s.unit {
                None => h.u8(0),
                Some(u) => {
                    h.u8(1);
                    u.hash(h);
                }
            }
        }
        self.pathing.hash(h);
        self.combat.hash(h);
        self.economy.hash(h);
        self.construction.hash(h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::UnitDef;
    use crate::map::StartPos;
    use crate::scenario::{PlayerSpec, SCENARIO_FORMAT_VERSION};

    fn world() -> World {
        World::new(&Scenario {
            version: SCENARIO_FORMAT_VERSION,
            seed: 1,
            map: MapInfo::flat(512, 512, vec![StartPos { x: 10, y: 10 }]),
            unit_defs: vec![UnitDef {
                name: "u".into(),
                maxvelocity: Fx::ONE,
                acceleration: Fx::HALF,
                brakerate: Fx::HALF,
                turnrate: 1000,
                footprintx: 1,
                footprintz: 1,
                economy: Default::default(),
                build: Default::default(),
                combat: Default::default(),
            }],
            players: vec![PlayerSpec {
                start: 0,
                monarch: UnitDefId(0),
            }],
            economy: Default::default(),
        })
        .unwrap()
    }

    #[test]
    fn generations_make_ids_stale() {
        let mut w = world();
        let p = PlayerId(0);
        let a = w
            .spawn(UnitDefId(0), p, WorldPos::ZERO, Angle::ZERO)
            .unwrap();
        let b = w
            .spawn(UnitDefId(0), p, WorldPos::ZERO, Angle::ZERO)
            .unwrap();
        assert_eq!((a.index, b.index), (0, 1));
        assert!(w.despawn(a).is_some());
        assert!(w.unit(a).is_none());
        assert!(w.despawn(a).is_none());
        let c = w
            .spawn(UnitDefId(0), p, WorldPos::ZERO, Angle::ZERO)
            .unwrap();
        assert_eq!(c, UnitId::new(0, 1));
        assert!(w.unit(a).is_none());
        assert!(w.unit(c).is_some());
        assert!(w.unit(UnitId::new(99, 0)).is_none());
        assert!(w.unit_mut_owned(c, PlayerId(1)).is_none());
        assert!(
            w.spawn(UnitDefId(5), p, WorldPos::ZERO, Angle::ZERO)
                .is_none()
        );
    }
}
