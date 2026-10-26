//! Victory conditions (WP-U) on synthetic scenarios: skirmish defeat rules,
//! teams, the defeat sweep, dropped commands, mission conditions, and
//! cross-process determinism.

mod combat_common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use combat_common::{fighter, melee};
use tak_commands::{
    LoggedCommand, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Angle, Vec2};
use tak_sim::combat::kind;
use tak_sim::victory::{Condition, MissionRules};
use tak_sim::{
    DefeatRule, DefeatedUnits, MapInfo, MatchState, NoScripts, PlayerSpec, SCENARIO_FORMAT_VERSION,
    Scenario, Sim, StartPos, VictorySetup,
};

const KING: UnitDefId = UnitDefId(0);
const SOLDIER: UnitDefId = UnitDefId(1);
const TOWER: UnitDefId = UnitDefId(2);

/// `players` monarchs (combat units, unarmed) far apart on a flat map, a
/// soldier type and an immobile tower type.
fn scenario(players: usize, victory: VictorySetup) -> Scenario {
    let mut king = fighter("king", 500, "human", vec![]);
    king.combat.sight = 0;
    let soldier = fighter("soldier", 100, "human", vec![]);
    let mut tower = fighter("tower", 100, "human", vec![]);
    tower.maxvelocity = tak_math::Fx::ZERO;
    let starts = vec![
        StartPos { x: 100, y: 100 },
        StartPos { x: 1900, y: 1900 },
        StartPos { x: 1900, y: 100 },
        StartPos { x: 100, y: 1900 },
    ];
    Scenario {
        version: SCENARIO_FORMAT_VERSION,
        initial_units: None,
        seed: 3,
        map: MapInfo::flat(2048, 2048, starts),
        unit_defs: vec![king, soldier, tower],
        players: (0..players)
            .map(|i| PlayerSpec {
                start: i as u32,
                monarch: KING,
            })
            .collect(),
        economy: Default::default(),
        features: Default::default(),
        victory: Box::new(victory),
    }
}

fn start(sc: &Scenario) -> Sim {
    let mut s = Sim::new(sc).unwrap();
    s.step(&[]);
    s
}

fn monarch(s: &Sim, p: u8) -> UnitId {
    s.world().players()[p as usize].monarch.unwrap()
}

fn place(s: &mut Sim, def: UnitDefId, p: u8, x: i32, y: i32) -> UnitId {
    s.spawn_unit(def, PlayerId(p), Vec2::from_ints(x, y), Angle::ZERO)
        .unwrap()
}

/// Kill outright through the ordinary damage path (between ticks).
fn kill(s: &mut Sim, id: UnitId) {
    let now = s.tick().0;
    let pos = s.world().unit(id).unwrap().pos;
    tak_sim::combat::apply_damage(
        s.world_mut(),
        &mut NoScripts,
        id,
        1_000_000,
        None,
        kind::NORMAL,
        pos,
        now,
    );
}

fn kill_monarch(s: &mut Sim, p: u8) {
    let id = monarch(s, p);
    kill(s, id);
}

fn steps(s: &mut Sim, n: u32) {
    for _ in 0..n {
        s.step(&[]);
    }
}

fn over(winners: &[u8]) -> MatchState {
    MatchState::Over {
        winners: winners.iter().map(|&p| PlayerId(p)).collect(),
    }
}

#[test]
fn victory_monarch_killed_ends_the_match_and_sweeps_the_army() {
    let mut s = start(&scenario(2, VictorySetup::default()));
    let soldier = place(&mut s, SOLDIER, 1, 1800, 1800);
    let mine = place(&mut s, SOLDIER, 0, 200, 200);
    steps(&mut s, 2);
    assert_eq!(*s.match_state(), MatchState::Running);
    kill_monarch(&mut s, 1);
    let decided = s.tick();
    s.step(&[]);
    assert!(s.world().is_defeated(PlayerId(1)));
    assert!(!s.world().is_defeated(PlayerId(0)));
    assert_eq!(*s.match_state(), over(&[0]));
    assert_eq!(s.world().victory().decided_at(), Some(decided));
    // The defeated army self-destructs; the winner's does not.
    assert!(s.world().unit(soldier).unwrap().combat.is_dying());
    steps(&mut s, 2);
    assert!(s.world().unit(soldier).is_none());
    assert!(s.world().unit(mine).is_some());
    // The state never changes again.
    steps(&mut s, 30);
    assert_eq!(*s.match_state(), over(&[0]));
}

