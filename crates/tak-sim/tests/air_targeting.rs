//! Ground-only weapon targeting. Synthetic fixtures, plus owned-definition mapping.
mod combat_common;

use combat_common::*;
use tak_commands::{CommandLog, FireState, PlayerId, QueueMode, SimCommand, Target, UnitSet};
use tak_math::{Fx, Vec2};
use tak_sim::{Scenario, UnitDefId};

const FLYER: UnitDefId = UnitDefId(4);

fn setup() -> Scenario {
    let mut sc = scenario(44);
    let mut flyer = sc.unit_defs[DUMMY_TARGET.0 as usize].clone();
    flyer.name = "flyer".into();
    flyer.motion.canfly = true;
    sc.unit_defs.push(flyer);
    let w = &mut sc.unit_defs[ARCHER.0 as usize].combat.weapons[0];
    w.no_air_weapon = true;
    w.velocity = 0;
    w.damage.default = 100;
    sc
}

#[test]
fn ground_only_slots_do_not_acquire_fire_at_or_chase_flyers() {
    for mode in 0..5 {
        let mut s = sim(&setup());
        let a = place(&mut s, ARCHER, 0, 800, 800);
        let t = place(&mut s, FLYER, 1, 900, 800);
        s.step(&[]);
        let start = s.world().unit(a).unwrap().pos;
        let mut log = CommandLog::new();
        let command = match mode {
            0 => attack(&[a], Target::Unit(t)),
            1 => SimCommand::AttackMove {
                units: UnitSet::single(a),
                to: Vec2::from_ints(800, 1100),
                queue: QueueMode::Replace,
            },
            2 => SimCommand::SetFireState {
                units: UnitSet::single(a),
                state: FireState::Defensive,
            },
            3 => {
                // ReturnFire must also reject an incompatible last attacker.
                s.world_mut().unit_mut(a).unwrap().combat.last_attacker = Some(t);
                SimCommand::SetFireState {
                    units: UnitSet::single(a),
                    state: FireState::ReturnFire,
                }
            }
            _ => SimCommand::SetFireState {
                units: UnitSet::single(a),
                state: FireState::FireAtWill,
            },
        };
        log.record(s.tick(), PlayerId(0), command);
        for _ in 0..160 {
            s.step(log.for_tick(s.tick()));
            let u = s.world().unit(a).unwrap();
            assert!(u.combat.weapons[0].target.is_none(), "mode {mode}");
            assert!(u.combat.goal.is_none(), "mode {mode} chased a flyer");
        }
        assert_eq!(health(&s, t), Some(1000), "mode {mode}");
        assert!(s.world().combat.impacts.is_empty());
        if mode == 1 {
            assert!(s.world().unit(a).unwrap().pos.y > start.y + Fx::from_int(80));
        } else {
            assert_eq!(s.world().unit(a).unwrap().pos, start);
        }
    }
}

#[test]
fn mixed_weapon_slots_only_fire_compatible_weapon_and_ground_target_still_works() {
    let mut sc = setup();
    let mut air = sc.unit_defs[ARCHER.0 as usize].combat.weapons[0].clone();
    air.no_air_weapon = false;
    sc.unit_defs[ARCHER.0 as usize].combat.weapons.push(air);
    let mut s = sim(&sc);
    let a = place(&mut s, ARCHER, 0, 800, 800);
    let t = place(&mut s, FLYER, 1, 900, 800);
    s.step(&[]);
    let mut log = CommandLog::new();
    log.record(s.tick(), PlayerId(0), attack(&[a], Target::Unit(t)));
    s.step(log.for_tick(s.tick()));
    assert_eq!(health(&s, t), Some(900)); // only the unrestricted slot fires, once before reloading
    assert!(
        s.world().unit(a).unwrap().combat.weapons[0]
            .target
            .is_none()
    );
    assert_eq!(
        s.world().unit(a).unwrap().combat.weapons[1].target,
        Some(Target::Unit(t))
    );
}

#[test]
fn ground_only_weapon_keeps_ground_attacks_and_prefers_compatible_enemy() {
    let mut s = sim(&setup());
    let a = place(&mut s, ARCHER, 0, 800, 800);
    let fly = place(&mut s, FLYER, 1, 850, 800);
    let ground = place(&mut s, DUMMY_TARGET, 1, 950, 800);
    s.step(&[]);
    assert_eq!(health(&s, fly), Some(1000));
    assert_eq!(health(&s, ground), Some(900));
    assert_eq!(
        s.world().unit(a).unwrap().combat.weapons[0].target,
        Some(Target::Unit(ground))
    );
    let mut log = CommandLog::new();
    log.record(
        s.tick(),
        PlayerId(0),
        attack(&[a], Target::Ground(Vec2::from_ints(950, 800))),
    );
    s.run(&log, 50);
    assert!(health(&s, ground).unwrap() < 900);
}

