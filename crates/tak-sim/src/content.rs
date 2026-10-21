//! Real content into sim terms: `tak_content` definitions and maps become a
//! [`Scenario`], and [`load_match`] builds the scenario plus the COB script
//! host straight from an install.
//!
//! Every unit conversion lives in one small function here so it is easy to
//! correct when better evidence turns up. The choices and their confidence
//! are documented in `docs/sim.md`.

use std::sync::Arc;

use tak_content::{MapDef, Ruleset, RulesetKind, SideDef};
use tak_formats::model3do;
use tak_math::Fx;
use tak_script::Program;
use tak_vfs::Vfs;

use crate::UnitDefId;
use crate::cob::{CobScripts, ScriptType};
use crate::defs::{MAX_VELOCITY_CAP, UnitDef};
use crate::map::{HeightGrid, MapInfo, StartPos, TerrainGrid};
use crate::scenario::{PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario, ScenarioError};

/// World units per TNT attribute cell (one height sample, one passability
/// cell). **Known**: a 32-px terrain piece covers 2 x 2 attribute cells.
pub const CELL_SIZE: i32 = 16;

/// FBI `maxvelocity`/`acceleration`/`brakerate` to sim units (16.16 world
/// units per tick, per tick²).
///
/// **Believed** (TA convention, also what the Spring engine assumes when it
/// reads TA FBIs): FBI linear speeds are world units (map pixels) per game
/// tick at 30 Hz, so the conversion is the identity. Per second would make
/// the Aramon monarch (`maxvelocity = 1.7`) need half an hour to cross a
/// 3072-px map; per tick it takes about a minute, which matches play.
pub const fn fbi_linear(v: Fx) -> Fx {
    v
}

/// FBI `turnrate` to sim units: binary angle units (65536 per turn) per
/// tick. **Believed** (TA convention): identity, clamped to `0..=32767`.
pub fn fbi_turnrate(v: i32) -> u16 {
    v.clamp(0, i32::from(i16::MAX)) as u16
}

/// The movement stand-in for one FBI unit.
///
/// Missing keys (buildings, some specials): `maxvelocity` 0;
/// `acceleration`/`brakerate` default to `maxvelocity` (instant), and are
/// at least one raw step so validation holds; `turnrate` falls back to
/// `turninplacerate`, then 0. Footprint defaults to 1 x 1.
pub fn unit_def_from_fbi(u: &tak_content::UnitDef) -> UnitDef {
    let m = &u.motion;
    let tiny = Fx::from_raw(1);
    let maxvelocity =
        fbi_linear(m.maxvelocity.unwrap_or(Fx::ZERO)).clamp(Fx::ZERO, MAX_VELOCITY_CAP);
    let rate = |v: Option<Fx>| {
        fbi_linear(v.unwrap_or(maxvelocity))
            .max(tiny)
            .min(MAX_VELOCITY_CAP)
    };
    UnitDef {
        name: u.unitname.clone(),
        maxvelocity,
        acceleration: rate(m.acceleration),
        brakerate: rate(m.brakerate),
        turnrate: fbi_turnrate(m.turnrate.or(m.turninplacerate).unwrap_or(0)),
        footprintx: u.footprint.x.unwrap_or(1).max(1),
        footprintz: u.footprint.z.unwrap_or(1).max(1),
        economy: crate::economy::economy_def_from_fbi(u),
        build: crate::construction::build_def_from_fbi(u),
    }
}

/// World position of a TNT attribute cell: its centre, `16 c + 8`
/// (**Chosen**: the OTA `xpos`/`zpos` of start markers are whole cells).
pub fn cell_center(cx: i32, cz: i32) -> StartPos {
    StartPos {
        x: cx.saturating_mul(CELL_SIZE).saturating_add(CELL_SIZE / 2),
        y: cz.saturating_mul(CELL_SIZE).saturating_add(CELL_SIZE / 2),
    }
}

