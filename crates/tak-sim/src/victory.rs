//! Victory conditions (WP-U): per-player defeat, the end of the match, and
//! the mission conditions of a map's OTA `[GLOBALHEADER]`. Rules, sources
//! and confidence tags: `docs/sim.md`, "Victory and defeat".
//!
//! [`tick`] runs once per tick right after combat (`systems.rs`). It works
//! in one of two modes, chosen at setup ([`VictorySetup::mission`]):
//!
//! - **Skirmish** (default; TA's skirmish and multiplayer sessions, which
//!   never read the OTA conditions): a player is defeated when its monarch
//!   dies ([`DefeatRule::MonarchKilled`], the lobby's "Monarch Expendable"
//!   box unticked) or, under either rule, when it has no units left. Newly
//!   defeated players' units self-destruct ([`DefeatedUnits`]). The match
//!   is over when the undefeated players all share one team.
//! - **Mission** (TA's campaign sessions): the OTA conditions, parsed into
//!   [`MissionRules`]. Player 0 is "local", player 1 the "enemy" (TA's
//!   literal slots). Victory when every victory condition holds, defeat
//!   when any defeat condition holds; victory wins a tie.
//!
//! Defeated players' commands are dropped ([`crate::systems::apply_commands`]).
//! Once [`MatchState::Over`], nothing here changes again; the sim keeps
//! ticking.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tak_commands::{PlayerId, Tick, UnitDefId, UnitId};

use crate::TICKS_PER_SECOND;
use crate::defs::UnitDef;
use crate::hash::StateHasher;
use crate::script::ScriptHost;
use crate::world::World;

/// Whether the match is still being played.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MatchState {
    /// No result yet.
    #[default]
    Running,
    /// Decided. `winners` are the undefeated players in id order; empty for
    /// a draw (everyone defeated on the same tick).
    Over {
        /// The players who won.
        winners: Vec<PlayerId>,
    },
}

/// What defeats a player in a skirmish.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefeatRule {
    /// The monarch's death (the lobby default, "Monarch Expendable"
    /// unticked). Losing every unit defeats too.
    #[default]
    MonarchKilled,
    /// Only losing every unit ("Monarch Expendable": the game continues
    /// after the monarch dies).
    AllUnitsKilled,
}

/// What happens to a defeated player's remaining units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefeatedUnits {
    /// Every unit takes [`DEFEAT_DAMAGE`] through the ordinary damage path
    /// and dies with its death animation (TA's commander-death sweep).
    #[default]
    SelfDestruct,
    /// Units stay where they are, still owned by the defeated player,
    /// defending themselves but taking no more orders.
    Remain,
}

/// Match-level victory rules, chosen at setup (`Scenario::victory`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VictorySetup {
    /// Skirmish defeat rule.
    pub defeat: DefeatRule,
    /// Skirmish: what a defeated player's units do.
    pub defeated_units: DefeatedUnits,
    /// Team per player (index = player id); `None` or a missing entry is a
    /// team of one. Players on the same team win together and do not
    /// automatically target each other. Explicit attacks and damage are
    /// unchanged; this does not imply shared vision, control or resources.
    pub teams: Vec<Option<u8>>,
    /// Mission conditions; `None` plays a skirmish.
    pub mission: Option<MissionRules>,
}

impl VictorySetup {
    /// `true` when nothing differs from the default (for JSON).
    pub fn is_default(&self) -> bool {
        *self == VictorySetup::default()
    }

    pub(crate) fn validate(
        &self,
        players: usize,
        defs: usize,
    ) -> Result<(), crate::scenario::ScenarioError> {
        let bad = |m: String| Err(crate::scenario::ScenarioError::Invalid(m));
        if self.teams.len() > players {
            return bad("victory.teams has more entries than players".into());
        }
        if let Some(m) = &self.mission {
            for c in m.victory.iter().chain(&m.defeat) {
                if c.unit_type()
                    .and_then(|t| t.def)
                    .is_some_and(|d| d.0 as usize >= defs)
                {
                    return bad(format!("victory condition {c:?}: unit def out of range"));
                }
            }
        }
        Ok(())
    }

