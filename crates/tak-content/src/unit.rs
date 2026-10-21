//! Units (`units/*.fbi`, `unitscb/*.fbi`) and their weapons.
//!
//! Field-to-key mapping and confidence: `docs/content.md`.

use crate::fields::{Ctx, Fields, WideFx};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tak_formats::tdf::{Document, Section};
use tak_math::Fx;

/// Boolean `[UNITINFO]` keys collected into [`UnitDef::flags`]. Each is
/// `1` when present in the corpus (`floater` and `bmcode` also use `0`).
pub const FLAG_KEYS: &[&str] = &[
    "activatewhenbuilt",
    "attractsgods",
    "bmcode",
    "builder",
    "builderlimited",
    "cananimate",
    "canattack",
    "canbuild",
    "cancapture",
    "cancloak",
    "canfly",
    "canguard",
    "canhover",
    "canload",
    "canmove",
    "canpatrol",
    "canreclaim",
    "canresurrect",
    "canstop",
    "cantbecaptured",
    "cantbefrozen",
    "cantbestoned",
    "cantbetransported",
    "cantransport",
    "commander",
    "fireatwillrandom",
    "floater",
    "gate",
    "ghost",
    "hoverattack",
    "isfeature",
    "noshadow",
    "noveteran",
    "onoffable",
    "shootme",
    "upright",
    "weaponswitching",
    "wind",
];

/// One unit type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnitDef {
    /// `unitname`, the id used by build menus, maps and scripts (file stem
    /// if the key is missing). Original case.
    pub unitname: String,
    /// VFS path of the `.fbi` this def came from.
    pub source: String,
    /// `name`: display name (runtime only; never committed).
    pub name: Option<String>,
    /// `side`: three-letter side prefix (`ARA`, `TAR`, `VER`, `ZON`, `CRE`, ...).
    pub side: Option<String>,
    /// `tedclass` (`Aramon`, `Veruna`, ...).
    pub tedclass: Option<String>,
    /// `description`: in Kingdoms a kingdom word, not prose.
    pub description: Option<String>,
    /// `objectname`: model `objects3d/<objectname>.3do`.
    pub objectname: Option<String>,
    /// Script `scripts/<unitname>.cob` (Believed; see `docs/content.md`).
    pub script: String,
    pub unitnumber: Option<i32>,
    /// `category`, split on whitespace, original case.
    pub categories: Vec<String>,
    /// `damagecategory`: the armor class looked up in weapon damage tables.
    pub damage_category: Option<String>,
    pub body_type: Option<String>,
    pub movement_class: Option<String>,
    pub default_mission_type: Option<String>,
    pub economy: UnitEconomy,
    pub motion: UnitMotion,
    pub footprint: Footprint,
    /// `maxdamage`: hit points.
    pub max_damage: i32,
    /// `healtime`.
    pub heal_time: Option<Fx>,
    pub sight_distance: Option<i32>,
    pub radar_distance: Option<i32>,
    pub sonar_distance: Option<i32>,
    pub min_cloak_distance: Option<i32>,
    pub cloak_cost: Option<Fx>,
    pub cloak_cost_moving: Option<Fx>,
    pub transport: Transport,
    /// Names of [`FLAG_KEYS`] set to a nonzero value.
    pub flags: BTreeSet<String>,
    /// `corpse` / `frozen` / `stone`: feature names.
    pub corpse: Option<String>,
    pub frozen: Option<String>,
    pub stone: Option<String>,
    pub sound_category: Option<String>,
    pub sound_class: Option<String>,
    pub experience_points: Option<i32>,
    /// `[WEAPONn]` sections in file order.
    pub weapons: Vec<WeaponDef>,
    /// `[EXPLODEAS]`: the death explosion.
    pub explode_as: Option<WeaponDef>,
    /// `[ADJUSTARMOR]`, `[ADJUSTATTACK]`, `[ADJUSTJOY]` keyed by lowercase name.
    pub auras: BTreeMap<String, AuraDef>,
    /// Filled by [`crate::Ruleset`] from the build-menu directory.
    pub build_options: Vec<crate::BuildOption>,
    /// `[UNITINFO]` entries no field maps (lowercase key -> value).
    pub extra: BTreeMap<String, String>,
    /// Unmapped top-level sections, kept whole by name.
    pub extra_sections: Vec<String>,
}

