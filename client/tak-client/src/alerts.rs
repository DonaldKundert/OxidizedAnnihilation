//! Alerts derived from sim state (WP-V, design system section 3.6): own
//! unit or structure attacked, construction complete, unit trained, mana
//! storage full, monarch in danger. Each becomes a minimap ping, a
//! bottom-bar line, and an entry in the alert-jump cycle (newest first).
//! Display only: nothing here feeds the sim.

use std::collections::BTreeMap;

use tak_commands::{UnitId, WorldPos};

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    /// An own mobile unit lost health.
    UnitAttacked,
    /// An own structure lost health.
    StructureAttacked,
    /// An own structure finished building.
    ConstructionComplete,
    /// An own mobile unit finished training.
    UnitTrained,
    /// The player's mana reached storage.
    ManaFull,
    /// The monarch fell below a third of its health.
    MonarchInDanger,
}

impl AlertKind {
    /// Ping colour (RGB 0..1): red for attacks and danger, gold for
    /// progress, blue for mana.
    pub fn rgb(self) -> [f32; 3] {
        match self {
            AlertKind::UnitAttacked | AlertKind::StructureAttacked => [1.0, 0.3, 0.2],
            AlertKind::MonarchInDanger => [1.0, 0.1, 0.6],
            AlertKind::ConstructionComplete | AlertKind::UnitTrained => [1.0, 0.85, 0.3],
            AlertKind::ManaFull => [0.4, 0.6, 1.0],
        }
    }
}

/// One alert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    /// What.
    pub kind: AlertKind,
    /// Sim tick it was raised.
    pub tick: u32,
    /// Where (ground position).
    pub pos: WorldPos,
    /// Bottom-bar text.
    pub text: String,
}

/// One own unit as the tracker sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertUnit {
    /// Handle.
    pub id: UnitId,
    /// Display name.
    pub name: String,
    /// Position.
    pub pos: WorldPos,
    /// Health now.
    pub health: i32,
    /// Most health.
    pub max_health: i32,
    /// Finished building.
    pub complete: bool,
    /// Moves (a unit, not a structure).
    pub mobile: bool,
    /// The monarch.
    pub monarch: bool,
}

/// The player's state for one update.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertInput {
    /// Sim tick.
    pub tick: u32,
    /// Own units.
    pub units: Vec<AlertUnit>,
    /// Mana stock and storage (whole units).
    pub mana: i64,
    /// See `mana`.
    pub storage: i64,
    /// Where the monarch is (for the mana alert's ping).
    pub home: WorldPos,
}

/// Ticks before the same unit can raise another "attacked" alert (10 s).
pub const ATTACK_COOLDOWN: u32 = 300;
/// Alerts kept for the jump cycle.
pub const KEEP: usize = 16;
/// Ticks a minimap ping shows (3 s).
pub const PING_TICKS: u32 = 90;

#[derive(Clone, Copy, Debug)]
struct Seen {
    health: i32,
    complete: bool,
    last_attack: Option<u32>,
}

/// Keeps what it saw last update and raises alerts on changes.
#[derive(Clone, Debug, Default)]
pub struct AlertTracker {
    seen: BTreeMap<UnitId, Seen>,
    started: bool,
    mana_full: bool,
    monarch_danger: bool,
    /// Alerts, newest last.
    pub alerts: Vec<Alert>,
    /// Next alert the jump goes to, counted from the newest.
    cycle: usize,
}

impl AlertTracker {
    /// Compare with the last update; returns the alerts raised now.
    pub fn update(&mut self, input: &AlertInput) -> Vec<Alert> {
        let mut new = Vec::new();
        let first = !self.started;
        self.started = true;
        let mut seen = BTreeMap::new();
        for u in &input.units {
            let prev = self.seen.get(&u.id).copied();
            let mut s = Seen {
                health: u.health,
                complete: u.complete,
                last_attack: prev.and_then(|p| p.last_attack),
            };
            if let Some(p) = prev {
                let cooled = p
                    .last_attack
                    .is_none_or(|t| input.tick.saturating_sub(t) >= ATTACK_COOLDOWN);
                // A frame gains health as it is built: only a complete
                // unit losing health counts.
                if u.complete && p.complete && u.health < p.health && cooled {
                    s.last_attack = Some(input.tick);
                    let (kind, what) = if u.mobile {
                        (AlertKind::UnitAttacked, "under attack")
                    } else {
                        (AlertKind::StructureAttacked, "under attack")
                    };
                    new.push(Alert {
                        kind,
                        tick: input.tick,
                        pos: u.pos,
                        text: format!("{} {what}", u.name),
                    });
                }
                if u.complete && !p.complete {
                    let (kind, text) = if u.mobile {
                        (AlertKind::UnitTrained, format!("Unit trained: {}", u.name))
                    } else {
                        (
                            AlertKind::ConstructionComplete,
                            format!("Construction complete: {}", u.name),
                        )
                    };
                    new.push(Alert {
                        kind,
                        tick: input.tick,
                        pos: u.pos,
                        text,
                    });
                }
            }
            if u.monarch && u.max_health > 0 {
                let frac3 = u.health * 3;
                if !self.monarch_danger && frac3 < u.max_health {
                    self.monarch_danger = true;
                    new.push(Alert {
                        kind: AlertKind::MonarchInDanger,
                        tick: input.tick,
                        pos: u.pos,
                        text: format!("{} is in danger", u.name),
                    });
                } else if self.monarch_danger && u.health * 2 > u.max_health {
                    self.monarch_danger = false;
                }
            }
            seen.insert(u.id, s);
        }
        self.seen = seen;
        let full = input.storage > 0 && input.mana >= input.storage;
        if full && !self.mana_full && !first {
            new.push(Alert {
                kind: AlertKind::ManaFull,
                tick: input.tick,
                pos: input.home,
                text: "Mana storage full".into(),
            });
        }
        self.mana_full = full;
        if !new.is_empty() {
            self.cycle = 0;
        }
        self.alerts.extend(new.iter().cloned());
        if self.alerts.len() > KEEP {
            let drop = self.alerts.len() - KEEP;
            self.alerts.drain(..drop);
        }
        new
    }

