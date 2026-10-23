//! Synthetic combat fixtures shared by the `combat_*` tests (no game data).

#![allow(dead_code)]

use tak_commands::{
    CommandLog, PlayerId, QueueMode, SimCommand, Target, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Angle, Fx, Rng, Vec2};
use tak_sim::combat::{CombatDef, DamageStats, WeaponKind, WeaponStats, kind};
use tak_sim::testing::monarch_def;
use tak_sim::{MapInfo, PlayerSpec, SCENARIO_FORMAT_VERSION, Scenario, Sim, StartPos, UnitDef};

pub const DUMMY: UnitDefId = UnitDefId(0);
pub const SWORD: UnitDefId = UnitDefId(1);
pub const ARCHER: UnitDefId = UnitDefId(2);
pub const DUMMY_TARGET: UnitDefId = UnitDefId(3);

pub fn melee(default: i32, reload_ticks: u32) -> WeaponStats {
    WeaponStats {
        name: "Sword".into(),
        kind: WeaponKind::Melee,
        range: 300,
        min_range: 0,
        reload_ticks,
        velocity: 0,
        area_of_effect: 0,
        edge_effectiveness: Fx::ZERO,
        aim_tolerance: None,
        mana_per_shot: 0,
        damage_kind: kind::NORMAL,
        damage: DamageStats {
            default,
            multipliers: vec![
                ("human".into(), Fx::from_ratio(3, 2).unwrap()),
                ("monster".into(), Fx::HALF),
            ],
        },
    }
}

pub fn ballistic(default: i32, range: i32, velocity: i32, aoe: i32) -> WeaponStats {
    WeaponStats {
        name: "Arrow".into(),
        kind: WeaponKind::Ballistic,
        range,
        min_range: 0,
        reload_ticks: 45,
        velocity,
        area_of_effect: aoe,
        edge_effectiveness: Fx::HALF,
        aim_tolerance: None,
        mana_per_shot: 0,
        damage_kind: kind::NORMAL,
        damage: DamageStats {
            default,
            multipliers: vec![],
        },
    }
}

pub fn fighter(name: &str, hp: i32, category: &str, weapons: Vec<WeaponStats>) -> UnitDef {
    let mut d = monarch_def(name);
    d.footprintx = 1;
    d.footprintz = 1;
    d.maxvelocity = Fx::ONE;
    d.acceleration = Fx::ONE;
    d.brakerate = Fx::ONE;
    d.turnrate = 2000;
    d.combat = CombatDef {
        max_damage: hp,
        heal_rate: Fx::ZERO,
        sight: 200,
        leash: 400,
        damage_category: Some(category.into()),
        radius: 8,
        weapons,
        explode_as: None,
    };
    d
}

/// Dummy monarchs (outside combat) far away in the corners, plus the
/// fighter definitions.
pub fn scenario(seed: u64) -> Scenario {
    let mut dummy = monarch_def("dummy");
    dummy.combat = CombatDef::default();
    let mut target = fighter("target", 1000, "human", vec![]);
    target.maxvelocity = Fx::ZERO;
    Scenario {
        version: SCENARIO_FORMAT_VERSION,
        initial_units: None,
        seed,
        economy: Default::default(),
        features: Default::default(),
        victory: Default::default(),
        map: MapInfo::flat(
            2048,
            2048,
            vec![StartPos { x: 16, y: 16 }, StartPos { x: 2032, y: 2032 }],
        ),
        unit_defs: vec![
            dummy,
            fighter("sword", 2500, "human", vec![melee(360, 51)]),
            fighter("archer", 1100, "human", vec![ballistic(213, 450, 750, 0)]),
            target,
        ],
        players: vec![
            PlayerSpec {
                start: 0,
                monarch: DUMMY,
            },
            PlayerSpec {
                start: 1,
                monarch: DUMMY,
            },
        ],
    }
}

pub fn sim(sc: &Scenario) -> Sim {
    let mut s = Sim::new(sc).unwrap();
    s.step(&[]);
    s
}

pub fn place(s: &mut Sim, def: UnitDefId, player: u8, x: i32, y: i32) -> UnitId {
    s.spawn_unit(def, PlayerId(player), Vec2::from_ints(x, y), Angle::ZERO)
        .expect("spawn")
}

pub fn health(s: &Sim, id: UnitId) -> Option<i32> {
    s.world().unit(id).map(|u| u.combat.health)
}

