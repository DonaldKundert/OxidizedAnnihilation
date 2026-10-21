//! Economy: mana (WP-N). The rules, their sources and confidence are in
//! `docs/economy.md`; every rule that is not Known is a field of
//! [`EconomyRules`].
//!
//! - **Player mana** (the mana bar): income is the sum of `mogriumincome`
//!   (per second) over the player's live, completed units, with lodestones
//!   (`yardmap = S`) scaled by the `sacredsite` value of the site they stand
//!   on. Storage is the sum of `mogriumstorage`. Stock above storage is lost.
//! - **Spending**: other systems call [`drain`] every tick for what they want
//!   to spend; in a shortage every consumer gets the same fraction (TA
//!   behaviour, computed from the previous tick's demand). [`credit`] adds
//!   mana (reclaim).
//! - **Unit mana**: units with `maxmana` have their own pool that recharges
//!   at `manarechargerate` per second; spells pay with [`spend_unit_mana`].
//!
//! All amounts are 16.16 fixed point; player totals are kept in `i64` (a
//! big base passes `Fx`'s range). Per-second rates become per-tick amounts
//! by exact division with a carried remainder, so 30 ticks add exactly one
//! second's worth.

use serde::{Deserialize, Serialize};
use tak_commands::{PlayerId, UnitId};
use tak_content::WideFx;
use tak_math::Fx;

use crate::TICKS_PER_SECOND;
use crate::hash::StateHasher;
use crate::scenario::ScenarioError;
use crate::script::ScriptHost;
use crate::world::World;

/// World units per TNT attribute cell (sacred-site footprints are in cells).
const CELL: i64 = 16;

/// Most sacred sites a scenario may carry (the corpus has a few dozen per
/// map at most).
pub const MAX_SACRED_SITES: usize = 4096;

/// Upper bound on per-unit economy numbers (sanity cap; the corpus tops out
/// at 5000).
pub const MAX_ECONOMY_VALUE: i32 = 1_000_000;

const TPS: i64 = TICKS_PER_SECOND as i64;

// ---------------------------------------------------------------------------
// Definitions and setup (static match data)
// ---------------------------------------------------------------------------

/// Economy keys of one unit definition (FBI names, FBI units).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EconomyDef {
    /// FBI `mogriumincome`: mana per second added to the owner's income.
    pub mogriumincome: i32,
    /// FBI `mogriumstorage`: mana storage added to the owner's cap.
    pub mogriumstorage: i32,
    /// FBI `economybonus` (meaning Unknown; see [`EconomyBonusRule`]).
    pub economybonus: i32,
    /// FBI `maxmana`: size of the unit's own mana pool (0 = none). At most
    /// 32767 (it is an `Fx`).
    pub maxmana: i32,
    /// FBI `manarechargerate`: own-pool recharge, mana per second.
    pub manarechargerate: i32,
    /// FBI `yardmap = S`: must stand on a sacred site (lodestones);
    /// its income is scaled by the site's `sacredsite` value.
    pub sacred_site: bool,
}

impl EconomyDef {
    pub(crate) fn validate(&self, name: &str) -> Result<(), ScenarioError> {
        let range = |v: i32, max: i32| (0..=max).contains(&v);
        let ok = range(self.mogriumincome, MAX_ECONOMY_VALUE)
            && range(self.mogriumstorage, MAX_ECONOMY_VALUE)
            && range(self.economybonus, 1000)
            && range(self.maxmana, i32::from(i16::MAX))
            && range(self.manarechargerate, MAX_ECONOMY_VALUE);
        if ok {
            Ok(())
        } else {
            Err(ScenarioError::Invalid(format!(
                "unit def {name:?}: economy value out of range"
            )))
        }
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.i32(self.mogriumincome);
        h.i32(self.mogriumstorage);
        h.i32(self.economybonus);
        h.i32(self.maxmana);
        h.i32(self.manarechargerate);
        h.bool(self.sacred_site);
    }
}

