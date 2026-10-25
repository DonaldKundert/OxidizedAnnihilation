//! Bounded, controller-first setup for the shared deterministic match engine.
//! Only identifiers and chosen training policy knobs live here, never retail
//! statistics, economy rules, or a second implementation of the opponent.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tak_commands::{CommandLog, PlayerId};
use tak_headless::bot::{
    ControllerConfig, Recipe, SIMPLE_SKIRMISH_V1, SkirmishController, Targeting,
};
use tak_headless::checkpoint::{CheckpointSource, ReplayBudget, ReplaySession};
use tak_math::Fx;
use tak_sim::{UnitDef, World};

use crate::game::{Session, SetupError};

/// Maps supported by this small setup flow. Other CLI maps remain unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkirmishMap {
    #[default]
    AdamantineGate,
}
impl SkirmishMap {
    pub const fn label(self) -> &'static str {
        "Adamantine Gate"
    }
    pub const fn path(self) -> &'static str {
        "kmap/Adamantine Gate"
    }
}

/// Supported local sides. The other side occupies player one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Faction {
    #[default]
    Aramon,
    Zhon,
}
impl Faction {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Aramon => "Aramon",
            Self::Zhon => "Zhon",
        }
    }
    pub const fn code(self) -> &'static str {
        match self {
            Self::Aramon => "ARA",
            Self::Zhon => "ZON",
        }
    }
    pub const fn opposite(self) -> Self {
        match self {
            Self::Aramon => Self::Zhon,
            Self::Zhon => Self::Aramon,
        }
    }
    pub const fn next(self) -> Self {
        self.opposite()
    }
}

/// No difficulty/retail-AI claims: these are explicit practice policies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Opponent {
    #[default]
    Training,
    PassivePractice,
}
impl Opponent {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Training => "Training opponent",
            Self::PassivePractice => "Passive practice",
        }
    }
    pub const fn next(self) -> Self {
        match self {
            Self::Training => Self::PassivePractice,
            Self::PassivePractice => Self::Training,
        }
    }
    pub const fn disclosure(self) -> &'static str {
        match self {
            Self::Training => {
                "simple-skirmish-v1: full-map/omniscient training opponent. Not original TA:K AI; no fog/scouting behavior."
            }
            Self::PassivePractice => {
                "No opponent planner. Enemy units retain normal simulation behavior, including automatic combat."
            }
        }
    }
}

/// Exact supported choices, including a retained seed for deterministic rematch.
/// Deserialization rejects unknown choices and fields instead of falling back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupConfig {
    pub map: SkirmishMap,
    pub faction: Faction,
    pub opponent: Opponent,
    pub seed: u64,
}
impl Default for SetupConfig {
    fn default() -> Self {
        Self {
            map: SkirmishMap::default(),
            faction: Faction::default(),
            opponent: Opponent::default(),
            seed: 1,
        }
    }
}
impl SetupConfig {
    pub const fn players(&self) -> &'static str {
        match self.faction {
            Faction::Aramon => "ARA,ZON",
            Faction::Zhon => "ZON,ARA",
        }
    }
    pub fn checkpoint_source(&self) -> CheckpointSource {
        CheckpointSource::map(self.map.path(), self.players(), self.seed, true)
    }
}
fn err(error: impl std::fmt::Display) -> SetupError {
    SetupError(error.to_string())
}

fn ordinary_troop(def: &UnitDef) -> bool {
    !def.build.builder
        && def.build.mobile
        && def.maxvelocity > Fx::ZERO
        && def
            .combat
            .weapons
            .iter()
            .any(|weapon| !weapon.is_spell() && weapon.damage.default > 0)
}