impl UnitDef {
    pub fn has_flag(&self, key: &str) -> bool {
        self.flags.contains(&key.to_ascii_lowercase())
    }
}

/// Cost and the mana economy (Kingdoms' only resource).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UnitEconomy {
    /// `buildcost`: mana to build (Believed).
    pub build_cost: i64,
    /// `buildtime` (up to 100000 in the corpus, so wider than [`Fx`]).
    pub build_time: Option<WideFx>,
    /// `workertime`: build power of a builder.
    pub worker_time: Option<i32>,
    pub build_distance: Option<i32>,
    /// `maxmana`: the unit's own mana pool (casters, monarchs).
    pub max_mana: Option<i32>,
    /// `manarechargerate`.
    pub mana_recharge_rate: Option<i32>,
    /// `mogriumincome`: mana the unit adds to its player's income (lodestones).
    pub mogrium_income: Option<i32>,
    /// `mogriumstorage`: mana storage added to the player.
    pub mogrium_storage: Option<i32>,
    /// `economybonus`.
    pub economy_bonus: Option<i32>,
    /// `totalallowed`: per-player cap.
    pub total_allowed: Option<i32>,
}

/// Locomotion. Key names match the FBI so tak-sim's stand-in lines up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UnitMotion {
    /// `maxvelocity`.
    pub maxvelocity: Option<Fx>,
    /// `acceleration`.
    pub acceleration: Option<Fx>,
    /// `brakerate`.
    pub brakerate: Option<Fx>,
    /// `turnrate` (angle units per tick, Believed TA).
    pub turnrate: Option<i32>,
    pub turninplacerate: Option<i32>,
    pub maxslope: Option<i32>,
    pub maxwaterdepth: Option<i32>,
    pub waterline: Option<i32>,
    /// `cruisealt` (also the misspelling `cruisalt`).
    pub cruisealt: Option<i32>,
    pub maneuverleashlength: Option<i32>,
    /// `roadmultiplier` (also `roadmultplier`): speed factor on paved cells.
    pub roadmultiplier: Option<Fx>,
    /// `watermultiplier` (also `watermultipliser`).
    pub watermultiplier: Option<Fx>,
    pub bankscale: Option<Fx>,
    pub pitchscale: Option<Fx>,
    pub moverate1: Option<Fx>,
    pub moverate2: Option<Fx>,
    pub attackrunlength: Option<i32>,
    pub hoverattackdistance: Option<i32>,
    pub hoverattackaltitude: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Footprint {
    /// `footprintx` / `footprintz` in attribute cells.
    pub x: Option<u8>,
    pub z: Option<u8>,
    pub yardmap: Option<String>,
    /// `buildangle` (u16 angle units).
    pub build_angle: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Transport {
    pub capacity: Option<i32>,
    pub size: Option<i32>,
    pub size_capacity: Option<i32>,
    pub distance: Option<i32>,
    pub max_units: Option<i32>,
}

/// A `[WEAPONn]` or `[EXPLODEAS]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WeaponDef {
    /// Section name (`WEAPON1`, `EXPLODEAS`).
    pub slot: String,
    /// `name`: display name (runtime only).
    pub name: Option<String>,
    /// `type` (`Ballistic`, ...).
    pub kind: Option<String>,
    pub subtype: Option<String>,
    pub damage_type: Option<String>,
    pub range: Option<i32>,
    pub min_range: Option<i32>,
    /// `reloadtime` in seconds (Believed TA).
    pub reload_time: Option<Fx>,
    pub weapon_velocity: Option<i32>,
    pub area_of_effect: Option<i32>,
    pub edge_effectiveness: Option<Fx>,
    pub aim_tolerance: Option<i32>,
    pub turnrate: Option<i32>,
    /// `model`: projectile `objects3d/<model>.3do`.
    pub model: Option<String>,
    /// `manapershot`: mana spent from the unit's pool per shot.
    pub mana_per_shot: Option<i32>,
    pub gravity_adjustment: Option<Fx>,
    pub buildup_time: Option<Fx>,
    pub decay_time: Option<Fx>,
    pub duration: Option<Fx>,
    pub explosion_class: Option<String>,
    pub water_explosion_class: Option<String>,
    pub sound_hit_class: Option<String>,
    pub sound_hit: Option<String>,
    pub weapon_art: Option<String>,
    pub lob_preferred: bool,
    pub no_air_weapon: bool,
    pub units_only: bool,
    pub fire_starter: bool,
    pub dont_lead_targets: bool,
    pub veteran_level: Option<i32>,
    /// `veteranmodel` (also `vetmodel`).
    pub veteran_model: Option<String>,
    pub damage: DamageTable,
    pub extra: BTreeMap<String, String>,
}