/// Convert a map. Size is cells × 16; heights and passability flags are
/// copied cell for cell; start positions (sorted by player) go to cell
/// centres, clamped into the map.
pub fn map_from_content(m: &MapDef) -> Result<MapInfo, ScenarioError> {
    let too_big = || ScenarioError::Invalid(format!("map {} too large", m.name));
    let width = i32::try_from(m.width)
        .ok()
        .and_then(|w| w.checked_mul(CELL_SIZE))
        .ok_or_else(too_big)?;
    let height = i32::try_from(m.height)
        .ok()
        .and_then(|h| h.checked_mul(CELL_SIZE))
        .ok_or_else(too_big)?;
    let cells = m.width as usize * m.height as usize;
    let flags = if m.passability.width == m.width
        && m.passability.height == m.height
        && m.passability.cells.len() == cells
    {
        m.passability.cells.clone()
    } else {
        return Err(ScenarioError::Invalid(format!(
            "map {}: passability grid does not match the map size",
            m.name
        )));
    };
    let start_positions = m
        .start_positions
        .iter()
        .map(|s| {
            let p = cell_center(s.x, s.z);
            StartPos {
                x: p.x.clamp(0, width),
                y: p.y.clamp(0, height),
            }
        })
        .collect();
    let map = MapInfo {
        width,
        height,
        heights: HeightGrid {
            cols: m.width,
            rows: m.height,
            cell_size: CELL_SIZE as u32,
            heights: m.heights.clone(),
            fill: 0,
        },
        start_positions,
        terrain: Some(TerrainGrid {
            cols: m.width,
            rows: m.height,
            cell_size: CELL_SIZE as u32,
            flags,
        }),
        sea_level: i32::try_from(m.sea_level).unwrap_or(i32::MAX),
    };
    map.validate()?;
    Ok(map)
}

/// One player of a real-map match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerSetup {
    /// Side, by `nameprefix` (`ARA`) or `name` (`ARAMON`), any case.
    pub side: String,
}

impl PlayerSetup {
    /// Parse a comma-separated list such as `ARA,ZON`.
    pub fn parse_list(s: &str) -> Vec<PlayerSetup> {
        s.split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| PlayerSetup {
                side: p.to_string(),
            })
            .collect()
    }
}

/// Find a side by prefix or name.
pub fn find_side<'a>(rs: &'a Ruleset, side: &str) -> Option<&'a SideDef> {
    rs.sides.iter().find(|s| {
        s.name_prefix
            .as_deref()
            .is_some_and(|p| p.eq_ignore_ascii_case(side))
            || s.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(side))
    })
}

impl Scenario {
    /// A match on a real map: player `i` takes the map's `i`-th start
    /// position (sorted by OTA player number) and spawns its side's
    /// `commander` (the monarch). One unit definition per distinct monarch,
    /// in first-use order. Seed 0; set `seed` afterwards.
    pub fn from_content(
        map: &MapDef,
        rs: &Ruleset,
        players: &[PlayerSetup],
    ) -> Result<Scenario, ScenarioError> {
        let info = map_from_content(map)?;
        if players.len() > info.start_positions.len() {
            return Err(ScenarioError::Invalid(format!(
                "{} players but map {} has {} start positions",
                players.len(),
                map.name,
                info.start_positions.len()
            )));
        }
        let mut unit_defs: Vec<UnitDef> = Vec::new();
        let mut specs = Vec::new();
        for (i, p) in players.iter().enumerate() {
            let side = find_side(rs, &p.side)
                .ok_or_else(|| ScenarioError::Invalid(format!("unknown side {:?}", p.side)))?;
            let commander = side.commander.as_deref().ok_or_else(|| {
                ScenarioError::Invalid(format!("side {:?} has no commander", p.side))
            })?;
            let fbi = rs.unit(commander).ok_or_else(|| {
                ScenarioError::Invalid(format!("commander {commander:?} has no unit def"))
            })?;
            let def = match unit_defs
                .iter()
                .position(|d| d.name.eq_ignore_ascii_case(&fbi.unitname))
            {
                Some(d) => d,
                None => {
                    unit_defs.push(unit_def_from_fbi(fbi));
                    unit_defs.len() - 1
                }
            };
            specs.push(PlayerSpec {
                start: i as u32,
                monarch: UnitDefId(def as u32),
            });
        }
        let sc = Scenario {
            version: SCENARIO_FORMAT_VERSION,
            seed: 0,
            map: info,
            unit_defs,
            players: specs,
            economy: Box::new(crate::economy::EconomySetup::from_content(map, rs)),
        };
        sc.validate()?;
        Ok(sc)
    }
}

