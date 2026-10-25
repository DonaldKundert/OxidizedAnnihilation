//! Explicitly opt-in, command-only training opponents. This is a chosen engine
//! policy, not a reconstruction of retail AI. The v1 target policy is
//! deliberately omniscient: it reads hostile units anywhere in the world.
//!
//! Planning has no mutable world access, clock, RNG or hidden continuation
//! state. All decisions depend on the current world, persisted configuration
//! and absolute tick. Commands still pass through normal simulation validation,
//! construction, charging, movement and combat.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use tak_commands::{LoggedCommand, PlayerId, QueueMode, SimCommand, Target, UnitDefId, UnitSet};
use tak_sim::construction::{placement, progress};
use tak_sim::{MatchState, Order, Unit, UnitDef, World, WorldPos, economy};

pub mod demo;

/// Stable version/name of the only implemented controller.
pub const SIMPLE_SKIRMISH_V1: &str = "simple-skirmish-v1";
/// Hard per-owner command bound at one decision.
pub const MAX_COMMAND_BUDGET: u32 = 32;
/// Hard per-owner placement-check bound at one decision.
pub const MAX_SITE_BUDGET: u32 = 256;
/// Hard living/promised troop cap per owner.
pub const MAX_ARMY_CAP: u32 = 128;

/// Bad setup is rejected before a driven session advances.
#[derive(Debug, thiserror::Error)]
#[error("skirmish controller: {0}")]
pub struct BotError(pub String);

/// An exact resolved definition identity, retained alongside the readable name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    /// Definition index in the verified source content.
    pub id: UnitDefId,
    /// Exact internal name at that index.
    pub name: String,
}

impl Definition {
    /// Resolve a unique case-insensitive internal name, never a display prefix.
    pub fn resolve(world: &World, name: &str) -> Result<Self, BotError> {
        let mut found = world
            .defs()
            .as_slice()
            .iter()
            .enumerate()
            .filter(|(_, def)| def.name.eq_ignore_ascii_case(name));
        let (index, def) = found
            .next()
            .ok_or_else(|| BotError(format!("unknown unit {name:?}")))?;
        if found.next().is_some() {
            return Err(BotError(format!("ambiguous unit {name:?}")));
        }
        Ok(Self {
            id: UnitDefId(index as u32),
            name: def.name.clone(),
        })
    }

    fn validate<'a>(&self, world: &'a World) -> Result<&'a UnitDef, BotError> {
        world
            .defs()
            .get(self.id)
            .filter(|def| def.name == self.name && !self.name.is_empty())
            .ok_or_else(|| BotError(format!("definition identity mismatch for {:?}", self.name)))
    }
}

/// One explicit income -> optional producer -> troop recipe. The producer
/// may be an immobile factory or a mobile intermediary builder. Without one,
/// the mobile root builder constructs the troop directly. No faction/profile/
/// mission-string guessing occurs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Mobile starting builder, normally the owner's monarch.
    pub builder: Definition,
    /// Optional single income structure.
    pub income: Option<Definition>,
    /// Optional single working factory or mobile intermediary builder.
    pub producer: Option<Definition>,
    /// Ordinary mobile nonbuilder combat unit.
    pub troop: Definition,
}

impl Recipe {
    /// Resolve names and validate actual build menu edges and capabilities.
    pub fn resolve(
        world: &World,
        builder: &str,
        income: Option<&str>,
        producer: Option<&str>,
        troop: &str,
    ) -> Result<Self, BotError> {
        let recipe = Self {
            builder: Definition::resolve(world, builder)?,
            income: income
                .map(|name| Definition::resolve(world, name))
                .transpose()?,
            producer: producer
                .map(|name| Definition::resolve(world, name))
                .transpose()?,
            troop: Definition::resolve(world, troop)?,
        };
        recipe.validate(world)?;
        Ok(recipe)
    }

