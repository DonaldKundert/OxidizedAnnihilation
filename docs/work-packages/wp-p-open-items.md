# WP-P: wave 3 open items

**Owns:** `crates/tak-sim/src/combat.rs`, `construction.rs`, `economy.rs` (and their folders), `pathfinding.rs` only for the dynamic-obstacle hook, `crates/tak-sim/src/content.rs` for `load_match`, related tests and docs (`docs/combat.md`, `construction.md`, `economy.md`, `sim.md`).

The Bevy client (WP-Q) is being built in parallel under `client/`; it consumes `tak-sim`'s public API. Keep public APIs stable; if you change one, note it for the client.

## Items
1. **Real matches load build trees.** `load_match` still loads only the monarchs (an old test checks the count). Make full build trees the default for `load_match` (and so for `tak-headless` and `tak-cli render`), update that test, keep a monarchs-only path if any test needs it.
2. **Frame health.** Units under construction start with health scaled by build progress and gain health as they are built (TA behaviour; check Nanolathe's research docs for TA's exact rule, and note confidence). Damage to a frame is permanent progress loss or health loss, whichever TA does.
3. **Wrecks and corpses.** `CombatState.corpses` holds corpses waiting for a feature system. Add a sim-side feature layer: placed map features from `MapDef` that matter to gameplay (blocking, reclaimable later) plus corpses spawned on death from the unit's `corpse` key, with footprint, blocking, and health if the data has it. Blocking features must update pathfinding's movement-class grids (add a small, well-defined hook in pathfinding). Hash it.
4. **Assist via Guard.** In TA a builder ordered to Guard a unit under construction (or a factory, or another builder) helps build. Implement that for construction (add the builder's workertime to the frame, paying mana through `economy::drain`), and document.
5. **Lodestones need sacred sites.** `yardmap = S` structures may only be placed on a sacred site (WP-N found this); enforce it in placement and show the reason in the build-rejection result.
6. **`economybonus`.** Its meaning is Unknown. Look at which units have it and with what values, the Crusades hint (Veruna monarch 15 instead of 10 in `cb`), and any documentation; pick the most likely reading per `CLAUDE.md`'s fidelity rule and record the confidence. Default may stay "ignored" if the evidence is too thin.

## Done when
- Each item has a test (synthetic defs for formulas, real data gated for integration), and a cross-process determinism test still covers the full sim with all systems on.
- A real-map run: two players with full build trees, a monarch builds a Keep, the Keep trains units, they fight, and corpses appear and block pathing. `tak-cli render` PNG of the aftermath; look at it before reporting.
- All core checks pass; docs updated with confidence tags.