#[test]
fn owned_ground_only_flags_reach_sim_weapon_slots() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("owned_noairweapon_mapping") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    let mut flagged = 0;
    for kind in [tak_content::RulesetKind::Base, tak_content::RulesetKind::Cb] {
        let rs = tak_content::Ruleset::load(&vfs, kind);
        for u in rs.units.values() {
            let def = tak_sim::content::unit_def_from_fbi(u);
            for (source, loaded) in u.weapons.iter().zip(&def.combat.weapons) {
                assert_eq!(
                    source.no_air_weapon, loaded.no_air_weapon,
                    "{:?} {}",
                    u.name, source.slot
                );
                flagged += usize::from(loaded.no_air_weapon);
            }
        }
    }
    assert!(flagged > 0, "owned corpus must exercise the flag");
    eprintln!("verified {flagged} ground-only slots across base/Crusades rulesets");
}

/// Delays a scripted strike to exercise retargeting while a launch is pending.
#[derive(Default)]
struct DelayedStrike {
    ports: std::collections::BTreeMap<(tak_sim::UnitId, i32), i32>,
    pending: std::collections::BTreeMap<tak_sim::UnitId, u32>,
}
impl tak_sim::script::ScriptHost for DelayedStrike {
    fn has_function(&self, _: tak_sim::UnitId, name: &str) -> bool {
        matches!(name, "AimWeapon" | "FireWeapon" | "TargetCleared")
    }
    fn port(&self, id: tak_sim::UnitId, port: i32) -> Option<i32> {
        Some(*self.ports.get(&(id, port)).unwrap_or(&-1))
    }
    fn set_port(&mut self, id: tak_sim::UnitId, port: i32, value: i32) {
        self.ports.insert((id, port), value);
    }
    fn call_script(
        &mut self,
        ctx: &tak_sim::script::ScriptCtx,
        _: &mut tak_math::Rng,
        name: &str,
        args: &[i32],
    ) -> Option<tak_sim::script::ScriptReturn> {
        use tak_sim::combat::port;
        match name {
            "AimWeapon" => {
                self.ports.insert((ctx.unit, port::WEAPON_READY), args[2]);
            }
            "FireWeapon" => {
                self.pending.insert(ctx.unit, ctx.tick.0 + 4);
            }
            // Deliberately leave the old strike running, as a late script signal
            // must not bypass the engine's weapon eligibility check.
            _ => {}
        }
        Some(Default::default())
    }
    fn on_tick(&mut self, ctx: &tak_sim::script::ScriptCtx) {
        if self.pending.get(&ctx.unit) == Some(&ctx.tick.0) {
            self.ports
                .insert((ctx.unit, tak_sim::combat::port::LAUNCH_NOW), 0);
            self.pending.remove(&ctx.unit);
        }
    }
    fn hash_state(&self, h: &mut tak_sim::hash::StateHasher) {
        for ((id, port), value) in &self.ports {
            h.unit_id(*id);
            h.i32(*port);
            h.i32(*value);
        }
        for (id, at) in &self.pending {
            h.unit_id(*id);
            h.u32(*at);
        }
    }
}

fn delayed_hashes() -> Vec<u64> {
    let mut s = tak_sim::Sim::with_scripts(&setup(), Box::<DelayedStrike>::default()).unwrap();
    s.step(&[]);
    let a = place(&mut s, ARCHER, 0, 800, 800);
    let ground = place(&mut s, DUMMY_TARGET, 1, 900, 800);
    let fly = place(&mut s, FLYER, 1, 900, 850);
    s.step(&[]);
    assert!(
        s.world().unit(a).unwrap().combat.weapons[0]
            .launch
            .is_some()
    );
    let mut log = CommandLog::new();
    log.record(
        s.tick(),
        PlayerId(0),
        SimCommand::SetFireState {
            units: UnitSet::single(a),
            state: FireState::HoldFire,
        },
    );
    log.record(s.tick(), PlayerId(0), attack(&[a], Target::Unit(fly)));
    let hashes = s.run(&log, 12);
    assert_eq!(health(&s, fly), Some(1000));
    assert_eq!(health(&s, ground), Some(1000));
    assert!(s.world().combat.impacts.is_empty());
    hashes
}

#[test]
fn late_script_launch_cannot_retarget_a_ground_weapon_to_a_flyer() {
    delayed_hashes();
}

#[test]
fn air_targeting_child() {
    let Ok(out) = std::env::var("OA_AIR_CHILD") else {
        return;
    };
    std::fs::write(out, format!("{:?}", delayed_hashes())).unwrap();
}

#[test]
fn air_targeting_agrees_across_processes() {
    let out = std::env::temp_dir().join(format!("oa-air-{}.txt", std::process::id()));
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "air_targeting_child"])
        .env("OA_AIR_CHILD", &out)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        format!("{:?}", delayed_hashes())
    );
    std::fs::remove_file(out).unwrap();
}

