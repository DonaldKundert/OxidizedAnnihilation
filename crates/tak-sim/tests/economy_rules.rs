//! Economy rules on synthetic content (`docs/economy.md`): monarch income,
//! lodestones on sacred sites, storage and overflow, `drain` (full,
//! partial, proportional sharing), `credit`, and unit mana pools.

use tak_commands::{PlayerId, UnitDefId, UnitId};
use tak_content::WideFx;
use tak_math::{Angle, Fx, Vec2};
use tak_sim::economy::{self, EconomyBonusRule, EconomyDef, EconomySetup, SacredSite, UnitMana};
use tak_sim::map::{MapInfo, StartPos};
use tak_sim::scenario::{PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario};
use tak_sim::testing::monarch_def;
use tak_sim::{NoScripts, World, systems};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const MONARCH: UnitDefId = UnitDefId(0);
const LODE: UnitDefId = UnitDefId(1);
const DIVINE: UnitDefId = UnitDefId(2);
const ARCHER: UnitDefId = UnitDefId(3);

fn wide(n: i64) -> WideFx {
    WideFx::from_int(n)
}

/// Monarch: 10/s, 5000 storage, a 1000 pool recharging 20/s (the Aramon
/// monarch's FBI numbers). Lodestone 10/s + 1000, Divine 20/s + 2000, both
/// `yardmap = S`. Sites: weak (1.0) at (256,256), swarthy (1.5) at
/// (512,256), powerful (2.0) at (768,256), each 32 x 32.
fn scenario(players: usize) -> Scenario {
    let mut monarch = monarch_def("king");
    monarch.economy = EconomyDef {
        mogriumincome: 10,
        mogriumstorage: 5000,
        economybonus: 30,
        maxmana: 1000,
        manarechargerate: 20,
        sacred_site: false,
    };
    let mut lode = monarch_def("lode");
    lode.maxvelocity = Fx::ZERO;
    lode.economy = EconomyDef {
        mogriumincome: 10,
        mogriumstorage: 1000,
        sacred_site: true,
        ..Default::default()
    };
    let mut divine = lode.clone();
    divine.name = "divine".into();
    divine.economy.mogriumincome = 20;
    divine.economy.mogriumstorage = 2000;
    let mut archer = monarch_def("archer");
    archer.economy = EconomyDef {
        maxmana: 1000,
        manarechargerate: 10,
        ..Default::default()
    };
    let site = |x, m: Fx| SacredSite {
        x,
        y: 256,
        w: 32,
        h: 32,
        multiplier: m,
    };
    let setup = EconomySetup {
        sacred_sites: vec![
            site(256, Fx::ONE),
            site(512, Fx::from_ratio(3, 2).unwrap()),
            site(768, Fx::from_int(2)),
        ],
        ..Default::default()
    };
    Scenario {
        version: SCENARIO_FORMAT_VERSION,
        seed: 3,
        map: MapInfo::flat(
            2048,
            2048,
            vec![StartPos { x: 100, y: 1000 }, StartPos { x: 1900, y: 1000 }],
        ),
        unit_defs: vec![monarch, lode, divine, archer],
        players: (0..players)
            .map(|i| PlayerSpec {
                start: i as u32,
                monarch: MONARCH,
            })
            .collect(),
        economy: Box::new(setup),
        features: Default::default(),
    }
}

fn world(sc: &Scenario) -> World {
    let mut w = World::new(sc).unwrap();
    systems::spawn_start_units(&mut w);
    w
}

fn ticks(w: &mut World, n: u32) {
    for _ in 0..n {
        economy::tick(w, &mut NoScripts);
    }
}

fn spawn(w: &mut World, def: UnitDefId, owner: PlayerId, x: i32, y: i32) -> UnitId {
    w.spawn(def, owner, Vec2::from_ints(x, y), Angle::ZERO)
        .unwrap()
}

