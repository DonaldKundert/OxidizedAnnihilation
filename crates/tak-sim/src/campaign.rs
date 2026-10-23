//! Explicit mission placement loading. Not a campaign/story interpreter.
//!
//! OTA coordinates are Believed cells; cell centres and east-zero headings
//! are Chosen pending original-engine comparison. Unsupported mission orders,
//! AI, alliances and trigger scripts are reported, never executed implicitly.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tak_content::{MapDef, Ruleset, RulesetKind};
use tak_math::Angle;
use tak_vfs::Vfs;

use crate::content::{
    ContentError, Match, PlayerSetup, cell_center, load_scripts, unit_def_from_ruleset,
};
use crate::{PlayerId, Scenario, ScenarioError, UnitDefId, UnitId, World, WorldPos};

/// A fully resolved startup unit. Vector order determines arena IDs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialUnit {
    pub def: UnitDefId,
    pub owner: PlayerId,
    pub pos: WorldPos,
    pub heading: Angle,
    /// 1..=100. Zero-health placement semantics are not implemented.
    pub health_percentage: u8,
    pub mana_percentage: u8,
    pub kills: u32,
}

impl InitialUnit {
    pub(crate) fn validate(&self, sc: &Scenario) -> Result<(), ScenarioError> {
        if self.def.0 as usize >= sc.unit_defs.len()
            || self.owner.0 as usize >= sc.players.len()
            || self.health_percentage == 0
            || self.health_percentage > 100
            || self.mana_percentage > 100
            || sc.map.clamp(self.pos) != self.pos
        {
            return Err(ScenarioError::Invalid(
                "invalid initial unit definition, owner, position or percentage".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn hash(&self, h: &mut crate::StateHasher) {
        h.unit_def(self.def);
        h.player(self.owner);
        h.vec2(self.pos);
        h.u16(self.heading.raw());
        h.u8(self.health_percentage);
        h.u8(self.mana_percentage);
        h.u32(self.kills);
    }
}

/// Explicit mapping avoids guessing undocumented `playerN` header strings.
/// Slice order is the simulation player order. OTA slots need not be contiguous.
#[derive(Clone, Debug)]
pub struct CampaignPlayer {
    pub ota_slot: u8,
    pub side: String,
}

/// A loadable match plus fidelity gaps the caller must surface.
pub struct CampaignMatch {
    pub game: Match,
    pub unsupported: Vec<String>,
}

/// Convert placements using explicit ownership. Does not enable objectives;
/// call `enable_objectives` separately after inspecting unsupported findings.
/// Returns every unsupported placement field with its source index.
pub fn scenario_from_campaign(
    map: &MapDef,
    rs: &Ruleset,
    players: &[CampaignPlayer],
) -> Result<(Scenario, Vec<String>), ScenarioError> {
    let bad = |s: String| ScenarioError::Invalid(s);
    if !map.issues.is_empty() {
        return Err(bad(
            "campaign map has OTA parsing issues; refusing defaulted malformed fields".into(),
        ));
    }
    if players.is_empty() || players.len() > crate::scenario::MAX_PLAYERS {
        return Err(bad("campaign requires 1..=MAX_PLAYERS mapped owners".into()));
    }
    let mut owners = BTreeMap::new();
    let mut mapped_map = map.clone();
    mapped_map.start_positions.clear();
    for (i, p) in players.iter().enumerate() {
        if p.ota_slot == 0
            || owners
                .insert(i32::from(p.ota_slot), PlayerId(i as u8))
                .is_some()
        {
            return Err(bad("duplicate or zero OTA owner slot".into()));
        }
        let start = map
            .start_positions
            .iter()
            .find(|s| s.player == p.ota_slot)
            .copied()
            .unwrap_or(tak_content::StartPosition {
                player: p.ota_slot,
                x: 0,
                z: 0,
                source: tak_content::StartSource::FirstUnit,
            });
        mapped_map.start_positions.push(start);
    }
    let setups: Vec<_> = players
        .iter()
        .map(|p| PlayerSetup {
            side: p.side.clone(),
        })
        .collect();
    let mut sc = Scenario::from_content(&mapped_map, rs, &setups)?;
    if map.ota.units.len() > crate::world::MAX_UNITS {
        return Err(bad("too many OTA initial units".into()));
    }
    let mut units = Vec::with_capacity(map.ota.units.len());
    let mut unsupported = Vec::new();
    for (i, u) in map.ota.units.iter().enumerate() {
        let owner = u
            .player
            .and_then(|p| owners.get(&p).copied())
            .ok_or_else(|| bad(format!("OTA unit {i}: unmapped owner {:?}", u.player)))?;
        let fbi = rs
            .unit(&u.unitname)
            .ok_or_else(|| bad(format!("OTA unit {i}: unknown type {:?}", u.unitname)))?;
        let def = match sc
            .unit_defs
            .iter()
            .position(|d| d.name.eq_ignore_ascii_case(&fbi.unitname))
        {
            Some(n) => n,
            None => {
                if sc.unit_defs.len() >= crate::defs::MAX_UNIT_DEFS {
                    return Err(bad("too many campaign unit definitions".into()));
                }
                sc.unit_defs.push(unit_def_from_ruleset(fbi, rs));
                sc.unit_defs.len() - 1
            }
        };
        if u.x < 0 || u.z < 0 || u.x as u32 >= map.width || u.z as u32 >= map.height {
            return Err(bad(format!("OTA unit {i}: position outside map")));
        }
        let percentage = |value: Option<i32>, default: i32| -> Result<u8, ScenarioError> {
            let n = value.unwrap_or(default);
            if !(0..=100).contains(&n) {
                return Err(bad(format!("OTA unit {i}: invalid percentage {n}")));
            }
            Ok(n as u8)
        };
        let initial = InitialUnit {
            def: UnitDefId(def as u32),
            owner,
            pos: cell_center(u.x, u.z).to_world(),
            heading: Angle::from_degrees(u.angle.unwrap_or(0).rem_euclid(360)),
            health_percentage: percentage(u.health_percentage, 100)?,
            mana_percentage: percentage(u.mana_percentage, 0)?,
            kills: u32::try_from(u.kills.unwrap_or(0))
                .map_err(|_| bad(format!("OTA unit {i}: negative kills")))?,
        };
        initial.validate(&sc)?;
        units.push(initial);
        if u.initial_mission
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            unsupported.push(format!("unit {i}: initialmission"));
        }
        if u.ai_ignore || u.ai_priority_target {
            unsupported.push(format!("unit {i}: AI flags"));
        }
        if u.mission_critical {
            unsupported.push(format!("unit {i}: missioncriticalunit"));
        }
        if u.ident.is_some() {
            unsupported.push(format!("unit {i}: ident"));
        }
        if u.y != 0 {
            unsupported.push(format!(
                "unit {i}: authored height (terrain sampled instead)"
            ));
        }
        if !u.extra.is_empty() {
            unsupported.push(format!("unit {i}: additional placement fields"));
        }
    }
    sc.initial_units = Some(units);
    sc = sc.with_build_trees(rs);
    sc.validate()?;
    unsupported.push(
        "player header AI/alliance settings and mission trigger scripts are not interpreted".into(),
    );
    Ok((sc, unsupported))
}

/// Opt in to the existing two-slot mission condition evaluator. Other owner
/// layouts must not silently use its hard-coded local/enemy semantics.
pub fn enable_objectives(
    sc: &mut Scenario,
    map: &MapDef,
    players: &[CampaignPlayer],
) -> Result<(), ScenarioError> {
    if players.len() != 2
        || players[0].ota_slot != 1
        || players[1].ota_slot != 2
        || sc.players.len() != 2
    {
        return Err(ScenarioError::Invalid(
            "mission objectives currently require exactly OTA slots 1, 2 in that order".into(),
        ));
    }
    sc.victory.mission = Some(crate::victory::MissionRules::from_conditions(
        &map.ota.conditions,
        &sc.unit_defs,
    ));
    sc.validate()
}

/// Load terrain, all authored unit types, their build trees and COB scripts.
/// Objectives remain opt-in; this does not implement story progression or AI.
pub fn load_campaign(
    vfs: &Vfs,
    name: &str,
    players: &[CampaignPlayer],
    seed: u64,
) -> Result<CampaignMatch, ContentError> {
    let rs = Ruleset::load(vfs, RulesetKind::Base);
    let map = MapDef::load(vfs, &rs, name)?;
    let (mut scenario, unsupported) = scenario_from_campaign(&map, &rs, players)?;
    scenario.seed = seed;
    let scripts = load_scripts(vfs, &rs, &scenario);
    Ok(CampaignMatch {
        game: Match {
            map,
            scenario,
            scripts,
        },
        unsupported,
    })
}

/// Tick-zero startup, before scripts see Create. No extra monarchs.
pub(crate) fn spawn_initial_units(world: &mut World, units: Vec<InitialUnit>) -> Vec<UnitId> {
    let mut spawned = Vec::with_capacity(units.len());
    for p in units {
        let Some(id) = world.spawn(p.def, p.owner, p.pos, p.heading) else {
            continue;
        };
        let def = world.defs().get(p.def).expect("validated placement");
        let max = def.combat.max_damage;
        let weapons = def.combat.weapons.len();
        let u = world.unit_mut(id).expect("just spawned");
        u.combat.ready = true;
        u.combat.max_health = max;
        u.combat.health = if max > 0 {
            (i64::from(max) * i64::from(p.health_percentage) / 100).max(1) as i32
        } else {
            0
        };
        u.combat.weapons = vec![Default::default(); weapons];
        u.combat.reload_sent = weapons == 0;
        u.combat.kills = p.kills;
        crate::economy::set_unit_mana_percent(world, id, p.mana_percentage);
        let player = &mut world.players[p.owner.0 as usize];
        if player.monarch_def == p.def && player.monarch.is_none() {
            player.monarch = Some(id);
        }
        spawned.push(id);
    }
    spawned
}