/// The economy keys of an FBI unit. Missing keys are 0; values are clamped
/// into the validated ranges.
pub fn economy_def_from_fbi(u: &tak_content::UnitDef) -> EconomyDef {
    let e = &u.economy;
    let v = |x: Option<i32>, max: i32| x.unwrap_or(0).clamp(0, max);
    EconomyDef {
        mogriumincome: v(e.mogrium_income, MAX_ECONOMY_VALUE),
        mogriumstorage: v(e.mogrium_storage, MAX_ECONOMY_VALUE),
        economybonus: v(e.economy_bonus, 1000),
        maxmana: v(e.max_mana, i32::from(i16::MAX)),
        manarechargerate: v(e.mana_recharge_rate, MAX_ECONOMY_VALUE),
        sacred_site: u
            .footprint
            .yardmap
            .as_deref()
            .is_some_and(|y| y.trim().eq_ignore_ascii_case("s")),
    }
}

/// A sacred site (a `*ManaNN` feature): a rectangle of the map in whole
/// world units and its `sacredsite` income multiplier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SacredSite {
    /// Left edge (world units).
    pub x: i32,
    /// Top edge (world units).
    pub y: i32,
    /// Width (world units).
    pub w: i32,
    /// Height (world units).
    pub h: i32,
    /// Feature `sacredsite` (1.0, 1.5 or 2.0 in the corpus).
    pub multiplier: Fx,
}

impl SacredSite {
    /// `true` if world point `(px, py)` (16.16) lies within the site plus
    /// `slack` world units on every side.
    fn contains(&self, p: crate::WorldPos, slack: i32) -> bool {
        let one = i64::from(Fx::ONE_RAW);
        let (px, py) = (i64::from(p.x.raw()), i64::from(p.y.raw()));
        let x0 = i64::from(self.x - slack) * one;
        let y0 = i64::from(self.y - slack) * one;
        let x1 = i64::from(self.x + self.w + slack) * one;
        let y1 = i64::from(self.y + self.h + slack) * one;
        px >= x0 && px < x1 && py >= y0 && py < y1
    }
}

/// What `economybonus` does. **Unknown** in the original; off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomyBonusRule {
    /// No effect (default).
    #[default]
    Ignore,
    /// Adds `economybonus` percent to that unit's own `mogriumincome`.
    UnitIncomePercent,
    /// Adds `economybonus` percent to the owner's whole income while the
    /// unit lives (several bonuses add up).
    PlayerIncomePercent,
}

/// Every economy rule that is not Known, in one place. See
/// `docs/economy.md`. The defaults are the most TA-faithful readings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EconomyRules {
    /// Player mana at tick 0, before the storage cap (**Chosen** 1000).
    pub start_mana: i32,
    /// Fill of each start monarch's own pool, percent (**Chosen** 0, the
    /// mission default `manapercentage`).
    pub monarch_start_mana_percent: u8,
    /// Income multiplier for a sacred-site unit that stands on no free site
    /// (**Chosen** 1.0; cannot happen in the original).
    pub off_site_multiplier: Fx,
    /// Slack around a site's footprint when matching a lodestone, world
    /// units (**Chosen** one cell, which also covers centre anchoring).
    pub site_slack: i32,
    /// What `economybonus` does (**Unknown**, ignored by default).
    pub economy_bonus: EconomyBonusRule,
}

impl Default for EconomyRules {
    fn default() -> Self {
        EconomyRules {
            start_mana: 1000,
            monarch_start_mana_percent: 0,
            off_site_multiplier: Fx::ONE,
            site_slack: CELL as i32,
            economy_bonus: EconomyBonusRule::Ignore,
        }
    }
}

/// The economy part of a scenario: rules and the map's sacred sites.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EconomySetup {
    /// Rules (defaults are the original's, as far as known).
    pub rules: EconomyRules,
    /// Sacred sites in map order.
    pub sacred_sites: Vec<SacredSite>,
}