    // A single key definition for victory and World's read-only diplomacy
    // predicates. Explicit u8 teams cannot collide with singleton players.
    pub(crate) fn team(&self, p: usize) -> u32 {
        match self.teams.get(p).copied().flatten() {
            Some(t) => u32::from(t),
            None => 0x100 + p as u32,
        }
    }
}

/// Damage the defeat sweep deals each unit (TA: 30000, damage kind 3).
pub const DEFEAT_DAMAGE: i32 = 30_000;

/// Mission "local" player (OTA `Player=1`).
pub const LOCAL: PlayerId = PlayerId(0);
/// Mission "enemy" player (OTA `Player=2`).
pub const ENEMY: PlayerId = PlayerId(1);

/// `UnitTypePasses…`: a unit is on the line when its cell is closer than
/// this (TA: two cells either side).
pub const PASS_TOLERANCE_CELLS: i32 = 3;

/// World units per OTA condition cell (Kingdoms authors cells).
pub const CELL: i32 = 16;

/// A unit type named by a condition, resolved against the scenario's
/// definitions by case-insensitive name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitType {
    /// The name as authored.
    pub name: String,
    /// The matching definition; `None` matches no unit.
    pub def: Option<UnitDefId>,
}

impl UnitType {
    fn resolve(name: &str, defs: &[UnitDef]) -> UnitType {
        let name = name.trim();
        UnitType {
            name: name.to_string(),
            def: defs
                .iter()
                .position(|d| d.name.eq_ignore_ascii_case(name))
                .map(|i| UnitDefId(i as u32)),
        }
    }

    fn is(&self, def: UnitDefId) -> bool {
        self.def == Some(def)
    }
}

/// One OTA win or lose condition. Positions and radii are in 16-unit map
/// cells; times in seconds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    // ---- victory ----
    /// The enemy has no units.
    DestroyAllUnits,
    /// The enemy's monarch has been removed.
    KillEnemyCommander,
    /// The enemy's last mobile unit has been removed.
    KillAllMobileUnits,
    /// The enemy's last unit of this type has been removed.
    KillAllOfType(UnitType),
    /// This many enemy units of this type have been removed.
    KillUnitType(UnitType, i32),
    /// A local unit of this type is complete.
    BuildUnitType(UnitType),
    /// A local unit (of the type, or any with `None`) is complete and
    /// within `r` cells of `(x, z)`.
    MoveUnitToRadius {
        /// Type, `None` for `ANYTYPE`.
        ty: Option<UnitType>,
        /// Centre cell x.
        x: i32,
        /// Centre cell z.
        z: i32,
        /// Radius, cells.
        r: i32,
    },
    /// A local unit (of the type) is on the column `x`.
    UnitTypePassesX(Option<UnitType>, i32),
    /// A local unit (of the type) is on the row `z`.
    UnitTypePassesZ(Option<UnitType>, i32),
    /// This many seconds have passed.
    VictoryTimerRunsOut(i32),
    // ---- defeat ----
    /// The local monarch has been removed.
    CommanderKilled,
    /// The local player has no complete, living unit.
    AllUnitsKilled,
    /// The last unit of this type (local or enemy) has been removed.
    AllUnitsKilledOfType(UnitType),
    /// This many units of this type (any owner) have been removed.
    UnitTypeKilled(UnitType, i32),
    /// This many seconds have passed.
    DeathTimerRunsOut(i32),
    /// An enemy unit is on the column `x`.
    AnyUnitPassesX(i32),
    /// An enemy unit is on the row `z`.
    AnyUnitPassesZ(i32),
}

impl Condition {
    fn unit_type(&self) -> Option<&UnitType> {
        match self {
            Condition::KillAllOfType(t)
            | Condition::KillUnitType(t, _)
            | Condition::BuildUnitType(t)
            | Condition::AllUnitsKilledOfType(t)
            | Condition::UnitTypeKilled(t, _) => Some(t),
            Condition::MoveUnitToRadius { ty, .. }
            | Condition::UnitTypePassesX(ty, _)
            | Condition::UnitTypePassesZ(ty, _) => ty.as_ref(),
            _ => None,
        }
    }

