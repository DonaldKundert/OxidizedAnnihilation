//! A [`Ruleset`]: one balance set of units and build menus, plus the shared
//! features, sides and movement classes, with cross-references checked.

use crate::feature::{FeatureDef, features_from_tdf};
use crate::fields::{Coverage, Ctx, Issue};
use crate::gamedata::{MoveClassDef, SideDef, move_classes_from_tdf, sides_from_tdf};
use crate::unit::{UnitDef, file_stem, unit_from_fbi};
use serde::Serialize;
use std::collections::BTreeMap;
use tak_formats::tdf;
use tak_math::Fx;
use tak_vfs::Vfs;

/// Which balance set to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Default)]
pub enum RulesetKind {
    /// `units/` + `canbuild/` (the default).
    #[default]
    Base,
    /// `unitscb/` + `canbuildcb/`. Units missing from `unitscb/` fall back
    /// to `units/` (Believed; see `docs/content.md`).
    Cb,
}

impl RulesetKind {
    pub fn unit_dirs(self) -> &'static [&'static str] {
        match self {
            RulesetKind::Base => &["units"],
            // Later directories override earlier ones.
            RulesetKind::Cb => &["units", "unitscb"],
        }
    }

    pub fn build_dir(self) -> &'static str {
        match self {
            RulesetKind::Base => "canbuild",
            RulesetKind::Cb => "canbuildcb",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RulesetKind::Base => "base",
            RulesetKind::Cb => "cb",
        }
    }
}

impl std::str::FromStr for RulesetKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "base" | "units" | "default" => Ok(RulesetKind::Base),
            "cb" | "unitscb" => Ok(RulesetKind::Cb),
            _ => Err(format!("unknown ruleset {s:?} (expected base or cb)")),
        }
    }
}

/// One entry of a builder's menu (`canbuild/<builder>/<unit>.tdf`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct BuildOption {
    /// The unit built (file stem, original case).
    pub unit: String,
    /// `priority`: menu order (Believed: lower first). Fractional values
    /// (`2.5`) occur, so it is fixed point.
    pub priority: Option<Fx>,
    pub source: String,
}

/// What kind of thing a reference points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum RefKind {
    /// `objects3d/<name>.3do`.
    Model,
    /// `scripts/<name>.cob`.
    Script,
    /// A feature definition.
    Feature,
    /// A unit definition.
    Unit,
    /// A `gamedata/moveinfo.tdf` class.
    MoveClass,
}

/// A reference whose target does not exist. Reported, never fatal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct DanglingRef {
    /// Who refers (`unit ARAARCH`, `feature TarTree01`, `map kmap/X`).
    pub from: String,
    /// Which key or table (`objectname`, `WEAPON1.model`, `tnt features`).
    pub field: String,
    pub kind: RefKind,
    /// The missing target as written.
    pub target: String,
}

/// Units, build menus, features, sides and movement classes for one balance set.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Ruleset {
    pub kind: RulesetKind,
    /// Keyed by lowercase `unitname`.
    pub units: BTreeMap<String, UnitDef>,
    /// Keyed by lowercase feature name.
    pub features: BTreeMap<String, FeatureDef>,
    /// Builder (lowercase) -> options sorted by priority then name.
    pub build_menus: BTreeMap<String, Vec<BuildOption>>,
    pub sides: Vec<SideDef>,
    /// Keyed by lowercase class name.
    pub move_classes: BTreeMap<String, MoveClassDef>,
    /// Files read, by kind (`fbi`, `feature`, `canbuild`).
    pub files: BTreeMap<String, u32>,
    /// Parse failures and bad values. Empty on the retail corpus.
    pub issues: Vec<Issue>,
    /// Definitions replaced by a later one with the same id
    /// (`"<id>: <old source> -> <new source>"`).
    pub overrides: Vec<String>,
    pub dangling: Vec<DanglingRef>,
    pub coverage: Coverage,
}