impl EconomySetup {
    /// Sacred sites of a real map: every placed feature whose def has a
    /// positive `sacredsite`, as its footprint rectangle anchored at the
    /// top-left of its cell (the `tak-content` default anchor). Default
    /// rules.
    pub fn from_content(map: &tak_content::MapDef, rs: &tak_content::Ruleset) -> EconomySetup {
        let sacred_sites = map
            .features
            .iter()
            .filter_map(|f| {
                let def = rs.feature(f.def.as_deref()?)?;
                let m = def.sacred_site.filter(|m| *m > Fx::ZERO)?;
                let cell = |c: u32| i32::try_from(i64::from(c) * CELL).ok();
                Some(SacredSite {
                    x: cell(f.x)?,
                    y: cell(f.z)?,
                    w: i32::from(def.footprint_x.max(1)) * CELL as i32,
                    h: i32::from(def.footprint_z.max(1)) * CELL as i32,
                    multiplier: m,
                })
            })
            .take(MAX_SACRED_SITES)
            .collect();
        EconomySetup {
            rules: EconomyRules::default(),
            sacred_sites,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ScenarioError> {
        let bad = |m: &str| Err(ScenarioError::Invalid(format!("economy: {m}")));
        let r = &self.rules;
        if !(0..=MAX_ECONOMY_VALUE).contains(&r.start_mana) {
            return bad("start_mana out of range");
        }
        if r.monarch_start_mana_percent > 100 {
            return bad("monarch_start_mana_percent above 100");
        }
        if r.off_site_multiplier < Fx::ZERO || r.off_site_multiplier > Fx::from_int(100) {
            return bad("off_site_multiplier out of range");
        }
        if !(0..=1024).contains(&r.site_slack) {
            return bad("site_slack out of range");
        }
        if self.sacred_sites.len() > MAX_SACRED_SITES {
            return bad("too many sacred sites");
        }
        for s in &self.sacred_sites {
            let side = 0..=crate::map::MAX_MAP_SIDE;
            if !side.contains(&s.x)
                || !side.contains(&s.y)
                || !(1..=1024).contains(&s.w)
                || !(1..=1024).contains(&s.h)
                || s.multiplier < Fx::ZERO
                || s.multiplier > Fx::from_int(100)
            {
                return bad("sacred site out of range");
            }
        }
        Ok(())
    }

    fn hash(&self, h: &mut StateHasher) {
        let r = &self.rules;
        h.i32(r.start_mana);
        h.u8(r.monarch_start_mana_percent);
        h.fx(r.off_site_multiplier);
        h.i32(r.site_slack);
        h.u8(r.economy_bonus as u8);
        h.len(self.sacred_sites.len());
        for s in &self.sacred_sites {
            h.i32(s.x);
            h.i32(s.y);
            h.i32(s.w);
            h.i32(s.h);
            h.fx(s.multiplier);
        }
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Per-unit economy state: the unit's own mana pool.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitEconomy {
    /// Own mana, `0..=maxmana`.
    mana: Fx,
    /// Recharge remainder (raw 16.16 units times ticks, `0..30`).
    carry: i64,
    /// Index of the sacred site this unit claimed last tick, if any.
    site: Option<u32>,
}

impl UnitEconomy {
    /// Own mana.
    pub fn mana(&self) -> Fx {
        self.mana
    }

    /// The sacred site (index into the scenario's list) this unit stands on.
    pub fn site(&self) -> Option<u32> {
        self.site
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"E");
        h.fx(self.mana);
        h.u64(self.carry as u64);
        h.option(self.site, StateHasher::u32);
    }
}

/// One player's mana bar. Amounts are raw 16.16 in `i64`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerEconomy {
    /// Stock.
    pub mana: i64,
    /// Storage cap (from last [`tick`]).
    pub storage: i64,
    /// Income per second (from last [`tick`]).
    pub income: i64,
    /// Income remainder carried between ticks (`0..30`).
    carry: i64,
    /// Fraction every [`drain`] gets this tick (shortage sharing), `0..=1`.
    pub ratio: Fx,
    /// Total asked for through [`drain`] this tick so far.
    pub demand: i64,
    /// Total granted this tick so far.
    pub granted: i64,
    /// `demand` and `granted` of the last completed tick (for reports).
    pub last_demand: i64,
    /// See `last_demand`.
    pub last_granted: i64,
}

impl PlayerEconomy {
    fn hash(&self, h: &mut StateHasher) {
        for v in [
            self.mana,
            self.storage,
            self.income,
            self.carry,
            self.demand,
            self.granted,
            self.last_demand,
            self.last_granted,
        ] {
            h.u64(v as u64);
        }
        h.fx(self.ratio);
    }
}

/// World-level economy state: the setup and every player's mana bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EconomyState {
    setup: EconomySetup,
    players: Vec<PlayerEconomy>,
    started: bool,
}