    fn validate(&self, world: &World) -> Result<(), BotError> {
        let root = self.builder.validate(world)?;
        if !worker(root) || !root.build.mobile || root.maxvelocity <= tak_math::Fx::ZERO {
            return Err(BotError(
                "recipe root must be a working mobile builder".into(),
            ));
        }
        let mut identities = BTreeSet::from([self.builder.id, self.troop.id]);
        if identities.len() != 2 {
            return Err(BotError(
                "recipe roles must use distinct definitions".into(),
            ));
        }
        for role in self.income.iter().chain(self.producer.iter()) {
            role.validate(world)?;
            if !identities.insert(role.id) || !root.build.canbuild.contains(&role.id) {
                return Err(BotError(
                    "recipe root menu must contain distinct income/producer roles".into(),
                ));
            }
        }
        if let Some(income) = &self.income {
            let income = income.validate(world)?;
            if income.maxvelocity > tak_math::Fx::ZERO {
                return Err(BotError("income must be a structure".into()));
            }
            if income.economy.mogriumincome <= 0 {
                return Err(BotError("income role has no ordinary income".into()));
            }
        }
        let source = if let Some(producer) = &self.producer {
            let factory = producer.validate(world)?;
            if !worker(factory) {
                return Err(BotError(
                    "producer must be a working factory or mobile builder".into(),
                ));
            }
            if factory.build.mobile != (factory.maxvelocity > tak_math::Fx::ZERO) {
                return Err(BotError(
                    "producer movement must match its production mode".into(),
                ));
            }
            factory
        } else {
            root
        };
        let troop = self.troop.validate(world)?;
        if !source.build.canbuild.contains(&self.troop.id) || !ordinary_troop(troop) {
            return Err(BotError(
                "troop must be an ordinary mobile nonbuilder on the actual production menu".into(),
            ));
        }
        if world
            .players()
            .iter()
            .any(|player| player.monarch_def == self.troop.id)
        {
            return Err(BotError(
                "a monarch definition cannot be the attack troop".into(),
            ));
        }
        Ok(())
    }
}

/// Explicit information advantage. This does not implement fog, scouting, or
/// the original game's AI targeting behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Targeting {
    /// Nearest hostile live target anywhere, integer distance then arena ID.
    #[serde(rename = "omniscient-training-v1")]
    OmniscientTrainingV1,
}

/// Complete stateless controller identity. Every field is saved/checksummed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkirmishController {
    /// Must equal `simple-skirmish-v1`; unknown versions fail closed.
    pub policy: String,
    /// Explicit simulation owner, not an OTA/header owner inference.
    pub player: PlayerId,
    /// Exact production recipe.
    pub recipe: Recipe,
    /// First decision is tick 1, then every this many absolute ticks.
    pub cadence_ticks: u32,
    /// At most this many ordinary commands per decision (one build maximum).
    pub command_budget: u32,
    /// At most this many legal-site checks per decision.
    pub site_budget: u32,
    /// Living troops, frames and pending troop orders together.
    pub army_cap: u32,
    /// Whole mana retained in addition to a new build's full ordinary cost.
    pub mana_reserve: u32,
    /// Named visibility/targeting policy.
    pub targeting: Targeting,
}

/// All defaults are off. Owner lists must be sorted and unique; controller and
/// human owners must be disjoint. External future input for a bot owner is
/// rejected rather than interleaved or silently overwritten.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerConfig {
    /// Explicit local/human-controlled owners, protected from takeover.
    pub human_players: Vec<PlayerId>,
    /// Explicit enabled controllers in ascending player order.
    pub controllers: Vec<SkirmishController>,
}

impl ControllerConfig {
    /// True only for legacy/default metadata with nothing to persist.
    pub fn is_empty(&self) -> bool {
        self.human_players.is_empty() && self.controllers.is_empty()
    }

