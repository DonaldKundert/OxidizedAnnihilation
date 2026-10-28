//! Cursor picking and context-order resolution.

use tak_commands::{QueueMode, SimCommand, SpellId, Target, UnitDefId, UnitSet, WorldPos};

use crate::world::{BuilderKind, UnitInfo, WorldView};

/// A pending order mode: the next context action completes it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// Plain context orders.
    #[default]
    None,
    /// Place a building of this type.
    Build(UnitDefId),
    /// Patrol to the cursor.
    Patrol,
    /// Guard the unit under the cursor.
    Guard,
    /// Clear a reclaimable map feature or wreck.
    Clear,
    /// Heal an owned unit, or assist its construction.
    Heal,
    /// Cast this spell at the cursor.
    CastSpell(SpellId),
    /// Attack the unit under the cursor, or the ground (TA A).
    Attack,
    /// Move to the cursor, engaging enemies and resuming the destination.
    AttackMove,
    /// Move to the cursor, even over a unit (TA M).
    Move,
}

/// The pickable unit under `pos`: the nearest whose pick radius contains it,
/// ties broken by lower id.
pub fn pick_unit<W: WorldView + ?Sized>(world: &W, pos: WorldPos) -> Option<UnitInfo> {
    let mut best: Option<(tak_math::Fx, UnitInfo)> = None;
    world.for_each_unit(&mut |u| {
        if !u.selectable {
            return;
        }
        let d = u.pos.distance(pos);
        if d > u.radius {
            return;
        }
        let better = match &best {
            None => true,
            Some((bd, b)) => d < *bd || (d == *bd && u.id < b.id),
        };
        if better {
            best = Some((d, *u));
        }
    });
    best.map(|(_, u)| u)
}

/// The units of `selection` that still exist and belong to the local player.
pub fn own_units<W: WorldView + ?Sized>(world: &W, selection: &UnitSet) -> UnitSet {
    let me = world.local_player();
    selection
        .iter()
        .copied()
        .filter(|&id| world.unit(id).is_some_and(|u| u.owner == me))
        .collect()
}

/// Operational owned builders of `units` that can build `def`, optionally
/// only of one kind. Incomplete frames may be targets, never workers.
pub fn builders_for<W: WorldView + ?Sized>(
    world: &W,
    units: &UnitSet,
    def: UnitDefId,
    kind: Option<BuilderKind>,
) -> UnitSet {
    units
        .iter()
        .copied()
        .filter(|&id| {
            world.can_build(id)
                && world.unit(id).is_some_and(|u| {
                    u.builder.is_some() && kind.is_none_or(|k| u.builder == Some(k))
                })
                && world.build_options(id).contains(&def)
        })
        .collect()
}

/// Complete, selectable mobile builders of the local player that can heal.
/// Missing units and adapter-specific capability failures are excluded.
pub fn healers_for<W: WorldView + ?Sized>(world: &W, units: &UnitSet) -> UnitSet {
    let me = world.local_player();
    units
        .iter()
        .copied()
        .filter(|&id| {
            world.unit(id).is_some_and(|u| {
                u.owner == me
                    && u.selectable
                    && !u.under_construction
                    && u.builder == Some(BuilderKind::Mobile)
            }) && world.can_heal(id)
        })
        .collect()
}

/// Complete, selectable owned units with explicit Clear capability.
/// Unlike Heal this may include nonbuilders or immobile units: the sim decides range.
pub fn clearers_for<W: WorldView + ?Sized>(world: &W, units: &UnitSet) -> UnitSet {
    let me = world.local_player();
    units
        .iter()
        .copied()
        .filter(|&id| {
            world
                .unit(id)
                .is_some_and(|u| u.owner == me && u.selectable && !u.under_construction)
                && world.can_clear(id)
        })
        .collect()
}

/// Units of `units` that can cast `spell`.
pub fn casters_for<W: WorldView + ?Sized>(world: &W, units: &UnitSet, spell: SpellId) -> UnitSet {
    units
        .iter()
        .copied()
        .filter(|&id| world.spells(id).contains(&spell))
        .collect()
}