impl EconomyState {
    /// Fresh state for `players` players with the scenario's setup.
    pub fn new(setup: &EconomySetup, players: usize) -> EconomyState {
        let start = i64::from(setup.rules.start_mana) << 16;
        EconomyState {
            setup: setup.clone(),
            players: (0..players)
                .map(|_| PlayerEconomy {
                    mana: start,
                    ratio: Fx::ONE,
                    ..Default::default()
                })
                .collect(),
            started: false,
        }
    }

    /// The rules in force.
    pub fn rules(&self) -> &EconomyRules {
        &self.setup.rules
    }

    /// The map's sacred sites.
    pub fn sacred_sites(&self) -> &[SacredSite] {
        &self.setup.sacred_sites
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.bytes(b"ECON");
        self.setup.hash(h);
        h.bool(self.started);
        h.len(self.players.len());
        for p in &self.players {
            p.hash(h);
        }
    }
}

// ---------------------------------------------------------------------------
// Tick
// ---------------------------------------------------------------------------

/// `(rate_raw + carry) / 30`, updating the carry. `rate_raw` is a 16.16
/// per-second amount; the result is the per-tick amount.
fn per_tick(rate_raw: i64, carry: &mut i64) -> i64 {
    let total = rate_raw.saturating_add(*carry);
    *carry = total.rem_euclid(TPS);
    total.div_euclid(TPS)
}

/// `a * pct / 100` on raw values.
fn percent(a: i64, pct: i64) -> i64 {
    a.saturating_mul(pct) / 100
}

/// Runs after construction, before orders: income, storage, overflow,
/// shortage sharing for the next tick, and unit-pool recharge.
pub fn tick(world: &mut World, _scripts: &mut dyn ScriptHost) {
    let n = world.players.len();
    if world.economy.players.len() != n {
        world.economy.players.resize_with(n, || PlayerEconomy {
            ratio: Fx::ONE,
            ..Default::default()
        });
    }
    // Refunds from cancelled construction (WP-O) go back into the pool,
    // capped by storage like any other credit. Applied in queue order.
    for r in world.construction.take_refunds() {
        let mut left = r.mana.max(0);
        while left > 0 {
            let chunk = left.min(i64::from(i32::MAX));
            credit(world, r.player, Fx::from_raw(chunk as i32));
            left -= chunk;
        }
    }
    if !world.economy.started {
        world.economy.started = true;
        let pct = world.economy.setup.rules.monarch_start_mana_percent;
        for i in 0..n {
            if let Some(id) = world.players[i].monarch {
                set_unit_mana_percent(world, id, pct);
            }
        }
    }

    // Taken out for the unit loop (which borrows the world) and put back.
    let setup = std::mem::take(&mut world.economy.setup);
    let rules = &setup.rules;
    let mut claimed = vec![false; setup.sacred_sites.len()];
    let mut income = vec![0i64; n];
    let mut bonus_pct = vec![0i64; n];
    let mut storage = vec![0i64; n];

    for id in world.unit_ids() {
        let Some((u, def, _)) = world.unit_and_def_mut(id) else {
            continue;
        };
        let e = &def.economy;
        let owner = u.owner.0 as usize;
        let complete = u.build.percent_left() <= 0;
        // Own pool recharge (completed units with a pool).
        if complete && e.maxmana > 0 {
            let max = Fx::from_int(e.maxmana);
            if u.economy.mana < max {
                let add = per_tick(i64::from(e.manarechargerate) << 16, &mut u.economy.carry);
                let m = (i64::from(u.economy.mana.raw()) + add).min(i64::from(max.raw()));
                u.economy.mana = Fx::from_raw(m as i32);
            } else {
                u.economy.mana = max;
                u.economy.carry = 0;
            }
        }
        // Site claim (recomputed every tick, slot order, first free site).
        u.economy.site = None;
        if !complete || owner >= n {
            continue;
        }
        let mut unit_income = i64::from(e.mogriumincome) << 16;
        if e.sacred_site {
            let site = setup
                .sacred_sites
                .iter()
                .enumerate()
                .position(|(i, s)| !claimed[i] && s.contains(u.pos, rules.site_slack));
            let mult = match site {
                Some(i) => {
                    claimed[i] = true;
                    u.economy.site = Some(i as u32);
                    setup.sacred_sites[i].multiplier
                }
                None => rules.off_site_multiplier,
            };
            unit_income = unit_income.saturating_mul(i64::from(mult.raw())) >> 16;
        }
        match rules.economy_bonus {
            EconomyBonusRule::Ignore => {}
            EconomyBonusRule::UnitIncomePercent => {
                unit_income += percent(unit_income, i64::from(e.economybonus));
            }
            EconomyBonusRule::PlayerIncomePercent => {
                bonus_pct[owner] += i64::from(e.economybonus);
            }
        }
        income[owner] = income[owner].saturating_add(unit_income);
        storage[owner] = storage[owner].saturating_add(i64::from(e.mogriumstorage) << 16);
    }
    world.economy.setup = setup;

    for (i, p) in world.economy.players.iter_mut().enumerate() {
        let inc = income[i].saturating_add(percent(income[i], bonus_pct[i]));
        p.income = inc;
        p.storage = storage[i];
        let add = per_tick(inc, &mut p.carry);
        p.mana = p.mana.saturating_add(add).clamp(0, p.storage);
        // Shortage sharing for next tick: the fraction of this tick's demand
        // the stock now covers.
        p.ratio = if p.demand <= 0 || p.mana >= p.demand {
            Fx::ONE
        } else {
            Fx::from_raw(((p.mana << 16) / p.demand) as i32)
        };
        p.last_demand = p.demand;
        p.last_granted = p.granted;
        p.demand = 0;
        p.granted = 0;
    }
}