pub fn attack(units: &[UnitId], target: Target) -> SimCommand {
    SimCommand::Attack {
        units: UnitSet::from_iter(units.iter().copied()),
        target,
        queue: QueueMode::Replace,
    }
}

/// A 4-v-4 melee + ranged brawl with a few random attack orders, for the
/// determinism tests. Returns the per-tick hashes.
pub fn brawl_hashes(seed: u64, ticks: u32) -> Vec<u64> {
    let sc = scenario(seed);
    let mut s = sim(&sc);
    let mut units = Vec::new();
    for i in 0..4 {
        units.push((
            place(
                &mut s,
                if i % 2 == 0 { SWORD } else { ARCHER },
                0,
                900 + 20 * i,
                900,
            ),
            0u8,
        ));
        units.push((
            place(
                &mut s,
                if i % 2 == 0 { ARCHER } else { SWORD },
                1,
                900 + 20 * i,
                1100,
            ),
            1u8,
        ));
    }
    let mut rng = Rng::new(seed ^ 0xA77AC);
    let mut log = CommandLog::new();
    for t in (5..ticks).step_by(97) {
        let (a, p) = units[rng.below(units.len() as u32) as usize];
        let (b, _) = units[rng.below(units.len() as u32) as usize];
        log.record(Tick(t), PlayerId(p), attack(&[a], Target::Unit(b)));
    }
    let mut out = Vec::new();
    for _ in 1..ticks {
        let t = s.tick();
        out.push(s.step(log.for_tick(t)));
    }
    out
}

// ---------------------------------------------------------------------------
// Real data (callers gate on `TAK_DATA_DIR`).

pub const MAP: &str = "kmap/Adamantine Gate";

/// A real-map match (ARA vs ZON monarchs) with extra unit types appended to
/// the scenario, scripts loaded for all of them. Returns the sim (tick 0
/// already run), the def ids of `extra` in order, and the map.
pub fn real_match(
    dir: &std::path::Path,
    extra: &[&str],
    seed: u64,
) -> (Sim, Vec<UnitDefId>, MapInfo) {
    let vfs = tak_vfs::open_kingdoms(dir).expect("open install");
    let rs = tak_content::Ruleset::load(&vfs, tak_content::RulesetKind::Base);
    let map = tak_content::MapDef::load(&vfs, &rs, MAP).expect("map");
    let mut sc = Scenario::from_content(&map, &rs, &tak_sim::PlayerSetup::parse_list("ARA,ZON"))
        .expect("scenario");
    sc.seed = seed;
    let mut ids = Vec::new();
    for name in extra {
        let fbi = rs.unit(name).unwrap_or_else(|| panic!("no unit {name}"));
        ids.push(UnitDefId(sc.unit_defs.len() as u32));
        sc.unit_defs.push(tak_sim::content::unit_def_from_fbi(fbi));
    }
    let scripts = tak_sim::content::load_scripts(&vfs, &rs, &sc);
    let mut s = Sim::with_scripts(&sc, Box::new(scripts)).expect("sim");
    s.step(&[]);
    (s, ids, sc.map)
}

/// Top-left of the open rectangle (`w` × `h` world units, all cells
/// passable) nearest the map centre that is at least `away` from every
/// monarch. Deterministic scan on a 32-unit grid.
pub fn open_area(s: &Sim, map: &MapInfo, w: i32, h: i32, away: i32) -> Vec2 {
    let c = map.center();
    let monarchs: Vec<Vec2> = s.world().units().map(|(_, u)| u.pos).collect();
    let mut best: Option<(Fx, Vec2)> = None;
    let mut y = 64;
    while y + h < map.height - 64 {
        let mut x = 64;
        while x + w < map.width - 64 {
            let corner = Vec2::from_ints(x, y);
            let mid = Vec2::from_ints(x + w / 2, y + h / 2);
            let far = monarchs
                .iter()
                .all(|m| m.distance(mid) > Fx::from_int(away));
            let d = mid.distance(c);
            if far && best.is_none_or(|(bd, _)| d < bd) {
                let open = (0..=h / 8).all(|j| {
                    (0..=w / 8).all(|i| {
                        map.terrain_at(Vec2::from_ints(x + 8 * i, y + 8 * j))
                            & tak_sim::map::terrain::BLOCKS
                            == 0
                    })
                });
                if open {
                    best = Some((d, corner));
                }
            }
            x += 32;
        }
        y += 32;
    }
    best.expect("open area").1
}

