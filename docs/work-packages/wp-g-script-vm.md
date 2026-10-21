# WP-G: COB script VM (M6)

**Owns:** `crates/tak-script/`, `docs/formats/cob-vm.md`, `crates/tak-cli/src/anim_cmd.rs`.
**Inputs on main:** `tak_formats::cob` (WP-D), `tak_formats::model3do` (WP-B), `tak_math` (WP-F). Read `docs/formats/cob.md` and `docs/formats/3do.md` first.

## What wave 1 established
- `cob::Op` has `from_raw`, `operands()`, `confidence()`; `cob::decode`, `decode_all`, `encode`, and `Script::to_bytes` build synthetic test scripts. 56 of 66 opcodes are used; 3 Kingdoms-only ones (`0x10039000..0x1003B000`) are **Believed** to be shift-left, shift-right, mod.
- 4,384 of 4,425 functions end `alloc-local; return`, which must return 0 without underflowing the stack. Functions open with one `alloc-local` per parameter or local; arguments probably seed the first locals.
- Statics: one block of the header's static count per unit; `Create` initialises them.
- Movement is a helper thread `MoveWatcher` started from `Create`. Kingdoms dropped TA's `StartMoving`, `StopMoving`, `AimPrimary`. Common callbacks: Create, Killed, SweetSpot, Dying, AimWeapon, FireWeapon, QueryWeapon, TargetCleared, QueryBlood, TurnDirection, SetMaxReloadTime, HitByWeapon, StartBuilding, StopBuilding, setSFXoccupy, Activate, Deactivate, MoveRate, QueryBuildInfo. Missions: Start, TriggerHit, UnitDestroyed, UnitCreated, and a `mission-command` opcode whose strings live in the sound table.
- Pieces are referenced by index into the COB piece table. `Model::pieces` is depth-first pre-order and **Believed** to match; confirm by name across the corpus (names are mixed case; compare case-insensitively).
- Engine ports above 20 (21, 22, 23, 26, 28, 29, 32, 34, ...) are **Unknown**.

## Build
1. **VM core**: per-unit script instance with statics, a set of threads (each with call stack, locals, data stack, program counter), deterministic round-robin scheduling in a fixed order (thread creation order), all integer state.
2. **Time**: the sim ticks at 30 Hz. `sleep(ms)` wakes on the first tick at or after the deadline. Define exactly how ms map to ticks and document it.
3. **Piece animation state**: per piece, per axis: current position/rotation, target, speed, acceleration, `turn-now`/`move-now`, `spin`/`stop-spin`, `wait-for-turn`/`wait-for-move` blocking, show/hide, cache/dont-cache, shade flags. Angles are 65,536 per turn (`tak_math::Angle`), linear units are the model's 16.16 fixed point (`tak_math::Fx`). Advance once per tick.
4. **Engine interface**: a trait the VM calls for everything outside the script (get/set unit values by port, emit-sfx, explode, play-sound, mission-command, start/stop sounds, random numbers from the sim's `Rng`). Unknown ports return 0 and are counted, never panic. `tak-script` must not depend on `tak-sim`.
5. **Call entry points**: `call(script_name, args)` and `start(script_name, args)`, returning query results the way the docs describe (the query callbacks write a piece index into a local).
6. **Limits**: cap threads, stack depth, call depth and instructions per tick; a runaway script errors that thread, not the process.
7. `tak-cli anim <unit> [--script Create] [--ticks 90] --out DIR`: run a unit's script headless against its model and write per-tick piece transforms as JSON, plus a PNG strip of a few frames using a simple rasterizer (reuse the WP-B approach: integer transforms, floats only in the export).

## Done when
- Every one of the 204 unit scripts runs `Create` for 300 ticks with zero VM errors (unknown ports allowed but counted and reported).
- A walking unit's limbs move plausibly: the PNG strip for one infantry unit (e.g. an Aramon swordsman) shows a walk cycle when its `MoveWatcher`-driven walk is triggered. Document how you triggered it.
- Unit tests on synthetic scripts cover every opcode family, sleep timing, wait-for-turn, thread start/kill, and stack underflow.
- Determinism test: same script and inputs give identical per-tick transform hashes on two runs.
