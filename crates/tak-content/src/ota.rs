//! Typed view of a map header (`.ota`): `[GLOBALHEADER]` with nested
//! `[MAP DATA]`, `[SPECIALS]/[SPECIALn]` and `[UNITS]/[UNITn]`.

use crate::fields::{Ctx, Fields};
use serde::Serialize;
use std::collections::BTreeMap;
use tak_formats::tdf::{Document, Section};

/// `[GLOBALHEADER]` keys that state win/lose conditions; kept verbatim in
/// [`OtaDef::conditions`].
pub const CONDITION_KEYS: &[&str] = &[
    "allunitskilled",
    "allunitskilledoftype",
    "commanderkilled",
    "deathtimerrunsout",
    "destroyallunits",
    "killallmobileunits",
    "killalloftype",
    "killenemycommander",
    "killunittype",
    "moveunittoradius",
    "unittypekilled",
    "unittypepassesx",
    "unittypepassesz",
    "useonlyunits",
    "victorytimerrunsout",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OtaDef {
    /// `kingdom`, lowercased: picks `palettes/<kingdom>.pcx`.
    pub kingdom: Option<String>,
    /// `numplayers`, raw (`4`, empty in missions, a list in a few maps).
    pub num_players_raw: Option<String>,
    /// Every number in `numplayers` (`2, 4` -> `[2, 4]`).
    pub num_players: Vec<u8>,
    /// `size` (`6 x 6`): map size in 32-cell blocks (Known for multiplayer).
    pub size: Option<(u32, u32)>,
    pub memory: Option<String>,
    /// `missionname` / `missiondescription` (runtime only, never committed).
    pub mission_name: Option<String>,
    pub mission_description: Option<String>,
    pub is_mission: bool,
    pub has_scenario: bool,
    pub max_units: Option<i32>,
    pub gravity: Option<i32>,
    pub kill_mul: Option<i32>,
    pub time_mul: Option<i32>,
    pub line_of_sight: Option<i32>,
    pub mapping: Option<i32>,
    pub min_wind_speed: Option<i32>,
    pub max_wind_speed: Option<i32>,
    pub solar_strength: Option<i32>,
    pub tidal_strength: Option<i32>,
    pub water_does_damage: bool,
    pub water_damage: Option<i32>,
    pub lava_world: bool,
    pub no_sea_level_trigger: bool,
    /// [`CONDITION_KEYS`] present, lowercase key -> raw value.
    pub conditions: BTreeMap<String, String>,
    /// `playerN` keys: N -> raw value (side/AI setup in missions).
    pub players: BTreeMap<u8, String>,
    /// `[MAP DATA]` `type` (`Network 1`, ...).
    pub map_type: Option<String>,
    pub ai_profile: Option<String>,
    pub specials: Vec<Special>,
    pub units: Vec<PlacedUnit>,
    pub extra: BTreeMap<String, String>,
}

/// A `[SPECIALn]`: a marker such as `StartPos3`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Special {
    pub what: String,
    /// `xpos` / `zpos` in attribute cells (Believed: within the TNT size on
    /// every map).
    pub x: i32,
    pub z: i32,
}

impl Special {
    /// `StartPosN` -> `N`.
    pub fn start_player(&self) -> Option<u8> {
        let w = self.what.to_ascii_lowercase();
        w.strip_prefix("startpos")?.parse().ok()
    }
}

/// A `[UNITn]`: a pre-placed unit (missions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlacedUnit {
    pub unitname: String,
    pub player: Option<i32>,
    /// Attribute-cell coordinates (Believed); `y` is height.
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `angle` in degrees (values 0..359 seen, Believed).
    pub angle: Option<i32>,
    pub health_percentage: Option<i32>,
    /// `manapercentage`: initial fill of the unit's mana pool.
    pub mana_percentage: Option<i32>,
    pub kills: Option<i32>,
    pub ident: Option<String>,
    pub initial_mission: Option<String>,
    pub ai_ignore: bool,
    pub ai_priority_target: bool,
    pub mission_critical: bool,
    pub extra: BTreeMap<String, String>,
}