// ---------------------------------------------------------------------------
// Player mana API
// ---------------------------------------------------------------------------

/// Ask to spend `want` mana for `player` this tick. Returns how much was
/// granted (between zero and `want`). Callers scale their progress by the
/// granted fraction.
///
/// In a shortage every consumer gets the same fraction of what it asks
/// (`ratio`, from the previous tick's demand), never more than the stock
/// left. See `docs/economy.md`.
pub fn drain(world: &mut World, player: PlayerId, want: Fx) -> Fx {
    let Some(p) = world.economy.players.get_mut(player.0 as usize) else {
        return Fx::ZERO;
    };
    if want <= Fx::ZERO {
        return Fx::ZERO;
    }
    let want_raw = i64::from(want.raw());
    p.demand = p.demand.saturating_add(want_raw);
    let share = (want_raw * i64::from(p.ratio.raw())) >> 16;
    let g = share.min(p.mana).clamp(0, want_raw);
    p.mana -= g;
    p.granted = p.granted.saturating_add(g);
    Fx::from_raw(g as i32)
}

/// Add mana to `player` (reclaim, refunds), capped by storage. Returns what
/// was actually added.
pub fn credit(world: &mut World, player: PlayerId, amount: Fx) -> Fx {
    let Some(p) = world.economy.players.get_mut(player.0 as usize) else {
        return Fx::ZERO;
    };
    if amount <= Fx::ZERO {
        return Fx::ZERO;
    }
    let room = (p.storage - p.mana).max(0);
    let a = i64::from(amount.raw()).min(room);
    p.mana += a;
    Fx::from_raw(a as i32)
}

