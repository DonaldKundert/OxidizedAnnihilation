//! Map features (`features/**/*.tdf`): each top-level section defines one
//! feature, named by the section.

use crate::fields::{Ctx, Fields};
use serde::Serialize;
use std::collections::BTreeMap;
use tak_formats::tdf::{Document, Section};
use tak_math::Fx;

/// One feature type (trees, rocks, ruins, corpses, lodestone sites).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeatureDef {
    /// Section name, original case. TNT feature tables reference it
    /// case-insensitively.
    pub name: String,
    /// VFS path of the defining file.
    pub source: String,
    /// `world`: the terrain set (`All Worlds`, ...).
    pub world: Option<String>,
    /// `description`: display text (runtime only).
    pub description: Option<String>,
    pub category: Vec<String>,
    /// `footprintx` / `footprintz` in attribute cells.
    pub footprint_x: u8,
    pub footprint_z: u8,
    /// `height`: blocking height (world units, Believed).
    pub height: Option<i32>,
    /// `blocking`: occupies its footprint for movement.
    pub blocking: bool,
    pub hit_density: Option<i32>,
    /// `damage`: hit points.
    pub damage: Option<i32>,
    pub indestructible: bool,
    pub reclaimable: bool,
    pub auto_reclaimable: bool,
    pub resurrectable: bool,
    pub flamable: bool,
    pub permanent: bool,
    pub is_building: bool,
    pub is_stone: bool,
    pub is_frozen: bool,
    pub no_display_info: bool,
    /// `object`: 3D model `objects3d/<object>.3do`.
    pub object: Option<String>,
    /// `filename`: sprite file `anims/<filename>.gaf` (Believed).
    pub filename: Option<String>,
    /// `seqname`: sequence inside `filename`.
    pub seqname: Option<String>,
    pub seqname_shadow: Option<String>,
    /// `energy`: mana gained by reclaiming (Believed).
    pub energy: Option<i32>,
    /// `sacredsite`: lodestone income multiplier on this site (Believed).
    pub sacred_site: Option<Fx>,
    /// Features this one turns into.
    pub feature_dead: Option<String>,
    pub feature_burnt: Option<String>,
    /// `featurereclamate` (also the misspelling `featurereclamamate`).
    pub feature_reclamate: Option<String>,
    pub burn_weapon: Option<String>,
    pub burn_min: Option<i32>,
    pub burn_max: Option<i32>,
    pub spread_chance: Option<i32>,
    pub decompose_time: Option<i32>,
    pub extra: BTreeMap<String, String>,
}

/// Every feature in one parsed file.
pub fn features_from_tdf(doc: &Document, source: &str, ctx: &mut Ctx) -> Vec<FeatureDef> {
    doc.sections
        .iter()
        .map(|s| feature(s, source, ctx))
        .collect()
}

fn feature(sec: &Section, source: &str, ctx: &mut Ctx) -> FeatureDef {
    let mut f = Fields::with_row(sec, source, "feature:*".into(), sec.name.clone(), ctx);
    let mut d = FeatureDef {
        name: sec.name.clone(),
        source: source.to_string(),
        world: f.string("world"),
        description: f.string("description"),
        category: f.words("category"),
        footprint_x: f.int("footprintx").unwrap_or(1),
        footprint_z: f.int("footprintz").unwrap_or(1),
        height: f.int("height"),
        blocking: f.flag("blocking"),
        hit_density: f.int("hitdensity"),
        damage: f.int("damage"),
        indestructible: f.flag("indestructible"),
        reclaimable: f.flag("reclaimable"),
        auto_reclaimable: f.flag("autoreclaimable"),
        resurrectable: f.flag("resurrectable"),
        flamable: f.flag("flamable"),
        permanent: f.flag("permanent"),
        is_building: f.flag("isbuilding"),
        is_stone: f.flag("isstone"),
        is_frozen: f.flag("isfrozen"),
        no_display_info: f.flag("nodisplayinfo"),
        object: f.string("object"),
        filename: f.string("filename"),
        seqname: f.string("seqname"),
        seqname_shadow: f.string("seqnameshad"),
        energy: f.int("energy"),
        sacred_site: f.fx("sacredsite"),
        feature_dead: f.string("featuredead"),
        feature_burnt: f.string("featureburnt"),
        feature_reclamate: f
            .string("featurereclamate")
            .or_else(|| f.string("featurereclamamate")),
        burn_weapon: f.string("burnweapon"),
        burn_min: f.int("burnmin"),
        burn_max: f.int("burnmax"),
        spread_chance: f.int("spreadchance"),
        decompose_time: f.int("decomposetime"),
        extra: BTreeMap::new(),
    };
    d.extra = f.finish();
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_formats::tdf;

    #[test]
    fn two_features_in_one_file() {
        let src = "[TstTree01]\n{\n\tworld=All Worlds;\n\tfootprintx=2;\n\tfootprintz=3;\n\tblocking=1;\n\treclaimable=1;\n\treclaimable=0;\n\tfeaturedead=TstStump;\n\tsacredsite=1.5;\n\tseqnameburnshad=;\n}\n[TstStump]\n{\n\tfootprintx=1;\n\tfootprintz=1;\n\tblocking=0;\n\tfeaturereclamamate=TstDust;\n}\n";
        let doc = tdf::parse(src.as_bytes()).unwrap();
        let mut ctx = Ctx::default();
        let fs = features_from_tdf(&doc, "features/tst.tdf", &mut ctx);
        assert!(ctx.issues.is_empty());
        assert_eq!(fs.len(), 2);
        assert_eq!((fs[0].footprint_x, fs[0].footprint_z), (2, 3));
        assert!(fs[0].blocking);
        assert!(!fs[0].reclaimable, "last duplicate wins");
        assert_eq!(fs[0].sacred_site, Some(Fx::from_ratio(3, 2).unwrap()));
        assert_eq!(fs[0].feature_dead.as_deref(), Some("TstStump"));
        assert_eq!(
            fs[0].extra.get("seqnameburnshad").map(String::as_str),
            Some("")
        );
        assert_eq!(fs[1].feature_reclamate.as_deref(), Some("TstDust"));
        assert_eq!(ctx.coverage.sections["feature:*"].sections, 2);
    }
}
