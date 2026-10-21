//! The interface the VM calls for everything outside the script: unit values
//! by port, effects, sounds, mission commands and random numbers.
//!
//! The sim implements [`Engine`] for a unit. [`HeadlessEngine`] is a
//! stand-alone implementation for tools and tests: a register file of port
//! values, a seeded [`Rng`], an effect log, and counters for ports whose
//! meaning is not known yet.

use std::collections::BTreeMap;
use tak_math::Rng;

/// Port IDs. Ports 1 to 20 carry TA's names and are **Believed** to mean the
/// same in Kingdoms (port 17 is used exactly like TA's `BUILD_PERCENT_LEFT`).
/// Everything above 20 is **Unknown**; see `docs/formats/cob-vm.md`.
pub mod port {
    pub const ACTIVATION: i32 = 1;
    pub const STANDINGMOVEORDERS: i32 = 2;
    pub const STANDINGFIREORDERS: i32 = 3;
    pub const HEALTH: i32 = 4;
    pub const INBUILDSTANCE: i32 = 5;
    pub const BUSY: i32 = 6;
    pub const PIECE_XZ: i32 = 7;
    pub const PIECE_Y: i32 = 8;
    pub const UNIT_XZ: i32 = 9;
    pub const UNIT_Y: i32 = 10;
    pub const UNIT_HEIGHT: i32 = 11;
    pub const XZ_ATAN: i32 = 12;
    pub const XZ_HYPOT: i32 = 13;
    pub const ATAN: i32 = 14;
    pub const HYPOT: i32 = 15;
    pub const GROUND_HEIGHT: i32 = 16;
    pub const BUILD_PERCENT_LEFT: i32 = 17;
    pub const YARD_OPEN: i32 = 18;
    pub const BUGGER_OFF: i32 = 19;
    pub const ARMORED: i32 = 20;
    /// Highest port with a TA meaning.
    pub const LAST_TA: i32 = 20;

    /// True for ports 1..=20.
    pub fn is_ta(port: i32) -> bool {
        (1..=LAST_TA).contains(&port)
    }
}

/// Everything a running script can ask of the world. All methods take and
/// return plain integers; the sim decides what they mean. Implementations
/// must be deterministic and must not panic on any input (unknown ports,
/// out-of-range pieces): return 0 instead.
///
/// `piece` arguments are indexes into the script's piece table
/// ([`tak_formats::cob::Script::pieces`]); the sim resolves them to model
/// pieces by name.
pub trait Engine {
    /// `get-unit-value`: `( port -- value )`.
    fn get_unit_value(&mut self, port: i32) -> i32;
    /// `get`: `( port a b c d -- value )`. Argument meaning depends on the
    /// port (TA: unit IDs, packed XZ coordinates). Mostly **Unknown**.
    fn get(&mut self, port: i32, args: [i32; 4]) -> i32;
    /// `set`: `( port value -- )`.
    fn set(&mut self, port: i32, value: i32);
    /// `rand`: uniform in `low..=high` from the world's RNG.
    fn rand(&mut self, low: i32, high: i32) -> i32;
    /// `emit-sfx`: `( sfxtype -- )` at a piece.
    fn emit_sfx(&mut self, piece: u32, sfx: i32);
    /// `explode`: `( flags -- )` a piece.
    fn explode(&mut self, piece: u32, flags: i32);
    /// `play-sound`: `( priority -- result )`; `sound` indexes the script's
    /// sound table. The result is discarded by every retail use.
    fn play_sound(&mut self, sound: u32, priority: i32) -> i32;
    /// `mission-command`: `( args... -- result )`; `command` indexes the
    /// script's string table. Only mission scripts use it.
    fn mission_command(&mut self, command: u32, args: &[i32]) -> i32;
    /// `attach-unit` (TA, unused in Kingdoms): `( unit piece extra -- )`.
    fn attach_unit(&mut self, _unit: i32, _piece: i32, _extra: i32) {}
    /// `drop-unit` (TA, unused in Kingdoms): `( unit -- )`.
    fn drop_unit(&mut self, _unit: i32) {}
}

/// Something a script asked the world to do, as logged by [`HeadlessEngine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Sfx { piece: u32, sfx: i32 },
    Explode { piece: u32, flags: i32 },
    Sound { sound: u32, priority: i32 },
    MissionCommand { command: u32, args: Vec<i32> },
}

/// Most effects kept in [`HeadlessEngine::effects`]; later ones are only
/// counted, so a long run cannot grow memory without bound.
pub const MAX_LOGGED_EFFECTS: usize = 4096;

