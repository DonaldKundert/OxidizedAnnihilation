# WP-U: victory conditions

**Owns:** a new `crates/tak-sim/src/victory.rs`, its wiring (tick order in `systems.rs`/`sim.rs`, one additive field on `World`/`Scenario`), tests prefixed `victory_`, `docs/sim.md` section, `crates/tak-headless/` (report the outcome). No client changes: WP-T is building the client display in parallel against the API below, so implement exactly this API (add to it, don't rename):

```rust
pub enum MatchState { Running, Over { winners: Vec<PlayerId> } }   // tak_sim::victory
impl Sim { pub fn match_state(&self) -> &MatchState }
impl World { pub fn is_defeated(&self, p: PlayerId) -> bool }
```

## What we know
- Maps' OTA `[GLOBALHEADER]` carries win/lose condition keys, kept verbatim in `tak_content::OtaDef::conditions`: `commanderkilled`, `killenemycommander`, `allunitskilled`, `destroyallunits`, `killallmobileunits`, `killalloftype`, `killunittype`, `unittypekilled`, `allunitskilledoftype`, `moveunittoradius`, `unittypepassesx/z`, `deathtimerrunsout`. Run `tak-cli keys --ext ota` to see which appear in multiplayer maps vs missions and with what values.
- In TA, a skirmish's common default is "commander dies, you lose" (with a lobby option for "continue" / "game ends"); missions use their OTA conditions. Kingdoms' monarch plays the commander role. Confirm what you can from the install's `readme.htm` / docs and the OTA data; tag Known/Believed/Chosen.

## Build
1. Per-player defeat detection in the sim (deterministic, in a fixed tick slot after combat), from a match-level rule chosen at setup: default for multiplayer maps from the data (likely "monarch killed → defeated"), plus the alternative "all units killed", as a named `Scenario` option. When a player is defeated, what happens to their remaining units (TA: blow up / stay / pass control) is a rule to look up; pick the TA-familiar default, keep it configurable.
2. `MatchState::Over` when one player (or allied team, if alliances exist in the data; otherwise free-for-all) remains. The sim keeps ticking but accepts no commands from defeated players.
3. Mission conditions: parse the OTA condition keys into a typed rule set as far as they are clear; implement the simple ones (kill commander, kill all, kill unit type, unit reaches radius) and log the rest as unsupported.
4. Hash the victory state. Cross-process determinism test. `tak-headless run` prints the outcome and the tick it was decided.

## Done when
Unit tests on synthetic scenarios for each implemented rule; a real-map test where one monarch kills the other and the match ends with the right winner; docs updated with confidence tags; all core checks pass.