/// `[DAMAGE]`: `default` plus per-damage-category entries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DamageTable {
    /// `default`: damage against any category not listed.
    pub default: i32,
    /// Other keys, lowercased (`fort`, `tier1`, `monarch`): a multiplier on
    /// `default` for targets whose `damagecategory` matches (Believed).
    pub multipliers: BTreeMap<String, Fx>,
}

impl DamageTable {
    /// Damage against a target of `category`, rounded down. Which rounding
    /// the engine uses is Unknown.
    pub fn against(&self, category: Option<&str>) -> i32 {
        let Some(m) = category.and_then(|c| self.multipliers.get(&c.to_ascii_lowercase())) else {
            return self.default;
        };
        let v = (i64::from(self.default) * i64::from(m.raw())) >> Fx::FRAC_BITS;
        i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
    }
}

/// `[ADJUSTARMOR]` / `[ADJUSTATTACK]` / `[ADJUSTJOY]`: an area effect on
/// nearby units (Believed: monarch/temple auras).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AuraDef {
    pub adjustment: Option<Fx>,
    pub affects_enemy: bool,
    pub edge_effectiveness: Option<Fx>,
    pub radius: Option<i32>,
    pub extra: BTreeMap<String, String>,
}

/// Builds a [`UnitDef`] from a parsed `.fbi`. Never fails: missing or bad
/// keys become `None` plus an [`crate::Issue`].
pub fn unit_from_fbi(doc: &Document, source: &str, ctx: &mut Ctx) -> Option<UnitDef> {
    let Some(info) = doc.section("UNITINFO") else {
        ctx.issues.push(crate::Issue {
            file: source.to_string(),
            section: String::new(),
            key: String::new(),
            message: "no [UNITINFO] section".into(),
        });
        return None;
    };
    let stem = file_stem(source);
    let mut f = Fields::new(info, source, "fbi", "UNITINFO".into(), ctx);
    let unitname = f.string("unitname").unwrap_or_else(|| stem.to_string());
    let script = format!("scripts/{}.cob", unitname.to_ascii_lowercase());
    let economy = UnitEconomy {
        build_cost: f.int("buildcost").unwrap_or(0),
        build_time: f.wide("buildtime"),
        worker_time: f.int("workertime"),
        build_distance: f.int("builddistance"),
        max_mana: f.int("maxmana"),
        mana_recharge_rate: f.int("manarechargerate"),
        mogrium_income: f.int("mogriumincome"),
        mogrium_storage: f.int("mogriumstorage"),
        economy_bonus: f.int("economybonus"),
        total_allowed: f.int("totalallowed"),
    };
    let motion = UnitMotion {
        maxvelocity: f.fx("maxvelocity"),
        acceleration: f.fx("acceleration"),
        brakerate: f.fx("brakerate"),
        turnrate: f.int("turnrate"),
        turninplacerate: f.int("turninplacerate"),
        maxslope: f.int("maxslope"),
        maxwaterdepth: f.int("maxwaterdepth"),
        waterline: f.int("waterline"),
        cruisealt: f.int("cruisealt").or_else(|| f.int("cruisalt")),
        maneuverleashlength: f.int("maneuverleashlength"),
        roadmultiplier: f.fx("roadmultiplier").or_else(|| f.fx("roadmultplier")),
        watermultiplier: f.fx("watermultiplier").or_else(|| f.fx("watermultipliser")),
        bankscale: f.fx("bankscale"),
        pitchscale: f.fx("pitchscale"),
        moverate1: f.fx("moverate1"),
        moverate2: f.fx("moverate2"),
        attackrunlength: f.int("attackrunlength"),
        hoverattackdistance: f.int("hoverattackdistance"),
        hoverattackaltitude: f.int("hoverattackaltitude"),
    };
    let footprint = Footprint {
        x: f.int("footprintx"),
        z: f.int("footprintz"),
        yardmap: f.string("yardmap"),
        build_angle: f.int("buildangle"),
    };
    let transport = Transport {
        capacity: f.int("transportcapacity"),
        size: f.int("transportsize"),
        size_capacity: f.int("transportsizecapacity"),
        distance: f.int("transportdistance"),
        max_units: f.int("transmaxunits"),
    };
    let mut flags = BTreeSet::new();
    for k in FLAG_KEYS {
        if f.flag(k) {
            flags.insert((*k).to_string());
        }
    }
    let mut def = UnitDef {
        unitname,
        source: source.to_string(),
        name: f.string("name"),
        side: f.string("side"),
        tedclass: f.string("tedclass"),
        description: f.string("description"),
        objectname: f.string("objectname"),
        script,
        unitnumber: f.int("unitnumber"),
        categories: f.words("category"),
        damage_category: f.string("damagecategory"),
        body_type: f.string("bodytype"),
        movement_class: f.string("movementclass"),
        default_mission_type: f.string("defaultmissiontype"),
        economy,
        motion,
        footprint,
        max_damage: f.int("maxdamage").unwrap_or(0),
        heal_time: f.fx("healtime"),
        sight_distance: f.int("sightdistance"),
        radar_distance: f.int("radardistance"),
        sonar_distance: f.int("sonardistance"),
        min_cloak_distance: f.int("mincloakdistance"),
        cloak_cost: f.fx("cloakcost"),
        cloak_cost_moving: f.fx("cloakcostmoving"),
        transport,
        flags,
        corpse: f.string("corpse"),
        frozen: f.string("frozen"),
        stone: f.string("stone"),
        sound_category: f.string("soundcategory"),
        sound_class: f.string("soundclass"),
        experience_points: f.int("experiencepoints"),
        weapons: Vec::new(),
        explode_as: None,
        auras: BTreeMap::new(),
        build_options: Vec::new(),
        extra: BTreeMap::new(),
        extra_sections: Vec::new(),
    };
    for sub in &info.sections {
        let lname = sub.name.to_ascii_lowercase();
        if lname.starts_with("adjust") {
            let path = format!("UNITINFO/{}", sub.name);
            let aura = aura(sub, source, path, f.ctx());
            def.auras.insert(lname, aura);
        } else {
            def.extra_sections.push(format!("UNITINFO/{}", sub.name));
        }
    }
    def.extra = f.finish();
    for sec in &doc.sections {
        let up = sec.name.to_ascii_uppercase();
        if up == "UNITINFO" {
            continue;
        } else if up.starts_with("WEAPON") || up == "EXPLODEAS" {
            let w = weapon(sec, source, ctx);
            if up == "EXPLODEAS" {
                def.explode_as = Some(w);
            } else {
                def.weapons.push(w);
            }
        } else {
            def.extra_sections.push(sec.name.clone());
        }
    }
    Some(def)
}