    /// The next alert to jump to: the newest first, then older ones on
    /// each press, wrapping; a new alert starts over.
    pub fn next_jump(&mut self) -> Option<&Alert> {
        if self.alerts.is_empty() {
            return None;
        }
        let i = self.cycle % self.alerts.len();
        self.cycle += 1;
        self.alerts.get(self.alerts.len() - 1 - i)
    }

    /// Alerts young enough for a minimap ping, with their age in ticks.
    pub fn pings(&self, tick: u32) -> impl Iterator<Item = (&Alert, u32)> {
        self.alerts.iter().filter_map(move |a| {
            let age = tick.saturating_sub(a.tick);
            (age < PING_TICKS).then_some((a, age))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_math::Vec2;

    fn unit(i: u32, health: i32, complete: bool, mobile: bool) -> AlertUnit {
        AlertUnit {
            id: UnitId::new(i, 0),
            name: format!("U{i}"),
            pos: Vec2::from_ints(i as i32 * 10, 0),
            health,
            max_health: 100,
            complete,
            mobile,
            monarch: i == 1,
        }
    }

    fn input(tick: u32, units: Vec<AlertUnit>, mana: i64) -> AlertInput {
        AlertInput {
            tick,
            units,
            mana,
            storage: 1000,
            home: Vec2::ZERO,
        }
    }

    #[test]
    fn raises_each_kind_once_and_cycles_newest_first() {
        let mut t = AlertTracker::default();
        assert!(
            t.update(&input(
                0,
                vec![
                    unit(1, 100, true, true),
                    unit(2, 40, false, false),
                    unit(3, 10, false, true)
                ],
                100
            ))
            .is_empty()
        );
        // Frames gain health: no attack alert; the monarch is hit.
        let a = t.update(&input(
            10,
            vec![
                unit(1, 90, true, true),
                unit(2, 60, false, false),
                unit(3, 20, false, true),
            ],
            100,
        ));
        assert_eq!(
            a.iter().map(|x| x.kind).collect::<Vec<_>>(),
            vec![AlertKind::UnitAttacked]
        );
        // Cooldown: hit again at once, no new alert.
        assert!(
            t.update(&input(
                20,
                vec![
                    unit(1, 80, true, true),
                    unit(2, 60, false, false),
                    unit(3, 20, false, true)
                ],
                100
            ))
            .is_empty()
        );
        // Structure done, unit trained, mana full, monarch in danger.
        let a = t.update(&input(
            30,
            vec![
                unit(1, 30, true, true),
                unit(2, 100, true, false),
                unit(3, 100, true, true),
            ],
            1000,
        ));
        let kinds: Vec<_> = a.iter().map(|x| x.kind).collect();
        assert!(
            kinds.contains(&AlertKind::ConstructionComplete),
            "{kinds:?}"
        );
        assert!(kinds.contains(&AlertKind::UnitTrained), "{kinds:?}");
        assert!(kinds.contains(&AlertKind::MonarchInDanger), "{kinds:?}");
        assert!(kinds.contains(&AlertKind::ManaFull), "{kinds:?}");
        // Newest first, then older, wrapping.
        let n = t.alerts.len();
        let first = t.next_jump().unwrap().clone();
        assert_eq!(first, t.alerts[n - 1]);
        let second = t.next_jump().unwrap().clone();
        assert_eq!(second, t.alerts[n - 2]);
        for _ in 2..n {
            t.next_jump();
        }
        assert_eq!(t.next_jump().unwrap(), &first);
        assert_eq!(t.pings(40).count(), n);
        assert_eq!(t.pings(10 + PING_TICKS + 30).count(), 0);
    }
}
