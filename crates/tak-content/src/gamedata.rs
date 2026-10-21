//! `gamedata/sidedata.tdf` (`[SIDEn]`) and `gamedata/moveinfo.tdf` (`[CLASSn]`).

use crate::fields::{Ctx, Fields};
use serde::Serialize;
use std::collections::BTreeMap;
use tak_formats::tdf::Document;

/// A side (kingdom or neutral faction).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SideDef {
    /// Section name (`SIDE0`).
    pub slot: String,
    /// `name` (`ARAMON`, `LIFEFORMS`).
    pub name: Option<String>,
    /// `nameprefix`: matches `UnitDef::side` (`ARA`).
    pub name_prefix: Option<String>,
    /// `commander`: the monarch unit.
    pub commander: Option<String>,
    /// `god`: the god unit.
    pub god: Option<String>,
    pub palette: Option<String>,
    pub build_palette: Option<String>,
    /// `waterheight`: matches the TNT sea levels seen (58, 40).
    pub water_height: Option<i32>,
    pub extra: BTreeMap<String, String>,
}

/// A movement class, referenced by `UnitDef::movement_class`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MoveClassDef {
    /// Section name (`CLASS0`).
    pub slot: String,
    /// `name` (`GROUND2`, `WATER3`, `HOVER2`).
    pub name: String,
    pub footprint_x: Option<u8>,
    pub footprint_z: Option<u8>,
    /// Steepest height step between neighbouring cells it can climb.
    pub max_slope: Option<i32>,
    pub max_water_depth: Option<i32>,
    pub min_water_depth: Option<i32>,
    pub max_water_slope: Option<i32>,
    pub bad_slope: Option<i32>,
    pub bad_water_slope: Option<i32>,
    pub bad_min_water_depth: Option<i32>,
    pub extra: BTreeMap<String, String>,
}

pub fn sides_from_tdf(doc: &Document, source: &str, ctx: &mut Ctx) -> Vec<SideDef> {
    doc.sections
        .iter()
        .map(|s| {
            let mut f = Fields::new(s, source, "sidedata", s.name.clone(), ctx);
            let mut d = SideDef {
                slot: s.name.clone(),
                name: f.string("name"),
                name_prefix: f.string("nameprefix"),
                commander: f.string("commander"),
                god: f.string("god"),
                palette: f.string("palette"),
                build_palette: f.string("buildpalette"),
                water_height: f.int("waterheight"),
                extra: BTreeMap::new(),
            };
            d.extra = f.finish();
            d
        })
        .collect()
}

pub fn move_classes_from_tdf(doc: &Document, source: &str, ctx: &mut Ctx) -> Vec<MoveClassDef> {
    doc.sections
        .iter()
        .map(|s| {
            let mut f = Fields::new(s, source, "moveinfo", s.name.clone(), ctx);
            let mut d = MoveClassDef {
                slot: s.name.clone(),
                name: f.string("name").unwrap_or_else(|| s.name.clone()),
                footprint_x: f.int("footprintx"),
                footprint_z: f.int("footprintz"),
                max_slope: f.int("maxslope"),
                max_water_depth: f.int("maxwaterdepth"),
                min_water_depth: f.int("minwaterdepth"),
                max_water_slope: f.int("maxwaterslope"),
                bad_slope: f.int("badslope"),
                bad_water_slope: f.int("badwaterslope"),
                bad_min_water_depth: f.int("badminwaterdepth"),
                extra: BTreeMap::new(),
            };
            d.extra = f.finish();
            d
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_formats::tdf;

    #[test]
    fn move_class() {
        let doc =
            tdf::parse(b"[CLASS0]\n{\n\tName=TST2;\n\tFootprintX=2;\n\tMaxSlope=30;\n}\n").unwrap();
        let mut ctx = Ctx::default();
        let m = move_classes_from_tdf(&doc, "gamedata/moveinfo.tdf", &mut ctx);
        assert_eq!(m[0].name, "TST2");
        assert_eq!(m[0].max_slope, Some(30));
        assert_eq!(m[0].max_water_depth, None);
    }

    #[test]
    fn side() {
        let doc = tdf::parse(
            b"[SIDE0]\n{\n\tname=TSTLAND;\n\tnameprefix=TST;\n\twaterheight=58;\n\tlogoart=x;\n}\n",
        )
        .unwrap();
        let mut ctx = Ctx::default();
        let s = sides_from_tdf(&doc, "gamedata/sidedata.tdf", &mut ctx);
        assert_eq!(s[0].name_prefix.as_deref(), Some("TST"));
        assert_eq!(s[0].water_height, Some(58));
        assert_eq!(s[0].extra["logoart"], "x");
    }
}
