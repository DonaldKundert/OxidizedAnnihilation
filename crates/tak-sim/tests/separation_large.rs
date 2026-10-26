//! Large ground footprints must interact across nonadjacent spatial buckets.
use tak_commands::{PlayerId, UnitDefId};
use tak_math::{Angle, Fx, Vec2};
use tak_sim::{Scenario, Sim, StartPos, UnitId};

fn cases() -> Vec<u64> {
    let mut hashes = Vec::new();
    for (first_size, second_size) in [((5, 5), (5, 5)), ((1, 1), (1, 9)), ((9, 1), (1, 1))] {
        for vertical in [false, true] {
            let mut sc = tak_sim::testing::example_scenario(27, 1);
            let (first, second) = if vertical {
                ((200, 127), (200, 192))
            } else {
                ((127, 200), (192, 200))
            };
            sc.map.start_positions[0] = StartPos {
                x: first.0,
                y: first.1,
            };
            sc.unit_defs[0].maxvelocity = Fx::from_int(2);
            sc.unit_defs[0].motion.footprint_x = first_size.0;
            sc.unit_defs[0].motion.footprint_z = first_size.1;
            sc.unit_defs[1] = sc.unit_defs[0].clone();
            sc.unit_defs[1].name = "other synthetic mover".into();
            sc.unit_defs[1].motion.footprint_x = second_size.0;
            sc.unit_defs[1].motion.footprint_z = second_size.1;
            let mut a = Sim::new(&sc).unwrap();
            let mut b = Sim::new(&Scenario::from_json(&sc.to_json_pretty()).unwrap()).unwrap();
            let first = UnitId::new(0, 0);
            for sim in [&mut a, &mut b] {
                sim.step(&[]);
                assert_eq!(
                    sim.spawn_unit(
                        UnitDefId(1),
                        PlayerId(0),
                        Vec2::from_ints(second.0, second.1),
                        Angle::ZERO
                    ),
                    Some(UnitId::new(1, 0))
                );
            }
            let second = UnitId::new(1, 0);
            let distance = |sim: &Sim| {
                sim.world()
                    .unit(first)
                    .unwrap()
                    .pos
                    .distance(sim.world().unit(second).unwrap().pos)
            };
            let before = distance(&a);
            let radii =
                i32::from(first_size.0.max(first_size.1) + second_size.0.max(second_size.1)) * 8;
            assert!(before < Fx::from_int(radii));
            for tick in 0..100 {
                let hash = a.step(&[]);
                assert_eq!(
                    hash,
                    b.step(&[]),
                    "JSON case {first_size:?}/{second_size:?} vertical={vertical} tick={tick}"
                );
                hashes.push(hash);
                if tick == 0 {
                    assert!(
                        distance(&a) > before,
                        "overlapping {first_size:?}/{second_size:?} movers in buckets 1 and 3 received no separation, vertical={vertical}"
                    );
                }
            }
        }
    }
    hashes
}

#[test]
fn overlapping_large_and_mixed_movers_separate_across_nonadjacent_buckets() {
    cases();
}

#[test]
fn separation_large_child() {
    let Ok(out) = std::env::var("OA_SEPARATION_LARGE_CHILD") else {
        return;
    };
    std::fs::write(out, format!("{:?}", cases())).unwrap();
}

#[test]
fn large_footprint_separation_agrees_across_processes() {
    let out = std::env::temp_dir().join(format!("oa-large-separation-{}.txt", std::process::id()));
    assert!(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "separation_large_child"])
            .env("OA_SEPARATION_LARGE_CHILD", &out)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        format!("{:?}", cases())
    );
    std::fs::remove_file(out).unwrap();
}
