# WP-V: implement the interaction design system

**Owns:** `crates/tak-input/`, `client/`, `docs/controller.md`, `docs/client.md`. The spec is `docs/ux/design-system.md`; it wins over older docs where they disagree, and you update the older docs to match.

## Order of work (commit after each)
1. **Controller profile "Kingdoms Console"** (design system §4) as the new default: reticle mode (cursor fixed at screen centre, left stick pans the camera, aim assist acts on the camera; free-cursor mode kept as a profile option), RT tap sub-select cycle, Y first spell / hold spell wheel, X context order, B cancel / hold Stop, RB tap local / double global / hold queue-and-add modifier, LB tap idle builder / hold build wheel, LT hold orders wheel, D-pad squads 1–4 (tap recall, double jump, hold assign), View tap alert cycle / hold Crystal Ball, L3 monarch, R3 reset zoom / hold track. Keep WP-S's feel-lab-chosen assist values and presets; keep the old profiles selectable (`wp_j`, previous console) for comparison.
2. **Feel lab:** add reticle mode to `tak_input::lab`; table comparing reticle vs free cursor for each preset; pick the default reticle assist values from it.
3. **"Classic TA:K" keyboard and mouse profile** (§5): exactly the original defaults (section 1 table), TA mouse model, no default build hotkeys (optional Grid add-on), and import of the player's `Keys.TDF` from the install (TDF syntax, `[CUSTOMKEYS]`, `LOWER_x`/`UPPER_x` = `UnitCommand ...` etc.; map what we support, report the rest).
4. **Wheels** (§4 layout): fixed slot positions, Stop at 12, sub-rings for Stance and Unit, build wheel pages of 8 with portrait/name/cost/time centre, shortfall shown, confirm on release / dead-centre cancel.
5. **HUD** (§6): original race skin and `igcommonbuttons.gaf` art where it decodes cleanly (fallback panels otherwise; say which), build portraits from `anims/buildpic/*.jpg`, unit card (portrait, health, mana, stance, current order, queue; experience/kills only if the sim has them), multi-selection portrait strip by type with counts and the sub-select highlight, contextual bottom bar (verbs + button prompts for the active profile), placement ghost with refusal reasons, collapsed left panel on controller profiles.
6. **Feedback and information** (§3.6, §7): order acknowledge sounds from `Sounds/*.WAV` (enable Bevy audio for WAV only; fail silent if no audio device, so headless runs still work), alerts derived from sim state (unit/structure attacked via `CombatState::impacts`, construction complete, unit trained, mana full, monarch in danger) with minimap pings and the alert-jump cycle, Crystal Ball full-screen, damage bars toggle (off / selected / all), bookmarks (F5–F8 / Ctrl+F5–F8), track unit, squads 1–9.
7. Hide every action the sim does not support yet (§9 list), reserving its slot.

## Done when
- Screenshots (checked by eye): reticle mode with snap, orders wheel with sub-ring, build wheel page with portraits, HUD on a controller profile and on Classic TA:K, multi-select strip mid sub-select cycle, placement ghost refused with reason, alert ping and jump, Crystal Ball.
- A recorded gamepad session fixture exercising every §4 binding produces the expected commands (test), and a Classic TA:K keyboard fixture does the same for §5.
- Feel-lab table for reticle vs free cursor in `docs/controller.md`.
- `docs/controller.md` fully rewritten to the new scheme (both glyph sets), and a short "what changed" section for Tyler.
- Core checks, client checks, replay and recording tests pass.