fn aura(sec: &Section, source: &str, path: String, ctx: &mut Ctx) -> AuraDef {
    let mut f = Fields::new(sec, source, "fbi", path, ctx);
    let mut a = AuraDef {
        adjustment: f.fx("adjustment"),
        affects_enemy: f.flag("affectsenemy"),
        edge_effectiveness: f.fx("edgeeffectiveness"),
        radius: f.int("radius"),
        extra: BTreeMap::new(),
    };
    a.extra = f.finish();
    a
}

/// Reads one `[WEAPONn]` / `[EXPLODEAS]` section with its `[DAMAGE]`.
pub fn weapon(sec: &Section, source: &str, ctx: &mut Ctx) -> WeaponDef {
    let damage = match sec.section("DAMAGE") {
        Some(d) => damage_table(d, source, format!("{}/DAMAGE", sec.name), ctx),
        None => DamageTable::default(),
    };
    let mut f = Fields::new(sec, source, "fbi", sec.name.clone(), ctx);
    let mut w = WeaponDef {
        slot: sec.name.clone(),
        name: f.string("name"),
        kind: f.string("type"),
        subtype: f.string("subtype"),
        damage_type: f.string("damagetype"),
        range: f.int("range"),
        min_range: f.int("minrange"),
        reload_time: f.fx("reloadtime"),
        weapon_velocity: f.int("weaponvelocity"),
        area_of_effect: f.int("areaofeffect"),
        edge_effectiveness: f.fx("edgeeffectiveness"),
        aim_tolerance: f.int("aimtolerance"),
        turnrate: f.int("turnrate"),
        model: f.string("model"),
        mana_per_shot: f.int("manapershot"),
        gravity_adjustment: f.fx("gravityadjustment"),
        buildup_time: f.fx("builduptime"),
        decay_time: f.fx("decaytime"),
        duration: f.fx("duration"),
        explosion_class: f.string("explosionclass"),
        water_explosion_class: f.string("waterexplosionclass"),
        sound_hit_class: f.string("soundhitclass"),
        sound_hit: f.string("soundhit"),
        weapon_art: f.string("weaponart"),
        lob_preferred: f.flag("lobpreferred"),
        no_air_weapon: f.flag("noairweapon"),
        units_only: f.flag("unitsonly"),
        fire_starter: f.flag("firestarter"),
        dont_lead_targets: f.flag("dontleadtargets"),
        veteran_level: f.int("veteranlevel"),
        veteran_model: f.string("veteranmodel").or_else(|| f.string("vetmodel")),
        damage,
        extra: BTreeMap::new(),
    };
    w.extra = f.finish();
    w
}

