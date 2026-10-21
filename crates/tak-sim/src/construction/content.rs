//! Real content for construction: every unit a match can build becomes a
//! definition, and build menus resolve to definition ids.

use tak_content::{MapDef, Ruleset, RulesetKind};
use tak_vfs::Vfs;

use crate::content::{ContentError, Match, PlayerSetup, load_scripts, unit_def_from_ruleset};
use crate::defs::{MAX_UNIT_DEFS, UnitDef};
use crate::scenario::Scenario;
use crate::{UnitDefId, defs};

use super::build_def::MAX_BUILD_OPTIONS;

/// Append every unit reachable through build menus from `defs` (breadth
/// first, in menu order, so ids are a pure function of the starting defs
/// and the ruleset), then resolve each definition's build menu to ids.
/// Menu entries with no FBI are skipped. Stops adding at
/// [`MAX_UNIT_DEFS`].
pub fn add_build_trees(rs: &Ruleset, defs: &mut Vec<UnitDef>) {
    let find =
        |defs: &[UnitDef], name: &str| defs.iter().position(|d| d.name.eq_ignore_ascii_case(name));
    let mut next = 0;
    while next < defs.len() {
        let name = defs[next].name.clone();
        next += 1;
        let Some(fbi) = rs.unit(&name) else { continue };
        for opt in &fbi.build_options {
            let Some(target) = rs.unit(&opt.unit) else {
                continue;
            };
            if find(defs, &target.unitname).is_none() && defs.len() < MAX_UNIT_DEFS {
                defs.push(unit_def_from_ruleset(target, rs));
            }
        }
    }
    for i in 0..defs.len() {
        let Some(fbi) = rs.unit(&defs[i].name) else {
            continue;
        };
        let menu: Vec<UnitDefId> = fbi
            .build_options
            .iter()
            .filter_map(|o| rs.unit(&o.unit))
            .filter_map(|t| find(defs, &t.unitname))
            .map(|j| UnitDefId(j as u32))
            .take(MAX_BUILD_OPTIONS)
            .collect();
        defs[i].build.canbuild = menu;
    }
}

impl Scenario {
    /// This scenario with every unit its players can build added (see
    /// [`add_build_trees`]). Existing definition ids do not change.
    pub fn with_build_trees(mut self, rs: &Ruleset) -> Scenario {
        add_build_trees(rs, &mut self.unit_defs);
        self
    }
}

/// [`crate::load_match`] plus the players' whole build trees, so `Build`
/// commands work: base ruleset, the map, one monarch per player, every unit
/// reachable through build menus, and all their scripts.
pub fn load_match_with_builds(
    vfs: &Vfs,
    map_name: &str,
    players: &[PlayerSetup],
    seed: u64,
) -> Result<Match, ContentError> {
    let rs = Ruleset::load(vfs, RulesetKind::Base);
    let map = MapDef::load(vfs, &rs, map_name)?;
    let mut scenario = Scenario::from_content(&map, &rs, players)?.with_build_trees(&rs);
    scenario.seed = seed;
    scenario.validate()?;
    let scripts = load_scripts(vfs, &rs, &scenario);
    Ok(Match {
        map,
        scenario,
        scripts,
    })
}

/// Find a definition id by unit name, any case.
pub fn def_by_name(defs: &defs::UnitDefs, name: &str) -> Option<UnitDefId> {
    defs.as_slice()
        .iter()
        .position(|d| d.name.eq_ignore_ascii_case(name))
        .map(|i| UnitDefId(i as u32))
}
