//! Combat rules on synthetic definitions (no game data, no scripts):
//! damage per hit from `[DAMAGE]`, area falloff, reload timing, closing to
//! range, fire states, regeneration, death and removal.

mod combat_common;

use combat_common::*;
use tak_commands::{FireState, PlayerId, SimCommand, Target, UnitSet};
use tak_math::{Fx, Vec2};
use tak_sim::combat::{DamageStats, area_damage, melee_reach, reload_ticks};

#[test]
fn damage_table_and_falloff_formulas() {
    let t = DamageStats {
        default: 1250,
        multipliers: vec![
            ("fort".into(), Fx::from_ratio(5, 4).unwrap()),
            ("monarch".into(), Fx::from_ratio(1, 4).unwrap()),
        ],
    };
    assert_eq!(t.against(None), 1250);
    assert_eq!(t.against(Some("human")), 1250);
    assert_eq!(t.against(Some("fort")), 1562); // 1562.5 truncated
    assert_eq!(t.against(Some("monarch")), 312);
    // TA falloff (1-e)(d/R-1)^2 + e.
    assert_eq!(area_damage(1000, 0, 50, Fx::HALF), 1000);
    assert_eq!(area_damage(1000, 25, 50, Fx::HALF), 625);
    assert_eq!(area_damage(1000, 50, 50, Fx::HALF), 500);
    assert_eq!(area_damage(1000, 10, 50, Fx::ONE), 1000);
    assert_eq!(area_damage(1000, 10, 50, Fx::ZERO), 640);
    // reloadtime 1.7 s -> trunc(51.0) ticks; 2.649993896 s (as parsed) -> 79.
    assert_eq!(reload_ticks(Fx::from_raw(111_411)), 51);
    assert_eq!(reload_ticks(Fx::from_raw(173_670)), 79);
    assert_eq!(reload_ticks(Fx::ZERO), 1);
    assert_eq!(melee_reach(8, 8), 32);
}

#[test]
fn melee_hits_match_the_damage_table_and_kill() {
    let sc = scenario(1);
    let mut s = sim(&sc);
    let a = place(&mut s, SWORD, 0, 1000, 1000);
    let t = place(&mut s, DUMMY_TARGET, 1, 1020, 1000);
    let mut seen = vec![1000];
    let mut hit_ticks = Vec::new();
    for _ in 0..600 {
        let tick = s.tick().0;
        s.step(&[]);
        match health(&s, t) {
            Some(h) if h != *seen.last().unwrap() => {
                seen.push(h);
                hit_ticks.push(tick);
            }
            Some(_) => {}
            None => break,
        }
    }
    // 360 default × 1.5 against "human" = 540 per hit; 1000 hp: two hits.
    assert_eq!(seen, vec![1000, 460, -80], "health after each hit");
    assert_eq!(hit_ticks[1] - hit_ticks[0], 51, "one hit per reload");
    assert!(s.world().unit(t).is_none(), "dead unit removed");
    assert_eq!(s.world().combat.deaths, 1);
    assert_eq!(s.world().combat.corpses.len(), 1);
    assert_eq!(s.world().unit(a).unwrap().combat.kills, 1);
}

#[test]
fn ballistic_shot_travels_and_splashes() {
    let mut sc = scenario(2);
    sc.unit_defs[ARCHER.0 as usize].combat.weapons[0] = ballistic(400, 450, 300, 100);
    let mut s = sim(&sc);
    let a = place(&mut s, ARCHER, 0, 1000, 1000);
    let t = place(&mut s, DUMMY_TARGET, 1, 1200, 1000);
    let near = place(&mut s, DUMMY_TARGET, 1, 1200, 1030);
    let far = place(&mut s, DUMMY_TARGET, 1, 1200, 1100);
    let _ = a;
    let mut fired = None;
    let mut landed = None;
    for _ in 0..200 {
        let tick = s.tick().0;
        s.step(&[]);
        if fired.is_none() && !s.world().combat.projectiles.is_empty() {
            fired = Some(tick);
        }
        if landed.is_none() && health(&s, t) != Some(1000) {
            landed = Some(tick);
            break;
        }
    }
    let (fired, landed) = (fired.expect("fired"), landed.expect("landed"));
    // 300 px/s = 10 px/tick over 200 px: 20 steps of flight, the first in
    // the launch tick itself (TA: a new shot moves in its creation tick).
    assert_eq!(landed - fired, 19);
    assert_eq!(health(&s, t), Some(600), "direct: full 400");
    // 30 px away, body radius 8: d = 22 of R = 50.
    assert_eq!(
        health(&s, near),
        Some(1000 - area_damage(400, 22, 50, Fx::HALF))
    );
    assert_eq!(health(&s, far), Some(1000), "outside the blast");
}