    /// Validate source-independent shape, before loading/replaying any content.
    pub fn validate_shape(&self) -> Result<(), BotError> {
        let humans = &self.human_players;
        if humans.len() > tak_sim::scenario::MAX_PLAYERS || humans.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(BotError("human owner IDs must be sorted and unique".into()));
        }
        if self.controllers.len() > tak_sim::scenario::MAX_PLAYERS
            || self
                .controllers
                .windows(2)
                .any(|w| w[0].player >= w[1].player)
        {
            return Err(BotError(
                "controller owner IDs must be sorted and unique".into(),
            ));
        }
        for controller in &self.controllers {
            if controller.policy != SIMPLE_SKIRMISH_V1 || humans.contains(&controller.player) {
                return Err(BotError(
                    "unknown policy version or a human-controlled bot owner".into(),
                ));
            }
            if !(1..=3600).contains(&controller.cadence_ticks)
                || !(1..=MAX_COMMAND_BUDGET).contains(&controller.command_budget)
                || !(1..=MAX_SITE_BUDGET).contains(&controller.site_budget)
                || !(1..=MAX_ARMY_CAP).contains(&controller.army_cap)
                || controller.mana_reserve > 1_000_000
            {
                return Err(BotError(
                    "cadence, command/site budgets, army cap or reserve out of range".into(),
                ));
            }
            for role in std::iter::once(&controller.recipe.builder)
                .chain(controller.recipe.income.iter())
                .chain(controller.recipe.producer.iter())
                .chain(std::iter::once(&controller.recipe.troop))
            {
                if role.name.is_empty()
                    || role.name.len() > 256
                    || role.name.chars().any(char::is_control)
                {
                    return Err(BotError("invalid recipe definition name".into()));
                }
            }
        }
        Ok(())
    }

    /// Validate against verified loaded content. Mission setups are out of scope.
    pub fn validate(&self, world: &World) -> Result<(), BotError> {
        self.validate_shape()?;
        if !self.controllers.is_empty() && world.victory().setup().mission.is_some() {
            return Err(BotError(
                "simple-skirmish-v1 does not support campaigns/missions".into(),
            ));
        }
        if self
            .human_players
            .iter()
            .any(|&player| !world.has_player(player))
        {
            return Err(BotError("human owner out of range".into()));
        }
        for controller in &self.controllers {
            let Some(player) = world.players().get(controller.player.0 as usize) else {
                return Err(BotError("controller owner out of range".into()));
            };
            controller.recipe.validate(world)?;
            if player.monarch_def != controller.recipe.builder.id {
                return Err(BotError(
                    "recipe root must match the owner's explicit starting builder".into(),
                ));
            }
        }
        Ok(())
    }

    /// Whether this owner belongs exclusively to an enabled controller.
    pub fn controls(&self, player: PlayerId) -> bool {
        self.controllers
            .iter()
            .any(|controller| controller.player == player)
    }

    /// Conservative maximum generated commands in `[start,end)`, checked before
    /// advancing. No sequence saturation is possible with the bounded budgets.
    pub(crate) fn maximum_commands(&self, start: u32, end: u32) -> u64 {
        self.controllers
            .iter()
            .map(|controller| {
                let decisions_before = |tick: u32| {
                    if tick <= 1 {
                        0
                    } else {
                        u64::from((tick - 2) / controller.cadence_ticks + 1)
                    }
                };
                decisions_before(end).saturating_sub(decisions_before(start))
                    * u64::from(controller.command_budget)
            })
            .sum()
    }

    /// Shared headless/client read-only planning entry point. Validates before
    /// producing a bounded, canonical command slice for the world's next tick.
    pub fn plan(&self, world: &World) -> Result<Vec<LoggedCommand>, BotError> {
        self.validate(world)?;
        Ok(self.plan_validated(world))
    }

    pub(crate) fn plan_validated(&self, world: &World) -> Vec<LoggedCommand> {
        let mut result = Vec::new();
        if world.tick().0 == 0 || !matches!(world.victory().state(), MatchState::Running) {
            return result;
        }
        for controller in &self.controllers {
            if world.is_defeated(controller.player)
                || (world.tick().0 - 1) % controller.cadence_ticks != 0
            {
                continue;
            }
            let mut commands = Vec::new();
            if let Some(build) = plan_build(world, controller) {
                commands.push(build);
            }
            for (id, unit) in world.units() {
                if commands.len() >= controller.command_budget as usize {
                    break;
                }
                if unit.owner != controller.player
                    || unit.def != controller.recipe.troop.id
                    || !idle(unit)
                    || !tak_sim::combat::is_target(world, id)
                {
                    continue;
                }
                if world
                    .players()
                    .iter()
                    .any(|player| player.monarch == Some(id))
                {
                    continue;
                }
                let target = world
                    .units()
                    .filter(|(target, unit)| {
                        world.are_hostile(controller.player, unit.owner)
                            && !world.is_defeated(unit.owner)
                            && tak_sim::combat::is_target(world, *target)
                    })
                    .min_by_key(|(target, other)| (distance_squared(unit.pos, other.pos), *target))
                    .map(|(target, _)| target);
                if let Some(target) = target {
                    commands.push(SimCommand::Attack {
                        units: UnitSet::single(id),
                        target: Target::Unit(target),
                        queue: QueueMode::Replace,
                    });
                }
            }
            result.extend(
                commands
                    .into_iter()
                    .enumerate()
                    .map(|(seq, command)| LoggedCommand {
                        tick: world.tick(),
                        player: controller.player,
                        seq: seq as u32,
                        command,
                    }),
            );
        }
        result
    }
}

fn worker(def: &UnitDef) -> bool {
    def.build.builder && def.build.workertime > 0
}
fn ordinary_troop(def: &UnitDef) -> bool {
    !def.build.builder
        && def.build.mobile
        && def.maxvelocity > tak_math::Fx::ZERO
        && def.combat.max_damage > 0
        && def
            .combat
            .weapons
            .iter()
            .any(|weapon| !weapon.is_spell() && weapon.damage.default > 0)
}
fn idle(unit: &Unit) -> bool {
    unit.build.is_complete()
        && !unit.combat.is_dying()
        && unit.orders.is_empty()
        && unit.combat.order.is_none()
        && unit.combat.attack_move.is_none()
        && unit.build.job.is_none()
        && unit.build.factory.product.is_none()
        && unit.build.assist.is_none()
}
fn distance_squared(a: WorldPos, b: WorldPos) -> i128 {
    let x = i128::from(a.x.raw()) - i128::from(b.x.raw());
    let y = i128::from(a.y.raw()) - i128::from(b.y.raw());
    x * x + y * y
}

