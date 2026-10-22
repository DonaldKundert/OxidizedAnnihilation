//! The match: the deterministic sim plus its command log, with no Bevy types.
//!
//! The client steps [`Session`] exactly the way `tak-headless` steps a
//! match (`Sim::with_scripts(&scenario, Box::new(scripts))`, then
//! `step(log.for_tick(tick))` once per tick), so a log recorded here replays
//! to the same hashes there. Orders from the input layer go into the same
//! log, stamped with the tick they will execute on.

use std::collections::BTreeMap;
use std::path::Path;

use tak_commands::{CommandLog, PlayerId, SimCommand, Tick, UnitDefId, UnitId};
use tak_content::{MapDef, Ruleset, RulesetKind};
use tak_script::PieceTransform;
use tak_sim::load_match;
use tak_sim::{PlayerSetup, Sim};
use tak_vfs::Vfs;

/// Errors while setting up a match.
#[derive(Debug)]
pub struct SetupError(pub String);

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SetupError {}

fn err(e: impl std::fmt::Display) -> SetupError {
    SetupError(e.to_string())
}

/// What the renderer needs to know about one unit definition.
#[derive(Clone, Debug)]
pub struct DefInfo {
    /// FBI `unitname` (sim name), e.g. `ARAKING`.
    pub unitname: String,
    /// Display name, e.g. `Elsin`.
    pub display: String,
    /// 3DO model name without extension (`objectname`, lowercase).
    pub object: String,
    /// Footprint in 16-px cells.
    pub footprint: (u8, u8),
    /// FBI `category` words (e.g. `MELEE`, `FLY`), for TA's select-category
    /// keys. Display/input only.
    pub categories: Vec<String>,
}

/// A running match and its log.
pub struct Session {
    /// The simulation.
    pub sim: Sim,
    /// Every command of the match (fixture plus anything submitted).
    pub log: CommandLog,
    /// Commands submitted by this client (for `--record`).
    pub recorded: CommandLog,
    /// The local player.
    pub local: PlayerId,
    /// Hash after every tick run so far (index = tick).
    pub hashes: Vec<u64>,
    /// Per-definition render info, indexed by `UnitDefId`.
    pub defs: Vec<DefInfo>,
    /// The loaded map (heights, features, OTA).
    pub map: MapDef,
    /// Map VFS path without extension.
    pub map_name: String,
    /// Placed map features (all of them, scenery included).
    pub features: Vec<FeatureBox>,
    /// Art of every feature definition in the ruleset, by lowercase name.
    pub feature_art: BTreeMap<String, FeatureArt>,
    /// How each weapon is drawn, by `(unit def, slot)`; slot
    /// [`EXPLODE_AS_SLOT`] is the death explosion.
    pub weapon_art: BTreeMap<(u32, u8), WeaponArt>,
    /// `gamedata/explosions`: class (lowercase) to its sprite names (one
    /// is picked per explosion).
    pub explosions: BTreeMap<String, Vec<String>>,
}

/// `weapon_art` slot of a unit's `[EXPLODEAS]`.
pub const EXPLODE_AS_SLOT: u8 = u8::MAX;

/// How a weapon is drawn (from its FBI section).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponArt {
    /// `weapontype`, lowercase (`ballistic`, `line of sight`, ...).
    pub kind: String,
    /// `model`: projectile 3DO.
    pub model: Option<String>,
    /// `weapongaf` (file) when set, else the art names its own file.
    pub art_file: Option<String>,
    /// `weaponart`: projectile sprite sequence.
    pub art: Option<String>,
    /// `explosionclass`, lowercase: hit explosion.
    pub explosion: Option<String>,
    /// `hweffect=lightning`: a bolt from shooter to target lasting
    /// `emittime` ticks, in `outercolor`.
    pub lightning: Option<(u32, [u8; 3])>,
}

