//! [`CobScripts`]: the COB VM (`tak-script`) as the sim's [`ScriptHost`].
//!
//! - One [`ScriptType`] per unit definition: the shared [`Program`] and the
//!   COB piece index → 3DO piece index map, built **by case-insensitive
//!   name** (the COB piece order never matches the 3DO's, `cob-vm.md`).
//! - One [`Instance`] per live unit, created on spawn with `Create` started.
//!   Each tick the instance advances once, in unit slot order, with an
//!   [`Engine`] that answers ports from the [`ScriptCtx`] snapshot and draws
//!   random numbers from the world RNG.
//! - The hash covers every instance's full VM state plus the port registers
//!   scripts wrote. [`ScriptStats`] (unknown ports, runaway loops) are not
//!   hashed.
//!
//! Port mapping: `docs/sim.md` ("Script ports").

use std::collections::BTreeMap;
use std::sync::Arc;

use tak_formats::model3do::Model;
use tak_math::{Angle, Fx, Rng, Vec2};
use tak_script::{CallOutcome, Engine, Instance, Program, VmError, port};

use crate::hash::StateHasher;
use crate::map::{MapInfo, terrain};
use crate::script::{ScriptCtx, ScriptHost, ScriptReturn, ScriptStats};
use crate::{UnitDefId, UnitId, WorldPos};

/// Port numbers above TA's 20 that the host implements. **Believed**: these
/// are the Kingdoms port numbers the Spring engine later adopted
/// (`IN_WATER`, `CURRENT_SPEED`, `VETERAN_LEVEL`, `ON_ROAD`, and the weapon
/// aim flags 21..23); the meanings fit how the retail scripts use them.
pub mod kport {
    /// Written by `AimWeapon`/`RestoreAfterDelay` (Believed: aim aborted).
    pub const WEAPON_AIM_ABORTED: i32 = 21;
    /// Believed: weapon ready.
    pub const WEAPON_READY: i32 = 22;
    /// Believed: launch now.
    pub const WEAPON_LAUNCH_NOW: i32 = 23;
    /// 1 while the unit stands in water.
    pub const IN_WATER: i32 = 28;
    /// Current speed, 16.16 world units per tick (`MoveWatcher`: `> 5` is
    /// moving).
    pub const CURRENT_SPEED: i32 = 29;
    /// Veterancy (no experience yet: always 0).
    pub const VETERAN_LEVEL: i32 = 32;
    /// 1 while the unit stands on paved ground.
    pub const ON_ROAD: i32 = 34;
}

/// `MoveWatcher`'s threshold: port 29 above this means "moving".
pub const MOVING_SPEED_THRESHOLD: i32 = 5;

/// `setSFXoccupy` argument for a unit on land (**Believed**, TA: 4 = land,
/// 2 = water surface). Zhon flyers treat 5 as airborne; nothing flies yet.
pub const OCCUPY_LAND: i32 = 4;
/// `setSFXoccupy` argument for a unit in water (**Believed**, TA).
pub const OCCUPY_WATER: i32 = 2;

/// Pack a ground point the way TA scripts expect: `x << 16 | (y & 0xffff)`,
/// whole world units (**Believed**, TA `PACKXZ`).
pub fn pack_xz(p: WorldPos) -> i32 {
    let x = p.x.to_int_floor() as i16 as u16 as u32;
    let z = p.y.to_int_floor() as i16 as u16 as u32;
    ((x << 16) | z) as i32
}

/// Inverse of [`pack_xz`] (both halves sign-extended).
pub fn unpack_xz(v: i32) -> WorldPos {
    Vec2::from_ints(i32::from((v >> 16) as i16), i32::from(v as i16))
}

/// Script data shared by every unit of one definition.
#[derive(Clone, Debug)]
pub struct ScriptType {
    /// Unit name, for diagnostics.
    pub name: String,
    /// Decoded script.
    pub program: Arc<Program>,
    /// COB piece index → model piece index (`None`: no piece of that name).
    pub piece_map: Vec<Option<u16>>,
    /// COB piece index → static model-space origin of the mapped model
    /// piece (sum of 3DO offsets from the root, raw 16.16 `[x, y, z]`).
    pub piece_origins: Vec<Option<[i32; 3]>>,
}

