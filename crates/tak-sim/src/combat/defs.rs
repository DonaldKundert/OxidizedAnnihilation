//! Combat stats of a unit definition, in sim units, plus the FBI conversion
//! and the damage formulas. Every number that comes from a guess lives in a
//! named constant or one small function here; `docs/combat.md` tags them.

use serde::{Deserialize, Serialize};
use tak_math::Fx;

use crate::TICKS_PER_SECOND;
use crate::hash::StateHasher;

/// Most weapon slots a unit may carry (Kingdoms FBIs use `WEAPON1..3`;
/// headroom for mods).
pub const MAX_WEAPONS: usize = 8;

/// Damage kind: what scripts receive as the "kind" argument of
/// `HitByWeapon`, `Dying` and `Killed`. **Believed** from the scripts'
/// branches (`Killed` blows the model apart on 3 and emits fire on 2;
/// `Dying` skips the death animation on 3; god scripts ignore hits of kind 2
/// and 4) matched to the FBI `damagetype` words.
pub mod kind {
    /// Ordinary damage (no `damagetype`).
    pub const NORMAL: i32 = 1;
    /// `damagetype = fire`.
    pub const FIRE: i32 = 2;
    /// `damagetype = explosion`.
    pub const EXPLOSION: i32 = 3;
    /// `damagetype = paralyzer`.
    pub const PARALYZER: i32 = 4;
}

/// How a weapon's shot travels. From the FBI `type` (case-insensitive).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeaponKind {
    /// `Melee`: hits the target unit at the launch moment, within contact
    /// reach (see [`melee_reach`]).
    #[default]
    Melee,
    /// `Ballistic`: flies to the aim point at `weaponvelocity`, then
    /// explodes there (no homing).
    Ballistic,
    /// `Line of Sight`: like ballistic but straight; instant without a
    /// velocity.
    LineOfSight,
    /// `Guided`: homes on the target unit.
    Guided,
    /// `Remote Effect` and `wandering`: lands on the target point at once.
    RemoteEffect,
}

impl WeaponKind {
    /// Parse the FBI `type` value. Unknown words are treated as ballistic.
    pub fn from_fbi(s: Option<&str>) -> WeaponKind {
        let s = s.unwrap_or("").to_ascii_lowercase();
        let s = s.trim();
        match s {
            "melee" => WeaponKind::Melee,
            "ballistic" => WeaponKind::Ballistic,
            "line of sight" => WeaponKind::LineOfSight,
            "guided" => WeaponKind::Guided,
            "remote effect" | "wandering" => WeaponKind::RemoteEffect,
            _ => WeaponKind::Ballistic,
        }
    }

    fn tag(self) -> u8 {
        self as u8
    }
}

/// `[DAMAGE]` in sim form: `default` and per-damage-category multipliers
/// (lowercase category, 16.16), sorted by category.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageStats {
    /// `default`.
    pub default: i32,
    /// `(category, multiplier)` pairs, sorted, unique.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub multipliers: Vec<(String, Fx)>,
}

impl DamageStats {
    /// Damage against a target of `category` before falloff: `default`
    /// times the matching multiplier, rounded toward zero. **Believed**
    /// (`docs/content.md`): non-`default` keys multiply `default`. Same
    /// arithmetic as `tak_content::DamageTable::against`.
    pub fn against(&self, category: Option<&str>) -> i32 {
        let Some(m) = category.and_then(|c| {
            self.multipliers
                .binary_search_by(|(k, _)| k.as_str().cmp(c))
                .ok()
                .map(|i| self.multipliers[i].1)
        }) else {
            return self.default;
        };
        let v = i64::from(self.default) * i64::from(m.raw()) / i64::from(Fx::ONE_RAW);
        v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }
}

/// One weapon slot in sim units.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponStats {
    /// FBI `name` (diagnostics only).
    #[serde(default)]
    pub name: String,
    /// FBI `type`.
    pub kind: WeaponKind,
    /// `range`, world units (**Believed**, TA: pixels). Ignored for melee
    /// reach (see [`melee_reach`]).
    pub range: i32,
    /// `minrange`, world units.
    #[serde(default)]
    pub min_range: i32,
    /// `reloadtime` in ticks: `trunc(seconds × 30)`, at least 1 (TA does
    /// exactly this; Believed for Kingdoms).
    pub reload_ticks: u32,
    /// `weaponvelocity`, world units per **second** (**Believed**, TA);
    /// 0 = instant.
    #[serde(default)]
    pub velocity: i32,
    /// `areaofeffect`: blast **diameter** in world units (TA).
    #[serde(default)]
    pub area_of_effect: i32,
    /// `edgeeffectiveness` (16.16); damage factor at the blast edge.
    #[serde(default)]
    pub edge_effectiveness: Fx,
    /// `aimtolerance`, angle units: how far off the unit's facing may be
    /// when it fires (**Believed**); `None` = [`DEFAULT_AIM_TOLERANCE`].
    #[serde(default)]
    pub aim_tolerance: Option<u16>,
    /// `manapershot`. Non-zero marks a spell: only fired by `CastSpell`.
    #[serde(default)]
    pub mana_per_shot: i32,
    /// [`kind`] value from `damagetype`.
    #[serde(default = "normal_kind")]
    pub damage_kind: i32,
    /// `[DAMAGE]`.
    pub damage: DamageStats,
}