impl Ruleset {
    /// Loads a ruleset from the VFS. Never fails: problems land in
    /// [`Ruleset::issues`] and [`Ruleset::dangling`].
    pub fn load(vfs: &Vfs, kind: RulesetKind) -> Ruleset {
        let mut ctx = Ctx::default();
        let mut rs = Ruleset {
            kind,
            ..Ruleset::default()
        };
        for dir in kind.unit_dirs() {
            for fi in vfs.list(dir) {
                if !fi.path.ends_with(".fbi") {
                    continue;
                }
                let Some(doc) = read_tdf(vfs, &fi.display_path, &mut ctx) else {
                    continue;
                };
                *rs.files.entry("fbi".into()).or_default() += 1;
                if let Some(u) = unit_from_fbi(&doc, &fi.display_path, &mut ctx) {
                    let key = u.unitname.to_ascii_lowercase();
                    if let Some(old) = rs.units.insert(key.clone(), u) {
                        rs.overrides.push(format!(
                            "unit {key}: {} -> {}",
                            old.source, rs.units[&key].source
                        ));
                    }
                }
            }
        }
        for fi in vfs.list("features") {
            if !fi.path.ends_with(".tdf") {
                continue;
            }
            let Some(doc) = read_tdf(vfs, &fi.display_path, &mut ctx) else {
                continue;
            };
            *rs.files.entry("feature".into()).or_default() += 1;
            for f in features_from_tdf(&doc, &fi.display_path, &mut ctx) {
                let key = f.name.to_ascii_lowercase();
                if let Some(old) = rs.features.insert(key.clone(), f) {
                    rs.overrides.push(format!(
                        "feature {key}: {} -> {}",
                        old.source, rs.features[&key].source
                    ));
                }
            }
        }
        let build_dir = kind.build_dir();
        for fi in vfs.list(build_dir) {
            if !fi.path.ends_with(".tdf") {
                continue;
            }
            let parts: Vec<&str> = fi.display_path.split('/').collect();
            let [_, builder, _file] = parts.as_slice() else {
                ctx.issues.push(Issue {
                    file: fi.display_path.clone(),
                    section: String::new(),
                    key: String::new(),
                    message: format!("expected {build_dir}/<builder>/<unit>.tdf"),
                });
                continue;
            };
            let Some(doc) = read_tdf(vfs, &fi.display_path, &mut ctx) else {
                continue;
            };
            *rs.files.entry("canbuild".into()).or_default() += 1;
            let priority = match doc.sections.first() {
                Some(sec) => {
                    let mut f = crate::fields::Fields::with_row(
                        sec,
                        &fi.display_path,
                        "canbuild:*".into(),
                        sec.name.clone(),
                        &mut ctx,
                    );
                    let p = f.fx("priority");
                    f.finish();
                    p
                }
                None => None,
            };
            rs.build_menus
                .entry(builder.to_ascii_lowercase())
                .or_default()
                .push(BuildOption {
                    unit: file_stem(&fi.display_path).to_string(),
                    priority,
                    source: fi.display_path.clone(),
                });
        }
        for menu in rs.build_menus.values_mut() {
            menu.sort_by(|a, b| {
                (a.priority.unwrap_or(Fx::MAX), a.unit.to_ascii_lowercase())
                    .cmp(&(b.priority.unwrap_or(Fx::MAX), b.unit.to_ascii_lowercase()))
            });
        }
        for (builder, menu) in &rs.build_menus {
            if let Some(u) = rs.units.get_mut(builder) {
                u.build_options = menu.clone();
            }
        }
        if let Some(doc) = read_tdf(vfs, "gamedata/sidedata.tdf", &mut ctx) {
            rs.sides = sides_from_tdf(&doc, "gamedata/sidedata.tdf", &mut ctx);
        }
        if let Some(doc) = read_tdf(vfs, "gamedata/moveinfo.tdf", &mut ctx) {
            for m in move_classes_from_tdf(&doc, "gamedata/moveinfo.tdf", &mut ctx) {
                rs.move_classes.insert(m.name.to_ascii_lowercase(), m);
            }
        }
        rs.issues = ctx.issues;
        rs.coverage = ctx.coverage;
        rs.dangling = rs.check_refs(|p| vfs.stat(p).is_some());
        rs
    }

    /// A unit by `unitname`, ignoring case.
    pub fn unit(&self, name: &str) -> Option<&UnitDef> {
        self.units.get(&name.to_ascii_lowercase())
    }

    /// A feature by name, ignoring case.
    pub fn feature(&self, name: &str) -> Option<&FeatureDef> {
        self.features.get(&name.to_ascii_lowercase())
    }