impl ScriptType {
    /// Build from a program and (optionally) the unit's model. Without a
    /// model every piece is unmapped; the VM does not need the map, only a
    /// renderer does.
    pub fn new(name: &str, program: Arc<Program>, model: Option<&Model>) -> ScriptType {
        let piece_map = program
            .script()
            .pieces
            .iter()
            .map(|p| {
                model
                    .and_then(|m| m.find(p))
                    .and_then(|i| u16::try_from(i).ok())
            })
            .collect::<Vec<_>>();
        let piece_origins = piece_map
            .iter()
            .map(|m| Some(model?.origin(usize::from((*m)?))))
            .collect();
        ScriptType {
            name: name.to_string(),
            program,
            piece_map,
            piece_origins,
        }
    }

    /// Pieces with no model piece of the same name.
    pub fn unmapped(&self) -> usize {
        self.piece_map.iter().filter(|m| m.is_none()).count()
    }
}

#[derive(Clone, Debug)]
struct UnitScript {
    def: UnitDefId,
    vm: Instance,
    /// Values scripts `set`, by port. Hashed.
    regs: BTreeMap<i32, i32>,
    /// Last `setSFXoccupy` argument sent (0 = never). Hashed.
    occupy: i32,
    // Diagnostics only (not hashed).
    last_pose: u64,
    logged_errors: usize,
}

/// COB scripts for every unit. See the module docs.
#[derive(Clone, Debug)]
pub struct CobScripts {
    map: MapInfo,
    /// Indexed by `UnitDefId`; `None` = no script for that definition.
    types: Vec<Option<ScriptType>>,
    units: BTreeMap<UnitId, UnitScript>,
    stats: ScriptStats,
}

impl CobScripts {
    /// A host for a match on `map` with one optional script type per unit
    /// definition (index = `UnitDefId`). `missing` lists definitions whose
    /// script could not be loaded, for [`ScriptStats::missing`].
    pub fn new(map: MapInfo, types: Vec<Option<ScriptType>>, missing: Vec<String>) -> CobScripts {
        let unmapped = types.iter().flatten().map(|t| t.unmapped() as u64).sum();
        CobScripts {
            map,
            types,
            units: BTreeMap::new(),
            stats: ScriptStats {
                missing,
                unmapped_pieces: unmapped,
                ..ScriptStats::default()
            },
        }
    }

    /// The script type of a definition.
    pub fn script_type(&self, def: UnitDefId) -> Option<&ScriptType> {
        self.types.get(def.0 as usize)?.as_ref()
    }

    /// A unit's VM (for renderers and tests).
    pub fn instance(&self, unit: UnitId) -> Option<&Instance> {
        self.units.get(&unit).map(|u| &u.vm)
    }

    fn type_name(&self, def: UnitDefId) -> &str {
        self.script_type(def).map_or("?", |t| t.name.as_str())
    }
}

impl ScriptHost for CobScripts {
    fn create_unit(&mut self, ctx: &ScriptCtx, _rng: &mut Rng) {
        let Some(t) = self.types.get(ctx.def.0 as usize).and_then(Option::as_ref) else {
            return;
        };
        let mut vm = Instance::new(Arc::clone(&t.program));
        // A missing Create is not an error: the unit just has no behaviour.
        let _ = vm.start("Create", &[]);
        let last_pose = vm.anim_hash();
        self.units.insert(
            ctx.unit,
            UnitScript {
                def: ctx.def,
                vm,
                regs: BTreeMap::new(),
                occupy: 0,
                last_pose,
                logged_errors: 0,
            },
        );
        self.stats.units = self.units.len() as u64;
    }