fn normal_kind() -> i32 {
    kind::NORMAL
}

impl WeaponStats {
    /// `true` if the weapon costs mana per shot (a spell).
    pub fn is_spell(&self) -> bool {
        self.mana_per_shot > 0
    }

    /// Blast radius `R = areaofeffect / 2` (TA: `(u16)aoe >> 1`).
    pub fn blast_radius(&self) -> i32 {
        self.area_of_effect.max(0) / 2
    }

    /// World units per tick for travelling shots, 16.16; zero = instant.
    pub fn speed_per_tick(&self) -> Fx {
        Fx::from_ratio(self.velocity.max(0), TICKS_PER_SECOND as i32).unwrap_or(Fx::ZERO)
    }

    fn hash(&self, h: &mut StateHasher) {
        h.u8(self.kind.tag());
        h.i32(self.range);
        h.i32(self.min_range);
        h.u32(self.reload_ticks);
        h.i32(self.velocity);
        h.i32(self.area_of_effect);
        h.fx(self.edge_effectiveness);
        h.option(self.aim_tolerance, StateHasher::u16);
        h.i32(self.mana_per_shot);
        h.i32(self.damage_kind);
        h.i32(self.damage.default);
        h.len(self.damage.multipliers.len());
        for (k, m) in &self.damage.multipliers {
            h.bytes(k.as_bytes());
            h.fx(*m);
        }
    }
}

/// Combat stats of a unit definition. The default (`max_damage` 0) is a
/// unit outside combat: it is never targeted, damaged or killed, which keeps
/// the synthetic movement scenarios unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatDef {
    /// `maxdamage`: hit points. 0 = not part of combat.
    #[serde(default)]
    pub max_damage: i32,
    /// `healtime` (16.16): **Believed** hit points regenerated per second
    /// (TA reads it as a per-second worker rate; see `docs/combat.md`).
    #[serde(default)]
    pub heal_rate: Fx,
    /// `sightdistance`, world units: how far an idle unit looks for enemies
    /// to chase.
    #[serde(default)]
    pub sight: i32,
    /// `maneuverleashlength`: how far an idle unit chases from where it
    /// started (falls back to `sight`).
    #[serde(default)]
    pub leash: i32,
    /// `damagecategory`, lowercase: the row looked up in `[DAMAGE]`.
    #[serde(default)]
    pub damage_category: Option<String>,
    /// Body radius for contact and blast tests, world units: half the
    /// larger footprint side in 16-unit squares (**Chosen**; TA uses the
    /// model bounding box).
    #[serde(default)]
    pub radius: i32,
    /// `[WEAPONn]` in slot order (script weapon index = position).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weapons: Vec<WeaponStats>,
    /// `[EXPLODEAS]`: blast when the unit dies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explode_as: Option<WeaponStats>,
}

impl CombatDef {
    /// `true` for the default (no combat), so scenarios without combat keep
    /// their JSON unchanged.
    pub fn is_empty(&self) -> bool {
        *self == CombatDef::default()
    }

    /// Longest weapon reload in milliseconds, for `SetMaxReloadTime`.
    pub fn max_reload_ms(&self) -> i32 {
        self.weapons
            .iter()
            .map(|w| w.reload_ticks.saturating_mul(1000) / TICKS_PER_SECOND)
            .max()
            .unwrap_or(0)
            .min(i32::MAX as u32) as i32
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.max_damage < 0 {
            return Err("maxdamage negative".into());
        }
        if self.weapons.len() > MAX_WEAPONS {
            return Err("too many weapons".into());
        }
        let ws = self.weapons.iter().chain(self.explode_as.iter());
        for w in ws {
            if w.damage.multipliers.len() > 64 || w.name.len() > 256 {
                return Err("weapon too large".into());
            }
            if w.damage.multipliers.windows(2).any(|p| p[0].0 >= p[1].0) {
                return Err("damage multipliers must be sorted and unique".into());
            }
        }
        Ok(())
    }

    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.i32(self.max_damage);
        h.fx(self.heal_rate);
        h.i32(self.sight);
        h.i32(self.leash);
        h.option(self.damage_category.as_deref(), |h, s| h.bytes(s.as_bytes()));
        h.i32(self.radius);
        h.len(self.weapons.len());
        for w in &self.weapons {
            w.hash(h);
        }
        h.option(self.explode_as.as_ref(), |h, w| w.hash(h));
    }
}