impl WeaponArt {
    fn from_fbi(w: &tak_content::unit::WeaponDef) -> WeaponArt {
        let nonempty = |v: &Option<String>| v.clone().filter(|s| !s.trim().is_empty());
        let color = w
            .extra
            .get("outercolor")
            .and_then(|c| {
                let v: Vec<u8> = c
                    .split_whitespace()
                    .filter_map(|x| x.parse().ok())
                    .collect();
                (v.len() == 3).then(|| [v[0], v[1], v[2]])
            })
            .unwrap_or([180, 200, 255]);
        let lightning = w
            .extra
            .get("hweffect")
            .filter(|e| e.eq_ignore_ascii_case("lightning"))
            .map(|_| {
                let t = w
                    .extra
                    .get("emittime")
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .unwrap_or(10);
                (t.clamp(1, 60), color)
            });
        WeaponArt {
            kind: w.kind.clone().unwrap_or_default().to_ascii_lowercase(),
            model: nonempty(&w.model).map(|m| m.trim_end_matches(".3do").to_ascii_lowercase()),
            art_file: w
                .extra
                .get("weapongaf")
                .cloned()
                .filter(|s| !s.trim().is_empty()),
            art: nonempty(&w.weapon_art),
            explosion: nonempty(&w.explosion_class).map(|c| c.trim().to_ascii_lowercase()),
            lightning,
        }
    }
}

/// Parse `gamedata/explosions/*.tdf`: `[class] { [0] { gaf = x; } ... }`.
pub fn load_explosions(vfs: &Vfs) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for f in vfs.list("gamedata/explosions/") {
        if !f.path.to_ascii_lowercase().ends_with(".tdf") {
            continue;
        }
        let Ok(bytes) = vfs.read(&f.path) else {
            continue;
        };
        let Ok(doc) = tak_formats::tdf::parse(&bytes) else {
            continue;
        };
        for sec in &doc.sections {
            let names: Vec<String> = sec
                .sections
                .iter()
                .filter_map(|v| {
                    v.entries
                        .iter()
                        .find(|e| e.key.eq_ignore_ascii_case("gaf"))
                        .map(|e| e.value.trim().to_string())
                })
                .filter(|n| !n.is_empty())
                .collect();
            if !names.is_empty() {
                out.insert(sec.name.trim().to_ascii_lowercase(), names);
            }
        }
    }
    out
}

/// How a feature is drawn (from its TDF).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeatureArt {
    /// `object`: a 3DO under `objects3d/`.
    pub object: Option<String>,
    /// `filename`: sprite file `anims/<filename>.gaf`.
    pub filename: Option<String>,
    /// `seqname`: the sequence in `filename`.
    pub seqname: Option<String>,
    /// `seqnameshad`: its shadow sequence.
    pub seqname_shadow: Option<String>,
    /// `animating=1`: play the sequence.
    pub animating: bool,
    /// Footprint in cells.
    pub footprint: (u8, u8),
}

/// A placed map feature.
#[derive(Clone, Debug)]
pub struct FeatureBox {
    /// Feature name.
    pub name: String,
    /// Lowercase definition name (key of `Session::feature_art`), if the
    /// definition exists.
    pub def: Option<String>,
    /// The sim keeps it as a live feature (it matters to gameplay); the
    /// renderer draws those from the sim's feature layer instead.
    pub in_sim: bool,
    /// Anchor cell (top-left, **Chosen** as in `tak-content`).
    pub cell: (u32, u32),
    /// Footprint in cells.
    pub footprint: (u8, u8),
    /// Blocks movement.
    pub blocking: bool,
    /// Height hint from the feature def.
    pub height: f32,
}

/// Open an install's VFS.
pub fn open_vfs(data: &Path) -> Result<Vfs, SetupError> {
    tak_vfs::open_kingdoms(data).map_err(|e| SetupError(format!("{}: {e}", data.display())))
}