/// Resolve a deterministic recipe from actual loaded menu edges and validate it
/// with the shared engine adapter. Aramon uses a factory; Zhon uses direct
/// mobile production, optionally through a mobile intermediary builder.
/// Player zero is always protected human input, even in synthetic contract tests.
pub fn training_config(world: &World, opponent: Faction) -> Result<ControllerConfig, SetupError> {
    if world.players().len() != 2 {
        return Err(err("training setup requires exactly two players"));
    }
    let root = world
        .defs()
        .get(world.players()[1].monarch_def)
        .ok_or_else(|| err("opponent starting builder is missing"))?;
    let income = root
        .build
        .canbuild
        .iter()
        .filter_map(|id| world.defs().get(*id))
        .find(|def| def.economy.mogriumincome > 0 && def.maxvelocity == Fx::ZERO);
    let resolve = |producer: Option<&UnitDef>, troop: &UnitDef| {
        Recipe::resolve(
            world,
            &root.name,
            income.map(|def| def.name.as_str()),
            producer.map(|def| def.name.as_str()),
            &troop.name,
        )
    };
    // Menu edges are authoritative. Direct builders can list an expensive
    // end-game creature first, beyond normal storage. Prefer the least costly
    // ordinary troop, then intermediary cost and build time, retaining menu
    // order for exact ties. Mobile intermediaries use the same paid Build path.
    // No cost, capability or production rule is invented or overridden here.
    let recipe = match opponent {
        Faction::Zhon => {
            let mut candidates = Vec::new();
            for def in root
                .build
                .canbuild
                .iter()
                .filter_map(|id| world.defs().get(*id))
            {
                if ordinary_troop(def) {
                    if let Ok(recipe) = resolve(None, def) {
                        candidates.push(((def.build.buildcost, 0, def.build.buildtime), recipe));
                    }
                } else if def.build.builder
                    && def.build.mobile
                    && def.build.workertime > 0
                    && def.maxvelocity > Fx::ZERO
                {
                    for troop in def
                        .build
                        .canbuild
                        .iter()
                        .filter_map(|id| world.defs().get(*id))
                        .filter(|troop| ordinary_troop(troop))
                    {
                        if let Ok(recipe) = resolve(Some(def), troop) {
                            candidates.push((
                                (
                                    troop.build.buildcost,
                                    def.build.buildcost,
                                    troop.build.buildtime,
                                ),
                                recipe,
                            ));
                        }
                    }
                }
            }
            candidates
                .into_iter()
                .min_by_key(|(cost, _)| *cost)
                .map(|(_, recipe)| recipe)
        }
        Faction::Aramon => root
            .build
            .canbuild
            .iter()
            .filter_map(|id| world.defs().get(*id))
            .filter(|def| {
                def.build.builder
                    && !def.build.mobile
                    && def.build.workertime > 0
                    && def.maxvelocity == Fx::ZERO
            })
            .find_map(|factory| {
                factory
                    .build
                    .canbuild
                    .iter()
                    .filter_map(|id| world.defs().get(*id))
                    .filter(|def| ordinary_troop(def))
                    .find_map(|troop| resolve(Some(factory), troop).ok())
            }),
    }
    .ok_or_else(|| {
        err(format!(
            "installed {} opponent has no supported {} training recipe",
            opponent.label(),
            if opponent == Faction::Aramon {
                "factory"
            } else {
                "mobile-production"
            }
        ))
    })?;
    let config = ControllerConfig {
        human_players: vec![PlayerId(0)],
        controllers: vec![SkirmishController {
            policy: SIMPLE_SKIRMISH_V1.into(),
            player: PlayerId(1),
            recipe,
            cadence_ticks: 30,
            command_budget: 8,
            site_budget: 64,
            army_cap: 3,
            mana_reserve: 0,
            targeting: Targeting::OmniscientTrainingV1,
        }],
    };
    config.validate(world).map_err(err)?;
    crate::training::validate_local(&config)?;
    Ok(config)
}

/// Validate content and build the entire new session before replacing live play.
/// COB scripts, source identity, command driver and saves use the existing path.
pub fn prepare_session(data: &Path, config: &SetupConfig) -> Result<Session, SetupError> {
    prepare_session_with_budget(data, config, ReplayBudget::default())
}

/// Same setup while retaining the caller's explicit save/replay limits.
pub fn prepare_session_with_budget(
    data: &Path,
    config: &SetupConfig,
    budget: ReplayBudget,
) -> Result<Session, SetupError> {
    let source = config.checkpoint_source();
    let controllers = match config.opponent {
        Opponent::Training => {
            let initial = ReplaySession::new(source.clone(), Some(data)).map_err(err)?;
            training_config(initial.sim().world(), config.faction.opposite())?
        }
        Opponent::PassivePractice => ControllerConfig {
            human_players: vec![PlayerId(0)],
            controllers: vec![],
        },
    };
    // A second canonical load verifies the resolved recipe against the exact
    // source retained for save/replay, not an independently constructed Sim.
    let replay =
        ReplaySession::new_with_controllers(source, Some(data), controllers).map_err(err)?;
    let checkpoint = replay.checkpoint().map_err(err)?;
    crate::saves::from_replay_with_budget(data, budget, checkpoint, replay, CommandLog::new())
}

/// Recover only an exact supported setup. Custom policies/recipes are never
/// silently converted to the built-in preset by the Rematch button.
pub fn from_session(session: &Session) -> Option<SetupConfig> {
    if session.local != PlayerId(0) {
        return None;
    }
    let CheckpointSource::Map {
        map,
        players,
        world_seed,
        scripts: true,
    } = session.save_origin.as_ref()?.source()
    else {
        return None;
    };
    if !map.eq_ignore_ascii_case(SkirmishMap::AdamantineGate.path()) || players.len() != 2 {
        return None;
    }
    let faction = match (players[0].as_str(), players[1].as_str()) {
        ("ARA", "ZON") => Faction::Aramon,
        ("ZON", "ARA") => Faction::Zhon,
        _ => return None,
    };
    let controllers = session.controllers();
    let opponent = if controllers.controllers.is_empty()
        && (controllers.is_empty() || controllers.human_players == [PlayerId(0)])
    {
        Opponent::PassivePractice
    } else if *controllers == training_config(session.sim.world(), faction.opposite()).ok()? {
        Opponent::Training
    } else {
        return None;
    };
    Some(SetupConfig {
        map: SkirmishMap::AdamantineGate,
        faction,
        opponent,
        seed: *world_seed,
    })
}
