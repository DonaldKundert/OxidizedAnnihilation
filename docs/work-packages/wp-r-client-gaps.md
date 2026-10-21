# WP-R: client gaps

**Owns:** `client/` and `docs/client.md`. Core changes only if unavoidable: minimal, additive, own commit, called out.
**Inputs:** the merged client (WP-Q), `docs/client.md` "Known gaps", `docs/formats/gaf.md`, `palettes.md`, `3do.md`, `tnt.md`, `features-keys.md`, `gamedata-sub-keys.md`, `fbi-keys.md`, `docs/combat.md`, `docs/construction.md`.

Work through the list in order and commit after each item that works, with a screenshot checked by eye. Correct and TA:K-familiar beats pretty.

1. **Missing textures.** Also search `anims/*.gaf` for 3DO texture names (WP-B: `textures/` resolves 3,326 of 3,470 names; adding `anims/` gives 3,411). Report what still resolves nowhere, and draw it in a neutral colour instead of flat grey if it is clearly untextured by design.
2. **Map features with their real art.** Features use `object` (a 3DO model: render like units, static) or `filename` + `seqname` (a GAF sprite sequence: billboard, animated if `animating`), with `seqnameshad` shadows. Render every placed map feature this way (trees, rocks, ruins, lodestone sites), and corpses/wrecks from `World.features` too. Keep it fast: thousands of features per map (Adamantine Gate has ~88k cells of features in total, 85 of them gameplay-relevant), so batch or instance them.
3. **Buildings under construction.** Show the frame growing with `combat::build_progress` / `UnitBuild::percent_left()`: e.g. the model clipped to a rising height with a translucent ghost above, or TA:K's own look if the data or docs reveal it. Health bar from `frame_health`.
4. **Effects and projectiles.** Weapon projectiles use the weapon's `model` (3DO) when set, else a sprite; hits and deaths spawn the explosion sprites the weapon or `gamedata/explosions` entries name (TAF 16-bit effects: ARGB1555/4444; GAF indexed with palette). Billboards with additive or alpha blending as the format implies; the `0xFF00` subframe flag in 74 TAFs is Unknown (probably a blend mode), so try the obvious readings and keep what looks right.
5. **Minimap.** The TNT's embedded large minimap with the kingdom palette (as `tak-cli dump-map` does), unit dots in team colours, the camera rectangle; click or controller-cursor to jump.
6. **Fog of war (visual only).** Unexplored black, explored-but-not-seen dimmed, seen clear, from friendly units' `sightdistance` (client-side; gameplay line of sight is the sim's job and is not part of this item). An option to turn it off for debugging.
7. **HUD.** Clickable buttons for the build menu and orders (same actions as the keys and wheels), build-rejection reasons from `ConstructionState::rejections` (`reason()`), a defeated player shown as defeated rather than "0/0", the selected unit's health/mana/build progress.
8. **Camera.** Match TA's view: an oblique orthographic projection (height shifts the image up the screen), not a tilted orthographic camera. Check the original's look against the terrain mosaic and model screenshots; document the projection parameters.

## Done when
- Each item has a screenshot in the report showing it working on Adamantine Gate (and one other map with lots of trees, e.g. a forest map from `kmap/`), checked with the Read tool.
- Frame rate on the headless box is no worse than ~half of WP-Q's 8 fps with all features drawn (it is software rendering; say what it is).
- Client replay test still passes; core checks unchanged; `docs/client.md` updated (gaps closed, any new ones).
