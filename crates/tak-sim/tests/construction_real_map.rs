//! Real data (gated on `TAK_DATA_DIR`): on Adamantine Gate the Aramon
//! monarch walks to a site and builds a keep with the original COB scripts
//! running (the keep's own script sees port 17 count 100 -> 0), then the
//! keep trains two swordsmen that appear at its build piece and walk out.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use tak_commands::{
    LoggedCommand, PlayerId, QueueMode, SimCommand, Tick, UnitDefId, UnitId, UnitSet,
};
use tak_math::{Fx, Rng, Vec2};
use tak_sim::construction::content::{def_by_name, load_match_with_builds};
use tak_sim::construction::placement;
use tak_sim::{
    CobScripts, PlayerSetup, ScriptCtx, ScriptHost, ScriptReturn, ScriptStats, Sim, StateHasher,
    WorldPos,
};

const MAP: &str = "kmap/Adamantine Gate";

#[derive(Default)]
struct Seen {
    tick: u32,
    calls: Vec<(u32, UnitId, String)>,
    port17: BTreeMap<UnitId, Vec<i32>>,
    created: BTreeMap<UnitId, (u32, WorldPos)>,
}

/// The real COB host, watched.
struct Watch {
    inner: CobScripts,
    seen: Rc<RefCell<Seen>>,
}

impl ScriptHost for Watch {
    fn create_unit(&mut self, ctx: &ScriptCtx, rng: &mut Rng) {
        let mut s = self.seen.borrow_mut();
        let t = s.tick;
        s.created.insert(ctx.unit, (t, ctx.pos));
        drop(s);
        self.inner.create_unit(ctx, rng);
    }
    fn tick_unit(&mut self, ctx: &ScriptCtx, rng: &mut Rng) {
        self.seen
            .borrow_mut()
            .port17
            .entry(ctx.unit)
            .or_default()
            .push(ctx.build_percent_left);
        self.inner.tick_unit(ctx, rng);
    }
    fn call_script(
        &mut self,
        ctx: &ScriptCtx,
        rng: &mut Rng,
        name: &str,
        args: &[i32],
    ) -> Option<ScriptReturn> {
        let mut s = self.seen.borrow_mut();
        let t = s.tick;
        s.calls.push((t, ctx.unit, name.into()));
        drop(s);
        self.inner.call_script(ctx, rng, name, args)
    }
    fn start_script(&mut self, unit: UnitId, name: &str, args: &[i32]) -> bool {
        let mut s = self.seen.borrow_mut();
        let t = s.tick;
        s.calls.push((t, unit, name.into()));
        drop(s);
        self.inner.start_script(unit, name, args)
    }
    fn port_value(&self, unit: UnitId, port: i32) -> Option<i32> {
        self.inner.port_value(unit, port)
    }
    fn piece_origin(&self, unit: UnitId, piece: i32) -> Option<[i32; 3]> {
        self.inner.piece_origin(unit, piece)
    }
    fn remove_unit(&mut self, unit: UnitId) {
        self.inner.remove_unit(unit);
    }
    fn hash_state(&self, h: &mut StateHasher) {
        self.inner.hash_state(h);
    }
    fn stats(&self) -> ScriptStats {
        self.inner.stats()
    }
}

fn build(tick: u32, unit: UnitId, def: UnitDefId, at: Option<WorldPos>, seq: u32) -> LoggedCommand {
    LoggedCommand {
        tick: Tick(tick),
        player: PlayerId(0),
        seq,
        command: SimCommand::Build {
            units: UnitSet::single(unit),
            def,
            at,
            queue: if seq == 0 {
                QueueMode::Replace
            } else {
                QueueMode::Append
            },
        },
    }
}