#[test]
fn victory_defeated_units_remain_and_take_no_orders() {
    let setup = VictorySetup {
        defeated_units: DefeatedUnits::Remain,
        ..Default::default()
    };
    let mut s = start(&scenario(2, setup));
    let soldier = place(&mut s, SOLDIER, 1, 1800, 1800);
    s.step(&[]);
    kill_monarch(&mut s, 1);
    s.step(&[]);
    assert!(s.world().is_defeated(PlayerId(1)));
    let t = s.tick();
    let cmd = LoggedCommand {
        tick: t,
        player: PlayerId(1),
        seq: 0,
        command: SimCommand::Move {
            units: UnitSet::single(soldier),
            to: Vec2::from_ints(1000, 1000),
            queue: QueueMode::Replace,
        },
    };
    s.step(&[cmd]);
    steps(&mut s, 5);
    let u = s.world().unit(soldier).expect("still alive");
    assert!(!u.combat.is_dying());
    assert!(
        u.orders.is_empty(),
        "command from a defeated player applied"
    );
    assert_eq!(u.pos, Vec2::from_ints(1800, 1800));
}

#[test]
fn victory_all_units_killed_rule_keeps_playing_without_a_monarch() {
    let setup = VictorySetup {
        defeat: DefeatRule::AllUnitsKilled,
        ..Default::default()
    };
    let mut s = start(&scenario(2, setup));
    let soldier = place(&mut s, SOLDIER, 1, 1800, 1800);
    s.step(&[]);
    kill_monarch(&mut s, 1);
    steps(&mut s, 5);
    assert!(!s.world().is_defeated(PlayerId(1)));
    assert_eq!(*s.match_state(), MatchState::Running);
    assert!(!s.world().unit(soldier).unwrap().combat.is_dying());
    kill(&mut s, soldier);
    s.step(&[]); // dies and (no death animation) is removed
    assert!(s.world().is_defeated(PlayerId(1)));
    assert_eq!(*s.match_state(), over(&[0]));
}

#[test]
fn victory_free_for_all_needs_one_survivor() {
    let mut s = start(&scenario(3, VictorySetup::default()));
    kill_monarch(&mut s, 2);
    steps(&mut s, 3);
    assert!(s.world().is_defeated(PlayerId(2)));
    assert_eq!(*s.match_state(), MatchState::Running);
    kill_monarch(&mut s, 0);
    steps(&mut s, 3);
    assert_eq!(*s.match_state(), over(&[1]));
}

#[test]
fn victory_teams_win_together() {
    let setup = VictorySetup {
        teams: vec![Some(1), Some(1), None],
        ..Default::default()
    };
    let mut s = start(&scenario(3, setup.clone()));
    kill_monarch(&mut s, 1);
    steps(&mut s, 3);
    assert_eq!(
        *s.match_state(),
        MatchState::Running,
        "teams 1 and solo left"
    );
    kill_monarch(&mut s, 2);
    steps(&mut s, 3);
    // Winners are the surviving members of the last team.
    assert_eq!(*s.match_state(), over(&[0]));

    let mut s = start(&scenario(3, setup));
    kill_monarch(&mut s, 2);
    steps(&mut s, 3);
    assert_eq!(*s.match_state(), over(&[0, 1]));
}

#[test]
fn victory_draw_and_single_team_matches() {
    let mut s = start(&scenario(2, VictorySetup::default()));
    kill_monarch(&mut s, 0);
    kill_monarch(&mut s, 1);
    s.step(&[]);
    assert_eq!(*s.match_state(), over(&[]));

    // One player (or one team) never ends by elimination.
    let mut s = start(&scenario(1, VictorySetup::default()));
    steps(&mut s, 10);
    assert_eq!(*s.match_state(), MatchState::Running);
    let setup = VictorySetup {
        teams: vec![Some(2), Some(2)],
        ..Default::default()
    };
    let mut s = start(&scenario(2, setup));
    kill_monarch(&mut s, 1);
    steps(&mut s, 3);
    assert!(s.world().is_defeated(PlayerId(1)));
    assert_eq!(*s.match_state(), MatchState::Running);
}