#[test]
fn monarch_alone_earns_ten_per_second() {
    let mut w = world(&scenario(1));
    assert_eq!(economy::mana(&w, P0), wide(1000), "start mana");
    ticks(&mut w, 30);
    assert_eq!(economy::income_per_second(&w, P0), wide(10));
    assert_eq!(economy::storage(&w, P0), wide(5000));
    assert_eq!(economy::mana(&w, P0), wide(1010));
    ticks(&mut w, 30 * 59);
    assert_eq!(economy::mana(&w, P0), wide(1600), "600 per minute");
}

#[test]
fn lodestones_scale_with_their_site() {
    let mut w = world(&scenario(1));
    let weak = spawn(&mut w, LODE, P0, 270, 270);
    ticks(&mut w, 1);
    assert_eq!(economy::income_per_second(&w, P0), wide(20));
    assert_eq!(economy::storage(&w, P0), wide(6000));
    spawn(&mut w, LODE, P0, 528, 270); // swarthy: 15
    spawn(&mut w, DIVINE, P0, 780, 270); // powerful divine: 40
    ticks(&mut w, 1);
    assert_eq!(economy::income_per_second(&w, P0), wide(10 + 10 + 15 + 40));
    assert_eq!(w.unit(weak).unwrap().economy.site(), Some(0));
    // A second lodestone on a claimed site, and one on open ground, get the
    // off-site multiplier (1.0).
    spawn(&mut w, LODE, P0, 266, 266);
    spawn(&mut w, LODE, P0, 1500, 1500);
    ticks(&mut w, 1);
    assert_eq!(economy::income_per_second(&w, P0), wide(75 + 20));
    // Killing the weak lodestone frees its site for the next one in slot
    // order; income drops by one lodestone.
    w.despawn(weak);
    ticks(&mut w, 1);
    assert_eq!(economy::income_per_second(&w, P0), wide(85));
    // Over a minute the stock rises by exactly the income (minus nothing:
    // storage is 5000 + 5 x 1000 + 2000).
    let before = economy::mana(&w, P0);
    ticks(&mut w, 30);
    assert_eq!(economy::mana(&w, P0).0 - before.0, wide(85).0);
}

#[test]
fn sites_slack_reaches_one_cell_beyond_the_footprint() {
    let mut w = world(&scenario(1));
    spawn(&mut w, LODE, P0, 512 + 32 + 10, 250); // right of swarthy, within 16
    ticks(&mut w, 1);
    assert_eq!(economy::income_per_second(&w, P0), wide(10 + 15));
}

#[test]
fn other_players_lodestones_do_not_count() {
    let mut w = world(&scenario(2));
    spawn(&mut w, LODE, P1, 270, 270);
    ticks(&mut w, 30);
    assert_eq!(economy::income_per_second(&w, P0), wide(10));
    assert_eq!(economy::income_per_second(&w, P1), wide(20));
}

#[test]
fn storage_caps_the_stock_and_losing_storage_loses_mana() {
    let mut sc = scenario(1);
    sc.economy.rules.start_mana = 4990;
    let mut w = world(&sc);
    let lode = spawn(&mut w, LODE, P0, 270, 270);
    ticks(&mut w, 30 * 60);
    assert_eq!(economy::mana(&w, P0), wide(6000), "full: overflow lost");
    w.despawn(lode);
    ticks(&mut w, 1);
    assert_eq!(economy::mana(&w, P0), wide(5000));
}

#[test]
fn start_mana_is_capped_by_storage() {
    let mut sc = scenario(1);
    sc.economy.rules.start_mana = 9000;
    let mut w = world(&sc);
    ticks(&mut w, 1);
    assert!(economy::mana(&w, P0) <= wide(5000));
}