impl Session {
    /// The input layer's read-only view of this match.
    pub fn view(&self) -> crate::view::SimView<'_> {
        crate::view::SimView {
            world: self.sim.world(),
            local: self.local,
            defs: Some(&self.defs),
            fog: None,
        }
    }

    /// [`Session::view`] limited to what the local player can see through
    /// `fog` (enemies outside current sight left out).
    pub fn view_fogged<'a>(
        &'a self,
        fog: Option<&'a crate::fog::FogGrid>,
    ) -> crate::view::SimView<'a> {
        crate::view::SimView { fog, ..self.view() }
    }

    /// Load a real map with every player's build tree (the
    /// `load_match_with_builds` path) and the given fixture log.
    pub fn load(
        vfs: &Vfs,
        map: &str,
        players: &str,
        seed: u64,
        log: CommandLog,
    ) -> Result<Session, SetupError> {
        let setups = PlayerSetup::parse_list(players);
        if setups.is_empty() {
            return Err(SetupError("--players names no side".into()));
        }
        let m = load_match(vfs, map, &setups, seed).map_err(err)?;
        let rs = Ruleset::load(vfs, RulesetKind::Base);
        let defs = m
            .scenario
            .unit_defs
            .iter()
            .map(|d| {
                let fbi = rs.unit(&d.name);
                DefInfo {
                    unitname: d.name.clone(),
                    display: fbi
                        .and_then(|u| u.name.clone())
                        .unwrap_or_else(|| d.name.clone()),
                    object: fbi
                        .and_then(|u| u.objectname.clone())
                        .unwrap_or_else(|| d.name.clone())
                        .to_ascii_lowercase(),
                    footprint: (d.footprintx.max(1), d.footprintz.max(1)),
                    categories: fbi.map(|u| u.categories.clone()).unwrap_or_default(),
                }
            })
            .collect();
        let features = m
            .map
            .features
            .iter()
            .map(|pf| {
                let fd = rs.feature(pf.def.as_deref().unwrap_or(&pf.name));
                FeatureBox {
                    name: pf.name.clone(),
                    def: fd.map(|d| d.name.to_ascii_lowercase()),
                    in_sim: fd
                        .is_some_and(|d| tak_sim::features::FeatureDef::from_content(d).matters()),
                    cell: (pf.x, pf.z),
                    footprint: fd.map_or((1, 1), |d| (d.footprint_x.max(1), d.footprint_z.max(1))),
                    blocking: fd.is_some_and(|d| d.blocking),
                    height: fd.and_then(|d| d.height).unwrap_or(12).clamp(4, 60) as f32,
                }
            })
            .collect();
        let feature_art = rs
            .features
            .values()
            .map(|d| {
                let nonempty = |v: &Option<String>| v.clone().filter(|s| !s.trim().is_empty());
                (
                    d.name.to_ascii_lowercase(),
                    FeatureArt {
                        object: nonempty(&d.object),
                        filename: nonempty(&d.filename),
                        seqname: nonempty(&d.seqname),
                        seqname_shadow: nonempty(&d.seqname_shadow),
                        animating: d.extra.get("animating").is_some_and(|v| v.trim() == "1"),
                        footprint: (d.footprint_x.max(1), d.footprint_z.max(1)),
                    },
                )
            })
            .collect();
        let mut weapon_art = BTreeMap::new();
        for (i, d) in m.scenario.unit_defs.iter().enumerate() {
            let Some(u) = rs.unit(&d.name) else { continue };
            for (slot, w) in u
                .weapons
                .iter()
                .enumerate()
                .take(usize::from(EXPLODE_AS_SLOT))
            {
                weapon_art.insert((i as u32, slot as u8), WeaponArt::from_fbi(w));
            }
            if let Some(w) = &u.explode_as {
                weapon_art.insert((i as u32, EXPLODE_AS_SLOT), WeaponArt::from_fbi(w));
            }
        }
        let explosions = load_explosions(vfs);
        let sim = Sim::with_scripts(&m.scenario, Box::new(m.scripts)).map_err(err)?;
        Ok(Session {
            sim,
            log,
            recorded: CommandLog::new(),
            local: PlayerId(0),
            hashes: Vec::new(),
            defs,
            map: m.map,
            map_name: map.to_string(),
            features,
            feature_art,
            weapon_art,
            explosions,
        })
    }

    /// The next tick to run.
    pub fn tick(&self) -> Tick {
        self.sim.tick()
    }

    /// Run one tick with its logged commands; returns the hash.
    pub fn step(&mut self) -> u64 {
        let t = self.sim.tick();
        let h = self.sim.step(self.log.for_tick(t));
        self.hashes.push(h);
        h
    }

    /// Queue an order from the local player for the next tick to run.
    /// Returns the tick it was stamped with.
    pub fn submit(&mut self, command: SimCommand) -> Tick {
        let t = self.sim.tick();
        self.log.record(t, self.local, command.clone());
        self.recorded.record(t, self.local, command);
        t
    }

    /// Render info for a definition.
    pub fn def(&self, id: UnitDefId) -> Option<&DefInfo> {
        self.defs.get(id.0 as usize)
    }

    /// The last hash, if any tick ran.
    pub fn last_hash(&self) -> Option<u64> {
        self.hashes.last().copied()
    }

    /// Snapshot the world for rendering.
    pub fn snapshot(&self) -> Snapshot {
        let w = self.sim.world();
        let cob = self.sim.cob();
        let units = w
            .units()
            .map(|(id, u)| UnitSnap {
                id,
                def: u.def,
                owner: u.owner.0,
                x: u.pos.x.raw() as f32 / 65536.0,
                y: u.pos.y.raw() as f32 / 65536.0,
                ground: u.ground.raw() as f32 / 65536.0,
                heading: u.heading.raw(),
                pieces: cob
                    .and_then(|c| c.instance(id))
                    .map(|i| i.transforms())
                    .unwrap_or_default(),
                health_percent: u.combat.health_percent(),
                build_percent_left: u.build.percent_left(),
                build_done: {
                    let (d, t) = u.build.built_fraction();
                    if t > 0 { d as f32 / t as f32 } else { 1.0 }
                },
                health: u.combat.health,
                max_health: u.combat.max_health,
                earned_health: {
                    let (d, t) = u.build.built_fraction();
                    let max = u.combat.max_health;
                    max - tak_sim::combat::frame_health(max, t - d, t)
                },
                dying: u.combat.is_dying(),
            })
            .collect();
        let tick = w.tick().0;
        let f = |v: tak_math::Fx| v.raw() as f32 / 65536.0;
        let projectiles = w
            .combat
            .projectiles
            .iter()
            .map(|p| ProjSnap {
                key: (p.shooter, weapon_key(p.weapon), tick.wrapping_sub(p.age)),
                weapon: weapon_key(p.weapon),
                x: f(p.pos.x),
                y: f(p.pos.y),
                ground: f(w.map().height_at(p.pos)),
                dest: [f(p.dest.x), f(p.dest.y)],
            })
            .collect();
        Snapshot {
            tick: w.tick().0,
            units,
            projectiles,
        }
    }
}