impl OtaDef {
    /// Reads and types the `.ota` at `path`. `None` if it is missing or
    /// does not parse; bad values inside become defaults.
    pub fn load(vfs: &tak_vfs::Vfs, path: &str) -> Option<OtaDef> {
        let mut ctx = Ctx::default();
        let doc = crate::ruleset::read_tdf(vfs, path, &mut ctx)?;
        Some(ota_from_tdf(&doc, path, &mut ctx))
    }

    /// `palettes/<kingdom>.pcx`, the palette for this map's minimaps.
    pub fn palette_path(&self) -> Option<String> {
        self.kingdom.as_ref().map(|k| format!("palettes/{k}.pcx"))
    }

    /// Start markers, `(player, x, z)`, sorted by player.
    pub fn start_positions(&self) -> Vec<(u8, i32, i32)> {
        let mut v: Vec<_> = self
            .specials
            .iter()
            .filter_map(|s| Some((s.start_player()?, s.x, s.z)))
            .collect();
        v.sort();
        v
    }
}

/// Builds the typed view. Missing sections give empty fields.
pub fn ota_from_tdf(doc: &Document, source: &str, ctx: &mut Ctx) -> OtaDef {
    let Some(gh) = doc.section("GLOBALHEADER") else {
        ctx.issues.push(crate::Issue {
            file: source.to_string(),
            section: String::new(),
            key: String::new(),
            message: "no [GLOBALHEADER] section".into(),
        });
        return OtaDef::default();
    };
    let mut f = Fields::new(gh, source, "ota", "GLOBALHEADER".into(), ctx);
    let num_players_raw = f.string("numplayers");
    let num_players = num_players_raw
        .as_deref()
        .map(|s| {
            s.split(|c: char| !c.is_ascii_digit())
                .filter_map(|t| t.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    let size = f.raw("size").and_then(parse_size);
    let mut conditions = BTreeMap::new();
    for k in CONDITION_KEYS {
        if let Some(v) = f.string(k) {
            conditions.insert((*k).to_string(), v);
        }
    }
    let mut players = BTreeMap::new();
    for e in &gh.entries {
        let k = e.key.to_ascii_lowercase();
        if let Some(n) = k.strip_prefix("player").and_then(|n| n.parse::<u8>().ok()) {
            f.consume(&[&k]);
            players.insert(n, e.value.trim().to_string());
        }
    }
    let mut ota = OtaDef {
        kingdom: f.string("kingdom").map(|k| k.to_ascii_lowercase()),
        num_players_raw,
        num_players,
        size,
        memory: f.string("memory"),
        mission_name: f.string("missionname"),
        mission_description: f.string("missiondescription"),
        is_mission: f.flag("ismission"),
        has_scenario: f.flag("hasscenario"),
        max_units: f.int("maxunits"),
        gravity: f.int("gravity"),
        kill_mul: f.int("killmul"),
        time_mul: f.int("timemul"),
        line_of_sight: f.int("lineofsight"),
        mapping: f.int("mapping"),
        min_wind_speed: f.int("minwindspeed"),
        max_wind_speed: f.int("maxwindspeed"),
        solar_strength: f.int("solarstrength"),
        tidal_strength: f.int("tidalstrength"),
        water_does_damage: f.flag("waterdoesdamage"),
        water_damage: f.int("waterdamage"),
        lava_world: f.flag("lavaworld"),
        no_sea_level_trigger: f.flag("nosealeveltrigger"),
        conditions,
        players,
        ..OtaDef::default()
    };
    ota.extra = f.finish();
    if let Some(md) = gh.section("MAP DATA") {
        map_data(md, source, &mut ota, ctx);
    }
    ota
}

fn map_data(md: &Section, source: &str, ota: &mut OtaDef, ctx: &mut Ctx) {
    let mut f = Fields::new(md, source, "ota", "GLOBALHEADER/MAP DATA".into(), ctx);
    ota.map_type = f.string("type");
    ota.ai_profile = f.string("aiprofile");
    for (k, v) in f.finish() {
        ota.extra.insert(format!("map data/{k}"), v);
    }
    for sp in md.sections_named("SPECIALS") {
        for s in &sp.sections {
            let path = format!("GLOBALHEADER/MAP DATA/SPECIALS/{}", s.name);
            let mut f = Fields::new(s, source, "ota", path, ctx);
            let what = f.string("specialwhat").unwrap_or_default();
            let x = f.int("xpos").unwrap_or(0);
            let z = f.int("zpos").unwrap_or(0);
            f.finish();
            ota.specials.push(Special { what, x, z });
        }
    }
    for us in md.sections_named("UNITS") {
        for u in &us.sections {
            let path = format!("GLOBALHEADER/MAP DATA/UNITS/{}", u.name);
            let mut f = Fields::new(u, source, "ota", path, ctx);
            let mut p = PlacedUnit {
                unitname: f.string("unitname").unwrap_or_default(),
                player: f.int("player"),
                x: f.int("xpos").unwrap_or(0),
                y: f.int("ypos").unwrap_or(0),
                z: f.int("zpos").unwrap_or(0),
                angle: f.int("angle"),
                health_percentage: f.int("healthpercentage"),
                mana_percentage: f.int("manapercentage"),
                kills: f.int("kills"),
                ident: f.string("ident"),
                initial_mission: f.string("initialmission"),
                ai_ignore: f.flag("aiignore"),
                ai_priority_target: f.flag("aiprioritytarget"),
                mission_critical: f.flag("missioncriticalunit"),
                extra: BTreeMap::new(),
            };
            p.extra = f.finish();
            ota.units.push(p);
        }
    }
}

/// `"6 x 6"` -> `(6, 6)`.
pub fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s
        .to_ascii_lowercase()
        .split_once('x')
        .map(|(a, b)| (a.trim().to_string(), b.trim().to_string()))?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_formats::tdf;

    const OTA: &str = "[GlobalHeader]\n\t{\n\tkingdom=Taros;\n\tnumplayers=2;\n\tsize=6 x 5;\n\tmissionname=;\n\tcommanderkilled=1;\n\tplayer10=x y;\n\twhatever=3;\n\t[Map Data]\n\t\t{\n\t\tType=Network 1;\n\t\taiprofile=DEFAULT;\n\t\t[specials]\n\t\t\t{\n\t\t\t[special0]\n\t\t\t\t{\n\t\t\t\tspecialwhat=StartPos2;\n\t\t\t\tXPos=60;\n\t\t\t\tZPos=153;\n\t\t\t\t}\n\t\t\t[special1]\n\t\t\t\t{\n\t\t\t\tspecialwhat=StartPos1;\n\t\t\t\tXPos=113;\n\t\t\t\tZPos=20;\n\t\t\t\t}\n\t\t\t}\n\t\t[units]\n\t\t\t{\n\t\t\t[unit0]\n\t\t\t\t{\n\t\t\t\tunitname=TSTKNIGHT;\n\t\t\t\tplayer=2;\n\t\t\t\tXPos=10;\n\t\t\t\tYPos=0;\n\t\t\t\tZPos=12;\n\t\t\t\tident=;\n\t\t\t\tmanapercentage=100;\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n";

    #[test]
    fn typed_ota() {
        let doc = tdf::parse(OTA.as_bytes()).unwrap();
        let mut ctx = Ctx::default();
        let o = ota_from_tdf(&doc, "kmap/tst.ota", &mut ctx);
        assert!(ctx.issues.is_empty(), "{:?}", ctx.issues);
        assert_eq!(o.kingdom.as_deref(), Some("taros"));
        assert_eq!(o.num_players, vec![2]);
        assert_eq!(o.size, Some((6, 5)));
        assert_eq!(o.mission_name, None);
        assert_eq!(o.conditions["commanderkilled"], "1");
        assert_eq!(o.players[&10], "x y");
        assert_eq!(o.extra["whatever"], "3");
        assert_eq!(o.map_type.as_deref(), Some("Network 1"));
        assert_eq!(o.start_positions(), vec![(1, 113, 20), (2, 60, 153)]);
        assert_eq!(o.units.len(), 1);
        assert_eq!(o.units[0].unitname, "TSTKNIGHT");
        assert_eq!(o.units[0].mana_percentage, Some(100));
        assert_eq!(o.units[0].ident, None);
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("6 x 6"), Some((6, 6)));
        assert_eq!(parse_size("10X12"), Some((10, 12)));
        assert_eq!(parse_size("big"), None);
    }
}