/// Order every idle unit of `mine` to attack the nearest live unit of
/// `theirs` (a test-level AI).
pub fn order_idle(s: &Sim, mine: &[UnitId], theirs: &[UnitId]) -> Vec<SimCommand> {
    let w = s.world();
    let mut out = Vec::new();
    for &m in mine {
        let Some(u) = w.unit(m) else { continue };
        if u.combat.is_dying() || u.combat.order.is_some() || !u.orders.is_empty() {
            continue;
        }
        let target = theirs
            .iter()
            .filter_map(|&t| {
                w.unit(t)
                    .filter(|tu| !tu.combat.is_dying())
                    .map(|tu| (u.pos.distance(tu.pos), t))
            })
            .min_by_key(|(d, _)| *d);
        if let Some((_, t)) = target {
            out.push(attack(&[m], Target::Unit(t)));
        }
    }
    out
}

/// Result of [`skirmish`].
pub struct Skirmish {
    pub hashes: Vec<u64>,
    pub ticks: u32,
    pub alive: [usize; 2],
    /// Health left on each side (sum over survivors) and at the start.
    pub health: [i32; 2],
    pub start_health: [i32; 2],
    pub stats: tak_sim::ScriptStats,
    pub deaths: u64,
}

/// `n` of `a` (player 0) against `n` of `b` (player 1) on Adamantine Gate,
/// 160 units apart, re-ordering idle units every 30 ticks, until one side
/// is wiped out or `max_ticks` pass.
pub fn skirmish(dir: &std::path::Path, a: &str, b: &str, n: usize, max_ticks: u32) -> Skirmish {
    let (mut s, defs, map) = real_match(dir, &[a, b], 11);
    let width = 32 * n as i32;
    let origin = open_area(&s, &map, width + 64, 224, 700);
    let mut sides: [Vec<UnitId>; 2] = [Vec::new(), Vec::new()];
    let (ox, oy) = (origin.x.to_int_floor(), origin.y.to_int_floor());
    for i in 0..n as i32 {
        let x = ox + 32 + 32 * i;
        let p0 = Vec2::from_ints(x, oy + 32);
        let p1 = Vec2::from_ints(x, oy + 192);
        sides[0].push(
            s.spawn_unit(defs[0], PlayerId(0), p0, Angle::QUARTER)
                .unwrap(),
        );
        sides[1].push(
            s.spawn_unit(defs[1], PlayerId(1), p1, Angle::THREE_QUARTERS)
                .unwrap(),
        );
    }
    let alive = |s: &Sim, v: &[UnitId]| v.iter().filter(|&&u| s.world().unit(u).is_some()).count();
    let hp = |s: &Sim, v: &[UnitId]| -> i32 {
        v.iter()
            .filter_map(|&u| s.world().unit(u))
            .map(|u| {
                if u.combat.ready {
                    u.combat.health.max(0)
                } else {
                    0
                }
            })
            .sum()
    };
    let max = |s: &Sim, v: &[UnitId]| -> i32 {
        v.iter()
            .filter_map(|&u| s.world().unit(u))
            .filter_map(|u| s.world().defs().get(u.def))
            .map(|d| d.combat.max_damage)
            .sum()
    };
    let start_health = [max(&s, &sides[0]), max(&s, &sides[1])];
    let mut hashes = Vec::new();
    let mut ticks = 0;
    while ticks < max_ticks {
        let t = s.tick();
        let mut cmds = Vec::new();
        if t.0 % 30 == 1 {
            for (p, mine, theirs) in [(0u8, &sides[0], &sides[1]), (1u8, &sides[1], &sides[0])] {
                for (seq, c) in order_idle(&s, mine, theirs).into_iter().enumerate() {
                    cmds.push(tak_commands::LoggedCommand {
                        tick: t,
                        player: PlayerId(p),
                        seq: seq as u32,
                        command: c,
                    });
                }
            }
        }
        hashes.push(s.step(&cmds));
        ticks += 1;
        if alive(&s, &sides[0]) == 0 || alive(&s, &sides[1]) == 0 {
            break;
        }
    }
    Skirmish {
        hashes,
        ticks,
        alive: [alive(&s, &sides[0]), alive(&s, &sides[1])],
        health: [hp(&s, &sides[0]), hp(&s, &sides[1])],
        start_health,
        stats: s.script_stats(),
        deaths: s.world().combat.deaths,
    }
}