/// The scripted demo session (`--demo`): player 0's monarch builds a wall
/// next to its start, then both monarchs attack each other across the map.
/// Sites are searched with the sim's own placement check, so the log is a
/// pure function of the match. Run on a freshly loaded session (tick 0).
pub fn demo_log(s: &Session) -> CommandLog {
    use tak_commands::{QueueMode, Target, UnitSet};
    use tak_math::{Angle, Fx, Vec2};
    use tak_sim::construction::content::def_by_name;
    use tak_sim::construction::placement;

    let w = s.sim.world();
    let mut log = CommandLog::new();
    // Monarchs spawn on tick 0 in player order into the lowest free slots.
    let king = |p: usize| UnitId::new(p as u32, 0);
    let Some(p0) = w.players().first() else {
        return log;
    };
    let wall = def_by_name(w.defs(), "ARAWALL").or_else(|| {
        let d = w.defs().get(p0.monarch_def)?;
        d.build.canbuild.first().copied()
    });
    if let Some(wall) = wall
        && let Some(wd) = w.defs().get(wall)
    {
        let start = p0.start;
        let site = (0..64).find_map(|i| {
            let a = Angle::from_raw((i as u16).wrapping_mul(4096 + 512));
            let r = 96 + (i / 16) * 32;
            let p = start + Vec2::from_angle(a).scale(Fx::from_int(r));
            let c = placement::site_center(wd, p);
            placement::check_site(w, wall, c, None).ok()?;
            Some(c)
        });
        if let Some(site) = site {
            log.record(
                Tick(1),
                PlayerId(0),
                SimCommand::Build {
                    units: UnitSet::single(king(0)),
                    def: wall,
                    at: Some(site),
                    queue: QueueMode::Replace,
                },
            );
        }
    }
    if w.players().len() >= 2 {
        log.record(
            Tick(1),
            PlayerId(0),
            SimCommand::Attack {
                units: UnitSet::single(king(0)),
                target: Target::Unit(king(1)),
                queue: QueueMode::Append,
            },
        );
        log.record(
            Tick(1),
            PlayerId(1),
            SimCommand::Attack {
                units: UnitSet::single(king(1)),
                target: Target::Unit(king(0)),
                queue: QueueMode::Replace,
            },
        );
    }
    log
}