/// Facing error a weapon tolerates when its `aimtolerance` is missing
/// (**Chosen**, 22.5 degrees).
pub const DEFAULT_AIM_TOLERANCE: u16 = 0x1000;

/// Extra gap allowed between two bodies for a melee hit (**Chosen**).
pub const MELEE_MARGIN: i32 = 16;

/// Contact reach of a melee weapon between two bodies: the sum of their
/// radii plus [`MELEE_MARGIN`]. **Chosen**: Kingdoms melee weapons author
/// `range = 300` almost everywhere, far more than the visible sword reach
/// (swordsmen walk up to their target in play), so the authored range is
/// not used as a reach.
pub fn melee_reach(attacker_radius: i32, target_radius: i32) -> i32 {
    attacker_radius
        .saturating_add(target_radius)
        .saturating_add(MELEE_MARGIN)
}

/// TA's area falloff (nanolathe retail spec §9.3, Established for TA,
/// Believed for Kingdoms): at distance `d < R` from the blast,
/// `falloff = (1 - e)·f² + e` with `f = d/R - 1`; exactly 1 at `d = 0`.
/// Returns `base × falloff`, truncated toward zero, in integers.
pub fn area_damage(base: i32, d: i32, r: i32, edge: Fx) -> i32 {
    if d <= 0 || r <= 0 {
        return base;
    }
    let d = i64::from(d.min(r));
    let r = i64::from(r);
    let e = i64::from(edge.raw());
    let one = i64::from(Fx::ONE_RAW);
    // falloff·2^16·r² = (1 - e)(r - d)² + e·r²
    let num = (one - e) * (r - d) * (r - d) + e * r * r;
    let v = i64::from(base) * num / (one * r * r);
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// FBI unit → combat stats.
pub fn combat_def_from_fbi(u: &tak_content::UnitDef) -> CombatDef {
    let footprint = i32::from(u.footprint.x.unwrap_or(1).max(u.footprint.z.unwrap_or(1)).max(1));
    let sight = u.sight_distance.unwrap_or(0).max(0);
    CombatDef {
        max_damage: u.max_damage.max(0),
        heal_rate: u.heal_time.unwrap_or(Fx::ZERO).max(Fx::ZERO),
        sight,
        leash: u
            .motion
            .maneuverleashlength
            .filter(|&l| l > 0)
            .unwrap_or(sight),
        damage_category: u.damage_category.as_ref().map(|c| c.to_ascii_lowercase()),
        radius: footprint * 8,
        weapons: u
            .weapons
            .iter()
            .take(MAX_WEAPONS)
            .map(weapon_from_fbi)
            .collect(),
        explode_as: u.explode_as.as_ref().map(weapon_from_fbi),
    }
}

/// FBI `reloadtime` (seconds, 16.16) → ticks: `trunc(s × 30)`, at least 1.
pub fn reload_ticks(seconds: Fx) -> u32 {
    let t = i64::from(seconds.raw()) * i64::from(TICKS_PER_SECOND) / i64::from(Fx::ONE_RAW);
    t.clamp(1, i64::from(u16::MAX)) as u32
}

fn weapon_from_fbi(w: &tak_content::WeaponDef) -> WeaponStats {
    let mut multipliers: Vec<(String, Fx)> = w
        .damage
        .multipliers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), *v))
        .collect();
    multipliers.sort_by(|a, b| a.0.cmp(&b.0));
    multipliers.dedup_by(|a, b| a.0 == b.0);
    multipliers.truncate(64);
    let damage_kind = match w.damage_type.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("fire") => kind::FIRE,
        Some("explosion") => kind::EXPLOSION,
        Some("paralyzer") => kind::PARALYZER,
        _ => kind::NORMAL,
    };
    WeaponStats {
        name: w.name.clone().unwrap_or_default().chars().take(64).collect(),
        kind: WeaponKind::from_fbi(w.kind.as_deref()),
        range: w.range.unwrap_or(0).max(0),
        min_range: w.min_range.unwrap_or(0).max(0),
        reload_ticks: reload_ticks(w.reload_time.unwrap_or(Fx::ONE)),
        velocity: w.weapon_velocity.unwrap_or(0).max(0),
        area_of_effect: w.area_of_effect.unwrap_or(0).max(0),
        edge_effectiveness: w.edge_effectiveness.unwrap_or(Fx::ZERO),
        aim_tolerance: w
            .aim_tolerance
            .map(|a| a.clamp(0, i32::from(u16::MAX)) as u16),
        mana_per_shot: w.mana_per_shot.unwrap_or(0).max(0),
        damage_kind,
        damage: DamageStats {
            default: w.damage.default,
            multipliers,
        },
    }
}
