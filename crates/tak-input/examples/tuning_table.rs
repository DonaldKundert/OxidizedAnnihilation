//! Print every tuning value with its presets as a Markdown table (for
//! `docs/controller.md`).
//!
//! ```sh
//! cargo run -p tak-input --example tuning_table
//! ```

use tak_input::{PRESETS, TUNING_FIELDS, Tuning};

fn main() {
    let presets: Vec<Tuning> = PRESETS
        .iter()
        .map(|p| Tuning::preset(p).expect("preset"))
        .collect();
    print!("| Group | Value | Unit |");
    for p in PRESETS {
        print!(" {p} |");
    }
    println!(" What it does |");
    print!("|---|---|---|");
    for _ in PRESETS {
        print!("---:|");
    }
    println!("---|");
    for f in TUNING_FIELDS {
        print!("| {} | `{}` | {} |", f.group, f.name, f.unit);
        for t in &presets {
            let v = t.get(f.name).unwrap_or(0);
            if f.boolean {
                print!(" {} |", if v != 0 { "on" } else { "off" });
            } else {
                print!(" {v} |");
            }
        }
        println!(" {} |", f.help);
    }
}
