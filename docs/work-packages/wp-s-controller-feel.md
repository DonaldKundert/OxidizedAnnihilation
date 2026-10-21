# WP-S: controller feel

**Owns:** `crates/tak-input/` (core, no Bevy), the client's input, camera and HUD code under `client/`, `docs/controller.md` (new).
**Inputs:** `crates/tak-input/` (WP-J: bindings, `Tuning`, magnetism, brush, wheels, `Mapper`), the client (WP-Q, WP-R), `docs/client.md`.

## Why
The headline goal of the project is that a controller is a first-class way to play TA:K. Feel is judged in Tyler's hands on his Mac, so this package does two things: make every feel parameter quick to tune and compare, and make the defaults as good as we can get them before he plays, using measurable proxies.

## Reference designs (console RTS)
- **Halo Wars / Halo Wars 2:** tap A selects one unit; double-tap A selects all of that type on screen; hold A paints a selection brush; a "local" button selects everything on screen and a "global" button everything on the map. Halo Wars 2 added control groups and hold position. Sources: waywardstrategy.com "Halo Wars: the ultimate design for console RTS" (2020).
- **Age of Empires IV (console, 2023):** context-sensitive radial menus built from the current selection, plus automation that reduces the number of actions needed. Source: caniplaythat.com, Aug 2023.
Our scheme (handoff table in `docs/work-packages/wp-j-input.md`) already follows Halo Wars for selection; check where it differs and adopt what is clearly better, behind profile settings.

## Build
1. **Live tuning.** Hot-reload the active profile JSON (watch the file) and an in-game tuning overlay (toggle key/button) to adjust every `Tuning` value with the controller or mouse while playing; save back to the profile. Every feel value must live in `Tuning`/profile, not in code.
2. **Session recording.** Record raw device events, cursor path, camera, selections and the commands produced, with timestamps, to a file; play it back in the client as a ghost cursor over the replayed match. This is how Tyler's play sessions come back to us for analysis.
3. **Cursor.** Frame-rate-independent motion; radial deadzone with an outer deadzone; response curve (exponent) and acceleration ramp; speed scaled by camera zoom so the cursor covers the same ground at any zoom; precision mode (RT) as a multiplier, not a separate code path.
4. **Aim assist.** Replace or extend magnetism with the standard pair: *friction* (cursor slows over a target) and *soft snap* (pull toward a nearby target when the stick is near idle), with hysteresis so it does not flicker between neighbours. Target priority by context: enemies when own units are selected, own units when nothing is selected, build sites in build mode. Off in precision mode.
5. **Camera.** Edge pan when the cursor pushes the screen edge (ramped), smooth pan and zoom, recentre / follow-selection, and camera-relative cursor so panning does not drag the cursor off its target.
6. **Selection and orders.** Double-tap A = same type on screen (Halo Wars) if better than the current Y mapping, local vs global select, brush growth curve, clear feedback (selection ring flash).
7. **Wheels.** Angle-to-slot with hysteresis, dead centre, slot count/size readable, context-sensitive contents (AoE IV style: what the selection can do), confirm on release or on press as a profile option.
8. **Feedback.** Use the game's own cursors from `anims/CURSORS.gaf` for context (move, attack, guard, build, invalid). Controller glyphs for Xbox and PlayStation layouts in the HUD and button hints (detect via gilrs). Optional rumble on snap, selection and taking damage (gilrs force feedback), off by default.
9. **Feel lab (core, headless).** A small harness in `tak-input` (test or example binary) with a simulated player: aims the stick at random targets with human-like reaction delay, corrective sub-movements and noise, on synthetic worlds with dense and sparse units. Measure time to acquire, overshoot, wrong-target selections and brush accuracy for each preset. Use it to pick defaults and to catch regressions; report the table in `docs/controller.md`.
10. **Presets.** At least: "Console" (Halo Wars-like defaults), "Assisted" (stronger snap and friction, slower cursor) and "Precise" (light assist, faster cursor). Selectable from the overlay and `--input-preset`.

## Done when
- All feel values are live-tunable and saved; presets switch at runtime.
- Recording and ghost playback work (headless screenshot of a ghost cursor over a replay).
- Feel-lab table shows the chosen defaults beat the WP-J defaults on acquisition time and wrong-target rate, with no regression on brush accuracy.
- `docs/controller.md`: full control reference (both glyph sets), what each tuning value does, how to record a session and send it back, and a short testing checklist for Tyler.
- Core checks, client checks and the client replay test pass.
