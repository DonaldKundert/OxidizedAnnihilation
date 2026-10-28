//! Construction data per unit definition ([`BuildDef`]) and its conversion
//! from the FBI. Units are the FBI's, documented on each field; the
//! formulas that use them live in [`super::progress`].

use serde::{Deserialize, Serialize};
use tak_commands::UnitDefId;

use crate::hash::StateHasher;
use crate::scenario::ScenarioError;

/// Most entries in one build menu (sanity cap; the retail maximum is 9).
pub const MAX_BUILD_OPTIONS: usize = 64;

/// Construction-relevant part of a unit definition. Part of
/// [`crate::UnitDef`] (`build`); absent in JSON means "not buildable, not a
/// builder" (all zero).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildDef {
    /// FBI `buildcost`: mana to build one, whole mana units. **Believed**
    /// (Kingdoms has one resource; `content.md`).
    pub buildcost: i64,
    /// FBI `buildtime`, raw 16.16 (the FBI value can reach 100000, past
    /// `Fx`). Build seconds = `buildtime / workertime` (**Believed**, TA).
    pub buildtime: i64,
    /// FBI `workertime`: build work per second the unit contributes as a
    /// builder or factory.
    pub workertime: i32,
    /// FBI `builddistance`, world units (**Believed** identity, like the
    /// other FBI distances).
    pub builddistance: i32,
    /// FBI `builder`.
    pub builder: bool,
    /// FBI `canreclaim`: explicit Clear capability, independent of the build menu.
    pub canreclaim: bool,
    /// FBI `bmcode`: set on mobile units. A builder with it places what it
    /// builds at a site (`Build { at: Some }`); one without it is a factory
    /// with a production queue (`Build { at: None }`). **Known** for TA
    /// (Nanolathe: the factory handler tests the bmcode bit, not mobility);
    /// in Kingdoms the monarchs and walkers checked have it, keeps and walls do not.
    pub mobile: bool,
    /// Build menu (FBI `canbuild*/<unit>/` for the active ruleset), resolved
    /// to definitions present in the scenario, in menu order.
    pub canbuild: Vec<UnitDefId>,
    /// Yard map expanded to one character per footprint cell, row-major
    /// from the top-left (north-west) cell (see [`expand_yardmap`]). Empty:
    /// every cell occupied (`o`).
    pub yardmap: String,
}

impl BuildDef {
    /// `true` for the all-zero default (skipped when serializing).
    pub fn is_default(&self) -> bool {
        *self == BuildDef::default()
    }

    pub(crate) fn validate(&self, name: &str) -> Result<(), ScenarioError> {
        let bad = |what: &str| {
            Err(ScenarioError::Invalid(format!(
                "unit def {name:?}: build: {what}"
            )))
        };
        if self.buildcost < 0 || self.buildcost > i64::from(i32::MAX) {
            return bad("buildcost out of range");
        }
        if self.buildtime < 0 || self.buildtime > i64::from(i32::MAX) << 16 {
            return bad("buildtime out of range");
        }
        if self.workertime < 0 || self.builddistance < 0 {
            return bad("workertime and builddistance must not be negative");
        }
        if self.canbuild.len() > MAX_BUILD_OPTIONS {
            return bad("build menu too long");
        }
        if self.yardmap.len() > 256 * 256 {
            return bad("yard map too long");
        }
        Ok(())
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"BD");
        h.u64(self.buildcost as u64);
        h.u64(self.buildtime as u64);
        h.i32(self.workertime);
        h.i32(self.builddistance);
        h.bool(self.builder);
        h.bool(self.mobile);

        h.len(self.canbuild.len());
        for &d in &self.canbuild {
            h.unit_def(d);
        }
        h.bytes(self.yardmap.as_bytes());
        if self.canreclaim {
            h.bytes(b"CAN_CLEAR");
        }
    }

    /// Yard character of footprint cell `(cx, cz)`, `o` if none.
    pub fn yard_cell(&self, cx: u32, cz: u32, footprintx: u8) -> u8 {
        let i = cz as usize * usize::from(footprintx) + cx as usize;
        self.yardmap.as_bytes().get(i).copied().unwrap_or(b'o')
    }
}

/// Yard characters that fill a cell. TA's table (Nanolathe research,
/// "Geothermal requirement") is `. c C f G o O w Y y`; Kingdoms adds `S`
/// (lodestone sites, **Known** from `ARALODE` etc.). Anything else (spaces
/// between rows, typos) is skipped without consuming a cell.
pub const YARD_CHARS: &[u8] = b".cCfGoOwYyS";

/// Expand an FBI yard map to one character per footprint cell, the way the
/// TA definition compiler does (**Known** for TA, Nanolathe; **Believed**
/// for Kingdoms): characters outside [`YARD_CHARS`] are skipped, the last
/// usable character repeats for every unfilled cell, extra characters are
/// ignored. `None` or no usable character gives an empty string (all `o`).
pub fn expand_yardmap(yard: Option<&str>, fx: u8, fz: u8) -> String {
    let chars: Vec<u8> = yard
        .unwrap_or("")
        .bytes()
        .filter(|c| YARD_CHARS.contains(c))
        .collect();
    let Some(&last) = chars.last() else {
        return String::new();
    };
    let cells = usize::from(fx) * usize::from(fz);
    (0..cells)
        .map(|i| char::from(chars.get(i).copied().unwrap_or(last)))
        .collect()
}

/// Construction data of one FBI unit. The build menu is left empty: it
/// names other units, so [`super::content::add_build_trees`] resolves it
/// once every definition has an id. A missing `buildtime` reads as 0
/// (built instantly, see [`super::progress::total_work`]).
pub fn build_def_from_fbi(u: &tak_content::UnitDef) -> BuildDef {
    let e = &u.economy;
    let fx = u.footprint.x.unwrap_or(1).max(1);
    let fz = u.footprint.z.unwrap_or(1).max(1);
    BuildDef {
        buildcost: e.build_cost.clamp(0, i64::from(i32::MAX)),
        buildtime: e
            .build_time
            .map_or(0, |t| t.0)
            .clamp(0, i64::from(i32::MAX) << 16),
        workertime: e.worker_time.unwrap_or(0).max(0),
        builddistance: e.build_distance.unwrap_or(0).max(0),
        builder: u.has_flag("builder"),
        canreclaim: u.has_flag("canreclaim"),
        mobile: u.has_flag("bmcode"),
        canbuild: Vec::new(),
        yardmap: expand_yardmap(u.footprint.yardmap.as_deref(), fx, fz),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yardmaps_expand_like_ta() {
        assert_eq!(expand_yardmap(Some("o"), 2, 2), "oooo");
        assert_eq!(expand_yardmap(Some("oc co"), 2, 2), "occo");
        assert_eq!(expand_yardmap(Some("oc"), 2, 2), "occc");
        assert_eq!(expand_yardmap(Some("ocooxx o"), 1, 2), "oc");
        assert_eq!(expand_yardmap(Some("   "), 2, 2), "");
        assert_eq!(expand_yardmap(None, 2, 2), "");
        let d = BuildDef {
            yardmap: "o.cS".into(),
            ..BuildDef::default()
        };
        assert_eq!(d.yard_cell(1, 0, 2), b'.');
        assert_eq!(d.yard_cell(1, 1, 2), b'S');
        assert_eq!(d.yard_cell(5, 5, 2), b'o');
    }
}