fn damage_table(sec: &Section, source: &str, path: String, ctx: &mut Ctx) -> DamageTable {
    let mut f = Fields::new(sec, source, "fbi", path, ctx);
    let mut t = DamageTable {
        default: f.int("default").unwrap_or(0),
        multipliers: BTreeMap::new(),
    };
    let keys: BTreeSet<String> = sec
        .entries
        .iter()
        .map(|e| e.key.to_ascii_lowercase())
        .filter(|k| k != "default")
        .collect();
    for k in keys {
        if let Some(v) = f.fx(&k) {
            t.multipliers.insert(k, v);
        }
    }
    f.finish();
    t
}

/// File name without directory or extension.
pub(crate) fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_formats::tdf;

    const FBI: &str = "[UNITINFO]\n{\n\tUnitName = TSTKNIGHT;\n\tObjectName = tstknight;\n\tside = TST;\n\tmaxvelocity = 1.25;\n\tacceleration = 10;\n\tbrakerate = 0.15;\n\tturnrate = 2400;\n\tbuildcost = 360000;\n\tbuildtime = 223.5;\n\tmaxdamage = 1100;\n\tcategory = TST GROUND  ATTACK;\n\tdamagecategory = Human;\n\tcanmove = 1;\n\tfloater = 0;\n\tmaxmana = 1000;\n\tmogriumincome = 10;\n\tbloodcolor1 = 1 2 3;\n\troadmultplier = 1.0;\n\t[ADJUSTJOY]\n\t{\n\t\tadjustment = 250;\n\t\tradius = 200;\n\t}\n}\n[WEAPON1]\n{\n\trange = 450;\n\treloadtime = 1.5;\n\tmanapershot = 300;\n\tmodel = tstarrow;\n\t[DAMAGE]\n\t{\n\t\tdefault = 1000;\n\t\tfort = 0.25;\n\t\tHuman = 2;\n\t}\n}\n[EXPLODEAS]\n{\n\tareaofeffect = 100;\n\t[DAMAGE]\n\t{\n\t\tdefault = 500;\n\t}\n}\n";

    #[test]
    fn parses_synthetic_unit() {
        let doc = tdf::parse(FBI.as_bytes()).unwrap();
        let mut ctx = Ctx::default();
        let u = unit_from_fbi(&doc, "units/tstknight.fbi", &mut ctx).unwrap();
        assert!(ctx.issues.is_empty(), "{:?}", ctx.issues);
        assert_eq!(u.unitname, "TSTKNIGHT");
        assert_eq!(u.script, "scripts/tstknight.cob");
        assert_eq!(u.motion.maxvelocity, Some(Fx::from_ratio(5, 4).unwrap()));
        assert_eq!(u.motion.acceleration, Some(Fx::from_int(10)));
        assert_eq!(u.motion.turnrate, Some(2400));
        assert_eq!(u.motion.roadmultiplier, Some(Fx::ONE));
        assert_eq!(u.economy.build_cost, 360_000);
        assert_eq!(
            u.economy.build_time,
            Some(WideFx(i64::from(Fx::from_ratio(447, 2).unwrap().raw())))
        );
        assert_eq!(u.economy.max_mana, Some(1000));
        assert_eq!(u.economy.mogrium_income, Some(10));
        assert_eq!(u.categories, vec!["TST", "GROUND", "ATTACK"]);
        assert!(u.has_flag("canmove"));
        assert!(!u.has_flag("floater"));
        assert_eq!(
            u.extra.get("bloodcolor1").map(String::as_str),
            Some("1 2 3")
        );
        assert_eq!(u.auras["adjustjoy"].radius, Some(200));
        assert_eq!(u.weapons.len(), 1);
        let w = &u.weapons[0];
        assert_eq!(w.mana_per_shot, Some(300));
        assert_eq!(w.damage.default, 1000);
        assert_eq!(w.damage.against(Some("FORT")), 250);
        assert_eq!(w.damage.against(u.damage_category.as_deref()), 2000);
        assert_eq!(w.damage.against(Some("naval")), 1000);
        assert_eq!(u.explode_as.as_ref().unwrap().damage.default, 500);
        let cov = &ctx.coverage.sections["fbi:UNITINFO"];
        assert_eq!(cov.unmapped.get("bloodcolor1"), Some(&1));
        assert!(ctx.coverage.sections.contains_key("fbi:WEAPON#/DAMAGE"));
    }

    #[test]
    fn missing_unitinfo_is_an_issue() {
        let doc = tdf::parse(b"[X]\n{\n}\n").unwrap();
        let mut ctx = Ctx::default();
        assert!(unit_from_fbi(&doc, "units/x.fbi", &mut ctx).is_none());
        assert_eq!(ctx.issues.len(), 1);
    }

    #[test]
    fn stem() {
        assert_eq!(file_stem("units/ARAARCH.FBI"), "ARAARCH");
        assert_eq!(file_stem("x"), "x");
    }
}
