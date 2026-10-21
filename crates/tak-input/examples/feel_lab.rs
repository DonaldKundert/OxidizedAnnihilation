//! Feel lab: compare tuning presets with a simulated controller player.
//!
//! ```sh
//! cargo run --release -p tak-input --example feel_lab              # all presets
//! cargo run --release -p tak-input --example feel_lab -- --trials 400
//! # sweep one value around a preset:
//! cargo run --release -p tak-input --example feel_lab -- --base console \
//!     --sweep friction_pct=30,40,50,60,100
//! # a different simulated player (robustness):
//! cargo run --release -p tak-input --example feel_lab -- --perceive-ms 150 --tau-ms 220
//! # a saved profile:
//! cargo run --release -p tak-input --example feel_lab -- --profile my.json
//! ```

use tak_input::lab::{LabConfig, default_presets, run_lab, table_markdown};
use tak_input::{Profile, Tuning};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = LabConfig::default();
    let mut base = "console".to_owned();
    let mut sweep: Option<(String, Vec<i64>)> = None;
    let mut profiles: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--trials" => cfg.trials = v.parse().expect("--trials N"),
            "--seed" => cfg.seed = v.parse().expect("--seed N"),
            "--base" => base = v,
            "--sweep" => {
                let (name, vals) = v.split_once('=').expect("--sweep name=a,b,c");
                let vals = vals
                    .split(',')
                    .map(|x| x.parse().expect("integer"))
                    .collect();
                sweep = Some((name.to_owned(), vals));
            }
            "--profile" => profiles.push(v),
            "--perceive-ms" => cfg.human.perceive_ms = v.parse().expect("--perceive-ms N"),
            "--tau-ms" => cfg.human.tau_ms = v.parse().expect("--tau-ms N"),
            "--fly-permille" => cfg.human.fly_permille = v.parse().expect("--fly-permille N"),
            other => panic!("unknown argument {other}"),
        }
        i += 2;
    }
    let presets = match sweep {
        Some((name, vals)) => {
            let t = Tuning::preset(&base).expect("--base preset");
            let mut v = Vec::new();
            for x in vals {
                let mut tt = t;
                assert!(tt.set(&name, x), "unknown field {name}");
                let mut p = Profile::default_controller();
                p.tuning = tt;
                v.push((format!("{base} {name}={x}"), p));
            }
            v
        }
        None if !profiles.is_empty() => profiles
            .iter()
            .map(|path| {
                let s = std::fs::read_to_string(path).expect("read profile");
                (path.clone(), Profile::from_json(&s).expect("valid profile"))
            })
            .collect(),
        None => default_presets(),
    };
    let rows = run_lab(&presets, &cfg);
    println!(
        "{} trials per task and density, seed {}\n",
        cfg.trials, cfg.seed
    );
    print!("{}", table_markdown(&rows));
}
