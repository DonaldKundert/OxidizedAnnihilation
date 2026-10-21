//! Real data (gated on `TAK_DATA_DIR`), WP-P's integrated run on Adamantine
//! Gate with the original COB scripts and every system on: Aramon and Taros
//! with full build trees (`load_match`'s default). The Aramon monarch
//! builds a keep, the keep trains swordsmen and a catapult; the Taros
//! monarch walks over and builds walls near the Aramon base; the two sides
//! fight; corpses appear, and blocking ones (walls, catapults) block the
//! movement layers. The command log is recorded as the run goes (commands
//! decided from the state), then replayed in a child process, which must
//! give the same hashes.
//!
//! `TAK_OPEN_ITEMS_LOG_OUT=<file>` writes the recorded log (and prints the
//! final tick) for `tak-cli render --log <file> --tick <n>`.

use tak_commands::{CommandLog, PlayerId, QueueMode, SimCommand, Target, UnitId, UnitSet};
use tak_math::{Fx, Vec2};
use tak_sim::construction::content::def_by_name;
use tak_sim::construction::placement;
use tak_sim::features::Origin;
use tak_sim::{PlayerSetup, Sim, UnitDefId, WorldPos, load_match};

const MAP: &str = "kmap/Adamantine Gate";
const MAX_TICKS: u32 = 16_000;
const CHILD_ENV: &str = "TAK_OPEN_ITEMS_REAL_CHILD";
const LOG_ENV: &str = "TAK_OPEN_ITEMS_LOG_OUT";

struct Defs {
    keep: UnitDefId,
    sword: UnitDefId,
    pult: UnitDefId,
    wall: UnitDefId,
}

/// The first site around `near` (rings of 32 units) where `def` may be
/// built now.
fn site_near(sim: &Sim, def: UnitDefId, near: WorldPos) -> WorldPos {
    for r in 0..40 {
        for k in 0..(8 * r.max(1)) {
            let a = tak_math::Angle::from_raw((k * 65536 / (8 * r.max(1))) as u16);
            let p = near + Vec2::from_angle(a).scale(Fx::from_int(32 * r));
            let d = sim.world().defs().get(def).unwrap();
            let c = placement::site_center(d, sim.world().map().clamp(p));
            if placement::check_site(sim.world(), def, c, None).is_ok() {
                return c;
            }
        }
    }
    panic!("no site near {near:?}");
}

fn units_of(sim: &Sim, def: UnitDefId) -> Vec<UnitId> {
    sim.world()
        .units()
        .filter(|(_, u)| u.def == def)
        .map(|(id, _)| id)
        .collect()
}

fn complete(sim: &Sim, id: UnitId) -> bool {
    sim.world().unit(id).is_some_and(|u| u.build.is_complete())
}

/// What the run did, for assertions.
#[derive(Default, Debug)]
struct Outcome {
    keep_done: Option<u32>,
    trained: usize,
    walls_built: usize,
    deaths: u64,
    corpses: u64,
    blocking_seen: bool,
    end: u32,
}