#[test]
fn monarch_builds_a_keep_and_the_keep_trains_two_swordsmen() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("construction real map") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).expect("open install");
    let m = load_match_with_builds(&vfs, MAP, &PlayerSetup::parse_list("ARA,TAR"), 11)
        .expect("load match");
    let sc = m.scenario.clone();
    assert!(m.scripts.stats().missing.is_empty());
    let defs = tak_sim::UnitDefs::new(sc.unit_defs.clone());
    let keep_def = def_by_name(&defs, "ARAKEEP").expect("keep in the Aramon tree");
    let sword_def = def_by_name(&defs, "ARASWORD").expect("swordsman in the tree");
    let king = defs.get(UnitDefId(0)).unwrap();
    assert_eq!(king.name, "ARAKING");
    assert!(king.build.canbuild.contains(&keep_def));
    let kd = defs.get(keep_def).unwrap();
    assert_eq!((kd.footprintx, kd.footprintz), (7, 12));
    assert_eq!(kd.build.buildtime, 600 << 16);
    assert!(!kd.build.mobile && kd.build.builder);
    assert!(kd.build.canbuild.contains(&sword_def));

    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut sim = Sim::with_scripts(
        &sc,
        Box::new(Watch {
            inner: m.scripts,
            seen: seen.clone(),
        }),
    )
    .unwrap();
    let step = |sim: &mut Sim, cmds: &[LoggedCommand]| {
        seen.borrow_mut().tick = sim.tick().0;
        let t = sim.tick();
        let now: Vec<_> = cmds.iter().filter(|c| c.tick == t).cloned().collect();
        sim.step(&now);
    };
    step(&mut sim, &[]);
    let king_id = UnitId::new(0, 0);
    let start = sim.world().unit(king_id).unwrap().pos;

    // A free site 400-650 units away with open ground on the way there.
    let map = sim.world().map().clone();
    let site = (0..64)
        .flat_map(|ring| (0..32).map(move |k| (ring, k)))
        .filter_map(|(ring, k)| {
            let r = 400 + ring * 4;
            let a = tak_math::Angle::from_raw((k * 2048) as u16);
            let p = start + Vec2::from_angle(a).scale(Fx::from_int(r));
            let s = placement::site_center(kd, p);
            placement::check_site(sim.world(), keep_def, s, None).ok()?;
            let open = (0..=40).all(|i| {
                let q = start + (s - start).scale(Fx::from_ratio(i, 40).unwrap());
                map.terrain_at(q) & tak_sim::map::terrain::BLOCKS == 0
            });
            open.then_some(s)
        })
        .next()
        .expect("a buildable keep site near the monarch");

    let mut cmds = vec![build(1, king_id, keep_def, Some(site), 0)];
    let mut keep = None;
    let mut placed_at = 0;
    for _ in 0..3000 {
        step(&mut sim, &cmds);
        if keep.is_none() {
            if let Some((id, _)) = sim.world().units().find(|(_, u)| u.def == keep_def) {
                keep = Some(id);
                placed_at = sim.tick().0 - 1;
                let k = sim.world().unit(king_id).unwrap();
                assert!(
                    k.pos.distance(start) > Fx::from_int(50),
                    "the monarch walked"
                );
                assert!(placement::in_range(k.pos, king, site, kd));
            }
        }
        if keep.is_some_and(|k| sim.world().unit(k).unwrap().build.is_complete()) {
            break;
        }
    }
    let keep = keep.expect("keep placed");
    let done = sim.tick().0 - 1;
    assert_eq!(sim.world().unit(keep).unwrap().pos, site);
    assert_eq!(done - placed_at, 1799, "600 / 10 seconds = 1800 ticks");
    {
        let s = seen.borrow();
        let p17 = &s.port17[&keep];
        assert_eq!(p17.first(), Some(&100));
        assert_eq!(p17.last(), Some(&0));
        assert!(p17.windows(2).all(|w| w[1] <= w[0]));
        assert!(
            s.calls
                .contains(&(placed_at, king_id, "StartBuilding".into()))
        );
        assert!(s.calls.contains(&(done, king_id, "StopBuilding".into())));
    }
    // A few ticks later the keep's Create has seen port 17 = 0 and started
    // its control threads (smoke, flags, damage).
    for _ in 0..10 {
        step(&mut sim, &cmds);
    }
    // (inspecting the VM needs the host back; thread starts show up as a
    // changing pose instead)
    let pose_before = sim.script_stats().pose_ticks;

    // Two swordsmen.
    let t = sim.tick().0;
    cmds.push(build(t, keep, sword_def, None, 0));
    cmds.push(build(t, keep, sword_def, None, 1));
    let mut swords: Vec<UnitId> = Vec::new();
    for _ in 0..1500 {
        step(&mut sim, &cmds);
        for (id, u) in sim.world().units() {
            if u.def == sword_def && !swords.contains(&id) {
                swords.push(id);
            }
        }
        let all_done = swords.len() == 2
            && swords.iter().all(|&s| {
                let u = sim.world().unit(s).unwrap();
                u.build.is_complete() && u.orders.is_empty() && u.speed == Fx::ZERO
            });
        if all_done {
            break;
        }
    }
    assert_eq!(swords.len(), 2, "two swordsmen");
    assert!(sim.script_stats().pose_ticks > pose_before);
    let s = seen.borrow();
    let activate = s
        .calls
        .iter()
        .find(|c| c.1 == keep && c.2 == "Activate")
        .expect("Activate")
        .0;
    let kpos = sim.world().unit(keep).unwrap().pos;
    for &sw in &swords {
        let (created, at) = s.created[&sw];
        assert!(
            created > activate,
            "waited for the yard door (INBUILDSTANCE)"
        );
        // At the build piece: inside the keep's footprint, south of centre.
        assert!(
            at.y > kpos.y && at.y < kpos.y + Fx::from_int(96),
            "{at:?} vs {kpos:?}"
        );
        assert!((at.x - kpos.x).abs() < Fx::from_int(56));
        let u = sim.world().unit(sw).unwrap();
        assert!(u.build.is_complete());
        assert!(
            u.pos.y > kpos.y + Fx::from_int(96),
            "walked out of the yard: {:?}",
            u.pos
        );
        let p17 = &s.port17[&sw];
        assert_eq!(p17.last(), Some(&0));
        assert_eq!(p17.iter().filter(|&&v| v > 0).count(), 296, "99 / 10 s");
    }
    assert!(s.calls.iter().any(|c| c.1 == keep && c.2 == "Deactivate"));
    let stats = sim.script_stats();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert!(stats.runaway.is_empty(), "{:?}", stats.runaway);
}