/// Errors from [`load_match`].
#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    /// The install could not be opened.
    #[error("install: {0}")]
    Vfs(#[from] tak_vfs::VfsError),
    /// The map could not be loaded.
    #[error("map: {0}")]
    Map(#[from] tak_content::MapError),
    /// The resulting scenario is invalid.
    #[error("{0}")]
    Scenario(#[from] ScenarioError),
}

/// Everything a real-map run needs.
pub struct Match {
    /// The loaded map (terrain art, features, passability).
    pub map: MapDef,
    /// The scenario derived from it.
    pub scenario: Scenario,
    /// The COB host for the scenario's unit definitions.
    pub scripts: CobScripts,
}

/// Build the COB host for a scenario: for each unit definition, read
/// `scripts/<unitname>.cob` (via the FBI's derived script path) and
/// `objects3d/<objectname>.3do`, and map pieces by name. A definition whose
/// script fails to load runs without one (listed in the stats).
pub fn load_scripts(vfs: &Vfs, rs: &Ruleset, scenario: &Scenario) -> CobScripts {
    let mut missing = Vec::new();
    let types = scenario
        .unit_defs
        .iter()
        .map(|d| {
            let Some(fbi) = rs.unit(&d.name) else {
                missing.push(format!("{}: no FBI", d.name));
                return None;
            };
            let program = match vfs.read(&fbi.script) {
                Ok(bytes) => match Program::from_bytes(&bytes) {
                    Ok(p) => p,
                    Err(e) => {
                        missing.push(format!("{}: {e}", d.name));
                        return None;
                    }
                },
                Err(e) => {
                    missing.push(format!("{}: {e}", d.name));
                    return None;
                }
            };
            let model = fbi
                .objectname
                .as_deref()
                .and_then(|o| vfs.read(&format!("objects3d/{o}.3do")).ok())
                .and_then(|b| model3do::parse(&b).ok());
            Some(ScriptType::new(&d.name, Arc::new(program), model.as_ref()))
        })
        .collect();
    CobScripts::new(scenario.map.clone(), types, missing)
}

/// Load a match from an open install: base ruleset, the map at `map_name`
/// (VFS path without extension, e.g. `kmap/Adamantine Gate`), one monarch
/// per player, and their scripts.
pub fn load_match(
    vfs: &Vfs,
    map_name: &str,
    players: &[PlayerSetup],
    seed: u64,
) -> Result<Match, ContentError> {
    let rs = Ruleset::load(vfs, RulesetKind::Base);
    let map = MapDef::load(vfs, &rs, map_name)?;
    let mut scenario = Scenario::from_content(&map, &rs, players)?;
    scenario.seed = seed;
    let scripts = load_scripts(vfs, &rs, &scenario);
    Ok(Match {
        map,
        scenario,
        scripts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(fbi_linear(Fx::from_ratio(17, 10).unwrap()).raw(), 111_411);
        assert_eq!(fbi_turnrate(1500), 1500);
        assert_eq!(fbi_turnrate(-3), 0);
        assert_eq!(fbi_turnrate(99_999), 32767);
        assert_eq!(cell_center(113, 20), StartPos { x: 1816, y: 328 });
        assert_eq!(cell_center(0, 0), StartPos { x: 8, y: 8 });
    }

    fn fbi(text: &str) -> tak_content::UnitDef {
        let doc = tak_formats::tdf::parse(text.as_bytes()).unwrap();
        let mut ctx = tak_content::fields::Ctx::default();
        tak_content::unit::unit_from_fbi(&doc, "units/tst.fbi", &mut ctx).unwrap()
    }

    #[test]
    fn fbi_motion_converts() {
        let d = unit_def_from_fbi(&fbi(concat!(
            "[UNITINFO]\n{\nunitname=TSTKING;\nmaxvelocity=1.7;\nacceleration=0.5;\n",
            "brakerate=0.2;\nturnrate=1500;\nfootprintx=2;\nfootprintz=3;\n}\n",
        )));
        assert_eq!(d.name, "TSTKING");
        assert_eq!(d.maxvelocity, Fx::from_ratio(17, 10).unwrap());
        assert_eq!(d.acceleration, Fx::HALF);
        assert_eq!(d.brakerate, Fx::from_ratio(1, 5).unwrap());
        assert_eq!(d.turnrate, 1500);
        assert_eq!((d.footprintx, d.footprintz), (2, 3));
    }

    #[test]
    fn fbi_defaults_keep_defs_valid() {
        let mut u = fbi("[UNITINFO]\n{\nunitname=TSTBLD;\n}\n");
        let d = unit_def_from_fbi(&u);
        assert_eq!(d.maxvelocity, Fx::ZERO);
        assert!(d.validate().is_ok());
        u.motion.maxvelocity = Some(Fx::from_int(2));
        u.motion.turninplacerate = Some(700);
        let d = unit_def_from_fbi(&u);
        assert_eq!(
            (d.acceleration, d.brakerate),
            (Fx::from_int(2), Fx::from_int(2))
        );
        assert_eq!(d.turnrate, 700);
        assert_eq!((d.footprintx, d.footprintz), (1, 1));
    }
}