/// A minimal, deterministic [`Engine`] with no world behind it.
///
/// - `set` stores into a register file; `get-unit-value` and `get` read it
///   back (defaulting to 0). TA port 4 (`HEALTH`) defaults to 100.
/// - [`HeadlessEngine::pin`] fixes a port's value; `set` no longer changes
///   it. This is how tools fake world state, e.g. "the unit is moving".
/// - Reads and writes of ports above 20 are counted in
///   [`HeadlessEngine::unknown_reads`] / [`HeadlessEngine::unknown_writes`].
#[derive(Debug, Clone)]
pub struct HeadlessEngine {
    pub rng: Rng,
    registers: BTreeMap<i32, i32>,
    pinned: BTreeMap<i32, i32>,
    /// Per unknown port: number of `get-unit-value`/`get` reads.
    pub unknown_reads: BTreeMap<i32, u64>,
    /// Per unknown port: number of `set` writes.
    pub unknown_writes: BTreeMap<i32, u64>,
    pub effects: Vec<Effect>,
    /// Total effects requested, including those past the log cap.
    pub effect_count: u64,
}

impl HeadlessEngine {
    pub fn new(seed: u64) -> Self {
        let mut registers = BTreeMap::new();
        registers.insert(port::HEALTH, 100);
        HeadlessEngine {
            rng: Rng::new(seed),
            registers,
            pinned: BTreeMap::new(),
            unknown_reads: BTreeMap::new(),
            unknown_writes: BTreeMap::new(),
            effects: Vec::new(),
            effect_count: 0,
        }
    }

    /// Fixes `port` at `value` for reads; `set` on it is ignored.
    pub fn pin(&mut self, port: i32, value: i32) {
        self.pinned.insert(port, value);
    }

    /// The value a read of `port` returns now.
    pub fn value(&self, port: i32) -> i32 {
        self.pinned
            .get(&port)
            .or_else(|| self.registers.get(&port))
            .copied()
            .unwrap_or(0)
    }

    fn read(&mut self, port: i32) -> i32 {
        if !port::is_ta(port) {
            *self.unknown_reads.entry(port).or_default() += 1;
        }
        self.value(port)
    }

    fn log(&mut self, e: Effect) {
        self.effect_count += 1;
        if self.effects.len() < MAX_LOGGED_EFFECTS {
            self.effects.push(e);
        }
    }
}

impl Engine for HeadlessEngine {
    fn get_unit_value(&mut self, port: i32) -> i32 {
        self.read(port)
    }
    fn get(&mut self, port: i32, _args: [i32; 4]) -> i32 {
        self.read(port)
    }
    fn set(&mut self, port: i32, value: i32) {
        if !port::is_ta(port) {
            *self.unknown_writes.entry(port).or_default() += 1;
        }
        if !self.pinned.contains_key(&port) {
            self.registers.insert(port, value);
        }
    }
    fn rand(&mut self, low: i32, high: i32) -> i32 {
        let (lo, hi) = if low <= high {
            (low, high)
        } else {
            (high, low)
        };
        if hi == i32::MAX {
            // Inclusive upper bound that cannot be expressed half-open.
            return lo.wrapping_add(self.rng.below(hi.wrapping_sub(lo) as u32) as i32);
        }
        self.rng.range_i32(lo, hi + 1)
    }
    fn emit_sfx(&mut self, piece: u32, sfx: i32) {
        self.log(Effect::Sfx { piece, sfx });
    }
    fn explode(&mut self, piece: u32, flags: i32) {
        self.log(Effect::Explode { piece, flags });
    }
    fn play_sound(&mut self, sound: u32, priority: i32) -> i32 {
        self.log(Effect::Sound { sound, priority });
        0
    }
    fn mission_command(&mut self, command: u32, args: &[i32]) -> i32 {
        self.log(Effect::MissionCommand {
            command,
            args: args.to_vec(),
        });
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_pins_and_unknown_counts() {
        let mut e = HeadlessEngine::new(1);
        assert_eq!(e.get_unit_value(port::HEALTH), 100);
        e.set(21, 5);
        assert_eq!(e.get_unit_value(21), 5);
        e.pin(29, 10);
        e.set(29, 0);
        assert_eq!(e.get_unit_value(29), 10);
        assert_eq!(e.unknown_reads.get(&21), Some(&1));
        assert_eq!(e.unknown_writes.get(&29), Some(&1));
        assert!(!e.unknown_reads.contains_key(&4));
    }

    #[test]
    fn rand_is_inclusive_and_tolerates_swapped_bounds() {
        let mut e = HeadlessEngine::new(7);
        let mut seen = [false; 3];
        for _ in 0..200 {
            let r = e.rand(3, 1);
            assert!((1..=3).contains(&r));
            seen[(r - 1) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
        let _ = e.rand(i32::MIN, i32::MAX);
    }
}