#[test]
fn economy_bonus_options() {
    let mut sc = scenario(1);
    let mut w = world(&sc);
    spawn(&mut w, LODE, P0, 270, 270);
    ticks(&mut w, 1);
    assert_eq!(economy::income_per_second(&w, P0), wide(20), "ignored");

    sc.economy.rules.economy_bonus = EconomyBonusRule::UnitIncomePercent;
    let mut w = world(&sc);
    spawn(&mut w, LODE, P0, 270, 270);
    ticks(&mut w, 1);
    assert_eq!(economy::income_per_second(&w, P0), wide(13 + 10));

    sc.economy.rules.economy_bonus = EconomyBonusRule::PlayerIncomePercent;
    let mut w = world(&sc);
    spawn(&mut w, LODE, P0, 270, 270);
    ticks(&mut w, 1);
    assert_eq!(economy::income_per_second(&w, P0), wide(26));
}

#[test]
fn drain_full_grant() {
    let mut w = world(&scenario(1));
    ticks(&mut w, 1);
    let before = economy::mana(&w, P0);
    let g = economy::drain(&mut w, P0, Fx::from_int(28));
    assert_eq!(g, Fx::from_int(28));
    assert_eq!(
        economy::mana(&w, P0).0,
        before.0 - Fx::from_int(28).raw() as i64
    );
    assert_eq!(economy::drain(&mut w, P0, Fx::ZERO), Fx::ZERO);
    assert_eq!(economy::drain(&mut w, P0, Fx::from_int(-5)), Fx::ZERO);
    assert_eq!(economy::drain(&mut w, PlayerId(9), Fx::ONE), Fx::ZERO);
}

#[test]
fn drain_partial_grant_empties_the_stock() {
    let mut sc = scenario(1);
    sc.economy.rules.start_mana = 50;
    let mut w = world(&sc);
    let left = economy::mana(&w, P0);
    let g = economy::drain(&mut w, P0, Fx::from_int(80));
    assert_eq!(i64::from(g.raw()), left.0);
    assert_eq!(economy::mana(&w, P0), wide(0));
    assert_eq!(economy::drain(&mut w, P0, Fx::ONE), Fx::ZERO);
}

#[test]
fn drain_shares_a_shortage_proportionally() {
    let mut sc = scenario(1);
    sc.economy.rules.start_mana = 0;
    let mut w = world(&sc);
    ticks(&mut w, 1); // first income
    // Two builders want 1 and 3 mana a tick against 10/s income.
    let (a, b) = (Fx::from_int(1), Fx::from_int(3));
    let mut got = (0i64, 0i64);
    for t in 0..300 {
        let ga = economy::drain(&mut w, P0, a);
        let gb = economy::drain(&mut w, P0, b);
        if t >= 2 {
            got.0 += i64::from(ga.raw());
            got.1 += i64::from(gb.raw());
            // Same fraction for both, to rounding.
            let lhs = i64::from(ga.raw()) * 3;
            let rhs = i64::from(gb.raw());
            assert!((lhs - rhs).abs() <= 3, "tick {t}: {ga:?} vs {gb:?}");
        }
        ticks(&mut w, 1);
    }
    // While stalled, total spending equals income (10/s over 298 ticks),
    // to within the one tick of stock held back.
    let spent = got.0 + got.1;
    let income = (10i64 << 16) * 298 / 30;
    assert!((spent - income).abs() <= (1 << 16), "{spent} vs {income}");
    assert!(got.1 > got.0 * 2, "b asks 3x as much and gets 3x as much");
    let p = economy::player(&w, P0).unwrap();
    assert_eq!(p.last_demand, 4 << 16);
    assert!(p.ratio < Fx::ONE);
}

#[test]
fn drain_recovers_full_grants_after_a_shortage() {
    let mut sc = scenario(1);
    sc.economy.rules.start_mana = 0;
    let mut w = world(&sc);
    ticks(&mut w, 1);
    economy::drain(&mut w, P0, Fx::from_int(100));
    ticks(&mut w, 30);
    // No demand for a second: ratio back to 1, full grant of a small ask.
    assert_eq!(economy::drain(&mut w, P0, Fx::ONE), Fx::ONE);
}