/// Count each live frame/product once, plus accepted unmaterialized Build
/// orders. Active Build orders backed by a live frame are not double-counted.
pub fn committed_count(world: &World, owner: PlayerId, def: UnitDefId) -> usize {
    let mut count = 0usize;
    for (_, unit) in world.units().filter(|(_, unit)| unit.owner == owner) {
        count += usize::from(unit.def == def);
        let backed = unit
            .build
            .job
            .and_then(|job| job.target)
            .or(unit.build.factory.product)
            .is_some_and(|target| {
                world
                    .unit(target)
                    .is_some_and(|product| product.owner == owner && product.def == def)
            });
        let mut skip_backed = backed;
        for order in &unit.orders {
            if matches!(order, Order::Build { def: target, .. } if *target == def) {
                if skip_backed {
                    skip_backed = false;
                } else {
                    count += 1;
                }
            }
        }
    }
    count
}

fn plan_build(world: &World, controller: &SkirmishController) -> Option<SimCommand> {
    let recipe = &controller.recipe;
    let (actor_def, target) = if let Some(income) = &recipe.income
        && committed_count(world, controller.player, income.id) == 0
    {
        (recipe.builder.id, income.id)
    } else if let Some(producer) = &recipe.producer
        && committed_count(world, controller.player, producer.id) == 0
    {
        (recipe.builder.id, producer.id)
    } else {
        if committed_count(world, controller.player, recipe.troop.id)
            >= controller.army_cap as usize
        {
            return None;
        }
        (
            recipe.producer.as_ref().unwrap_or(&recipe.builder).id,
            recipe.troop.id,
        )
    };
    let target_def = world.defs().get(target)?;
    let required = progress::cost_raw(target_def.build.buildcost)
        .saturating_add(i64::from(controller.mana_reserve) << 16);
    if economy::mana(world, controller.player).0 < required {
        return None;
    }
    let (actor, unit) = world.units().find(|(_, unit)| {
        unit.owner == controller.player
            && unit.def == actor_def
            && idle(unit)
            && !matches!(
                unit.path.outcome,
                tak_sim::pathfinding::Outcome::Unreachable | tak_sim::pathfinding::Outcome::GaveUp
            )
    })?;
    let def = world.defs().get(unit.def)?;
    if !worker(def) || !def.build.canbuild.contains(&target) {
        return None;
    }
    let at = if def.build.mobile {
        Some(find_site(world, target, unit.pos, controller.site_budget)?)
    } else {
        None
    };
    Some(SimCommand::Build {
        units: UnitSet::single(actor),
        def: target,
        at,
        queue: QueueMode::Replace,
    })
}

fn find_site(world: &World, target: UnitDefId, origin: WorldPos, budget: u32) -> Option<WorldPos> {
    let def = world.defs().get(target)?;
    if def.economy.sacred_site {
        let mut sites: Vec<_> = world
            .economy
            .sacred_sites()
            .iter()
            .enumerate()
            .map(|(index, site)| {
                let center = WorldPos::from_ints(site.x + site.w / 2, site.y + site.h / 2);
                (
                    distance_squared(origin, center),
                    index,
                    placement::site_center(def, center),
                )
            })
            .collect();
        sites.sort_by_key(|&(distance, index, _)| (distance, index));
        return sites
            .into_iter()
            .take(budget as usize)
            .map(|(_, _, site)| site)
            .find(|&site| {
                economy::free_site_at(world, site, None).is_some()
                    && placement::check_site(world, target, site, None).is_ok()
            });
    }
    // A fixed local square, nearest first then row/column. The minimum two-cell
    // gap avoids building on top of the ordinary 2x2 root builder's centre.
    let mut offsets: Vec<_> = (-9i32..=9)
        .flat_map(|y| (-9i32..=9).map(move |x| (x, y)))
        .filter(|&(x, y)| x.abs().max(y.abs()) >= 2)
        .collect();
    offsets.sort_by_key(|&(x, y)| (x * x + y * y, y, x));
    offsets
        .into_iter()
        .take(budget as usize)
        .filter_map(|(x, y)| {
            let x = origin.x.to_int_floor().saturating_add(x * 16);
            let y = origin.y.to_int_floor().saturating_add(y * 16);
            (x >= 0 && y >= 0 && x < world.map().width && y < world.map().height)
                .then(|| placement::site_center(def, WorldPos::from_ints(x, y)))
        })
        .find(|&site| placement::check_site(world, target, site, None).is_ok())
}