    fn initial(&self) -> CondState {
        let remaining = match self {
            Condition::KillUnitType(_, n) | Condition::UnitTypeKilled(_, n) => *n,
            _ => 0,
        };
        CondState {
            satisfied: false,
            remaining,
        }
    }
}

/// Typed mission conditions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MissionRules {
    /// All must hold to win; empty means [`Condition::DestroyAllUnits`].
    pub victory: Vec<Condition>,
    /// Any one loses; empty means [`Condition::AllUnitsKilled`].
    pub defeat: Vec<Condition>,
    /// `key=value` entries not understood or not implemented.
    pub unsupported: Vec<String>,
}

impl MissionRules {
    /// Parse `tak_content::OtaDef::conditions` (keys any case). Unit type
    /// names resolve against `defs`; a name with arguments where none are
    /// expected stays verbatim and matches nothing (TA). Malformed and
    /// unimplemented keys go to [`MissionRules::unsupported`].
    pub fn from_conditions(conds: &BTreeMap<String, String>, defs: &[UnitDef]) -> MissionRules {
        let mut m = MissionRules::default();
        for (k, v) in conds {
            let key = k.to_ascii_lowercase();
            match parse_condition(&key, v, defs) {
                Parsed::Victory(c) => m.victory.push(c),
                Parsed::Defeat(c) => m.defeat.push(c),
                Parsed::Off => {}
                Parsed::Unsupported => m.unsupported.push(format!("{k}={v}")),
            }
        }
        m
    }
}

enum Parsed {
    Victory(Condition),
    Defeat(Condition),
    /// A flag set to 0: no condition.
    Off,
    Unsupported,
}

fn parse_condition(key: &str, v: &str, defs: &[UnitDef]) -> Parsed {
    let parts: Vec<&str> = v.split(',').map(str::trim).collect();
    let int = |i: usize| parts.get(i).and_then(|s| s.parse::<i32>().ok());
    let flag = |c: Condition, victory: bool| match v.trim().parse::<i32>() {
        Ok(0) => Parsed::Off,
        Ok(_) if victory => Parsed::Victory(c),
        Ok(_) => Parsed::Defeat(c),
        Err(_) => Parsed::Unsupported,
    };
    let name = || UnitType::resolve(v, defs);
    let typed = || -> Option<Option<UnitType>> {
        let n = parts.first()?;
        if n.is_empty() {
            return None;
        }
        Some((!n.eq_ignore_ascii_case("ANYTYPE")).then(|| UnitType::resolve(n, defs)))
    };
    let seconds = |victory: bool| match v.trim().parse::<i32>() {
        Ok(s) if s > 0 && victory => Parsed::Victory(Condition::VictoryTimerRunsOut(s)),
        Ok(s) if s > 0 => Parsed::Defeat(Condition::DeathTimerRunsOut(s)),
        _ => Parsed::Unsupported,
    };
    let counted = |f: fn(UnitType, i32) -> Condition, victory: bool| match (
        parts.first(),
        int(1),
        parts.len(),
    ) {
        (Some(n), Some(c), 2) if !n.is_empty() => {
            let c = f(UnitType::resolve(n, defs), c);
            if victory {
                Parsed::Victory(c)
            } else {
                Parsed::Defeat(c)
            }
        }
        _ => Parsed::Unsupported,
    };
    match key {
        "destroyallunits" => flag(Condition::DestroyAllUnits, true),
        "killenemycommander" => flag(Condition::KillEnemyCommander, true),
        "killallmobileunits" => flag(Condition::KillAllMobileUnits, true),
        "killalloftype" => Parsed::Victory(Condition::KillAllOfType(name())),
        "buildunittype" => Parsed::Victory(Condition::BuildUnitType(name())),
        "killunittype" => counted(Condition::KillUnitType, true),
        "moveunittoradius" => match (typed(), int(1), int(2), int(3), parts.len()) {
            (Some(ty), Some(x), Some(z), Some(r), 4) => {
                Parsed::Victory(Condition::MoveUnitToRadius { ty, x, z, r })
            }
            _ => Parsed::Unsupported,
        },
        "unittypepassesx" | "unittypepassesz" => match (typed(), int(1), parts.len()) {
            (Some(ty), Some(c), 2) if key.ends_with('x') => {
                Parsed::Victory(Condition::UnitTypePassesX(ty, c))
            }
            (Some(ty), Some(c), 2) => Parsed::Victory(Condition::UnitTypePassesZ(ty, c)),
            _ => Parsed::Unsupported,
        },
        "victorytimerrunsout" => seconds(true),
        "commanderkilled" => flag(Condition::CommanderKilled, false),
        "allunitskilled" => flag(Condition::AllUnitsKilled, false),
        "allunitskilledoftype" => Parsed::Defeat(Condition::AllUnitsKilledOfType(name())),
        "unittypekilled" => counted(Condition::UnitTypeKilled, false),
        "deathtimerrunsout" => seconds(false),
        "anyunitpassesx" | "anyunitpassesz" => match v.trim().parse::<i32>() {
            Ok(c) if c >= 0 && key.ends_with('x') => Parsed::Defeat(Condition::AnyUnitPassesX(c)),
            Ok(c) if c >= 0 => Parsed::Defeat(Condition::AnyUnitPassesZ(c)),
            _ => Parsed::Unsupported,
        },
        // `captureunittype` (no capture in the sim yet), `useonlyunits` (a
        // build restriction, not a condition) and anything unknown.
        _ => Parsed::Unsupported,
    }
}