#[test]
fn credit_is_capped_by_storage() {
    let mut sc = scenario(1);
    sc.economy.rules.start_mana = 4900;
    let mut w = world(&sc);
    ticks(&mut w, 1);
    let room = wide(5000).0 - economy::mana(&w, P0).0;
    let added = economy::credit(&mut w, P0, Fx::from_int(250));
    assert_eq!(i64::from(added.raw()), room);
    assert_eq!(economy::mana(&w, P0), wide(5000));
    assert_eq!(economy::credit(&mut w, P0, Fx::from_int(-1)), Fx::ZERO);
}

#[test]
fn unit_pools_recharge_and_pay_whole_spells() {
    let mut w = world(&scenario(1));
    let king = w.players()[0].monarch.unwrap();
    let archer = spawn(&mut w, ARCHER, P0, 600, 600);
    let lode = spawn(&mut w, LODE, P0, 270, 270);
    assert_eq!(economy::unit_mana(&w, lode), None, "no pool");
    let m = |w: &World, id| economy::unit_mana(w, id).unwrap();
    assert_eq!(m(&w, archer).mana, Fx::ZERO, "built units start empty");
    ticks(&mut w, 30);
    assert_eq!(
        m(&w, king),
        UnitMana {
            mana: Fx::from_int(20),
            max: Fx::from_int(1000)
        }
    );
    assert_eq!(m(&w, archer).mana, Fx::from_int(10));
    assert!(!economy::spend_unit_mana(&mut w, king, Fx::from_int(21)));
    assert_eq!(m(&w, king).mana, Fx::from_int(20), "no partial casts");
    assert!(economy::spend_unit_mana(&mut w, king, Fx::from_int(20)));
    assert_eq!(m(&w, king).mana, Fx::ZERO);
    // Full after 50 s and stays full.
    ticks(&mut w, 30 * 60);
    assert_eq!(m(&w, king).mana, Fx::from_int(1000));
    // Player mana is untouched by spells.
    let before = economy::mana(&w, P0);
    assert!(economy::spend_unit_mana(&mut w, king, Fx::from_int(900)));
    assert_eq!(economy::mana(&w, P0), before);
    economy::set_unit_mana_percent(&mut w, archer, 50);
    assert_eq!(m(&w, archer).mana, Fx::from_int(500));
}

#[test]
fn monarch_start_pool_follows_the_rule() {
    let mut sc = scenario(1);
    sc.economy.rules.monarch_start_mana_percent = 100;
    let mut w = world(&sc);
    ticks(&mut w, 1);
    let king = w.players()[0].monarch.unwrap();
    assert_eq!(
        economy::unit_mana(&w, king).unwrap().mana,
        Fx::from_int(1000)
    );
}

#[test]
fn scenario_json_round_trips_and_old_json_still_loads() {
    let sc = scenario(2);
    assert_eq!(Scenario::from_json(&sc.to_json_pretty()).unwrap(), sc);
    // A scenario without economy fields (the pre-WP-N form) loads with
    // the default rules.
    let mut v: serde_json::Value = serde_json::from_str(&sc.to_json_pretty()).unwrap();
    v.as_object_mut().unwrap().remove("economy");
    for d in v["unit_defs"].as_array_mut().unwrap() {
        d.as_object_mut().unwrap().remove("economy");
    }
    let old = Scenario::from_json(&v.to_string()).unwrap();
    assert_eq!(*old.economy, EconomySetup::default());
    // Out-of-range economy values are rejected.
    let mut bad = sc.clone();
    bad.unit_defs[0].economy.maxmana = 40_000;
    assert!(bad.validate().is_err());
    let mut bad = sc;
    bad.economy.rules.monarch_start_mana_percent = 101;
    assert!(bad.validate().is_err());
}
