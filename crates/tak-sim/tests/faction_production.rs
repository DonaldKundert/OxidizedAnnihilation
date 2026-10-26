//! Opt-in owned-data audit of ordinary land production across all five factions.
//! No unit spawns, resource edits, stat changes or retail fixtures.
use tak_commands::{CommandLog, PlayerId, QueueMode, SimCommand, UnitDefId, UnitSet};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::construction::placement::{check_site, site_center};
use tak_sim::{PlayerSetup, ScriptHost, Sim, UnitDef, WorldPos};

fn troop(d: &UnitDef) -> bool {
    !d.build.builder
        && d.build.mobile
        && d.maxvelocity > Fx::ZERO
        && d.motion.min_water_depth <= 0
        && d.build.buildcost > 0
        && d.combat.max_damage > 0
        && d.combat
            .weapons
            .iter()
            .any(|w| !w.is_spell() && w.damage.default > 0)
}
fn site(s: &Sim, def: UnitDefId, near: WorldPos) -> WorldPos {
    let d = s.world().defs().get(def).unwrap();
    for r in 0..30 {
        for k in 0..(8 * r.max(1)) {
            let a = Angle::from_raw((k * 65536 / (8 * r.max(1))) as u16);
            let at = site_center(
                d,
                s.world()
                    .map()
                    .clamp(near + Vec2::from_angle(a).scale(Fx::from_int(r * 32))),
            );
            if check_site(s.world(), def, at, None).is_ok() {
                return at;
            }
        }
    }
    panic!("no ordinary placement site for {}", d.name)
}

#[test]
#[ignore = "owned-data five-faction paid production and raw replay audit; run explicitly"]
fn all_five_factions_build_affordable_land_troops_through_authored_menu_edges() {
    let Some(data) = tak_vfs::testing::data_dir_or_skip("five_faction_production") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&data).unwrap();
    for faction in ["ARA", "TAR", "VER", "ZON", "CRE"] {
        let sides = format!("{faction},{}", if faction == "ARA" { "ZON" } else { "ARA" });
        let loaded = tak_sim::load_match(
            &vfs,
            "kmap/Adamantine Gate",
            &PlayerSetup::parse_list(&sides),
            71,
        )
        .unwrap();
        assert!(loaded.scripts.stats().missing.is_empty());
        let sc = loaded.scenario.clone();
        let root_id = sc.players[0].monarch;
        let root = &sc.unit_defs[root_id.0 as usize];
        let mut candidates = Vec::new();
        for &id in &root.build.canbuild {
            let d = &sc.unit_defs[id.0 as usize];
            if troop(d) && d.build.buildcost <= i64::from(root.economy.mogriumstorage) {
                candidates.push(((d.build.buildcost, 0), None, id));
            }
            if d.build.builder
                && d.build.workertime > 0
                && d.build.buildcost > 0
                && d.build.buildcost <= i64::from(root.economy.mogriumstorage)
            {
                for &child in &d.build.canbuild {
                    let child_def = &sc.unit_defs[child.0 as usize];
                    if troop(child_def)
                        && child_def.build.buildcost
                            <= i64::from(root.economy.mogriumstorage + d.economy.mogriumstorage)
                    {
                        candidates.push((
                            (child_def.build.buildcost, d.build.buildcost),
                            Some(id),
                            child,
                        ));
                    }
                }
            }
        }
        let (_, producer_def, troop_def) = candidates
            .into_iter()
            .min_by_key(|(cost, _, _)| *cost)
            .unwrap_or_else(|| {
                panic!("{faction} has no affordable supported ordinary land production path")
            });
        let producer_name = producer_def
            .map(|id| sc.unit_defs[id.0 as usize].name.clone())
            .unwrap_or(root.name.clone());
        let mut s = Sim::with_scripts(&sc, Box::new(loaded.scripts)).unwrap();
        let mut replay_hashes = vec![s.step(&[])];
        let mut log = CommandLog::new();
        let king = s.world().players()[0].monarch.unwrap();
        let mut producer = producer_def.is_none().then_some(king);
        let mut producer_ordered = false;
        let mut troop_ordered = false;
        let mut producer_paid = producer_def.is_none();
        let mut troop_paid = false;
        let mut completed = None;
        for _ in 0..24_000 {
            let mut commands = Vec::new();
            if !producer_ordered {
                producer_ordered = true;
                if let Some(def) = producer_def {
                    let near = s.world().unit(king).unwrap().pos + Vec2::from_ints(160, 160);
                    commands.push(SimCommand::Build {
                        units: UnitSet::single(king),
                        def,
                        at: Some(site(&s, def, near)),
                        queue: QueueMode::Replace,
                    });
                }
            }
            if let Some(def) = producer_def {
                for (id, u) in s
                    .world()
                    .units()
                    .filter(|(_, u)| u.owner == PlayerId(0) && u.def == def)
                {
                    producer_paid |= u.build.frame.is_some_and(|f| f.paid > 0);
                    if u.build.is_complete() {
                        producer = Some(id);
                    }
                }
            }
            if let Some(id) = producer.filter(|_| !troop_ordered) {
                troop_ordered = true;
                let u = s.world().unit(id).unwrap();
                let d = s.world().defs().get(u.def).unwrap();
                let at = if d.build.mobile {
                    Some(site(&s, troop_def, u.pos + Vec2::from_ints(80, 80)))
                } else {
                    None
                };
                commands.push(SimCommand::Build {
                    units: UnitSet::single(id),
                    def: troop_def,
                    at,
                    queue: QueueMode::Replace,
                });
            }
            for command in commands {
                log.record(s.tick(), PlayerId(0), command);
            }
            replay_hashes.push(s.step(log.for_tick(s.tick())));
            for (id, u) in s
                .world()
                .units()
                .filter(|(_, u)| u.owner == PlayerId(0) && u.def == troop_def)
            {
                troop_paid |= u.build.frame.is_some_and(|f| f.paid > 0);
                if u.build.is_complete() {
                    completed = Some(id);
                }
            }
            if completed.is_some() {
                break;
            }
        }
        let id = completed.unwrap_or_else(|| {
            panic!(
                "{faction} production stalled at {}: {:?}",
                s.tick().0,
                s.world().unit(king)
            )
        });
        assert!(
            producer_paid && troop_paid,
            "{faction} must exercise paid construction"
        );
        let release_tick = s.tick().0;
        for _ in 0..240 {
            replay_hashes.push(s.step(&[]));
        }
        if producer_def.is_some_and(|id| !sc.unit_defs[id.0 as usize].build.mobile) {
            let factory = s.world().unit(producer.unwrap()).unwrap();
            let product = s.world().unit(id).unwrap();
            assert!(
                product.pos.distance(factory.pos) > Fx::from_int(32),
                "{faction} product failed to leave the factory"
            );
        }
        let again = tak_sim::load_match(
            &vfs,
            "kmap/Adamantine Gate",
            &PlayerSetup::parse_list(&sides),
            71,
        )
        .unwrap();
        let mut replay = Sim::with_scripts(&again.scenario, Box::new(again.scripts)).unwrap();
        assert_eq!(
            replay.run(&log, s.tick().0),
            replay_hashes,
            "{faction} paid production raw replay"
        );
        assert!(
            s.script_stats().errors.is_empty(),
            "{faction} script errors: {:?}",
            s.script_stats().errors
        );
        eprintln!(
            "{faction}: {} -> {producer_name} -> {}, paid product complete at {release_tick}, {} ticks replay-verified",
            root.name,
            sc.unit_defs[troop_def.0 as usize].name,
            s.tick().0
        );
    }
}