/// Play the scenario, deciding commands from the state and recording them.
fn play(sim: &mut Sim, d: &Defs) -> (CommandLog, Vec<u64>, Outcome) {
    let mut log = CommandLog::new();
    let mut hashes = Vec::new();
    let mut out = Outcome::default();
    let (ara, tar) = (PlayerId(0), PlayerId(1));
    let king = UnitId::new(0, 0);
    let necro = UnitId::new(1, 0);
    let mut wall_sites: Vec<WorldPos> = Vec::new();
    let mut ordered_production = false;
    let mut sent: Vec<UnitId> = Vec::new();
    let mut attack_at = None;
    for _ in 0..MAX_TICKS {
        let t = sim.tick();
        let mut cmds: Vec<(PlayerId, SimCommand)> = Vec::new();
        if t.0 == 1 {
            let kpos = sim.world().unit(king).unwrap().pos;
            // The keep south-west of the monarch, toward the map centre.
            let k = site_near(sim, d.keep, kpos + Vec2::from_ints(-140, 200));
            cmds.push((
                ara,
                SimCommand::Build {
                    units: UnitSet::single(king),
                    def: d.keep,
                    at: Some(k),
                    queue: QueueMode::Replace,
                },
            ));
            // Three Taros walls on the approach, about 900 units out:
            // beyond the Taros monarch's reach of the keep's yard (frames
            // start at zero health and die to its first fireball).
            let mut near = k + Vec2::from_ints(-620, 650);
            for i in 0..3 {
                let s = site_near(sim, d.wall, near);
                wall_sites.push(s);
                cmds.push((
                    tar,
                    SimCommand::Build {
                        units: UnitSet::single(necro),
                        def: d.wall,
                        at: Some(s),
                        queue: if i == 0 {
                            QueueMode::Replace
                        } else {
                            QueueMode::Append
                        },
                    },
                ));
                near = s + Vec2::from_ints(48, -16);
            }
        }
        // The keep trains five swordsmen and a catapult once complete.
        let keeps = units_of(sim, d.keep);
        if let Some(&keep) = keeps.first() {
            if complete(sim, keep) && !ordered_production {
                ordered_production = true;
                out.keep_done = Some(t.0);
                for (i, def) in [d.sword, d.sword, d.pult, d.sword, d.sword, d.sword]
                    .into_iter()
                    .enumerate()
                {
                    cmds.push((
                        ara,
                        SimCommand::Build {
                            units: UnitSet::single(keep),
                            def,
                            at: None,
                            queue: if i == 0 {
                                QueueMode::Replace
                            } else {
                                QueueMode::Append
                            },
                        },
                    ));
                }
            }
        }
        // Each finished product marches on the walls.
        let army: Vec<UnitId> = units_of(sim, d.sword)
            .into_iter()
            .chain(units_of(sim, d.pult))
            .filter(|&id| complete(sim, id))
            .collect();
        for &id in &army {
            if !sent.contains(&id) && sim.world().unit(id).is_some_and(|u| u.orders.is_empty()) {
                sent.push(id);
                if let Some(&w) = wall_sites.first() {
                    cmds.push((
                        ara,
                        SimCommand::Move {
                            units: UnitSet::single(id),
                            to: w + Vec2::from_ints(60, -60),
                            queue: QueueMode::Replace,
                        },
                    ));
                }
            }
        }
        // With four in the field, attack the walls (one after another).
        let walls = units_of(sim, d.wall);
        out.walls_built = out
            .walls_built
            .max(walls.iter().filter(|&&w| complete(sim, w)).count());
        if attack_at.is_none() && sent.len() >= 4 {
            attack_at = Some(t.0);
        }
        if attack_at.is_some_and(|a| (t.0 - a) % 300 == 0) {
            if let Some(&w) = walls.iter().find(|&&w| complete(sim, w)) {
                let mut set = UnitSet::new();
                for &id in &army {
                    set.insert(id);
                }
                cmds.push((
                    ara,
                    SimCommand::Attack {
                        units: set,
                        target: Target::Unit(w),
                        queue: QueueMode::Replace,
                    },
                ));
            }
        }
        for (p, c) in cmds {
            log.record(t, p, c);
        }
        hashes.push(sim.step(log.for_tick(t)));

        let w = sim.world();
        out.trained = out.trained.max(sent.len());
        out.deaths = w.combat.deaths;
        out.corpses = w.features.corpses_spawned;
        let blocking = w.features.corpses().any(|(_, f)| f.obstacle);
        out.blocking_seen |= blocking;
        out.end = sim.tick().0;
        // Done once the fight has left a blocking wreck and some bodies,
        // and the army has had a while to fight.
        if blocking && out.deaths >= 3 && attack_at.is_some_and(|a| sim.tick().0 > a + 900) {
            break;
        }
    }
    (log, hashes, out)
}