/// One unit at one tick, in render-side floats (positions) and raw script
/// poses.
#[derive(Clone, Debug)]
pub struct UnitSnap {
    /// Handle.
    pub id: UnitId,
    /// Definition.
    pub def: UnitDefId,
    /// Owner index.
    pub owner: u8,
    /// World x (east).
    pub x: f32,
    /// World y (south).
    pub y: f32,
    /// Terrain height under the unit.
    pub ground: f32,
    /// Heading, 65536 per turn, 0 = east, 0x4000 = south.
    pub heading: u16,
    /// COB piece poses, indexed by COB piece.
    pub pieces: Vec<PieceTransform>,
    /// Health 0..=100.
    pub health_percent: i32,
    /// Build percent left (0 = complete).
    pub build_percent_left: i32,
    /// Built fraction 0..=1 (1 = complete), render-side float.
    pub build_done: f32,
    /// Hit points.
    pub health: i32,
    /// Maximum hit points.
    pub max_health: i32,
    /// Health the frame has earned by its build progress
    /// (`max - frame_health`); equals `max_health` once complete. Damage
    /// shows as `earned_health - health`.
    pub earned_health: i32,
    /// Playing its death animation.
    pub dying: bool,
}

/// The world at one tick.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Ticks run when taken.
    pub tick: u32,
    /// Live units in slot order.
    pub units: Vec<UnitSnap>,
    /// Projectiles in flight.
    pub projectiles: Vec<ProjSnap>,
}

/// `(unit def, slot)` of a weapon, as in [`Session::weapon_art`].
pub fn weapon_key(w: tak_sim::combat::WeaponRef) -> (u32, u8) {
    match w {
        tak_sim::combat::WeaponRef::Slot(d, s) => (d.0, s),
        tak_sim::combat::WeaponRef::Explosion(d) => (d.0, EXPLODE_AS_SLOT),
    }
}

/// One projectile at one tick.
#[derive(Clone, Debug)]
pub struct ProjSnap {
    /// Identity across ticks: shooter, weapon, launch tick.
    pub key: (UnitId, (u32, u8), u32),
    /// Weapon (`Session::weapon_art` key).
    pub weapon: (u32, u8),
    /// World x.
    pub x: f32,
    /// World y (south).
    pub y: f32,
    /// Terrain height under it.
    pub ground: f32,
    /// Where it lands.
    pub dest: [f32; 2],
}

/// Linear interpolation.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Interpolate a binary angle the short way round; returns radians.
pub fn lerp_angle(a: u16, b: u16, t: f32) -> f32 {
    let d = b.wrapping_sub(a) as i16;
    angle_rad(a) + f32::from(d) * t * std::f32::consts::TAU / 65536.0
}

/// A binary angle in radians.
pub fn angle_rad(a: u16) -> f32 {
    f32::from(a) * std::f32::consts::TAU / 65536.0
}

/// Interpolated piece pose: offsets in model units, rotations in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    /// Script move per axis.
    pub offset: [f32; 3],
    /// Script turn per axis (radians).
    pub rotation: [f32; 3],
    /// Visible (`hide`/`show`).
    pub visible: bool,
}

/// Blend two script poses (`b` wins on visibility).
pub fn lerp_pose(a: Option<&PieceTransform>, b: &PieceTransform, t: f32) -> Pose {
    let a = a.unwrap_or(b);
    let fx = |v: tak_math::Fx| v.raw() as f32 / 65536.0;
    Pose {
        offset: [0, 1, 2].map(|i| lerp(fx(a.offset[i]), fx(b.offset[i]), t)),
        rotation: [0, 1, 2].map(|i| lerp_angle(a.rotation[i].raw(), b.rotation[i].raw(), t)),
        visible: b.visible,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_take_the_short_way() {
        let r = lerp_angle(65000, 500, 0.5);
        let expect = (65000.0 + 518.0) * std::f32::consts::TAU / 65536.0;
        assert!((r - expect).abs() < 1e-3, "{r} vs {expect}");
        assert!((lerp_angle(0x4000, 0x4000, 0.3) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }
}
