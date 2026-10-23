//! Shared read-only admission and canonical command realization for interactive
//! and headless training sessions. No decision is generated during replay.

use tak_commands::{CommandLog, LoggedCommand};
use tak_sim::World;

use crate::bot::ControllerConfig;

use super::{CheckpointError, MAX_CHECKPOINT_COMMANDS, bot_error};

/// Check a complete future range before any tick/history mutation. In addition
/// to replay history and resource limits, all future bot-owner input is refused,
/// even beyond the requested range. Returns the exclusive absolute end tick.
/// This is admission only: it neither plans commands nor changes the world.
pub fn admit_driven(
    world: &World,
    controllers: &ControllerConfig,
    executed: &CommandLog,
    input: &CommandLog,
    ticks: u32,
) -> Result<u32, CheckpointError> {
    let start = world.tick().0;
    let end = validate_controller_input(world, controllers, input, ticks)?;
    validate_advance(
        start,
        executed,
        input,
        ticks,
        controllers.maximum_commands(start, end),
    )
}

/// Validate, plan and merge exactly one future tick, without modifying anything.
/// Public callers cannot bypass admission or controller validation. The returned
/// ordinary commands are canonical and have unique keys; feed them to `Sim::step`
/// and retain them verbatim as executed history. Do not use this during replay.
pub fn prepare_driven_tick(
    world: &World,
    controllers: &ControllerConfig,
    executed: &CommandLog,
    input: &CommandLog,
) -> Result<Vec<LoggedCommand>, CheckpointError> {
    admit_driven(world, controllers, executed, input, 1)?;
    realize_admitted_tick(world, controllers, input)
}

/// Checked interactive single-tick admission. Only FUTURE input is considered:
/// earlier entries cannot affect this tick and are intentionally not reverified.
/// Supply the count from privately retained actual execution history, not from
/// the editable input log. Capture/restore must separately verify all history.
/// This avoids re-reading an ever-growing historical prefix every live tick.
/// Controller identity, all competing future owners, current key uniqueness,
/// elapsed-tick and conservative command limits are still checked before planning.
pub fn prepare_future_tick(
    world: &World,
    controllers: &ControllerConfig,
    executed_commands: usize,
    input: &CommandLog,
) -> Result<Vec<LoggedCommand>, CheckpointError> {
    let start = world.tick().0;
    let end = validate_controller_input(world, controllers, input, 1)?;
    validate_future(
        start,
        executed_commands,
        input,
        1,
        controllers.maximum_commands(start, end),
    )?;
    realize_admitted_tick(world, controllers, input)
}

fn validate_controller_input(
    world: &World,
    controllers: &ControllerConfig,
    input: &CommandLog,
    ticks: u32,
) -> Result<u32, CheckpointError> {
    controllers.validate(world).map_err(bot_error)?;
    let start = world.tick().0;
    let end = checked_end(start, ticks)?;
    if !controllers.controllers.is_empty() {
        let first = input
            .entries()
            .partition_point(|entry| entry.tick.0 < start);
        for entry in &input.entries()[first..] {
            if controllers.controls(entry.player) {
                return Err(CheckpointError::Invalid(format!(
                    "future input competes with controller owner {} at tick {}",
                    entry.player.0, entry.tick.0
                )));
            }
        }
    }
    Ok(end)
}

// Only the owning checkpoint module can skip repeated full-log admission after
// preflighting an entire immutable input range. Public callers always validate.
pub(super) fn realize_admitted_tick(
    world: &World,
    controllers: &ControllerConfig,
    input: &CommandLog,
) -> Result<Vec<LoggedCommand>, CheckpointError> {
    let mut realized = input.for_tick(world.tick()).to_vec();
    realized.extend(controllers.plan_validated(world));
    realized.sort_by_key(LoggedCommand::key);
    if realized
        .windows(2)
        .any(|pair| pair[0].key() >= pair[1].key())
    {
        return Err(CheckpointError::Invalid(
            "realized command keys are not unique and ordered".into(),
        ));
    }
    Ok(realized)
}

fn checked_end(start: u32, ticks: u32) -> Result<u32, CheckpointError> {
    start
        .checked_add(ticks)
        .filter(|tick| *tick <= crate::MAX_TICKS)
        .ok_or_else(|| CheckpointError::Limit("elapsed ticks exceed hard limit".into()))
}

pub(super) fn validate_advance(
    start: u32,
    executed: &CommandLog,
    log: &CommandLog,
    ticks: u32,
    additional: u64,
) -> Result<u32, CheckpointError> {
    checked_end(start, ticks)?;
    for entry in log.iter().take_while(|entry| entry.tick.0 < start) {
        let existing = executed
            .entries()
            .binary_search_by_key(&entry.key(), LoggedCommand::key);
        if existing.ok().map(|index| &executed.entries()[index]) != Some(entry) {
            return Err(CheckpointError::Invalid(format!(
                "continuation changes executed command at tick {}",
                entry.tick.0
            )));
        }
    }
    validate_future(start, executed.len(), log, ticks, additional)
}

fn validate_future(
    start: u32,
    executed_commands: usize,
    log: &CommandLog,
    ticks: u32,
    additional: u64,
) -> Result<u32, CheckpointError> {
    let end = checked_end(start, ticks)?;
    let mut count = 0usize;
    let mut previous_key = None;
    let first = log.entries().partition_point(|entry| entry.tick.0 < start);
    for entry in log.entries()[first..]
        .iter()
        .take_while(|entry| entry.tick.0 < end)
    {
        // CommandLog::record can saturate sequence numbers, so public builders
        // alone are not enough to prove uniqueness before changing history.
        if previous_key.is_some_and(|previous| previous >= entry.key()) {
            return Err(CheckpointError::Invalid(
                "continuation command keys are not unique and ordered".into(),
            ));
        }
        previous_key = Some(entry.key());
        count = count.saturating_add(1);
    }
    if (executed_commands as u64)
        .saturating_add(count as u64)
        .saturating_add(additional)
        > MAX_CHECKPOINT_COMMANDS as u64
    {
        return Err(CheckpointError::Limit("too many executed commands".into()));
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak_commands::{PlayerId, SimCommand, Tick, UnitId, UnitSet};

    #[test]
    fn future_only_admission_ignores_past_but_full_admission_verifies_it() {
        let scenario = tak_sim::testing::example_scenario(7, 2);
        let mut sim = tak_sim::Sim::new(&scenario).unwrap();
        let mut executed = CommandLog::new();
        executed.record(
            Tick(0),
            PlayerId(0),
            SimCommand::Stop {
                units: UnitSet::single(UnitId::new(0, 0)),
            },
        );
        sim.step(executed.for_tick(Tick(0)));
        let mut edited = CommandLog::new();
        edited.record(
            Tick(0),
            PlayerId(1),
            SimCommand::Stop {
                units: UnitSet::single(UnitId::new(1, 0)),
            },
        );
        let config = ControllerConfig::default();
        assert!(
            prepare_future_tick(sim.world(), &config, executed.len(), &edited)
                .unwrap()
                .is_empty()
        );
        assert!(prepare_driven_tick(sim.world(), &config, &executed, &edited).is_err());
        assert!(admit_driven(sim.world(), &config, &executed, &edited, 10).is_err());
    }
}