/// Run-time state of one mission condition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CondState {
    /// Latched result (notification and latching poll conditions).
    pub satisfied: bool,
    /// Kills still needed (`KillUnitType`, `UnitTypeKilled`).
    pub remaining: i32,
}

/// Victory state in the world (hashed every tick).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VictoryState {
    setup: VictorySetup,
    setup_digest: u64,
    state: MatchState,
    decided_at: Option<Tick>,
    defeated: Vec<Option<Tick>>,
    had_units: Vec<bool>,
    /// Mission conditions, victory list then defeat list.
    conds: Vec<CondState>,
    /// Mission mode: the units alive after the last [`tick`], to detect
    /// removals.
    prev: Vec<(UnitId, UnitDefId, PlayerId)>,
}

impl VictoryState {
    pub(crate) fn new(setup: &VictorySetup, players: usize) -> VictoryState {
        let mut h = StateHasher::new();
        h.bytes(serde_json::to_string(setup).unwrap_or_default().as_bytes());
        let conds = setup
            .mission
            .as_ref()
            .map(|m| {
                m.victory
                    .iter()
                    .chain(&m.defeat)
                    .map(Condition::initial)
                    .collect()
            })
            .unwrap_or_default();
        VictoryState {
            setup: setup.clone(),
            setup_digest: h.finish(),
            state: MatchState::Running,
            decided_at: None,
            defeated: vec![None; players],
            had_units: vec![false; players],
            conds,
            prev: Vec::new(),
        }
    }

    /// The match state.
    pub fn state(&self) -> &MatchState {
        &self.state
    }

    /// The tick on which the match was decided.
    pub fn decided_at(&self) -> Option<Tick> {
        self.decided_at
    }

    /// The tick on which `p` was defeated, if it was.
    pub fn defeated_at(&self, p: PlayerId) -> Option<Tick> {
        self.defeated.get(p.0 as usize).copied().flatten()
    }

    /// The rules in force.
    pub fn setup(&self) -> &VictorySetup {
        &self.setup
    }

    /// Mission condition states, victory list then defeat list.
    pub fn conditions(&self) -> &[CondState] {
        &self.conds
    }