    fn tick_unit(&mut self, ctx: &ScriptCtx, rng: &mut Rng) {
        let Some(us) = self.units.get_mut(&ctx.unit) else {
            return;
        };
        let mut eng = UnitEngine {
            ctx,
            map: &self.map,
            rng,
            regs: &mut us.regs,
            stats: &mut self.stats,
        };
        us.vm.tick(&mut eng);
        // TA calls setSFXoccupy when the terrain class under the unit
        // changes; after the tick so `Create` has initialised the statics.
        let occupy = if ctx.terrain & terrain::WATER != 0 {
            OCCUPY_WATER
        } else {
            OCCUPY_LAND
        };
        if occupy != us.occupy {
            us.occupy = occupy;
            if us.vm.program().function("setSFXoccupy").is_some() {
                let _ = us.vm.call(&mut eng, "setSFXoccupy", &[occupy]);
            }
        }

        let pose = us.vm.anim_hash();
        if pose != us.last_pose {
            self.stats.pose_ticks += 1;
            if ctx.speed > Fx::ZERO {
                self.stats.pose_ticks_moving += 1;
            }
            us.last_pose = pose;
        }
        let errors = us.vm.errors();
        if errors.len() > us.logged_errors {
            let def = us.def;
            let new: Vec<_> = errors[us.logged_errors..].to_vec();
            us.logged_errors = errors.len();
            let program = Arc::clone(us.vm.program());
            let unit = self.type_name(def).to_string();
            for e in new {
                let func = program
                    .script()
                    .functions
                    .get(e.function as usize)
                    .map_or("?", |f| f.name.as_str());
                if let VmError::InstructionLimit(_) = e.error {
                    *self
                        .stats
                        .runaway
                        .entry(format!("{unit}:{func}"))
                        .or_default() += 1;
                } else {
                    *self
                        .stats
                        .errors
                        .entry(format!("{unit}:{func}: {}", e.error))
                        .or_default() += 1;
                }
            }
        }
    }

    fn call_script(
        &mut self,
        ctx: &ScriptCtx,
        rng: &mut Rng,
        name: &str,
        args: &[i32],
    ) -> Option<ScriptReturn> {
        let us = self.units.get_mut(&ctx.unit)?;
        us.vm.program().function(name)?;
        let mut eng = UnitEngine {
            ctx,
            map: &self.map,
            rng,
            regs: &mut us.regs,
            stats: &mut self.stats,
        };
        match us.vm.call(&mut eng, name, args) {
            Ok(CallOutcome::Returned { value, locals }) => Some(ScriptReturn { value, locals }),
            _ => None,
        }
    }

    fn start_script(&mut self, unit: UnitId, name: &str, args: &[i32]) -> bool {
        self.units
            .get_mut(&unit)
            .is_some_and(|us| us.vm.start(name, args).is_ok())
    }

    fn port_value(&self, unit: UnitId, port: i32) -> Option<i32> {
        let us = self.units.get(&unit)?;
        Some(us.regs.get(&port).copied().unwrap_or(0))
    }

    fn piece_origin(&self, unit: UnitId, piece: i32) -> Option<[i32; 3]> {
        let def = self.units.get(&unit)?.def;
        let t = self.script_type(def)?;
        *t.piece_origins.get(usize::try_from(piece).ok()?)?
    }

    fn remove_unit(&mut self, unit: UnitId) {
        self.units.remove(&unit);
        self.stats.units = self.units.len() as u64;
    }

    fn has_function(&self, unit: UnitId, name: &str) -> bool {
        self.units
            .get(&unit)
            .is_some_and(|us| us.vm.program().function(name).is_some())
    }

    fn port(&self, unit: UnitId, port: i32) -> Option<i32> {
        self.units
            .get(&unit)
            .map(|us| us.regs.get(&port).copied().unwrap_or(0))
    }

    fn set_port(&mut self, unit: UnitId, port: i32, value: i32) {
        if let Some(us) = self.units.get_mut(&unit) {
            us.regs.insert(port, value);
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn hash_state(&self, h: &mut StateHasher) {
        h.bytes(b"COB");
        h.len(self.units.len());
        for (id, u) in &self.units {
            h.unit_id(*id);
            h.unit_def(u.def);
            h.u64(u.vm.state_hash());
            h.len(u.regs.len());
            for (k, v) in &u.regs {
                h.i32(*k);
                h.i32(*v);
            }
            h.i32(u.occupy);
        }
    }

    fn stats(&self) -> ScriptStats {
        self.stats.clone()
    }
}

/// The world as one unit's script sees it during one call.
struct UnitEngine<'a> {
    ctx: &'a ScriptCtx,
    map: &'a MapInfo,
    rng: &'a mut Rng,
    regs: &'a mut BTreeMap<i32, i32>,
    stats: &'a mut ScriptStats,
}

impl UnitEngine<'_> {
    fn reg(&self, port: i32) -> i32 {
        self.regs.get(&port).copied().unwrap_or(0)
    }