#[test]
fn ground_only_spell_rejection_does_not_spend_mana() {
    let mut sc = setup();
    let d = &mut sc.unit_defs[ARCHER.0 as usize];
    d.combat.weapons[0].mana_per_shot = 10;
    d.economy.maxmana = 100;
    let mut s = sim(&sc);
    let a = place(&mut s, ARCHER, 0, 800, 800);
    let t = place(&mut s, FLYER, 1, 900, 800);
    s.step(&[]);
    tak_sim::economy::set_unit_mana_percent(s.world_mut(), a, 100);
    let before = tak_sim::economy::unit_mana(s.world(), a).unwrap();
    let mut log = CommandLog::new();
    log.record(
        s.tick(),
        PlayerId(0),
        SimCommand::CastSpell {
            units: UnitSet::single(a),
            spell: tak_commands::SpellId(0),
            target: Some(Target::Unit(t)),
            queue: QueueMode::Replace,
        },
    );
    s.run(&log, 10);
    assert_eq!(tak_sim::economy::unit_mana(s.world(), a).unwrap(), before);
    assert_eq!(health(&s, t), Some(1000));
    assert!(s.world().unit(a).unwrap().combat.order.is_none());
}

#[test]
fn incompatible_long_range_slot_does_not_extend_air_acquisition_range() {
    for attack_move in [false, true] {
        let mut sc = setup();
        let mut air = sc.unit_defs[ARCHER.0 as usize].combat.weapons[0].clone();
        air.no_air_weapon = false;
        air.range = 50;
        sc.unit_defs[ARCHER.0 as usize].combat.sight = 100;
        sc.unit_defs[ARCHER.0 as usize].combat.weapons.push(air);
        let mut s = sim(&sc);
        let a = place(&mut s, ARCHER, 0, 800, 800);
        place(&mut s, FLYER, 1, 1100, 800);
        let mut log = CommandLog::new();
        if attack_move {
            log.record(
                s.tick(),
                PlayerId(0),
                SimCommand::AttackMove {
                    units: UnitSet::single(a),
                    to: Vec2::from_ints(800, 1100),
                    queue: QueueMode::Replace,
                },
            );
        }
        s.run(&log, 40);
        let u = s.world().unit(a).unwrap();
        assert!(u.combat.goal.is_none());
        assert!(
            u.combat
                .attack_move
                .as_ref()
                .is_none_or(|a| a.target.is_none())
        );
        assert!(u.combat.weapons.iter().all(|w| w.target.is_none()));
    }
}

#[test]
fn explicit_guard_never_chases_or_fires_ground_only_slots_at_flyers() {
    let mut s = sim(&setup());
    let a = place(&mut s, ARCHER, 0, 800, 800);
    let guarded = place(&mut s, DUMMY_TARGET, 0, 800, 850);
    let fly = place(&mut s, FLYER, 1, 1000, 800);
    s.step(&[]);
    let start = s.world().unit(a).unwrap().pos;
    let mut log = CommandLog::new();
    log.record(
        s.tick(),
        PlayerId(0),
        SimCommand::Guard {
            units: UnitSet::single(a),
            target: guarded,
            queue: QueueMode::Replace,
        },
    );
    s.run(&log, 150);
    let u = s.world().unit(a).unwrap();
    assert_eq!(u.pos, start);
    assert!(u.combat.goal.is_none());
    assert!(u.combat.weapons[0].target.is_none());
    assert_eq!(health(&s, fly), Some(1000));
}

#[test]
fn a_scripted_strike_that_arrives_after_default_defeat_cannot_fire() {
    let mut sc = setup();
    sc.unit_defs[ARCHER.0 as usize].combat.max_damage = 50_000;
    sc.map
        .start_positions
        .push(tak_sim::StartPos { x: 100, y: 1900 });
    sc.players.push(tak_sim::PlayerSpec {
        start: 2,
        monarch: DUMMY,
    });
    let mut s = tak_sim::Sim::with_scripts(&sc, Box::<DelayedStrike>::default()).unwrap();
    s.step(&[]);
    let a = place(&mut s, ARCHER, 0, 800, 800);
    let target = place(&mut s, DUMMY_TARGET, 1, 900, 800);
    s.step(&[]);
    assert!(
        s.world().unit(a).unwrap().combat.weapons[0]
            .launch
            .is_some()
    );
    let king = s.world().players()[0].monarch.unwrap();
    s.world_mut().despawn(king).unwrap();
    s.step(&[]);
    assert!(s.world().is_defeated(PlayerId(0)));
    assert!(!s.world().unit(a).unwrap().combat.is_dying());
    assert_eq!(s.match_state(), &tak_sim::MatchState::Running);
    s.run(&CommandLog::new(), 20);
    assert_eq!(health(&s, target), Some(1000));
    assert!(
        s.world().unit(a).unwrap().combat.weapons[0]
            .launch
            .is_none()
    );
    assert!(s.world().combat.impacts.is_empty());
}
