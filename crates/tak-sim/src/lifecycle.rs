//! Spawning and killing units mid-match, with their scripts. Every system
//! that creates or destroys units goes through here so scripts stay in step
//! with the arena.

use crate::script::{ScriptCtx, ScriptHost, ScriptReturn};
use crate::world::World;
use tak_commands::{PlayerId, UnitDefId, UnitId, WorldPos};
use tak_math::Angle;

/// The read-only script context for a unit, if it is alive.
pub fn script_ctx(world: &World, id: UnitId) -> Option<ScriptCtx> {
    let u = world.unit(id)?;
    Some(ScriptCtx {
        tick: world.tick(),
        unit: id,
        def: u.def,
        owner: u.owner,
        pos: u.pos,
        ground_height: u.ground,
        terrain: u.terrain,
        heading: u.heading,
        speed: u.speed,
        moving: !u.orders.is_empty(),
        fire_state: u.fire_state,
        health_percent: u.combat.health_percent(),
        build_percent_left: u.build.percent_left(),
    })
}

/// Spawn a unit and start its `Create` script. `None` if the arena is full
/// or `def` does not exist.
pub fn spawn_unit(
    world: &mut World,
    scripts: &mut dyn ScriptHost,
    def: UnitDefId,
    owner: PlayerId,
    pos: WorldPos,
    heading: Angle,
) -> Option<UnitId> {
    let id = world.spawn(def, owner, pos, heading)?;
    if let Some(ctx) = script_ctx(world, id) {
        scripts.create_unit(&ctx, &mut world.rng);
    }
    Some(id)
}

/// Call a script function on a unit now (COB `call-script`). `None` if the
/// unit is dead, has no script, or lacks the function.
pub fn call_script(
    world: &mut World,
    scripts: &mut dyn ScriptHost,
    id: UnitId,
    name: &str,
    args: &[i32],
) -> Option<ScriptReturn> {
    let ctx = script_ctx(world, id)?;
    scripts.call_script(&ctx, &mut world.rng, name, args)
}

/// Remove a unit and its script. The caller runs `Killed` first if it wants
/// the death animation; this only tears down. Returns whether it existed.
pub fn remove_unit(world: &mut World, scripts: &mut dyn ScriptHost, id: UnitId) -> bool {
    crate::construction::cancel_heal(world, scripts, id);
    scripts.remove_unit(id);
    world.despawn(id).is_some()
}