#[test]
fn attack_order_closes_distance_and_hold_fire_holds() {
    let sc = scenario(3);
    let mut s = sim(&sc);
    let a = place(&mut s, SWORD, 0, 1000, 1000);
    let t = place(&mut s, DUMMY_TARGET, 1, 1000, 1300);
    // Out of sight and reach: nothing happens on its own.
    for _ in 0..60 {
        s.step(&[]);
    }
    assert_eq!(health(&s, t), Some(1000));
    assert_eq!(s.world().unit(a).unwrap().pos, Vec2::from_ints(1000, 1000));
    let tick = s.tick();
    let cmds = [tak_commands::LoggedCommand {
        tick,
        player: PlayerId(0),
        seq: 0,
        command: attack(&[a], Target::Unit(t)),
    }];
    s.step(&cmds);
    for _ in 0..800 {
        s.step(&[]);
        if s.world().unit(t).is_none() {
            break;
        }
    }
    assert!(
        s.world().unit(t).is_none(),
        "attacker walked up and killed the target"
    );
    assert!(s.world().unit(a).unwrap().combat.order.is_none());

    // Hold fire: an adjacent enemy is ignored.
    let mut s = sim(&sc);
    let a = place(&mut s, SWORD, 0, 1000, 1000);
    let t = place(&mut s, DUMMY_TARGET, 1, 1020, 1000);
    let tick = s.tick();
    let cmds = [tak_commands::LoggedCommand {
        tick,
        player: PlayerId(0),
        seq: 0,
        command: SimCommand::SetFireState {
            units: UnitSet::single(a),
            state: FireState::HoldFire,
        },
    }];
    s.step(&cmds);
    for _ in 0..200 {
        s.step(&[]);
    }
    assert_eq!(health(&s, t), Some(1000));
}

#[test]
fn idle_units_chase_within_sight_and_regenerate() {
    let mut sc = scenario(4);
    sc.unit_defs[DUMMY_TARGET.0 as usize].combat.heal_rate = Fx::from_int(3);
    let mut s = sim(&sc);
    let a = place(&mut s, SWORD, 0, 1000, 1000);
    let t = place(&mut s, DUMMY_TARGET, 1, 1150, 1000); // within sight 200
    let mut hit = false;
    for _ in 0..300 {
        s.step(&[]);
        if health(&s, t).is_some_and(|h| h < 1000) {
            hit = true;
            break;
        }
    }
    assert!(hit, "idle swordsman chased and struck");
    let h0 = health(&s, t).unwrap();
    // Stop the attacker and watch the target heal 3 hp/s.
    let tick = s.tick();
    let cmds = [tak_commands::LoggedCommand {
        tick,
        player: PlayerId(0),
        seq: 0,
        command: SimCommand::SetFireState {
            units: UnitSet::single(a),
            state: FireState::HoldFire,
        },
    }];
    s.step(&cmds);
    for _ in 0..60 {
        s.step(&[]);
    }
    let h1 = health(&s, t).unwrap();
    assert!(
        (h0 + 5..=h0 + 6).contains(&h1),
        "healed {} in 61 ticks",
        h1 - h0
    );
}

#[test]
fn brawl_is_reproducible_in_process() {
    let a = brawl_hashes(9, 1500);
    let b = brawl_hashes(9, 1500);
    assert_eq!(a, b);
    assert_ne!(brawl_hashes(10, 1500), a);
}