    /// Checks every cross-reference. `exists` answers whether a VFS path exists.
    pub fn check_refs(&self, exists: impl Fn(&str) -> bool) -> Vec<DanglingRef> {
        let mut out = Vec::new();
        let mut miss = |from: String, field: &str, kind: RefKind, target: &str| {
            out.push(DanglingRef {
                from,
                field: field.to_string(),
                kind,
                target: target.to_string(),
            });
        };
        // A few weapons write the extension (`model = zonrock.3do`).
        let model = |n: &str| {
            let n = n.to_ascii_lowercase();
            let n = n.strip_suffix(".3do").unwrap_or(&n);
            format!("objects3d/{n}.3do")
        };
        for u in self.units.values() {
            let from = format!("unit {}", u.unitname);
            match &u.objectname {
                Some(o) if exists(&model(o)) => {}
                Some(o) => miss(from.clone(), "objectname", RefKind::Model, o),
                None => miss(from.clone(), "objectname", RefKind::Model, ""),
            }
            if !exists(&u.script) {
                miss(from.clone(), "script", RefKind::Script, &u.script);
            }
            for (field, v) in [
                ("corpse", &u.corpse),
                ("frozen", &u.frozen),
                ("stone", &u.stone),
            ] {
                if let Some(v) = v.as_ref().filter(|v| self.feature(v).is_none()) {
                    miss(from.clone(), field, RefKind::Feature, v);
                }
            }
            let class_missing =
                |m: &&String| !self.move_classes.contains_key(&m.to_ascii_lowercase());
            if let Some(m) = u.movement_class.as_ref().filter(class_missing) {
                miss(from.clone(), "movementclass", RefKind::MoveClass, m);
            }
            for w in u.weapons.iter().chain(u.explode_as.iter()) {
                if let Some(m) = w.model.as_ref().filter(|m| !exists(&model(m))) {
                    miss(
                        from.clone(),
                        &format!("{}.model", w.slot),
                        RefKind::Model,
                        m,
                    );
                }
            }
        }
        for (builder, menu) in &self.build_menus {
            if !self.units.contains_key(builder) {
                miss(
                    format!("build menu {builder}"),
                    "builder",
                    RefKind::Unit,
                    builder,
                );
            }
            for o in menu {
                if self.unit(&o.unit).is_none() {
                    miss(
                        format!("build menu {builder}"),
                        "option",
                        RefKind::Unit,
                        &o.unit,
                    );
                }
            }
        }
        for f in self.features.values() {
            let from = format!("feature {}", f.name);
            if let Some(o) = f.object.as_ref().filter(|o| !exists(&model(o))) {
                miss(from.clone(), "object", RefKind::Model, o);
            }
            for (field, v) in [
                ("featuredead", &f.feature_dead),
                ("featureburnt", &f.feature_burnt),
                ("featurereclamate", &f.feature_reclamate),
            ] {
                if let Some(v) = v.as_ref().filter(|v| self.feature(v).is_none()) {
                    miss(from.clone(), field, RefKind::Feature, v);
                }
            }
        }
        for s in &self.sides {
            for (field, v) in [("commander", &s.commander), ("god", &s.god)] {
                if let Some(v) = v.as_ref().filter(|v| self.unit(v).is_none()) {
                    miss(format!("side {}", s.slot), field, RefKind::Unit, v);
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Reads and parses a TDF-syntax file, recording failures as issues.
pub(crate) fn read_tdf(vfs: &Vfs, path: &str, ctx: &mut Ctx) -> Option<tdf::Document> {
    let issue = |message: String| Issue {
        file: path.to_string(),
        section: String::new(),
        key: String::new(),
        message,
    };
    match vfs.read(path) {
        Ok(bytes) => match tdf::parse(&bytes) {
            Ok(d) => Some(d),
            Err(e) => {
                ctx.issues.push(issue(format!("parse error: {e}")));
                None
            }
        },
        Err(e) => {
            ctx.issues.push(issue(format!("read error: {e}")));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_vfs::testing::HapiBuilder;

    fn vfs_with(files: &[(&str, &str)]) -> (Vfs, std::path::PathBuf) {
        let mut b = HapiBuilder::new();
        for (p, d) in files {
            b = b.file(p, d.as_bytes(), true);
        }
        let dir = std::env::temp_dir().join(format!(
            "tak-content-test-{}-{}",
            std::process::id(),
            files.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.hpi");
        std::fs::write(&path, b.build()).unwrap();
        let mut vfs = Vfs::new();
        vfs.mount_archive(&path).unwrap();
        (vfs, dir)
    }

    #[test]
    fn loads_and_checks_refs() {
        let (vfs, dir) = vfs_with(&[
            (
                "units/tstbuild.fbi",
                "[UNITINFO]\n{\n\tunitname=TSTBUILD;\n\tobjectname=tstbuild;\n\tcorpse=tstwreck;\n\tmovementclass=GROUND2;\n}\n",
            ),
            (
                "units/tstknight.fbi",
                "[UNITINFO]\n{\n\tunitname=TSTKNIGHT;\n\tobjectname=nomodel;\n}\n[WEAPON1]\n{\n\tmodel=noarrow;\n\t[DAMAGE]\n\t{\n\t\tdefault=5;\n\t}\n}\n",
            ),
            (
                "unitscb/tstknight.fbi",
                "[UNITINFO]\n{\n\tunitname=TSTKNIGHT;\n\tobjectname=tstbuild;\n}\n",
            ),
            ("objects3d/tstbuild.3do", "x"),
            ("scripts/tstbuild.cob", "x"),
            (
                "canbuild/tstbuild/tstknight.tdf",
                "[Menu]\n{\n\tpriority=2;\n}\n",
            ),
            (
                "canbuild/tstbuild/tstghost.tdf",
                "[Menu]\n{\n\tpriority=1.5;\n}\n",
            ),
            (
                "canbuildcb/tstbuild/tstknight.tdf",
                "[Menu]\n{\n\tpriority=1;\n}\n",
            ),
            (
                "features/all worlds/tst.tdf",
                "[TstWreck]\n{\n\tfootprintx=2;\n\tfootprintz=2;\n\tfeaturedead=TstGone;\n}\n",
            ),
            (
                "gamedata/moveinfo.tdf",
                "[CLASS0]\n{\n\tName=GROUND2;\n\tMaxSlope=30;\n}\n",
            ),
            (
                "gamedata/sidedata.tdf",
                "[SIDE0]\n{\n\tname=TST;\n\tcommander=TSTBUILD;\n\tgod=TSTGOD;\n}\n",
            ),
        ]);
        let base = Ruleset::load(&vfs, RulesetKind::Base);
        assert!(base.issues.is_empty(), "{:?}", base.issues);
        assert_eq!(base.units.len(), 2);
        assert_eq!(base.files["fbi"], 2);
        let menu = &base.build_menus["tstbuild"];
        assert_eq!(menu[0].unit, "tstghost");
        assert_eq!(menu[0].priority, Fx::from_ratio(3, 2));
        assert_eq!(base.unit("tstbuild").unwrap().build_options.len(), 2);
        let targets: Vec<(RefKind, &str)> = base
            .dangling
            .iter()
            .map(|d| (d.kind, d.target.as_str()))
            .collect();
        assert!(targets.contains(&(RefKind::Model, "nomodel")));
        assert!(targets.contains(&(RefKind::Model, "noarrow")));
        assert!(targets.contains(&(RefKind::Script, "scripts/tstknight.cob")));
        assert!(targets.contains(&(RefKind::Unit, "tstghost")));
        assert!(targets.contains(&(RefKind::Feature, "TstGone")));
        assert!(!targets.iter().any(|t| t.1 == "tstwreck"));
        assert!(!targets.iter().any(|t| t.0 == RefKind::MoveClass));
        assert!(targets.contains(&(RefKind::Unit, "TSTGOD")));
        assert!(!targets.contains(&(RefKind::Unit, "TSTBUILD")));

        let cb = Ruleset::load(&vfs, RulesetKind::Cb);
        assert_eq!(cb.units.len(), 2);
        assert_eq!(cb.overrides.len(), 1);
        assert_eq!(
            cb.unit("TSTKNIGHT").unwrap().objectname.as_deref(),
            Some("tstbuild")
        );
        assert_eq!(cb.build_menus["tstbuild"].len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn kind_parse() {
        assert_eq!("CB".parse::<RulesetKind>(), Ok(RulesetKind::Cb));
        assert_eq!("units".parse::<RulesetKind>(), Ok(RulesetKind::Base));
        assert!("x".parse::<RulesetKind>().is_err());
    }
}
