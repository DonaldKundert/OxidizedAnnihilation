//! Real data (gated on `TAK_DATA_DIR`): 5 Aramon swordsmen against 5 Zhon
//! undead wolves on Adamantine Gate with the original COB scripts. Units close in,
//! the scripts run their aim/fire/death callbacks with zero VM errors, units
//! die and are removed, and one side wins. Also re-tests the 11 scripts whose
//! `walk`/`swim` loop ran away under faked inputs (WP-G sweep).

mod combat_common;

use combat_common::*;
use tak_commands::{LoggedCommand, PlayerId, QueueMode, SimCommand, Target, UnitSet};
use tak_math::{Angle, Vec2};

#[test]
fn swordsmen_versus_wolves_play_out_to_a_winner() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("combat_skirmish") else {
        return;
    };
    let r = skirmish(&dir, "ARASWORD", "ZONWOLF2", 5, 9000);
    eprintln!(
        "skirmish: {} ticks, alive ARA {} ZON {}, health {:?} of {:?}, deaths {}\n{}",
        r.ticks,
        r.alive[0],
        r.alive[1],
        r.health,
        r.start_health,
        r.deaths,
        r.stats.report()
    );
    assert!(
        r.alive[0] == 0 || r.alive[1] == 0,
        "no winner after {} ticks",
        r.ticks
    );
    assert!(r.alive[0] + r.alive[1] > 0, "somebody survives");
    assert!(r.deaths >= 5);
    assert!(r.stats.errors.is_empty(), "VM errors: {:?}", r.stats.errors);
    assert!(
        r.stats.runaway.is_empty(),
        "runaway loops: {:?}",
        r.stats.runaway
    );
    // Both sides drew blood: the winners are hurt too.
    let w = usize::from(r.alive[0] == 0);
    assert!(r.health[w] < r.start_health[w], "winners took no damage");
    // The weapon protocol ran: aim ready (22), launch now (23), and the
    // death animations reported done (26, or 31 for some Zhon scripts).
    let writes = |p| r.stats.writes.get(&p).copied().unwrap_or(0);
    assert!(
        writes(22) > 0 && writes(23) > 0,
        "aim/launch ports never written"
    );
    assert!(writes(26) + writes(31) >= r.deaths, "death ports");
    // Same run, same hashes.
    let again = skirmish(&dir, "ARASWORD", "ZONWOLF2", 5, 9000);
    assert_eq!(again.hashes, r.hashes);
}

#[test]
fn archers_versus_terrors_ranged_skirmish() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("combat_ranged_skirmish") else {
        return;
    };
    let r = skirmish(&dir, "ARAARCH", "ZONTER", 5, 9000);
    eprintln!(
        "ranged: {} ticks, alive ARA {} ZON {}, deaths {}",
        r.ticks, r.alive[0], r.alive[1], r.deaths
    );
    assert!(
        r.alive[0] == 0 || r.alive[1] == 0,
        "no winner after {} ticks",
        r.ticks
    );
    assert!(r.stats.errors.is_empty(), "VM errors: {:?}", r.stats.errors);
}

/// The units whose `walk`/`swim` loop spun without sleeping in the WP-G
/// sweep (faked inputs: `TargetCleared` during `FireWeapon` while moving).
pub const RUNAWAY_UNITS: [&str; 11] = [
    "ARAPULT", "MONDEV", "MONGHOUL", "MONPIRAN", "NPCAYLA", "NPCPOW2", "VERBAL", "VERLIHR",
    "VERPULT", "ZONGIANT", "ZONHURT",
];

/// Each runaway unit attacks a troll under real combat, and is ordered to
/// walk away and re-attack mid-fight, so it moves while attack animations
/// run and targets are cleared. Reports which loops still run away.
#[test]
fn runaway_loops_under_real_combat() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("combat_runaway_retest") else {
        return;
    };
    let mut still = Vec::new();
    let mut errors = Vec::new();
    for name in RUNAWAY_UNITS {
        let (mut s, defs, map) = real_match(&dir, &[name, "ZONTROLL"], 3);
        let o = open_area(&s, &map, 256, 256, 700);
        let (ox, oy) = (o.x.to_int_floor(), o.y.to_int_floor());
        let a = s
            .spawn_unit(
                defs[0],
                PlayerId(0),
                Vec2::from_ints(ox + 64, oy + 64),
                Angle::ZERO,
            )
            .unwrap();
        let t = s
            .spawn_unit(
                defs[1],
                PlayerId(1),
                Vec2::from_ints(ox + 160, oy + 64),
                Angle::HALF,
            )
            .unwrap();
        let mut fired = 0;
        for k in 0..1200u32 {
            let tick = s.tick();
            let command = match k % 200 {
                1 => Some(attack(&[a], Target::Unit(t))),
                // Walk off mid-attack (the troll follows and keeps hitting).
                90 => Some(SimCommand::Move {
                    units: UnitSet::single(a),
                    to: Vec2::from_ints(ox + 64, oy + 200),
                    queue: QueueMode::Replace,
                }),
                _ => None,
            };
            let cmds: Vec<LoggedCommand> = command
                .into_iter()
                .map(|command| LoggedCommand {
                    tick,
                    player: PlayerId(0),
                    seq: 0,
                    command,
                })
                .collect();
            s.step(&cmds);
            fired = s.script_stats().writes.get(&23).copied().unwrap_or(0);
            if s.world().unit(a).is_none() || s.world().unit(t).is_none() {
                break;
            }
        }
        let st = s.script_stats();
        eprintln!(
            "{name}: launches {fired}, runaway {:?}, errors {:?}",
            st.runaway, st.errors
        );
        if st
            .runaway
            .keys()
            .any(|k| k.to_ascii_uppercase().starts_with(name))
        {
            still.push(name);
        }
        errors.extend(st.errors.into_keys());
    }
    eprintln!("runaway loops remaining under real combat: {still:?}");
    assert!(errors.is_empty(), "VM errors: {errors:?}");
}