/// The first sacred site a sacred-site structure (`yardmap = S`) centred at
/// `pos` would stand on and claim: its footprint plus the rules'
/// `site_slack` contains `pos` (the same test income uses), and no other
/// live sacred-site unit (built or under construction), other than
/// `ignore`, stands on it. `None` if there is no such site. Placement uses
/// this to keep lodestones on sacred sites (WP-P, Known from the data:
/// only lodestones carry `yardmap = S`, and the community guides say a
/// lodestone is built on a sacred site).
pub fn free_site_at(world: &World, pos: crate::WorldPos, ignore: Option<UnitId>) -> Option<usize> {
    let setup = &world.economy.setup;
    let slack = setup.rules.site_slack;
    let taken = |s: &SacredSite| {
        world.units().any(|(id, u)| {
            Some(id) != ignore
                && world
                    .defs()
                    .get(u.def)
                    .is_some_and(|d| d.economy.sacred_site)
                && s.contains(u.pos, slack)
        })
    };
    setup
        .sacred_sites
        .iter()
        .position(|s| s.contains(pos, slack) && !taken(s))
}

/// A player's mana bar, if the player exists.
pub fn player(world: &World, player: PlayerId) -> Option<&PlayerEconomy> {
    world.economy.players.get(player.0 as usize)
}

/// A player's stock (0 for unknown players).
pub fn mana(world: &World, p: PlayerId) -> WideFx {
    WideFx(player(world, p).map_or(0, |e| e.mana))
}

/// A player's storage cap as of the last tick.
pub fn storage(world: &World, p: PlayerId) -> WideFx {
    WideFx(player(world, p).map_or(0, |e| e.storage))
}

/// A player's income per second as of the last tick.
pub fn income_per_second(world: &World, p: PlayerId) -> WideFx {
    WideFx(player(world, p).map_or(0, |e| e.income))
}

// ---------------------------------------------------------------------------
// Unit mana API (spellcasters)
// ---------------------------------------------------------------------------

/// A unit's own mana pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitMana {
    /// Current mana.
    pub mana: Fx,
    /// `maxmana`.
    pub max: Fx,
}

/// The unit's own pool, `None` if the unit is dead or has no pool.
pub fn unit_mana(world: &World, id: UnitId) -> Option<UnitMana> {
    let u = world.unit(id)?;
    let max = world.defs().get(u.def)?.economy.maxmana;
    (max > 0).then(|| UnitMana {
        mana: u.economy.mana,
        max: Fx::from_int(max),
    })
}

/// Pay `cost` (a weapon's `manapershot`) from the unit's own pool, all or
/// nothing. Returns `true` if paid (a zero cost always succeeds for a live
/// unit).
pub fn spend_unit_mana(world: &mut World, id: UnitId, cost: Fx) -> bool {
    let Some(u) = world.unit_mut(id) else {
        return false;
    };
    if cost <= Fx::ZERO {
        return true;
    }
    if u.economy.mana < cost {
        return false;
    }
    u.economy.mana -= cost;
    true
}

/// Set the unit's own pool to `percent` (clamped to 100) of `maxmana` (OTA
/// `manapercentage`). No effect on dead units or units without a pool.
pub fn set_unit_mana_percent(world: &mut World, id: UnitId, percent: u8) {
    let Some(max) = unit_mana(world, id).map(|m| m.max) else {
        return;
    };
    if let Some(u) = world.unit_mut(id) {
        let raw = i64::from(max.raw()) * i64::from(percent.min(100)) / 100;
        u.economy.mana = Fx::from_raw(raw as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_tick_is_exact_over_a_second() {
        let mut carry = 0;
        let rate = 10i64 << 16;
        let sum: i64 = (0..30).map(|_| per_tick(rate, &mut carry)).sum();
        assert_eq!(sum, rate);
        assert_eq!(carry, 0);
        let rate = (15i64 << 16) + 7;
        let sum: i64 = (0..300).map(|_| per_tick(rate, &mut carry)).sum();
        assert_eq!(sum, rate * 10);
    }

    #[test]
    fn site_contains_uses_slack() {
        let s = SacredSite {
            x: 32,
            y: 32,
            w: 32,
            h: 32,
            multiplier: Fx::ONE,
        };
        let p = |x, y| tak_math::Vec2::from_ints(x, y);
        assert!(s.contains(p(40, 40), 0));
        assert!(!s.contains(p(64, 40), 0));
        assert!(s.contains(p(70, 40), 16));
        assert!(!s.contains(p(10, 40), 16));
    }
}