    /// One line for logs: `running`, `over at tick T: winners [..]`.
    pub fn summary(&self) -> String {
        match (&self.state, self.decided_at) {
            (MatchState::Over { winners }, t) => {
                let w: Vec<String> = winners.iter().map(|p| p.0.to_string()).collect();
                let t = t.map_or(String::from("?"), |t| t.0.to_string());
                if w.is_empty() {
                    format!("over at tick {t}: draw")
                } else {
                    format!("over at tick {t}: winners [{}]", w.join(", "))
                }
            }
            (MatchState::Running, _) => "running".into(),
        }
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"V");
        h.u64(self.setup_digest);
        match &self.state {
            MatchState::Running => h.u8(0),
            MatchState::Over { winners } => {
                h.u8(1);
                h.len(winners.len());
                for &w in winners {
                    h.player(w);
                }
            }
        }
        h.option(self.decided_at, StateHasher::tick);
        h.len(self.defeated.len());
        for &d in &self.defeated {
            h.option(d, StateHasher::tick);
        }
        for &b in &self.had_units {
            h.bool(b);
        }
        h.len(self.conds.len());
        for c in &self.conds {
            h.bool(c.satisfied);
            h.i32(c.remaining);
        }
        h.len(self.prev.len());
        for &(id, def, owner) in &self.prev {
            h.unit_id(id);
            h.unit_def(def);
            h.player(owner);
        }
    }

    fn decide(&mut self, now: Tick, winners: Vec<PlayerId>) {
        self.state = MatchState::Over { winners };
        self.decided_at = Some(now);
    }
}

impl World {
    /// `true` once `p` has been defeated (unknown players: `false`).
    pub fn is_defeated(&self, p: PlayerId) -> bool {
        self.victory.defeated_at(p).is_some()
    }

    /// The victory state.
    pub fn victory(&self) -> &VictoryState {
        &self.victory
    }
}

impl crate::Sim {
    /// Whether the match is over, and who won.
    pub fn match_state(&self) -> &MatchState {
        self.world().victory.state()
    }
}

/// The victory system (after combat). See the module docs.
pub fn tick(world: &mut World, scripts: &mut dyn ScriptHost) {
    if world.victory.state != MatchState::Running {
        return;
    }
    let n = world.players.len();
    let mut live = vec![0u32; n];
    for (_, u) in world.units() {
        if let Some(c) = live.get_mut(u.owner.0 as usize) {
            *c += 1;
        }
    }
    for (had, &c) in world.victory.had_units.iter_mut().zip(&live) {
        *had |= c > 0;
    }
    if world.victory.setup.mission.is_some() {
        mission_tick(world, &live);
    } else {
        skirmish_tick(world, scripts, &live);
    }
}

fn skirmish_tick(world: &mut World, scripts: &mut dyn ScriptHost, live: &[u32]) {
    let now = world.tick;
    let rule = world.victory.setup.defeat;
    let mut newly = Vec::new();
    for (i, &count) in live.iter().enumerate() {
        if world.victory.defeated[i].is_some() {
            continue;
        }
        let wiped = world.victory.had_units[i] && count == 0;
        let monarch_down = rule == DefeatRule::MonarchKilled
            && world.players[i]
                .monarch
                .is_some_and(|m| world.unit(m).is_none_or(|u| u.combat.is_dying()));
        if wiped || monarch_down {
            world.victory.defeated[i] = Some(now);
            newly.push(PlayerId(i as u8));
        }
    }
    if world.victory.setup.defeated_units == DefeatedUnits::SelfDestruct {
        for p in newly {
            self_destruct(world, scripts, p);
        }
    }
    // Over when at least two teams started and at most one is left.
    let setup = &world.victory.setup;
    let mut all: Vec<u32> = (0..live.len()).map(|i| setup.team(i)).collect();
    all.sort_unstable();
    all.dedup();
    if all.len() < 2 {
        return;
    }
    let alive: Vec<usize> = (0..live.len())
        .filter(|&i| world.victory.defeated[i].is_none())
        .collect();
    let mut teams: Vec<u32> = alive.iter().map(|&i| setup.team(i)).collect();
    teams.sort_unstable();
    teams.dedup();
    if teams.len() <= 1 {
        let winners = alive.into_iter().map(|i| PlayerId(i as u8)).collect();
        world.victory.decide(now, winners);
    }
}

