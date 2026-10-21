# Economy: mana (`tak_sim::economy`)

Work package WP-N (M10). Code: `crates/tak-sim/src/economy.rs`. Report:
`tak-headless economy-report`. Tags: **Known** (observed in the retail data
or the install's own docs), **Believed** (TA-derived or inferred from the
data, not contradicted), **Chosen** (our rule where the original is
unknown; deterministic, kept in one place, easy to change), **Unknown**.

Goal (Tyler's fidelity rule): the economy should *feel* like TA:K to an
experienced player: the same numbers, income pace, storage and the way
shortages slow building. Where an exact match is unknowable we get close and
say so here. Every rule that is not Known lives in
[`EconomyRules`](../crates/tak-sim/src/economy.rs) (a field of the
scenario), so a rebalance or a correction is one value, never a code change
spread across systems.

## Sources

- The retail data: `units/*.fbi`, `unitscb/*.fbi`, `features/**/*mana.tdf`,
  mission and map `.ota` files (read through `tak-cli`; nothing copied).
- The install's `Docs/*.txt` unit sheets (six bonus units). They list
  "Mana Production" and "Mana Storage" for the Aramon Flying Builder, and its
  numbers equal that unit's FBI `mogriumincome` (1) and `mogriumstorage`
  (150); its "Mana Cost" equals `buildcost` (2529). `readme.htm` says
  nothing about the economy.
- The game's GUI and translation files: there is a `ManaBar` in every
  in-game GUI and an `AutoShareMana` toggle in the diplomacy screen; the
  translation file keeps a "Starting resource levels" comment with no
  strings under it, and the skirmish/multiplayer battle menus have no
  starting-resources control.
- Community documentation (read only, paraphrased):
  - Wikipedia, [Total Annihilation: Kingdoms](https://en.wikipedia.org/wiki/Total_Annihilation:_Kingdoms):
    one resource, mana; mana-tapping structures built on special map
    locations give a steady supply; reclaiming wreckage, trees and stones
    gives an instant boost; an alternative "Crusades" balance option
    (Believed to be our `cb` ruleset, `unitscb/` + `canbuildcb/`; the
    install also has `Translate/Crusades.tdf`).
  - The original manual ([scan](https://retrogamer.biz/wp-content/uploads/2015/10/Total_Annihilation_Kingdoms_Manual.pdf)):
    Lodestones extract mana and are vital for armies; Divine Lodestones
    gather it faster but cost much more time and mana; some units
    (Mage Archers, the Assassin's cloak) spend a "personal Mana supply";
    spells against groups cost more mana. Its "Mana" / "Personal Mana"
    page was not in the text we could read.
  - [Total Annihilation Wiki: Lodestone (Veruna)](https://totalannihilation.fandom.com/wiki/Lodestone_(TAK_Unit_Veruna)),
    [TAK overview](https://totalannihilation.fandom.com/wiki/Total_Annihilation_Kingdoms?oldid=5552),
    [StrategyWiki walkthrough](https://strategywiki.org/wiki/Total_Annihilation:_Kingdoms/Walkthrough)
    (a Lodestone is built *on* the Sacred Site),
    [GameFAQs strategy guide](https://gamefaqs.gamespot.com/pc/142927-total-annihilation-kingdoms/faqs/2537)
    (monarchs build lodestones early; Lokken has fast mana regeneration;
    spells want "full mana").
  None of these give income formulas or numbers, so the rules below come
  from the data.

## What the data says

| Who | `mogriumincome` | `mogriumstorage` | other | Count |
|---|--:|--:|---|---|
| Monarchs (`ARAKING`, `TARNECRO`, `ZONHUNT`, `CRESAGE`, `ZONHURT`, `TARNECR2`) | 10 | 5000 | `economybonus = 30`, `maxmana` 1000 (5000 `TARNECR2`), `manarechargerate` 10–25 | Known |
| `VERMAGE` (Veruna monarch) | 10 (`cb`: 15) | 5000 | **no** `economybonus` | Known |
| `TARHEL` (mission hero) | 5 | 2500 | `economybonus = 15` | Known |
| Lodestone (`*LODE`, `yardmap = S`) | 10 | 1000 | cost 280, buildtime 92 | Known |
| Divine Lodestone (`*MANA`, `yardmap = S`) | 20 | 2000 | cost 8564, buildtime 1450 | Known |
| Builders, factories, keeps, castles, priests, ships (about 30) | 1 | 100–300 | | Known |
| Spellcasters (archers, mages, dragons, ...) | — | — | `maxmana` 20–5000, `manarechargerate` 8–125 | Known |

- `yardmap = S` appears on exactly the ten lodestone units (plus
  `NPCTEMP2`): **Believed** to mean "must stand on a sacred site".
- Sacred sites are the features `*Mana01..03` (`AraMana01`, `TarMana02`,
  `CReMana03`, ...; `category = Mana`, 2 x 2 footprint, indestructible, not
  reclaimable) with `sacredsite` = 1.0 ("weak"), 1.5 ("swarthy",
  "robust", "middling") or 2.0 ("powerful", "strong"); the developers'
  comments in the TDFs name the tiers. The `*Henge*` features are scenery.
- Features with `energy` (250, 525 or 75; trees, crops, rocks, wreckage):
  **Believed** mana gained by reclaiming them (Wikipedia's "instant boost").
- Weapons with `manapershot` (100–900): **Believed** spell cost from the
  casting unit's own pool (`maxmana`).
- Mission units carry `manapercentage`: 0 on 7270 of 7341 placed units
  (including archers, mages and monarchs), 100 on 70 (mostly `ARANULL`, a
  20-mana spell helper, plus a few monarchs and heroes). **Believed**: the
  starting fill of the unit's own pool, so a unit starts empty unless the
  designer asked for full.

## Rules

### Player mana (the "mana bar")

| Rule | Value | Tag |
|---|---|---|
| Income | sum of `mogriumincome` over the player's live, **completed** units, per **second** | Known keys (install doc calls it "Mana Production"); per second **Believed** (TA's `MetalMake`/`EnergyMake` are per second, and the pace fits: a lone monarch makes 10/s, a swordsman costs 285) |
| Lodestones | income x the `sacredsite` value of the site under them (10, 15 or 20 for a Lodestone; 20, 30, 40 for a Divine Lodestone) | **Believed** (the key's only plausible job; tier names say weak to powerful) |
| Lodestone not on a site | x `off_site_multiplier` = 1.0 | **Chosen** (cannot happen through building: placement requires a free site, WP-P) |
| Site claims | a site serves one lodestone; lodestones claim the first free site whose footprint (top-left anchored, plus one 16-px cell of slack on every side, which also covers centre anchoring) contains their position, in slot order | **Chosen** |
| Storage cap | sum of `mogriumstorage` over the player's live, completed units (monarch 5000) | Known keys ("Mana Storage"); **Believed** semantics (TA) |
| Overflow | income beyond the cap is lost; if the cap shrinks (a lodestone dies) the excess is lost | **Believed** (TA) |
| Units under construction | give no income and no storage until complete | **Believed** (TA) |
| Starting mana | 1000 per player (`start_mana`), then capped by storage | **Chosen**: the menus have no setting, TA's default start is 1000 |
| `economybonus` | ignored by default (`economy_bonus = Ignore`). Options: `UnitIncomePercent` (adds N% to that unit's own income: 10 → 13) or `PlayerIncomePercent` (adds N% to the player's whole income while the unit lives) | **Unknown**, default kept after a second look (WP-P, below) |
| Alliance sharing (`AutoShareMana`) | not modelled | Unknown (diplomacy is later) |

**`economybonus`, the evidence (WP-P, `economybonus_evidence` test).**

- Who: exactly seven units, all monarch-class: `ARAKING`, `CRESAGE`,
  `TARNECRO`, `TARNECR2`, `ZONHUNT`, `ZONHURT` (30) and the mission hero
  `TARHEL` (15). In every case `economybonus = 3 × mogriumincome` and
  `mogriumstorage = 500 × mogriumincome` (Known). The Veruna monarch
  `VERMAGE` (income 10) has none.
- Crusades (`cb`): `VERMAGE` income 10 → 15, but `cb` also raises every
  priest-class unit 1 → 5 (`ARAPRIES`, `CRECHIE`, `TARPRIES`, `VERLIHR`,
  `ZONSHAM`) and the Aramon flying builder 1 → 2, while the bonus
  carriers stay at 10. So the Veruna change reads as part of a general
  mana rebalance, which weakens WP-N's "15 ≈ 10 + 30 %" hint.
- Documentation: the install's six unit sheets (`Docs/*.txt`) and
  `readme.htm` never mention it; a web search for the key (October 2026)
  finds only general game pages, none giving a meaning.
- Readings: a percentage of the unit's own income (`UnitIncomePercent`,
  monarch 10 → 13/s), of the player's income (`PlayerIncomePercent`), or a
  flat amount (monarch 10 → 40/s, which would make lodestones nearly
  pointless and contradicts the pace players describe). The fixed 3:1
  ratio suggests a value derived from income by the designers' tool, which
  fits a percentage better than a flat amount.
- Decision (per `CLAUDE.md`'s fidelity rule): keep `Ignore` as the default
  (confidence that it has *some* effect: low-moderate; no reading is
  supported well enough to change everyone's income). If footage shows a
  lone monarch earning 13/s instead of 10/s, switch the default to
  `UnitIncomePercent`, which is already implemented and tested.

Arithmetic: income is kept in 16.16 per second (`i64`), and each tick adds
`(income + carry) / 30` with the remainder carried, so a second of ticks adds
exactly the per-second income (no drift). Stock and storage are 16.16 in
`i64` (a big base passes `Fx`'s 32767).

Example (Known numbers, Believed units): a lone Aramon monarch earns
10 mana/s (600/min) and stores 5000. Each Lodestone on a weak / swarthy /
powerful site adds 10 / 15 / 20 per second and 1000 storage; it costs 280,
so it repays itself in 14–28 seconds.

### Spending (`drain`)

Building in TA is paid while it progresses, not up front; Kingdoms keeps
TA's `buildcost` / `buildtime` / `workertime` keys, so construction is
**Believed** to work the same way: each tick a builder asks for
`buildcost x progress_this_tick / buildtime` and scales its progress by
what it gets.

`economy::drain(world, player, want) -> granted`:

- `granted` is between 0 and `want`, never more than the stock left; the
  stock drops by `granted`. `want <= 0` or an unknown player grants 0.
- **Shortage (TA behaviour, Believed)**: when the player cannot afford what
  everyone wants, every consumer is slowed by the same fraction instead of
  the first one taking everything. We do this with a one-tick lag, which is
  deterministic and needs no two-phase API: at the end of each tick
  `economy::tick` computes `ratio = min(1, stock / demand)` from this tick's
  total demand and the stock after income, and next tick every `drain`
  grants `want x ratio` (still capped by the stock left). With steady demand
  (builders), each gets the same share of the income, as in TA. In the first
  tick of a new shortage consumers are served in call order (slot order).
  **Chosen** mechanism; the observable result (everyone slows evenly, total
  spending equals income while stalled) is the TA behaviour.
- `economy::credit(world, player, amount)` adds mana (reclaim, refunds),
  capped by storage; returns what was added.

### Unit mana (spellcasters)

| Rule | Value | Tag |
|---|---|---|
| Pool | every unit with `maxmana > 0` has its own pool, 0..`maxmana` | Known keys; manual's "personal Mana supply" |
| Recharge | `manarechargerate` per **second** while complete (16.16, remainder carried) | **Believed** (TA rates are per second; Elsin's 20/s refills 1000 in 50 s) |
| Spell cost | the weapon's `manapershot`, paid whole from the caster's pool; no partial casts | **Believed** |
| Spawned / built units | start empty | **Believed** (mission default `manapercentage = 0`) |
| Start monarchs | `monarch_start_mana_percent` = 0 | **Chosen**, following the mission default |
| Placed units | `set_unit_mana_percent` (the future mission loader feeds OTA `manapercentage`) | Believed |

The player's mana and unit pools are separate: spells never touch the mana
bar, building never touches unit pools (**Believed**).

## API (for construction, combat, tools and the UI)

```rust
economy::drain(world, player, want: Fx) -> Fx          // construction, per tick
economy::credit(world, player, amount: Fx) -> Fx       // reclaim
economy::tick(world, scripts)                          // lane 4 of the tick
economy::player(world, player) -> Option<&PlayerEconomy> // stock, storage, income, last demand/granted
economy::mana / storage / income_per_second(world, player) -> WideFx
economy::unit_mana(world, id) -> Option<UnitMana>      // { mana, max }
economy::spend_unit_mana(world, id, cost: Fx) -> bool  // combat: all or nothing
economy::set_unit_mana_percent(world, id, percent)
economy::free_site_at(world, pos, ignore) -> Option<usize> // placement: lodestones (WP-P)
```

**Placement** (WP-P) refuses a `yardmap = S` structure unless
`free_site_at` finds a sacred site containing its centre (plus
`site_slack`) that no other sacred-site unit stands on, so every placed
lodestone earns its site's multiplier; `off_site_multiplier` now only
matters for units spawned directly (tools, missions).

**Construction** should, per building unit per tick, compute the mana for
this tick's progress, call `drain`, and advance progress by
`progress x granted / want`. It must not call `drain` for finished or
paused work, so demand reflects real consumers. The economy reads
`unit.build.percent_left() == 0` to decide whether a unit is complete.
**Combat** calls `spend_unit_mana(caster, manapershot)` when a spell would
fire and holds fire if it returns `false`.

## Headless report

```sh
tak-headless economy-report --map "kmap/Adamantine Gate" --players ARA,ZON \
    --ticks 1800 --every 300
```

prints one line per player every `--every` ticks: stock, storage, income per
second, and the demand and grant of the last tick.

## Determinism

No floats; players in id order, units in slot order, sites in map order;
all state (`EconomyState`, per-unit pools and carries) is hashed every
tick. `crates/tak-sim/tests/economy_determinism.rs` runs the same economy
scenario in two processes and compares every hash.

## Unknown / open

- What `economybonus` does (option, off by default).
- Whether income is truly per second (moderate confidence; check against
  footage: a lone monarch should add 600 mana per minute).
- Starting mana in skirmish (Chosen 1000) and the start monarch's pool
  (Chosen empty).
- How the original shares mana in a shortage (TA proportional assumed).
- Whether a site's multiplier also scales the lodestone's storage (assumed
  not).
- Reclaim rates for `energy` features (construction/reclaim package).
- Allied mana sharing.
