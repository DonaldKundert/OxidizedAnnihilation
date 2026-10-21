//! Real data (gated on `TAK_DATA_DIR`): the economy keys of every monarch
//! and lodestone, the sacred sites of Adamantine Gate, and income with a
//! monarch alone and with real lodestones spawned on real sites.

use tak_commands::{PlayerId, UnitDefId};
use tak_content::{Ruleset, RulesetKind, WideFx};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::content::{load_match, unit_def_from_fbi};
use tak_sim::{NoScripts, PlayerSetup, World, economy, systems};

const MAP: &str = "kmap/Adamantine Gate";

fn setup(test: &str) -> Option<(tak_vfs::Vfs, Ruleset)> {
    let dir = tak_vfs::testing::data_dir_or_skip(test)?;
    let vfs = tak_vfs::open_kingdoms(&dir).expect("open install");
    let rs = Ruleset::load(&vfs, RulesetKind::Base);
    Some((vfs, rs))
}

#[test]
fn fbi_economy_keys_convert() {
    let Some((_vfs, rs)) = setup("fbi_economy_keys_convert") else {
        return;
    };
    for name in ["ARAKING", "TARNECRO", "ZONHUNT", "CRESAGE", "VERMAGE"] {
        let d = unit_def_from_fbi(rs.unit(name).unwrap()).economy;
        assert_eq!((d.mogriumincome, d.mogriumstorage), (10, 5000), "{name}");
        assert_eq!(d.maxmana, 1000, "{name}");
        assert!(d.manarechargerate > 0 && !d.sacred_site, "{name}");
    }
    let mut lodes = 0;
    for u in rs.units.values() {
        let d = unit_def_from_fbi(u).economy;
        let n = u.unitname.to_ascii_uppercase();
        if n.ends_with("LODE") {
            assert_eq!((d.mogriumincome, d.mogriumstorage), (10, 1000), "{n}");
            assert!(d.sacred_site, "{n}");
            lodes += 1;
        }
        if d.sacred_site {
            assert!(d.mogriumincome > 0 || n == "NPCTEMP2", "{n}");
        }
    }
    assert_eq!(lodes, 5);
    let divine = unit_def_from_fbi(rs.unit("ARAMANA").unwrap()).economy;
    assert_eq!((divine.mogriumincome, divine.sacred_site), (20, true));
}

#[test]
fn adamantine_gate_income() {
    let Some((vfs, rs)) = setup("adamantine_gate_income") else {
        return;
    };
    let m = load_match(&vfs, MAP, &PlayerSetup::parse_list("ARA,ZON"), 1).unwrap();
    let mut sc = m.scenario;
    let sites = sc.economy.sacred_sites.clone();
    assert!(!sites.is_empty(), "map has sacred sites");
    let tiers = [Fx::ONE, Fx::from_ratio(3, 2).unwrap(), Fx::from_int(2)];
    for s in &sites {
        assert!(tiers.contains(&s.multiplier), "{s:?}");
        assert_eq!((s.w, s.h), (32, 32));
    }
    eprintln!("Adamantine Gate: {} sacred sites", sites.len());

    sc.unit_defs
        .push(unit_def_from_fbi(rs.unit("ARALODE").unwrap()));
    sc.unit_defs
        .push(unit_def_from_fbi(rs.unit("ARAMANA").unwrap()));
    let lode = UnitDefId(sc.unit_defs.len() as u32 - 2);
    let divine = UnitDefId(sc.unit_defs.len() as u32 - 1);
    let mut w = World::new(&sc).unwrap();
    systems::spawn_start_units(&mut w);
    let tick = |w: &mut World, n| {
        for _ in 0..n {
            economy::tick(w, &mut NoScripts);
        }
    };
    let p = PlayerId(0);
    tick(&mut w, 30 * 60);
    assert_eq!(economy::income_per_second(&w, p), WideFx::from_int(10));
    assert_eq!(economy::storage(&w, p), WideFx::from_int(5000));
    assert_eq!(economy::mana(&w, p), WideFx::from_int(1600));

    // A Lodestone on the first site, a Divine Lodestone on the second.
    let centre = |i: usize| {
        let s = &sites[i];
        Vec2::from_ints(s.x + s.w / 2, s.y + s.h / 2)
    };
    w.spawn(lode, p, centre(0), Angle::ZERO).unwrap();
    let mut expect = (10 << 16) + (10 * i64::from(sites[0].multiplier.raw()));
    if sites.len() > 1 {
        w.spawn(divine, p, centre(1), Angle::ZERO).unwrap();
        expect += 20 * i64::from(sites[1].multiplier.raw());
    }
    tick(&mut w, 30);
    assert_eq!(economy::income_per_second(&w, p), WideFx(expect));
    assert!(economy::income_per_second(&w, p) > WideFx::from_int(10));
    assert_eq!(
        economy::income_per_second(&w, PlayerId(1)),
        WideFx::from_int(10)
    );
}
