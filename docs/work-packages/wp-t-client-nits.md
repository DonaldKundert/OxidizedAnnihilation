# WP-T: client nits

**Owns:** `client/`, `docs/client.md`, `docs/controller.md`. Small additive `tak-input` changes allowed (own commit).

1. **Minimap scale.** It draws far too small on Adamantine Gate (mostly black panel, tiny map in a corner). Fit the TNT minimap to the panel preserving aspect for every map size (6×6 multiplayer maps and the larger mission maps); check that fog darkening and the camera rectangle line up after scaling. Screenshots on Adamantine Gate, Grimm Farms, and one mission map.
2. **Wheel readability.** The centre label overlaps the cursor and adjacent slots crowd ("Return fire" against "Hold fire"). Lay slots out on a radius that scales with slot count and label width, move the centre hint off the cursor (or drop it in favour of the bottom hint bar), highlight the selected slot clearly. Same for the build wheel with 8+ entries.
3. **Spells.** Spells are weapon slots with `manapershot > 0` (see `docs/combat.md`, `SimCommand::CastSpell { spell: SpellId(slot), target }`). Show the selected unit's spells (name from the weapon def, mana cost, the caster's mana) in the order wheel, the HUD buttons and a keyboard binding; enter targeting mode with the cast cursor; disable when the caster lacks mana.
4. **Build list beyond F8.** Build buttons and the build wheel must reach every entry in the menu (paging or more slots), with keyboard access past F8.
5. **Match end (display).** WP-U is adding victory conditions to `tak-sim` in parallel with this API: `tak_sim::victory::MatchState { Running, Over { winners: Vec<PlayerId> } }`, `Sim::match_state() -> &MatchState`, and `World::is_defeated(PlayerId) -> bool`. When that is on `main` (check with `git log main`; rebase onto `main` if so), show "Victory"/"Defeat" banners and mark defeated players in the HUD from it, replacing the client-side "DEFEATED" guess. If it is not on `main` by the time items 1–4 are done, leave a clearly marked TODO and say so in the report.

## Done when
Screenshots of each item checked by eye; client replay and recording tests pass; core checks unchanged.
