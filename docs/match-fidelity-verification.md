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
could not resolve the supplied parent thread id (`thread not found`). The
parent subsequently authorized connector publication: draft
[PR #7](https://github.com/DonaldKundert/OxidizedAnnihilation/pull/7) preserves
only the first checkpoint. Its remote head is
`9c44f256c6e108907e4c6b2bf873d2b48fc2c921`; transferred tree
`490c3f391f379934b756d0e447a66ac5ff0ef988` exactly matches local
`64dddbb959b441d54bc728185f2684d42d696fa2` (plus preceding fixture commit
`fbcea0dca488ae194b1f1f3eaa67c8accde5661d`). No in-progress structure changes
were included. The connector reports no PR workflow runs. Actions was not touched.


## Structure-routing checkpoint

The implementation is described in `docs/pathfinding.md`. It preserves the
scripted yard-door selection and rejects a transition that newly covers a ground
unit. Paid products can leave their factory; flying units ignore these ground
blockers. Structure and feature counts remain separate, so placement diagnostics
and overlapping lifecycle removal retain their meaning. Suspended searches are
restarted when occupancy changes, under the same fixed step budget.

The wall regression fails before this slice (the class grid reports a clear line
through the living wall) and passes after it. Focused tests also cover actual
detours, doors/port readback, no-script factories, paid frame completion and death,
wreck removal, overlapping blockers, generational slot reuse, placement identity,
JSON replay and separate-process hashes. A bounded 100-mover / 30-structure soak
compares every tick of independently constructed simulations and checks cell
occupancy against a separate point-in-rectangle oracle, then checks the entire
grid for ghost blockers after teardown. The extended local soak is running;
only completed seeds/ticks will be reported.

Existing queued-placement replay coverage needed a longer verification window:
its builder now walks around the walls it has paid for. The same four walls
complete at ticks 30, 60, 241 and 258; the old 200-tick window assumed transparent
structures. The 600-tick window changes no sim rate, cost or path budget.

The owned client long result diagnostic was explicitly run after the initial
structure fix: four natural ordinary-production outcomes and their checkpoint /
raw replay checks pass. Current result ticks are 39532 (Aramon human vs training),
6790 (scripted Aramon human vs passive), 7209 (Zhon human vs training), and 33396
(scripted Zhon human vs passive). These are observations, not targets or retail
AI parity claims. A separate owned setup test verifies both training recipes
have a compatible ordinary slot against their opposing monarch; it preserves
the authored unrestricted slot on Aramon's swordsman rather than inventing a
blanket melee-versus-flying rule.

This second behavioral slice uses checkpoint rules version 4 and rejects earlier
versions, including the first checkpoint's version 3. Raw command logs remain
readable; existing constructed-base hashes change under real occupancy.


### Review follow-ups and five-faction paid production

After PR7's independent source review and merge at
`2057d9df5afe8386abae814f4d26a5ad0266b411`, the routing branch was safely
rebased onto that main commit. The nonblocking review suggestions now have
explicit regressions: Guard versus an incompatible flyer, an active paid healer
surviving default defeat (one StartBuilding/StopBuilding pair), and a delayed
script launch signal arriving after defeat. These preserve Remain's defensive
behavior and do not add new simulation rules.

The opt-in owned `faction_production` diagnostic exercises all five ordinary
land-production paths, with normal starting stock/income, no world spawns or
resource/stat edits, original COB scripts and zero script errors. It verifies
paid work on both producer and product, product egress for factories, and every
recorded raw-replay hash. It is not a retail AI or balance test.

| Faction | Authored path | Paid product complete | Replay-verified ticks |
|---|---|---:|---:|
| Aramon | ARAKING → ARAKEEP → ARASWORD | 3645 | 3885 |
| Taros | TARNECRO → TARCASTL → TARZOM | 2492 | 2732 |
| Veruna | VERMAGE → VERKEEP → VERSWORD | 4073 | 4313 |
| Zhon | ZONHUNT → ZONHAND → ZONGOB | 2529 | 2769 |
| Creon | CRESAGE → CRESMIT → CREAUTO | 2527 | 2767 |

Command: `TAK_DATA_DIR=<owned install> cargo +1.97.0 test -p tak-sim --test
faction_production -- --ignored --nocapture`. This is an explicitly run owned
check; the ignored test's normal-suite non-execution is not counted as coverage.


### Final source checks before the second checkpoint

After synchronous COB yard admission and the review follow-ups:

- `cargo +1.97.0 fmt --all --check` and `cargo +1.97.0 clippy --workspace
  --all-targets -- -D warnings` pass.
- `TAK_DATA_DIR=<owned install> cargo +1.97.0 test --workspace` passes:
  849 aggregate harness passes, zero failures, five explicitly ignored long
  diagnostics. This is the sum of harness result lines, including child helpers,
  not a unique-test count. The five-faction diagnostic was run separately.
- In `client/`, task-scoped Rust 1.97.1 formatting and all-target clippy pass;
  the full owned workspace run reports 351 aggregate passes and one ignored
  long diagnostic. That diagnostic was then explicitly run and passes all four
  ordinary-production results plus replay/checkpoint assertions. Final result
  ticks are 39532, 6947, 7260 and 33396 in the order listed above; the earlier
  observations preceded synchronous yard-port admission.
- The extended synthetic occupancy soak is still running. Its fixture uses
  NoScripts and fixed ordinary walls (no transition cells); it tests the
  structure-plane/cache/replay mechanism. Synchronous COB admission is validated
  separately on final source by the immediate-readback unit test, world-derived
  admission-mask coverage and owned scripted factory/result tests. Partial soak
  progress is not presented as a completed full run.

No rendered GUI or physical-controller test is added to these claims.