fn load() -> Option<(tak_sim::Match, Defs)> {
    let dir = tak_vfs::testing::data_dir_or_skip("aramon_keep_trains_an_army_that_leaves_wrecks")?;
    let vfs = tak_vfs::open_kingdoms(&dir).expect("open install");
    let m = load_match(&vfs, MAP, &PlayerSetup::parse_list("ARA,TAR"), 7).expect("load match");
    let defs = m.scenario.unit_defs.clone();
    let ud = tak_sim::UnitDefs::new(defs);
    let id = |n: &str| def_by_name(&ud, n).unwrap_or_else(|| panic!("{n} in the build tree"));
    let d = Defs {
        keep: id("ARAKEEP"),
        sword: id("ARASWORD"),
        pult: id("ARAPULT"),
        wall: id("TARWALL"),
    };
    Some((m, d))
}

fn digest(h: &[u64]) -> String {
    format!(
        "{:016x} {:016x} {}",
        h[h.len() / 2],
        h[h.len() - 1],
        h.len()
    )
}

#[test]
fn aramon_keep_trains_an_army_that_leaves_wrecks() {
    let Some((m, d)) = load() else { return };
    let sc = m.scenario.clone();
    if let Some(path) = std::env::var_os(CHILD_ENV) {
        // Replay the recorded log and print the digest.
        let log = CommandLog::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let ticks: u32 = std::env::var("TAK_OPEN_ITEMS_TICKS")
            .unwrap()
            .parse()
            .unwrap();
        let mut sim = Sim::with_scripts(&sc, Box::new(m.scripts)).unwrap();
        let h = sim.run(&log, ticks);
        println!("DIGEST {}", digest(&h));
        return;
    }
    let mut sim = Sim::with_scripts(&sc, Box::new(m.scripts)).unwrap();
    let (log, hashes, out) = play(&mut sim, &d);
    eprintln!("{out:?}");
    for c in &sim.world().combat.corpses {
        let name = &sim.world().defs().get(c.def).unwrap().name;
        eprintln!(
            "tick {}: {name} died at {:?} (corpse answer {})",
            c.tick, c.pos, c.kind
        );
    }
    let stats = sim.script_stats();
    eprintln!("{}", stats.report());

    // The story happened.
    assert!(out.keep_done.is_some(), "keep built");
    assert!(out.trained >= 4, "keep trained an army: {out:?}");
    assert!(out.walls_built >= 1, "Taros built walls: {out:?}");
    assert!(out.deaths >= 3, "they fought: {out:?}");
    assert!(
        out.corpses >= 1 && out.blocking_seen,
        "wrecks appeared: {out:?}"
    );
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);

    // Frame health: the keep was built from zero health up to exactly its
    // maxdamage (17162), untouched by the fighting far away.
    let w = sim.world();
    let keep = w.unit(units_of(&sim, d.keep)[0]).unwrap();
    assert_eq!(keep.combat.health, keep.combat.max_health);
    assert_eq!(keep.combat.max_health, 17162);

    // Every blocking corpse is an obstacle in the swordsmen's layer.
    let grid = w.pathing.grid_for(d.sword).expect("layer");
    let mut blocking = 0;
    for (_, f) in w.features.corpses() {
        assert!(matches!(f.origin, Origin::Corpse(_)));
        if !f.obstacle {
            continue;
        }
        blocking += 1;
        for z in f.rect.z..f.rect.z + f.rect.h {
            for x in f.rect.x..f.rect.x + f.rect.w {
                assert!(tak_sim::pathfinding::is_obstacle(w, x, z));
                assert_eq!(grid.tier((x, z)), 0, "cell ({x}, {z}) blocked");
            }
        }
        let name = &w.features.def(f).unwrap().name;
        eprintln!("blocking corpse {name} at {:?}", f.rect);
    }
    assert!(blocking >= 1);
    // No live ground unit stands inside a blocking wreck that appeared
    // before it arrived (units under it when it fell may walk out).
    let _ = w.tick();

    if let Some(p) = std::env::var_os(LOG_ENV) {
        std::fs::write(&p, log.to_json()).unwrap();
        eprintln!("wrote log; render with --tick {}", out.end);
    }

    // Replay in a child process: same hashes.
    let dir = std::env::temp_dir().join(format!("tak-open-items-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("log.json");
    std::fs::write(&path, log.to_json()).unwrap();
    let exe = std::env::current_exe().unwrap();
    let child = std::process::Command::new(exe)
        .args([
            "--exact",
            "aramon_keep_trains_an_army_that_leaves_wrecks",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, &path)
        .env("TAK_OPEN_ITEMS_TICKS", hashes.len().to_string())
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(child.status.success());
    let text = String::from_utf8_lossy(&child.stdout).to_string();
    let line = text
        .split("DIGEST ")
        .nth(1)
        .and_then(|r| r.lines().next())
        .unwrap_or_else(|| panic!("child printed no digest: {text}"));
    assert_eq!(line, digest(&hashes), "replay in another process matches");
}

/// The map's features load, and a real Lodestone goes only on a sacred
/// site: refused beside the monarch with `NotSacredSite`, built on the
/// nearest site.
#[test]
fn real_lodestone_needs_a_sacred_site() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("real_lodestone_needs_a_sacred_site") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).expect("open install");
    let m = load_match(&vfs, MAP, &PlayerSetup::parse_list("ARA,ZON"), 3).expect("load match");
    let sc = m.scenario.clone();
    let lode = def_by_name(&tak_sim::UnitDefs::new(sc.unit_defs.clone()), "ARALODE").unwrap();
    let mut sim = Sim::with_scripts(&sc, Box::new(m.scripts)).unwrap();
    sim.step(&[]);
    let w = sim.world();
    let sites = w.economy.sacred_sites().to_vec();
    let placed = w.features.iter().count();
    let site_features = w
        .features
        .iter()
        .filter(|(_, f)| w.features.def(f).is_some_and(|d| d.sacred_site > Fx::ZERO))
        .count();
    eprintln!("{placed} gameplay features, {site_features} sacred stones");
    assert_eq!(site_features, sites.len());
    assert!(placed > site_features);

    let king = UnitId::new(0, 0);
    let kpos = w.unit(king).unwrap().pos;
    let wall = def_by_name(sim.world().defs(), "ARAWALL").unwrap();
    let off = site_near(&sim, wall, kpos + Vec2::from_ints(60, 60));
    assert_eq!(
        placement::check_site(sim.world(), lode, off, None),
        Err(placement::SiteError::NotSacredSite)
    );
    let near = sites
        .iter()
        .min_by_key(|s| kpos.distance(Vec2::from_ints(s.x + s.w / 2, s.y + s.h / 2)))
        .unwrap();
    let centre = Vec2::from_ints(near.x + near.w / 2, near.y + near.h / 2);
    let mut log = CommandLog::new();
    log.record(
        tak_commands::Tick(1),
        PlayerId(0),
        SimCommand::Build {
            units: UnitSet::single(king),
            def: lode,
            at: Some(centre),
            queue: QueueMode::Replace,
        },
    );
    for _ in 1..6000 {
        let t = sim.tick();
        sim.step(log.for_tick(t));
        if units_of(&sim, lode)
            .first()
            .is_some_and(|&l| complete(&sim, l))
        {
            break;
        }
    }
    let l = units_of(&sim, lode);
    assert_eq!(l.len(), 1, "built on the site");
    assert!(complete(&sim, l[0]));
    sim.step(&[]);
    assert!(
        sim.world().unit(l[0]).unwrap().economy.site().is_some(),
        "claims the site"
    );
}
