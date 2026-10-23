//! Cross-process determinism for the economy: the same economy run (two
//! players, lodestones on sacred sites, several consumers draining through
//! a shortage, spells paid from unit pools, lodestones dying) gives the
//! same world hash every tick in this process and in two child processes
//! (this test binary re-run with `ECONOMY_CHILD=1`).

use std::process::Command;

use tak_commands::{PlayerId, UnitDefId};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::economy::{self, EconomyDef, EconomyRules, EconomySetup, SacredSite};
use tak_sim::map::{MapInfo, StartPos};
use tak_sim::scenario::{PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario};
use tak_sim::testing::monarch_def;
use tak_sim::{NoScripts, StateHasher, World, systems};

const TICKS: u32 = 3000;
const CHILD_ENV: &str = "ECONOMY_CHILD";

fn scenario() -> Scenario {
    let mut king = monarch_def("king");
    king.economy = EconomyDef {
        mogriumincome: 10,
        mogriumstorage: 5000,
        maxmana: 1000,
        manarechargerate: 25,
        ..Default::default()
    };
    let mut lode = monarch_def("lode");
    lode.economy = EconomyDef {
        mogriumincome: 10,
        mogriumstorage: 1000,
        sacred_site: true,
        ..Default::default()
    };
    let setup = EconomySetup {
        rules: EconomyRules {
            start_mana: 300,
            ..Default::default()
        },
        sacred_sites: (0..8)
            .map(|i| SacredSite {
                x: 200 + 150 * i,
                y: 400,
                w: 32,
                h: 32,
                multiplier: Fx::from_ratio(2 + (i % 3), 2).unwrap(),
            })
            .collect(),
    };
    Scenario {
        version: SCENARIO_FORMAT_VERSION,
        initial_units: None,
        seed: 11,
        map: MapInfo::flat(
            2048,
            2048,
            vec![StartPos { x: 100, y: 100 }, StartPos { x: 1900, y: 1900 }],
        ),
        unit_defs: vec![king, lode],
        players: (0..2)
            .map(|i| PlayerSpec {
                start: i,
                monarch: UnitDefId(0),
            })
            .collect(),
        economy: Box::new(setup),
        features: Default::default(),
        victory: Default::default(),
    }
}

fn run() -> Vec<u64> {
    let mut w = World::new(&scenario()).unwrap();
    systems::spawn_start_units(&mut w);
    let mut lodes = Vec::new();
    let mut out = Vec::with_capacity(TICKS as usize);
    for t in 0..TICKS {
        // A lodestone every 200 ticks, alternating owners, onto the sites
        // (some on top of each other); one dies every 700 ticks.
        if t % 200 == 100 {
            let i = (t / 200) as i32;
            let owner = PlayerId((i % 2) as u8);
            let at = Vec2::from_ints(210 + 150 * (i % 8), 410);
            lodes.extend(w.spawn(UnitDefId(1), owner, at, Angle::ZERO));
        }
        if t % 700 == 699 && !lodes.is_empty() {
            let id = lodes.remove(0);
            w.despawn(id);
        }
        // Three "builders" per player with uneven appetites.
        for p in 0..2u8 {
            for k in 1..=3 {
                let want = Fx::from_ratio(k * (1 + (t as i32 % 5)), 7).unwrap();
                economy::drain(&mut w, PlayerId(p), want);
            }
        }
        // Monarchs cast a 200-mana spell whenever they can.
        for p in 0..2 {
            if let Some(king) = w.players()[p].monarch {
                economy::spend_unit_mana(&mut w, king, Fx::from_int(200));
            }
        }
        if t % 500 == 0 {
            economy::credit(&mut w, PlayerId(1), Fx::from_int(250));
        }
        economy::tick(&mut w, &mut NoScripts);
        let mut h = StateHasher::new();
        w.hash_into(&mut h);
        out.push(h.finish());
    }
    out
}

/// In a child process: print the hashes. Otherwise does nothing.
#[test]
fn economy_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    for h in run() {
        println!("HASH {h:016x}");
    }
}

fn child() -> Vec<u64> {
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe)
        .args([
            "--exact",
            "economy_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .expect("spawn child");
    assert!(out.status.success(), "child failed");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        // libtest may print "test economy_child ... " in front of the
        // first line.
        .filter_map(|l| l.find("HASH ").map(|i| &l[i + 5..]))
        .map(|h| u64::from_str_radix(h, 16).unwrap())
        .collect()
}

#[test]
fn economy_is_deterministic_across_processes() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    let here = run();
    let a = child();
    let b = child();
    assert_eq!(a.len(), TICKS as usize);
    assert_eq!(here, a);
    assert_eq!(a, b);
    let mut distinct = here.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(
        distinct.len() > TICKS as usize / 2,
        "state changes over time"
    );
}

#[test]
fn economy_state_is_in_the_hash() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    let sc = scenario();
    let mut a = World::new(&sc).unwrap();
    let mut b = World::new(&sc).unwrap();
    systems::spawn_start_units(&mut a);
    systems::spawn_start_units(&mut b);
    economy::tick(&mut a, &mut NoScripts);
    economy::tick(&mut b, &mut NoScripts);
    let hash = |w: &World| {
        let mut h = StateHasher::new();
        w.hash_into(&mut h);
        h.finish()
    };
    assert_eq!(hash(&a), hash(&b));
    economy::drain(&mut b, PlayerId(0), Fx::ONE);
    assert_ne!(hash(&a), hash(&b), "player mana is hashed");
    let mut c = a.clone();
    let king = c.players()[0].monarch.unwrap();
    economy::set_unit_mana_percent(&mut c, king, 10);
    assert_ne!(hash(&a), hash(&c), "unit mana is hashed");
}
