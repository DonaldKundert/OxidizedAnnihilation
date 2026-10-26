# Playable-match fidelity audit — 26 October 2026

Started from main `bfeca6827e84793ef79d816c4426dbc0fd0ba355` (PR6),
tree `c38c06727340b210808e4febb5b1eec27460fb4d`, in an isolated task checkout.
GitHub Actions stays disabled; all validation here runs locally. PR6's prior
818-core / 350-client and 221590-tick results are historical, not validation
of these changes. No physical controller has been tested in this pass.

## Environment and evidence

The owned GOG Mac wrapper and its archives are present and readable on this
computer. The test environment explicitly sets `TAK_DATA_DIR` to that install.
No owned assets, extracted retail data or private saves are added to this
repository. Synthetic tests create definitions in code.

The preinstalled Rust 1.97.0 is selected explicitly for core checks. The client
requires 1.97.1 through its locked Bevy/WESL dependencies; that toolchain was
installed separately in `/tmp/oa-rustup` without changing the user's default.
The MIT Nanolathe research checkout is outside this repository in `/tmp`;
only research documents were consulted, no GPL engine source.

The original [Kingdoms manual](https://retrogamer.biz/wp-content/uploads/2015/10/Total_Annihilation_Kingdoms_Manual.pdf)
(printed pp. 66–67) confirms structures can form physical barriers, the HUD
shows mana rates per second, and spellcasters keep regenerating personal pools.
It supports the current economy direction without justifying a numerical
rebalance. Yard-cell specifics come from the approved MIT
[Nanolathe TA research, R-COLL-01 §4](https://github.com/nanolathe-gg/nanolathe/blob/main/research/retail-executable-spec/04-units-orders-scripts-and-movement.md),
so their use for Kingdoms must remain tagged Believed.

## Prioritized findings

| Area | Evidence and impact | Disposition |
|---|---|---|
| Ground-only weapons | Content parses `noairweapon`, but sim conversion dropped it; ground-only attacks could select flyers. 58 flagged slots across owned base/Crusades rulesets. | Implemented; `docs/combat.md`, `tests/air_targeting.rs`. |
| Default defeat survivors | The ordinary 30000-damage sweep leaves high-health/outside-combat units alive. A synthetic three-player regression moved a survivor from x=306 to x=606 over 300 ticks after its monarch died. | Implemented inert survivors; Remain option preserved. `docs/sim.md`, `tests/victory.rs`. |
| Structure routing | Movement separation pushes around circular bodies, but class-grid/A* blockers contain only terrain and wrecks. Buildings/walls are absent from route planning. | Next slice; must preserve yard doors, product exits and flyer behavior. |
| Battlefield clearing | Feature metadata exists, but no player reclaim command clears persistent blocking wrecks. | Open: do not invent original reclaim rates/payouts. |
| Paralyzer effects | Damage-kind mapping exists; impact still applies ordinary HP damage. | Open: accumulation, duration, immunities and petrification need primary evidence. |
| Economy interpretation | `economybonus`, personal-mana spawn fill and shortage sharing retain documented uncertainty. | No speculative rebalance. |

Replay checkpoints now use rules version 3: old behavioral snapshots are rejected
explicitly rather than resumed under different combat/defeat rules. Raw commands
remain readable, but replaying them applies the current simulation.

## First checkpoint validation

- Focused ground-only targeting suite: 9 harness tests pass, including an
  owned mapping check (58 flagged slots), ordinary/mixed-slot acquisition,
  explicit commands, AttackMove, ReturnFire, a rejected spell without mana
  cost, delayed script launch and a separate-process hash comparison.
- Default-survivor regression fails before the fix (300 units of post-defeat
  movement) and passes after it. Victory coverage includes the defensive
  Remain option, outside-combat survivors, damage/removal and cross-process
  hashes with survivors.
- Baseline full core run found a VFS fixture assuming a case-sensitive
  filesystem. Its two filenames refer to the same file on this Mac. The
  fixture now verifies collision ordering when distinct files exist and
  case-insensitive lookup otherwise; engine behavior is unchanged.
- Core formatting and all-target clippy with warnings denied pass. The full
  owned core run passes (831 aggregate harness passes, zero failures, three
  explicitly ignored long diagnostics). The no-install core run also passes;
  its asset-dependent early returns are not counted as owned-data validation.
- The full owned client workspace run passes on the task-scoped Rust 1.97.1
  toolchain. These are simulated-input and headless tests, not a rendered
  playthrough or a physical-controller session.
- Checkpoint rules-version tests pass after the version-3 bump and explicitly
  reject rules versions 1 and 2 before loading their payloads.

Publication is coordinated with the parent. The desktop thread-message tool
could not resolve the supplied parent thread id (`thread not found`); this
checkout has not been pushed, no PR created, and Actions has not been touched.
