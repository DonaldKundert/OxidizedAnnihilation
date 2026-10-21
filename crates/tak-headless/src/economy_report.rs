//! `tak-headless economy-report`: run a match and print every player's mana
//! over time (stock, storage, income per second, last tick's demand and
//! grant). Rules: `docs/economy.md`.

use tak_commands::CommandLog;
use tak_sim::{Sim, economy};

/// Header line of the report.
pub const HEADER: &str = "#  tick pl         mana      storage   income/s     demand    granted";

/// A 16.16 amount in an `i64` as whole and hundredths (rounded down), e.g.
/// `1234.56`.
pub fn amount(raw: i64) -> String {
    let whole = raw >> 16;
    let frac = ((raw & 0xffff) * 100) >> 16;
    format!("{whole}.{frac:02}")
}

/// One report line per player at the sim's current tick.
pub fn lines(sim: &Sim) -> String {
    let w = sim.world();
    let mut s = String::new();
    for p in w.players() {
        let Some(e) = economy::player(w, p.id) else {
            continue;
        };
        s.push_str(&format!(
            "{:>7} {:>2} {:>12} {:>12} {:>10} {:>10} {:>10}\n",
            sim.tick().0,
            p.id.0,
            amount(e.mana),
            amount(e.storage),
            amount(e.income),
            amount(e.last_demand),
            amount(e.last_granted),
        ));
    }
    s
}

/// Run `ticks` ticks of `sim` under `log`, reporting every `every` ticks
/// (at least 1) and after the last tick.
pub fn run(sim: &mut Sim, log: &CommandLog, ticks: u32, every: u32) -> String {
    let every = every.max(1);
    let mut s = format!("{HEADER}\n");
    for _ in 0..ticks {
        let t = sim.tick();
        sim.step(log.for_tick(t));
        if sim.tick().0 % every == 0 {
            s.push_str(&lines(sim));
        }
    }
    if ticks % every != 0 {
        s.push_str(&lines(sim));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_format() {
        assert_eq!(amount(1600 << 16), "1600.00");
        assert_eq!(amount((10 << 16) + (1 << 15)), "10.50");
        assert_eq!(amount(0), "0.00");
    }

    #[test]
    fn report_has_a_line_per_player_per_interval() {
        let sc = tak_sim::testing::example_scenario(1, 2);
        let mut sim = Sim::new(&sc).unwrap();
        let r = run(&mut sim, &CommandLog::new(), 95, 30);
        // Header + 3 intervals (30, 60, 90) + final (95), two players each.
        assert_eq!(r.lines().count(), 1 + 4 * 2);
        assert!(r.lines().nth(1).unwrap().trim_start().starts_with("30  0"));
    }
}