    /// Answer a port read. `args` are `get`'s operands (zero for
    /// `get-unit-value`); unit-id arguments other than 0 (self) are not
    /// supported yet and read as self.
    fn read(&mut self, p: i32, args: [i32; 4]) -> i32 {
        *self.stats.reads.entry(p).or_default() += 1;
        let c = self.ctx;
        match p {
            port::ACTIVATION
            | port::STANDINGMOVEORDERS
            | port::INBUILDSTANCE
            | port::BUSY
            | port::YARD_OPEN
            | port::BUGGER_OFF
            | port::ARMORED
            | kport::WEAPON_AIM_ABORTED
            | kport::WEAPON_READY
            | kport::WEAPON_LAUNCH_NOW => self.reg(p),
            port::STANDINGFIREORDERS => match c.fire_state {
                tak_commands::FireState::HoldFire => 0,
                // Chosen: Defensive shares the middle script-port value.
                // The legacy ReturnFire state and its port value are unchanged.
                tak_commands::FireState::ReturnFire | tak_commands::FireState::Defensive => 1,
                tak_commands::FireState::FireAtWill => 2,
            },
            // Percent, from the combat lane.
            port::HEALTH => c.health_percent,
            // Piece positions are not resolved yet: the unit's own.
            port::PIECE_XZ | port::UNIT_XZ => pack_xz(c.pos),
            port::PIECE_Y | port::UNIT_Y => c.ground_height.raw(),
            port::UNIT_HEIGHT => 0,
            port::XZ_ATAN => {
                let d = unpack_xz(args[0]) - c.pos;
                i32::from(c.heading.delta_to(d.angle()))
            }
            port::XZ_HYPOT => unpack_xz(args[0]).distance(c.pos).raw(),
            port::ATAN => {
                i32::from(Angle::atan2(Fx::from_raw(args[0]), Fx::from_raw(args[1])).raw())
            }
            port::HYPOT => Vec2::new(Fx::from_raw(args[0]), Fx::from_raw(args[1]))
                .length()
                .raw(),
            port::GROUND_HEIGHT => self.map.height_at(unpack_xz(args[0])).raw(),
            // Every unit is fully built.
            // Percent left to build, from the construction lane.
            port::BUILD_PERCENT_LEFT => c.build_percent_left,
            kport::IN_WATER => i32::from(c.terrain & terrain::WATER != 0),
            kport::CURRENT_SPEED => {
                let v = c.speed.raw();
                if v > MOVING_SPEED_THRESHOLD {
                    self.stats.speed_reads_moving += 1;
                }
                v
            }
            kport::VETERAN_LEVEL => 0,
            kport::ON_ROAD => i32::from(c.terrain & terrain::PAVED != 0),
            _ => {
                *self.stats.unknown_reads.entry(p).or_default() += 1;
                0
            }
        }
    }
}

