//! Real data (gated on `TAK_DATA_DIR`): the Aramon and Zhon monarchs duel on
//! Adamantine Gate with their original scripts until one dies; the match
//! ends on that tick with the survivor as the winner (WP-U).

mod combat_common;

use combat_common::*;
use tak_commands::{LoggedCommand, PlayerId};
use tak_math::Vec2;
use tak_sim::MatchState;

#[test]
fn victory_real_map_monarch_duel_has_the_right_winner() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("victory_real_map") else {
        return;
    };
    let (mut s, _, map) = real_match(&dir, &[], 21);
    let kings: Vec<_> = s
        .world()
        .players()
        .iter()
        .map(|p| p.monarch.unwrap())
        .collect();
    // Bring the monarchs face to face (test setup, not a command).
    let o = open_area(&s, &map, 160, 64, 0);
    for (i, &k) in kings.iter().enumerate() {
        let u = s.world_mut().unit_mut(k).unwrap();
        u.pos = o + Vec2::from_ints(40 + 80 * i as i32, 32);
    }
    let mut seq = 0;
    let mut end = None;
    for _ in 0..12_000 {
        let t = s.tick();
        let mut cmds = Vec::new();
        if t.0 % 30 == 0 {
            for p in 0..2u8 {
                let mine = [kings[p as usize]];
                let theirs = [kings[1 - p as usize]];
                for c in order_idle(&s, &mine, &theirs) {
                    cmds.push(LoggedCommand {
                        tick: t,
                        player: PlayerId(p),
                        seq,
                        command: c,
                    });
                    seq += 1;
                }
            }
        }
        s.step(&cmds);
        if *s.match_state() != MatchState::Running {
            end = Some(t);
            break;
        }
    }
    let end = end.expect("one monarch kills the other");
    let alive: Vec<bool> = kings
        .iter()
        .map(|&k| s.world().unit(k).is_some_and(|u| !u.combat.is_dying()))
        .collect();
    eprintln!(
        "duel decided at tick {}: {} (monarchs alive {:?})",
        end.0,
        s.world().victory().summary(),
        alive
    );
    assert_eq!(alive.iter().filter(|&&a| a).count(), 1);
    let winner = PlayerId(alive.iter().position(|&a| a).unwrap() as u8);
    let loser = PlayerId(1 - winner.0);
    assert_eq!(
        *s.match_state(),
        MatchState::Over {
            winners: vec![winner]
        }
    );
    assert!(s.world().is_defeated(loser));
    assert!(!s.world().is_defeated(winner));
    assert_eq!(s.world().victory().decided_at(), Some(end));
    assert!(s.script_stats().errors.is_empty());
}

/// Every OTA in the install parses into mission rules and only
/// `useonlyunits` is left unsupported. Type names that the base ruleset
/// does not define are counted (they match nothing).
#[test]
fn victory_real_map_every_ota_condition_parses() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("victory_ota_corpus") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).expect("open install");
    let rs = tak_content::Ruleset::load(&vfs, tak_content::RulesetKind::Base);
    let defs: Vec<tak_sim::UnitDef> = rs
        .units
        .values()
        .map(tak_sim::content::unit_def_from_fbi)
        .collect();
    let (mut files, mut with, mut victory, mut defeat, mut unresolved) = (0, 0, 0, 0, 0);
    let mut unsupported = Vec::new();
    for f in vfs.list("") {
        if !f.path.ends_with(".ota") {
            continue;
        }
        files += 1;
        let ota = tak_content::OtaDef::load(&vfs, &f.path).expect("ota");
        let m = tak_sim::victory::MissionRules::from_conditions(&ota.conditions, &defs);
        with += usize::from(!ota.conditions.is_empty());
        victory += m.victory.len();
        defeat += m.defeat.len();
        unresolved += format!("{m:?}").matches("def: None").count();
        unsupported.extend(m.unsupported);
    }
    eprintln!(
        "{files} OTAs, {with} with conditions: {victory} victory, {defeat} defeat, \
         {unresolved} unresolved type names, unsupported {unsupported:?}"
    );
    assert!(files >= 300);
    assert!(victory + defeat > 100);
    assert!(
        unsupported.iter().all(|u| u.starts_with("useonlyunits=")),
        "{unsupported:?}"
    );
}