/// TA's commander-death sweep: every living unit of `p`, in slot order,
/// takes [`DEFEAT_DAMAGE`] (explosion kind). Units outside combat and
/// units with more health survive (and stay inert).
fn self_destruct(world: &mut World, scripts: &mut dyn ScriptHost, p: PlayerId) {
    let now = world.tick.0;
    let ids: Vec<(UnitId, crate::WorldPos)> = world
        .units()
        .filter(|(_, u)| u.owner == p && !u.combat.is_dying())
        .map(|(id, u)| (id, u.pos))
        .collect();
    for (id, pos) in ids {
        crate::combat::apply_damage(
            world,
            scripts,
            id,
            DEFEAT_DAMAGE,
            None,
            crate::combat::kind::EXPLOSION,
            pos,
            now,
        );
    }
}

fn mission_tick(world: &mut World, live: &[u32]) {
    let now = world.tick;
    let Some(rules) = world.victory.setup.mission.clone() else {
        return;
    };
    let removed: Vec<(UnitDefId, PlayerId)> = world
        .victory
        .prev
        .iter()
        .filter(|(id, _, _)| world.unit(*id).is_none())
        .map(|&(_, d, o)| (d, o))
        .collect();
    let all: Vec<&Condition> = rules.victory.iter().chain(&rules.defeat).collect();
    let mut states = std::mem::take(&mut world.victory.conds);
    let mut results = Vec::with_capacity(all.len());
    for (c, st) in all.iter().zip(states.iter_mut()) {
        for &(def, owner) in &removed {
            notify(world, c, st, def, owner);
        }
        results.push(poll(world, c, st, live));
    }
    world.victory.conds = states;
    let nv = rules.victory.len();
    let won = if nv == 0 {
        poll_stateless(world, &Condition::DestroyAllUnits, live)
    } else {
        results[..nv].iter().all(|&r| r)
    };
    let lost = if rules.defeat.is_empty() {
        poll_stateless(world, &Condition::AllUnitsKilled, live)
    } else {
        results[nv..].iter().any(|&r| r)
    };
    let has_enemy = world.has_player(ENEMY);
    if won {
        if has_enemy {
            world.victory.defeated[ENEMY.0 as usize] = Some(now);
        }
        world.victory.decide(now, vec![LOCAL]);
    } else if lost {
        if world.has_player(LOCAL) {
            world.victory.defeated[LOCAL.0 as usize] = Some(now);
        }
        world
            .victory
            .decide(now, if has_enemy { vec![ENEMY] } else { Vec::new() });
    }
    world.victory.prev = world.units().map(|(id, u)| (id, u.def, u.owner)).collect();
}

fn commander(world: &World, p: PlayerId) -> Option<UnitDefId> {
    world.players.get(p.0 as usize).map(|pl| pl.monarch_def)
}

fn mobile(world: &World, def: UnitDefId) -> bool {
    world
        .defs()
        .get(def)
        .is_some_and(|d| d.maxvelocity > tak_math::Fx::ZERO)
}

fn count(world: &World, owners: &[PlayerId], f: impl Fn(UnitDefId) -> bool) -> usize {
    world
        .units()
        .filter(|(_, u)| owners.contains(&u.owner) && f(u.def))
        .count()
}