/// The order a context action issues for `units` (already filtered to own,
/// living units) with the cursor at `cursor` over `hovered`:
///
/// - `Mode::Build(def)`: `Build { at: Some(cursor) }` by mobile builders
///   that can build `def`.
/// - `Mode::Patrol`: `Patrol { to: cursor }`.
/// - `Mode::Guard`: `Guard` the hovered non-enemy unit; nothing over ground
///   or an enemy.
/// - `Mode::Clear`: clear the current reclaimable feature under the cursor;
///   invalid clicks never fall back to moving or attacking.
/// - `Mode::Heal`: heal an owned hovered unit (including unfinished frames),
///   by complete mobile builders other than the target; nothing over ground,
///   another player's unit or the only selected healer itself.
/// - `Mode::CastSpell(s)`: `CastSpell` at the hovered unit or the ground, by
///   units that know `s`.
/// - `Mode::Attack`: `Attack` the hovered unit (outside the selection), or
///   the ground under the cursor.
/// - `Mode::Move`: `Move { to: cursor }`, whatever is under it.
/// - `Mode::AttackMove`: `AttackMove { to: cursor }`, whatever is under it.
/// - No mode: hovered enemy → `Attack`; hovered friendly unit outside the
///   selection → `Guard`; otherwise → `Move { to: cursor }`.
///
/// `None` when no unit can carry out the order.
pub fn resolve_context<W: WorldView + ?Sized>(
    world: &W,
    units: &UnitSet,
    cursor: WorldPos,
    hovered: Option<&UnitInfo>,
    mode: Mode,
    queue: QueueMode,
) -> Option<SimCommand> {
    if units.is_empty() {
        return None;
    }
    let nonempty = |s: UnitSet| if s.is_empty() { None } else { Some(s) };
    match mode {
        Mode::Build(def) => Some(SimCommand::Build {
            units: nonempty(builders_for(world, units, def, Some(BuilderKind::Mobile)))?,
            def,
            at: Some(cursor),
            queue,
        }),
        Mode::Patrol => Some(SimCommand::Patrol {
            units: units.clone(),
            to: cursor,
            queue,
        }),
        Mode::Guard => {
            let h = hovered.filter(|h| !world.is_enemy(h.owner))?;
            Some(SimCommand::Guard {
                units: units.clone(),
                target: h.id,
                queue,
            })
        }
        Mode::Clear => Some(SimCommand::Clear {
            units: nonempty(clearers_for(world, units))?,
            target: world.clear_target_at(cursor)?,
            queue,
        }),
        Mode::Heal => {
            // Resolve the handle again so direct callers cannot target a stale
            // snapshot that has disappeared or changed ownership.
            let h = world.unit(hovered?.id)?;
            if h.owner != world.local_player() || !h.selectable {
                return None;
            }
            let healers = healers_for(world, units)
                .iter()
                .copied()
                .filter(|&id| id != h.id)
                .collect();
            Some(SimCommand::Heal {
                units: nonempty(healers)?,
                target: h.id,
                queue,
            })
        }
        Mode::CastSpell(spell) => Some(SimCommand::CastSpell {
            units: nonempty(casters_for(world, units, spell))?,
            spell,
            target: Some(match hovered {
                Some(h) => Target::Unit(h.id),
                None => Target::Ground(cursor),
            }),
            queue,
        }),
        Mode::Attack => Some(SimCommand::Attack {
            units: units.clone(),
            target: match hovered {
                Some(h) if !units.contains(h.id) => Target::Unit(h.id),
                _ => Target::Ground(cursor),
            },
            queue,
        }),
        Mode::Move => Some(SimCommand::Move {
            units: units.clone(),
            to: cursor,
            queue,
        }),
        Mode::AttackMove => Some(SimCommand::AttackMove {
            units: units.clone(),
            to: cursor,
            queue,
        }),
        Mode::None => Some(match hovered {
            Some(h) if world.is_enemy(h.owner) => SimCommand::Attack {
                units: units.clone(),
                target: Target::Unit(h.id),
                queue,
            },
            Some(h) if !units.contains(h.id) => SimCommand::Guard {
                units: units.clone(),
                target: h.id,
                queue,
            },
            _ => SimCommand::Move {
                units: units.clone(),
                to: cursor,
                queue,
            },
        }),
    }
}
