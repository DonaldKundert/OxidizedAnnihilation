//! Real content for construction: every unit a match can build becomes a
//! definition, and build menus resolve to definition ids.

use tak_content::Ruleset;
use tak_vfs::Vfs;

use crate::content::{ContentError, Match, PlayerSetup, unit_def_from_ruleset};
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
        self.features.resolve_corpses(rs, &self.unit_defs);
        self
    }
}

/// The same as [`crate::load_match`], which now loads the players' whole
/// build trees by default. Kept for existing callers.
pub fn load_match_with_builds(
    vfs: &Vfs,
    map_name: &str,
    players: &[PlayerSetup],
    seed: u64,
) -> Result<Match, ContentError> {
    crate::content::load_match(vfs, map_name, players, seed)
}

/// Find a definition id by unit name, any case.
pub fn def_by_name(defs: &defs::UnitDefs, name: &str) -> Option<UnitDefId> {
    defs.as_slice()
        .iter()
        .position(|d| d.name.eq_ignore_ascii_case(name))
        .map(|i| UnitDefId(i as u32))
}