#[test]
fn victory_setup_json_round_trip_and_validation() {
    let mut sc = scenario(2, VictorySetup::default());
    let json = sc.to_json_pretty();
    assert!(!json.contains("victory"), "default setup stays out of JSON");
    sc.victory.defeat = DefeatRule::AllUnitsKilled;
    sc.victory.teams = vec![Some(1)];
    let back = Scenario::from_json(&sc.to_json_pretty()).unwrap();
    assert_eq!(back, sc);
    sc.victory.teams = vec![None, None, None];
    assert!(sc.validate().is_err());
}

// ---- missions ----

fn conds(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn mission(pairs: &[(&str, &str)]) -> (Scenario, MissionRules) {
    let mut sc = scenario(2, VictorySetup::default());
    let rules = MissionRules::from_conditions(&conds(pairs), &sc.unit_defs);
    sc.victory.mission = Some(rules.clone());
    (sc, rules)
}

#[test]
fn victory_parses_mission_conditions() {
    let (sc, m) = mission(&[
        ("moveunittoradius", "SOLDIER, 130, 76, 15"),
        ("killunittype", "tower, 17"),
        ("killalloftype", "NPCALCH"),
        ("allunitskilled", "1"),
        ("destroyallunits", "0"),
        ("unittypepassesz", "ANYTYPE, 321"),
        ("deathtimerrunsout", "1800"),
        ("useonlyunits", "ARASWORD, ARAARCH"),
        ("killenemycommander", "x"),
    ]);
    let soldier = Some(tak_sim::victory::UnitType {
        name: "SOLDIER".into(),
        def: Some(SOLDIER),
    });
    assert!(m.victory.contains(&Condition::MoveUnitToRadius {
        ty: soldier,
        x: 130,
        z: 76,
        r: 15
    }));
    assert!(m.victory.iter().any(|c| matches!(c,
        Condition::KillUnitType(t, 17) if t.def == Some(TOWER))));
    assert!(m.victory.iter().any(|c| matches!(c,
        Condition::KillAllOfType(t) if t.def.is_none() && t.name == "NPCALCH")));
    assert!(m.victory.contains(&Condition::UnitTypePassesZ(None, 321)));
    assert!(!m.victory.contains(&Condition::DestroyAllUnits), "0 is off");
    assert!(m.defeat.contains(&Condition::AllUnitsKilled));
    assert!(m.defeat.contains(&Condition::DeathTimerRunsOut(1800)));
    assert_eq!(m.unsupported.len(), 2, "{:?}", m.unsupported);
    sc.validate().unwrap();
}

#[test]
fn victory_mission_defaults_destroy_all_and_all_killed() {
    let (sc, _) = mission(&[]);
    let mut s = start(&sc);
    let tower = place(&mut s, TOWER, 1, 1800, 1700);
    s.step(&[]);
    kill_monarch(&mut s, 1);
    steps(&mut s, 3);
    assert_eq!(*s.match_state(), MatchState::Running, "enemy tower left");
    kill(&mut s, tower);
    steps(&mut s, 3);
    assert_eq!(*s.match_state(), over(&[0]));
    assert!(s.world().is_defeated(PlayerId(1)));

    let (sc, _) = mission(&[]);
    let mut s = start(&sc);
    kill_monarch(&mut s, 0);
    s.step(&[]);
    assert_eq!(
        *s.match_state(),
        over(&[1]),
        "a dying unit is no longer live"
    );
}

#[test]
fn victory_mission_commander_conditions() {
    let (sc, _) = mission(&[("killenemycommander", "1"), ("commanderkilled", "1")]);
    let mut s = start(&sc);
    place(&mut s, SOLDIER, 1, 1800, 1700);
    kill_monarch(&mut s, 1);
    s.step(&[]);
    assert_eq!(
        *s.match_state(),
        over(&[0]),
        "enemy soldier left, commander gone"
    );

    let (sc, _) = mission(&[("killenemycommander", "1"), ("commanderkilled", "1")]);
    let mut s = start(&sc);
    kill_monarch(&mut s, 0);
    steps(&mut s, 3);
    assert_eq!(*s.match_state(), over(&[1]));
}

#[test]
fn victory_mission_kill_counts_and_types() {
    let (sc, _) = mission(&[("killunittype", "SOLDIER, 2"), ("killalloftype", "TOWER")]);
    let mut s = start(&sc);
    let a = place(&mut s, SOLDIER, 1, 1800, 1700);
    let b = place(&mut s, SOLDIER, 1, 1700, 1700);
    let t = place(&mut s, TOWER, 1, 1600, 1700);
    place(&mut s, SOLDIER, 0, 200, 200);
    s.step(&[]);
    kill(&mut s, a);
    kill(&mut s, t);
    steps(&mut s, 3);
    assert_eq!(*s.match_state(), MatchState::Running, "one soldier to go");
    kill(&mut s, b);
    steps(&mut s, 3);
    assert_eq!(*s.match_state(), over(&[0]));

    // Escort: losing every unit of a type (either side) loses.
    let (sc, _) = mission(&[
        ("allunitskilledoftype", "TOWER"),
        ("unittypekilled", "SOLDIER, 1"),
    ]);
    let mut s = start(&sc);
    let t = place(&mut s, TOWER, 0, 200, 300);
    s.step(&[]);
    kill(&mut s, t);
    steps(&mut s, 3);
    assert_eq!(*s.match_state(), over(&[1]));

    let (sc, _) = mission(&[("unittypekilled", "SOLDIER, 1")]);
    let mut s = start(&sc);
    let e = place(&mut s, SOLDIER, 1, 1800, 1700);
    s.step(&[]);
    kill(&mut s, e);
    steps(&mut s, 3);
    assert_eq!(*s.match_state(), over(&[1]), "any owner's loss counts");
}

#[test]
fn victory_mission_reach_radius_and_lines() {
    let (sc, _) = mission(&[("moveunittoradius", "SOLDIER, 40, 12, 3")]);
    let mut s = start(&sc);
    let u = place(&mut s, SOLDIER, 0, 500, 200);
    steps(&mut s, 2);
    assert_eq!(*s.match_state(), MatchState::Running);
    // Centre cell (40, 12) = world (648, 200); radius 48.
    let t = s.tick();
    let go = LoggedCommand {
        tick: t,
        player: PlayerId(0),
        seq: 0,
        command: SimCommand::Move {
            units: UnitSet::single(u),
            to: Vec2::from_ints(700, 200),
            queue: QueueMode::Replace,
        },
    };
    s.step(&[go]);
    steps(&mut s, 250);
    assert_eq!(*s.match_state(), over(&[0]));

    let (sc, _) = mission(&[("unittypepassesx", "ANYTYPE, 50")]);
    let mut s = start(&sc);
    let u = place(&mut s, SOLDIER, 0, 600, 200);
    let t = s.tick();
    let go = LoggedCommand {
        tick: t,
        player: PlayerId(0),
        seq: 0,
        command: SimCommand::Move {
            units: UnitSet::single(u),
            to: Vec2::from_ints(1000, 200),
            queue: QueueMode::Replace,
        },
    };
    s.step(&[go]);
    let mut decided = None;
    for _ in 0..400 {
        s.step(&[]);
        if *s.match_state() != MatchState::Running {
            decided = Some(s.world().unit(u).unwrap().pos.x.to_int_floor());
            break;
        }
    }
    // Cell 50 ± 2 cells: x in [768, 848).
    let x = decided.expect("line reached");
    assert!((768..848).contains(&x), "decided at x = {x}");
}

#[test]
fn victory_mission_timers() {
    let (sc, _) = mission(&[("victorytimerrunsout", "2"), ("allunitskilled", "1")]);
    let mut s = start(&sc);
    steps(&mut s, 58);
    assert_eq!(*s.match_state(), MatchState::Running);
    s.step(&[]); // tick 59
    s.step(&[]); // tick 60
    assert_eq!(*s.match_state(), over(&[0]));
    assert_eq!(s.world().victory().decided_at(), Some(Tick(60)));

    let (sc, _) = mission(&[("deathtimerrunsout", "1"), ("killenemycommander", "1")]);
    let mut s = start(&sc);
    steps(&mut s, 40);
    assert_eq!(*s.match_state(), over(&[1]));
    assert_eq!(s.world().victory().decided_at(), Some(Tick(30)));
}

// ---- determinism ----

const CHILD_ENV: &str = "OXA_VICTORY_CHILD_OUT";

/// Two armed monarchs duel next to each other (monarch rule, sweep on), and
/// a mission run with a kill counter; the hashes of both.
fn duel_hashes() -> String {
    let mut sc = scenario(2, VictorySetup::default());
    sc.map.start_positions[1] = StartPos { x: 160, y: 100 };
    sc.unit_defs[0] = fighter("king", 900, "human", vec![melee(40, 30)]);
    sc.unit_defs[1] = fighter("soldier", 300, "human", vec![melee(25, 25)]);
    let mut s = Sim::new(&sc).unwrap();
    let mut out = String::new();
    for t in 0..1500u32 {
        if t == 1 {
            s.spawn_unit(SOLDIER, PlayerId(0), Vec2::from_ints(90, 160), Angle::ZERO);
            s.spawn_unit(SOLDIER, PlayerId(1), Vec2::from_ints(170, 160), Angle::ZERO);
            s.spawn_unit(SOLDIER, PlayerId(1), Vec2::from_ints(400, 400), Angle::ZERO);
        }
        out.push_str(&format!("{:016x}\n", s.step(&[])));
    }
    assert_ne!(*s.match_state(), MatchState::Running, "the duel ends");
    out.push_str(&format!("{}\n", s.world().victory().summary()));
    let (mut sc, _) = mission(&[("killunittype", "SOLDIER, 1")]);
    sc.map.start_positions[1] = StartPos { x: 160, y: 100 };
    sc.unit_defs[0] = fighter("king", 900, "human", vec![melee(40, 30)]);
    let mut s = Sim::new(&sc).unwrap();
    for t in 0..600u32 {
        if t == 1 {
            s.spawn_unit(SOLDIER, PlayerId(1), Vec2::from_ints(120, 120), Angle::ZERO);
        }
        out.push_str(&format!("{:016x}\n", s.step(&[])));
    }
    out.push_str(&format!("{}\n", s.world().victory().summary()));
    out.push_str(&defeated_survivor_hashes());
    out
}

#[test]
fn victory_child_writes_hashes() {
    let Ok(out) = std::env::var(CHILD_ENV) else {
        return;
    };
    std::fs::write(out, duel_hashes()).expect("write hashes");
}

#[test]
fn victory_hashes_agree_across_processes() {
    let dir = std::env::temp_dir().join(format!("oxa-victory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out: PathBuf = dir.join("child.txt");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["victory_child_writes_hashes", "--exact", "--test-threads=1"])
        .env(CHILD_ENV, &out)
        .status()
        .expect("spawn child");
    assert!(status.success());
    let child = std::fs::read_to_string(&out).expect("child output");
    let _ = std::fs::remove_dir_all(&dir);
    let mine = duel_hashes();
    eprintln!(
        "{}",
        mine.lines()
            .filter(|l| l.contains("over"))
            .collect::<Vec<_>>()
            .join("; ")
    );
    assert!(mine.contains("over at tick"));
    assert_eq!(child, mine, "child process diverged");
}

fn defeated_survivor_hashes() -> String {
    let mut sc = scenario(3, VictorySetup::default());
    sc.unit_defs[SOLDIER.0 as usize].combat.max_damage = 50_000;
    sc.unit_defs[SOLDIER.0 as usize].combat.weapons = vec![combat_common::ballistic(10, 200, 0, 0)];
    sc.unit_defs[TOWER.0 as usize].combat.max_damage = 50_000;
    sc.unit_defs[TOWER.0 as usize].build = tak_sim::construction::BuildDef {
        builder: true,
        mobile: false,
        workertime: 30,
        buildcost: 10,
        buildtime: 60 << 16,
        canbuild: vec![SOLDIER],
        ..Default::default()
    };
    sc.unit_defs[SOLDIER.0 as usize].build.buildcost = 10;
    sc.unit_defs[SOLDIER.0 as usize].build.buildtime = 60 << 16;
    sc.unit_defs[KING.0 as usize].economy.mogriumstorage = 1000;
    sc.unit_defs[KING.0 as usize].economy.mogriumincome = 100;
    let mut s = start(&sc);
    let mover = place(&mut s, SOLDIER, 0, 300, 300);
    let gun = place(&mut s, SOLDIER, 0, 400, 800);
    let factory = place(&mut s, TOWER, 0, 800, 800);
    let target = place(&mut s, TOWER, 1, 450, 800);
    let mut log = tak_commands::CommandLog::new();
    log.record(
        s.tick(),
        PlayerId(0),
        SimCommand::SetFireState {
            units: UnitSet::single(mover),
            state: tak_commands::FireState::HoldFire,
        },
    );
    log.record(
        s.tick(),
        PlayerId(0),
        SimCommand::Move {
            units: UnitSet::single(mover),
            to: Vec2::from_ints(700, 300),
            queue: QueueMode::Replace,
        },
    );
    log.record(
        s.tick(),
        PlayerId(0),
        SimCommand::Build {
            units: UnitSet::single(factory),
            def: SOLDIER,
            at: None,
            queue: QueueMode::Replace,
        },
    );
    s.run(&log, 5);
    assert!(s.world().unit(mover).unwrap().speed > tak_math::Fx::ZERO);
    assert!(
        s.world().unit(gun).unwrap().combat.weapons[0]
            .target
            .is_some()
    );
    assert!(
        s.world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product
            .is_some()
    );
    kill_monarch(&mut s, 0);
    s.step(&[]);
    assert_eq!(
        *s.match_state(),
        MatchState::Running,
        "other players still compete"
    );
    let pos = s.world().unit(mover).unwrap().pos;
    let hp = s.world().unit(target).unwrap().combat.health;
    assert!(
        !s.world().unit(mover).unwrap().combat.is_dying(),
        "survived 30000 sweep"
    );
    let mut hashes = String::new();
    for _ in 0..300 {
        hashes.push_str(&format!("{:016x}\n", s.step(&[])));
    }
    assert_eq!(
        s.world().unit(mover).unwrap().pos,
        pos,
        "defeated survivor kept moving"
    );
    assert_eq!(
        s.world().unit(target).unwrap().combat.health,
        hp,
        "defeated survivor kept firing"
    );
    assert!(
        s.world()
            .unit(factory)
            .unwrap()
            .build
            .factory
            .product
            .is_none(),
        "defeated factory retained production"
    );
    assert!(s.world().unit(factory).unwrap().orders.is_empty());
    kill(&mut s, mover);
    s.step(&[]);
    assert!(
        s.world().unit(mover).is_none(),
        "inert units still process death"
    );
    hashes
}

#[test]
fn default_defeat_survivors_stop_moving_firing_and_producing() {
    defeated_survivor_hashes();
}

#[test]
fn explicit_remain_option_preserves_defensive_fire() {
    let mut sc = scenario(
        3,
        VictorySetup {
            defeated_units: DefeatedUnits::Remain,
            ..Default::default()
        },
    );
    sc.unit_defs[SOLDIER.0 as usize].combat.weapons = vec![combat_common::ballistic(10, 200, 0, 0)];
    sc.unit_defs[TOWER.0 as usize].combat.max_damage = 50_000;
    let mut s = start(&sc);
    let gun = place(&mut s, SOLDIER, 0, 400, 800);
    let target = place(&mut s, TOWER, 1, 450, 800);
    s.step(&[]);
    kill_monarch(&mut s, 0);
    s.step(&[]);
    assert!(s.world().is_defeated(PlayerId(0)));
    assert!(!s.world().is_inert_player(PlayerId(0)));
    let hp = s.world().unit(target).unwrap().combat.health;
    steps(&mut s, 100);
    assert!(s.world().unit(gun).is_some());
    assert!(s.world().unit(target).unwrap().combat.health < hp);
}

#[test]
fn outside_combat_survivors_are_also_inert_after_default_defeat() {
    let mut sc = scenario(3, VictorySetup::default());
    sc.unit_defs[SOLDIER.0 as usize].combat = Default::default();
    let mut s = start(&sc);
    let mover = place(&mut s, SOLDIER, 0, 300, 300);
    let mut log = tak_commands::CommandLog::new();
    log.record(
        s.tick(),
        PlayerId(0),
        SimCommand::Move {
            units: UnitSet::single(mover),
            to: Vec2::from_ints(700, 300),
            queue: QueueMode::Replace,
        },
    );
    s.run(&log, 5);
    kill_monarch(&mut s, 0);
    s.step(&[]);
    let pos = s.world().unit(mover).unwrap().pos;
    steps(&mut s, 100);
    assert_eq!(s.world().unit(mover).unwrap().pos, pos);
    assert!(s.world().unit(mover).unwrap().orders.is_empty());
}

#[path = "construction_support/mod.rs"]
mod work_callbacks;

#[test]
fn default_defeat_cancels_active_heal_and_pairs_its_work_callback() {
    let mut sc = scenario(3, VictorySetup::default());
    sc.unit_defs[KING.0 as usize].economy.mogriumstorage = 5000;
    let healer_def = &mut sc.unit_defs[SOLDIER.0 as usize];
    healer_def.combat.max_damage = 50_000;
    healer_def.build.builder = true;
    healer_def.build.mobile = true;
    healer_def.build.workertime = 30;
    healer_def.build.builddistance = 200;
    healer_def.economy.mogriumstorage = 5000;
    let target_def = &mut sc.unit_defs[TOWER.0 as usize];
    target_def.combat.max_damage = 100_000;
    target_def.build.buildcost = 100;
    target_def.build.buildtime = 60 << 16;
    let rec = work_callbacks::Recorder::default();
    let mut s = Sim::with_scripts(&sc, Box::new(rec.clone())).unwrap();
    s.step(&[]);
    let healer = place(&mut s, SOLDIER, 0, 300, 300);
    let target = place(&mut s, TOWER, 0, 350, 300);
    s.step(&[]);
    let now = s.tick().0;
    tak_sim::combat::apply_damage(
        s.world_mut(),
        &mut NoScripts,
        target,
        40_000,
        None,
        kind::NORMAL,
        Vec2::from_ints(350, 300),
        now,
    );
    let mut log = tak_commands::CommandLog::new();
    log.record(
        s.tick(),
        PlayerId(0),
        SimCommand::Heal {
            units: UnitSet::single(healer),
            target,
            queue: QueueMode::Replace,
        },
    );
    s.run(&log, 3);
    assert!(
        s.world()
            .unit(healer)
            .unwrap()
            .build
            .heal
            .unwrap()
            .work
            .building
    );
    assert!(s.world().unit(target).unwrap().combat.health > 60_000);
    kill_monarch(&mut s, 0);
    s.step(&[]);
    let hp = s.world().unit(target).unwrap().combat.health;
    let mana = tak_sim::economy::player(s.world(), PlayerId(0))
        .unwrap()
        .mana;
    assert!(hp > 0 && !s.world().unit(healer).unwrap().combat.is_dying());
    steps(&mut s, 100);
    assert_eq!(s.world().unit(target).unwrap().combat.health, hp);
    assert_eq!(
        tak_sim::economy::player(s.world(), PlayerId(0))
            .unwrap()
            .mana,
        mana
    );
    assert!(s.world().unit(healer).unwrap().build.heal.is_none());
    let calls = rec.log.borrow();
    assert_eq!(calls.calls_of(healer, "StartBuilding").len(), 1);
    assert_eq!(calls.calls_of(healer, "StopBuilding").len(), 1);
}