/// A unit removal (TA's kill notifications).
fn notify(world: &World, c: &Condition, st: &mut CondState, def: UnitDefId, owner: PlayerId) {
    let enemy = owner == ENEMY;
    let done = match c {
        Condition::KillEnemyCommander => enemy && commander(world, owner) == Some(def),
        Condition::KillAllMobileUnits => {
            enemy && mobile(world, def) && count(world, &[ENEMY], |d| mobile(world, d)) == 0
        }
        Condition::KillAllOfType(t) => {
            enemy && t.is(def) && count(world, &[ENEMY], |d| t.is(d)) == 0
        }
        Condition::KillUnitType(t, _) if st.remaining > 0 && enemy && t.is(def) => {
            st.remaining -= 1;
            st.remaining < 1
        }
        Condition::CommanderKilled => owner == LOCAL && commander(world, owner) == Some(def),
        Condition::AllUnitsKilledOfType(t) => {
            t.is(def) && count(world, &[LOCAL, ENEMY], |d| t.is(d)) == 0
        }
        Condition::UnitTypeKilled(t, _) if t.is(def) => {
            st.remaining = st.remaining.saturating_sub(1);
            st.remaining < 1
        }
        _ => false,
    };
    st.satisfied |= done;
}

fn type_ok(ty: &Option<UnitType>, def: UnitDefId) -> bool {
    ty.as_ref().is_none_or(|t| t.is(def))
}

fn cell(v: tak_math::Fx) -> i32 {
    v.to_int_floor().div_euclid(CELL)
}

/// Poll a condition; latching ones set `st.satisfied`.
fn poll(world: &World, c: &Condition, st: &mut CondState, live: &[u32]) -> bool {
    if st.satisfied {
        return true;
    }
    let latch = match c {
        Condition::BuildUnitType(t) => world
            .units()
            .any(|(_, u)| u.owner == LOCAL && t.is(u.def) && u.build.is_complete()),
        Condition::MoveUnitToRadius { ty, x, z, r } => {
            let cx = i64::from(*x) * i64::from(CELL) + i64::from(CELL / 2);
            let cz = i64::from(*z) * i64::from(CELL) + i64::from(CELL / 2);
            let rr = i64::from(*r) * i64::from(CELL);
            world.units().any(|(_, u)| {
                let dx = i64::from(u.pos.x.to_int_floor()) - cx;
                let dz = i64::from(u.pos.y.to_int_floor()) - cz;
                u.owner == LOCAL
                    && type_ok(ty, u.def)
                    && u.build.is_complete()
                    && !u.combat.is_dying()
                    && dx * dx + dz * dz <= rr * rr
            })
        }
        Condition::UnitTypePassesX(ty, v) => world.units().any(|(_, u)| {
            u.owner == LOCAL
                && type_ok(ty, u.def)
                && (cell(u.pos.x) - v).abs() < PASS_TOLERANCE_CELLS
        }),
        Condition::UnitTypePassesZ(ty, v) => world.units().any(|(_, u)| {
            u.owner == LOCAL
                && type_ok(ty, u.def)
                && (cell(u.pos.y) - v).abs() < PASS_TOLERANCE_CELLS
        }),
        Condition::AnyUnitPassesX(v) => world
            .units()
            .any(|(_, u)| u.owner == ENEMY && (cell(u.pos.x) - v).abs() < PASS_TOLERANCE_CELLS),
        Condition::AnyUnitPassesZ(v) => world
            .units()
            .any(|(_, u)| u.owner == ENEMY && (cell(u.pos.y) - v).abs() < PASS_TOLERANCE_CELLS),
        _ => return poll_stateless(world, c, live),
    };
    st.satisfied = latch;
    latch
}

/// Pure predicates, re-evaluated every tick; notification-only conditions
/// read `false` here.
fn poll_stateless(world: &World, c: &Condition, live: &[u32]) -> bool {
    let seconds = |s: i32| world.tick.0 >= (s.max(0) as u32).saturating_mul(TICKS_PER_SECOND);
    match c {
        Condition::DestroyAllUnits => live.get(ENEMY.0 as usize).is_none_or(|&n| n == 0),
        Condition::AllUnitsKilled => !world
            .units()
            .any(|(_, u)| u.owner == LOCAL && u.build.is_complete() && !u.combat.is_dying()),
        Condition::VictoryTimerRunsOut(s) | Condition::DeathTimerRunsOut(s) => seconds(*s),
        _ => false,
    }
}
