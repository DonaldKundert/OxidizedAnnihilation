//! The fixed-step driver: a [`World`] plus a [`ScriptHost`].

use tak_commands::{CommandLog, LoggedCommand, Tick};

use crate::hash::StateHasher;
use crate::scenario::{Scenario, ScenarioError};
use crate::script::{NoScripts, ScriptCtx, ScriptHost};
use crate::systems;
use crate::world::World;
use crate::{TICKS_PER_SECOND, UnitId};

/// A running match. Call [`Sim::step`] once per tick with that tick's
/// commands; it returns the state hash after the tick.
pub struct Sim {
    world: World,
    scripts: Box<dyn ScriptHost>,
}

impl Sim {
    /// A match with no scripts.
    pub fn new(scenario: &Scenario) -> Result<Sim, ScenarioError> {
        Sim::with_scripts(scenario, Box::new(NoScripts))
    }

    /// A match driving the given script host.
    pub fn with_scripts(
        scenario: &Scenario,
        scripts: Box<dyn ScriptHost>,
    ) -> Result<Sim, ScenarioError> {
        Ok(Sim {
            world: World::new(scenario)?,
            scripts,
        })
    }

    /// The world.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The next tick to run.
    pub fn tick(&self) -> Tick {
        self.world.tick()
    }

    /// Ticks per simulated second (fixed).
    pub const fn ticks_per_second() -> u32 {
        TICKS_PER_SECOND
    }

    /// Run one tick. `commands` should be `log.for_tick(self.tick())`; entries
    /// stamped with any other tick are ignored. Returns the state hash after
    /// the tick (see [`Sim::state_hash`]).
    pub fn step(&mut self, commands: &[LoggedCommand]) -> u64 {
        let tick = self.world.tick;
        if tick == Tick(0) {
            for id in systems::spawn_start_units(&mut self.world) {
                if let Some(ctx) = self.ctx(tick, id) {
                    self.scripts.on_create(&ctx);
                }
            }
        }
        systems::apply_commands(&mut self.world, commands);
        systems::orders(&mut self.world);
        systems::movement(&mut self.world);
        for id in self.world.unit_ids() {
            if let Some(ctx) = self.ctx(tick, id) {
                self.scripts.on_tick(&ctx);
            }
        }
        self.world.tick = tick.next();
        self.state_hash()
    }

    /// Run `ticks` ticks from the current one, feeding each its commands from
    /// `log`. Returns the per-tick hashes.
    pub fn run(&mut self, log: &CommandLog, ticks: u32) -> Vec<u64> {
        (0..ticks)
            .map(|_| {
                let t = self.tick();
                self.step(log.for_tick(t))
            })
            .collect()
    }

    /// xxHash64 of the canonical world serialization followed by the script
    /// host's state.
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        self.world.hash_into(&mut h);
        self.scripts.hash_state(&mut h);
        h.finish()
    }

    fn ctx(&self, tick: Tick, id: UnitId) -> Option<ScriptCtx> {
        let u = self.world.unit(id)?;
        Some(ScriptCtx {
            tick,
            unit: id,
            def: u.def,
            owner: u.owner,
            pos: u.pos,
            ground_height: self.world.map().height_at(u.pos),
            heading: u.heading,
            speed: u.speed,
            moving: !u.orders.is_empty(),
        })
    }
}

/// Convenience: run a scenario from scratch for `ticks` ticks.
pub fn run_scenario(
    scenario: &Scenario,
    log: &CommandLog,
    ticks: u32,
) -> Result<Vec<u64>, ScenarioError> {
    Ok(Sim::new(scenario)?.run(log, ticks))
}