impl Engine for UnitEngine<'_> {
    fn get_unit_value(&mut self, port: i32) -> i32 {
        self.read(port, [0; 4])
    }

    fn get(&mut self, port: i32, args: [i32; 4]) -> i32 {
        self.read(port, args)
    }

    fn set(&mut self, p: i32, value: i32) {
        *self.stats.writes.entry(p).or_default() += 1;
        let known = port::is_ta(p)
            || matches!(
                p,
                kport::WEAPON_AIM_ABORTED | kport::WEAPON_READY | kport::WEAPON_LAUNCH_NOW
            );
        if !known {
            *self.stats.unknown_writes.entry(p).or_default() += 1;
        }
        self.regs.insert(p, value);
    }

    fn rand(&mut self, low: i32, high: i32) -> i32 {
        let (lo, hi) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        if hi == i32::MAX {
            return lo.wrapping_add(self.rng.below(hi.wrapping_sub(lo) as u32) as i32);
        }
        self.rng.range_i32(lo, hi + 1)
    }

    fn emit_sfx(&mut self, _piece: u32, _sfx: i32) {
        self.stats.effects += 1;
    }

    fn explode(&mut self, _piece: u32, _flags: i32) {
        self.stats.effects += 1;
    }

    fn play_sound(&mut self, _sound: u32, _priority: i32) -> i32 {
        self.stats.effects += 1;
        0
    }

    fn mission_command(&mut self, _command: u32, _args: &[i32]) -> i32 {
        self.stats.effects += 1;
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_commands::{FireState, PlayerId, Tick};

    fn ctx(speed: Fx, terrain: u8) -> ScriptCtx {
        ScriptCtx {
            tick: Tick(0),
            unit: UnitId::new(0, 0),
            def: UnitDefId(0),
            owner: PlayerId(0),
            pos: Vec2::from_ints(100, 200),
            ground_height: Fx::from_int(60),
            terrain,
            heading: Angle::ZERO,
            speed,
            moving: speed > Fx::ZERO,
            fire_state: FireState::FireAtWill,
            health_percent: 100,
            build_percent_left: 0,
        }
    }

    #[test]
    fn packing_round_trips() {
        let p = Vec2::from_ints(3000, 1234);
        assert_eq!(unpack_xz(pack_xz(p)), p);
        assert_eq!(pack_xz(p), (3000 << 16) | 1234);
    }

    #[test]
    fn defensive_uses_middle_script_port_without_changing_legacy_values() {
        let map = MapInfo::flat(512, 512, vec![]);
        let mut rng = Rng::new(1);
        let mut regs = BTreeMap::new();
        let mut stats = ScriptStats::default();
        for (state, expected) in [
            (FireState::HoldFire, 0),
            (FireState::ReturnFire, 1),
            (FireState::FireAtWill, 2),
            (FireState::Defensive, 1),
        ] {
            let mut c = ctx(Fx::ZERO, 0);
            c.fire_state = state;
            let mut e = UnitEngine {
                ctx: &c,
                map: &map,
                rng: &mut rng,
                regs: &mut regs,
                stats: &mut stats,
            };
            assert_eq!(e.get_unit_value(port::STANDINGFIREORDERS), expected);
        }
    }

    #[test]
    fn ports_answer_from_unit_state() {
        let map = MapInfo::flat(512, 512, vec![]);
        let mut rng = Rng::new(1);
        let mut regs = BTreeMap::new();
        let mut stats = ScriptStats::default();
        let c = ctx(
            Fx::from_ratio(17, 10).unwrap(),
            terrain::WATER | terrain::PAVED,
        );
        let mut e = UnitEngine {
            ctx: &c,
            map: &map,
            rng: &mut rng,
            regs: &mut regs,
            stats: &mut stats,
        };
        assert!(e.get_unit_value(kport::CURRENT_SPEED) > MOVING_SPEED_THRESHOLD);
        assert_eq!(e.get_unit_value(kport::IN_WATER), 1);
        assert_eq!(e.get_unit_value(kport::ON_ROAD), 1);
        assert_eq!(e.get_unit_value(port::BUILD_PERCENT_LEFT), 0);
        assert_eq!(e.get_unit_value(port::HEALTH), 100);
        assert_eq!(e.get_unit_value(port::STANDINGFIREORDERS), 2);
        assert_eq!(e.get_unit_value(port::UNIT_Y), Fx::from_int(60).raw());
        assert_eq!(
            e.get_unit_value(port::UNIT_XZ),
            pack_xz(Vec2::from_ints(100, 200))
        );
        // Point straight south of a unit facing east: a quarter turn.
        let south = pack_xz(Vec2::from_ints(100, 300));
        assert_eq!(e.get(port::XZ_ATAN, [south, 0, 0, 0]), 0x4000);
        assert_eq!(
            e.get(port::XZ_HYPOT, [south, 0, 0, 0]),
            Fx::from_int(100).raw()
        );
        e.set(kport::WEAPON_READY, 7);
        e.set(41, 3);
        assert_eq!(e.get_unit_value(kport::WEAPON_READY), 7);
        assert_eq!(e.get_unit_value(41), 0, "unknown ports read 0");
        for _ in 0..50 {
            assert!((1..=3).contains(&e.rand(3, 1)));
        }
        assert_eq!(stats.unknown_reads.get(&41), Some(&1));
        assert_eq!(stats.unknown_writes.get(&41), Some(&1));
        assert!(!stats.unknown_writes.contains_key(&kport::WEAPON_READY));
        assert_eq!(stats.speed_reads_moving, 1);

        let still = ctx(Fx::ZERO, 0);
        let mut stats = ScriptStats::default();
        let mut e = UnitEngine {
            ctx: &still,
            map: &map,
            rng: &mut rng,
            regs: &mut regs,
            stats: &mut stats,
        };
        assert_eq!(e.get_unit_value(kport::CURRENT_SPEED), 0);
        assert_eq!(e.get_unit_value(kport::IN_WATER), 0);
        assert_eq!(stats.speed_reads_moving, 0);
    }
}
