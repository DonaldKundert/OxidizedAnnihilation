# COB VM: execution semantics (WP-G, M6)

Implementation: `crates/tak-script` (`Program`, `Instance`, `Engine`,
`HeadlessEngine`). Bytecode layout and opcode table: [`cob.md`](cob.md).
CLI: `tak-cli anim <unit> [--script Create] [--ticks 90] [--set PORT=VALUE]...
[--warmup N] [--frames 8] --out DIR`.

Tags: **Known** = observed in the retail files. **Believed** = TA-derived or
inferred, not contradicted. **Unknown** = open. **Chosen** = our definition
where the original's behavior is unknown and something has to be picked; it
is deterministic and documented, and can change when evidence turns up.

## Corpus results

- **Known**: all 204 unit scripts run `Create` for 300 ticks with **zero VM
  errors** (`tests/corpus.rs`, gated on `TAK_DATA_DIR`). 175 change pose;
  584 threads are still alive at the end (watchers and control loops).
- **Known**: a wider sweep (Create, 150 ticks with port 29 pinned to
  "moving", 1,297 calls to 17 common callbacks with zero arguments, then
  `Killed(50, 0)` and 150 more ticks) gives zero errors other than 11
  runaway loops caught by the instruction cap (see "Runaway loops").
- Unknown ports hit (reads, Create run): 27 ×9, 28 ×1,005, 29 ×12,018,
  30 ×100, 32 ×1,140, 40 ×1, 46 ×178. Sweep adds reads of 33, 34, 41, 45 and
  writes of 21 ×243, 22 ×132, 23 ×66.

## Pieces

- **Known, differs from the TA assumption**: the COB piece table is **not**
  the 3DO's depth-first pre-order. Across 203 unit/model pairs (FBI
  `unitname` → script, `objectname` → model) the orders match in **0**; all
  5,732 COB piece names resolve to a model piece by case-insensitive name.
  The COB table also omits pieces scripts never touch (e.g. the `AraGP`
  ground plate). The sim must map COB piece index → model piece **by name**
  (`Model::find`). The VM itself works purely in COB piece indexes.

## Data and calling conventions

- Data stack: one per thread, `i32`, all arithmetic wrapping. Each call
  frame records the stack height at entry; an instruction may not pop below
  it (`StackUnderflow`), so a callee cannot eat its caller's operands.
- Stack order (**Known** from usage): `move`/`turn` pop target then speed
  (`push speed; push target; turn`: every retail `move ... speed` has a huge
  speed and a small target on top). `spin` pops speed then accel (retail
  always pushes accel 0 first). `get` pops d, c, b, a, then port.
- **Believed**: `call-script`/`start-script` pop `argc` values, which become
  locals `0..argc` of the new frame in push order; `alloc-local` appends a
  zeroed local. Every local index referenced in the corpus is below the
  function's total `alloc-local` count, so params also get an `alloc-local`;
  with args seeding the first locals that only appends unused zeros, and both
  readings give the same indexes.
- **Chosen**: `return` pops a value if the frame pushed one, else returns 0.
  This makes the retail epilogue `alloc-local; return` return 0. A
  `call-script` discards the callee's return value (nothing is pushed).
- `Instance::call(name, args)` runs a function immediately. If it returns
  without blocking: `CallOutcome::Returned { value, locals }`; query
  callbacks (`QueryWeapon`, `SweetSpot`, `QueryBlood`) answer through
  `locals[0]`. If it sleeps or waits, it keeps running as a normal thread
  (`Blocked(id)`).
- `Instance::start(name, args)` spawns a thread (signal mask 0) that first
  runs on the next tick.
- Division or `mod` by zero errors the thread (**Chosen**; never hit in the
  corpus). `mod` sign follows the dividend; shifts by counts outside 0..32
  give 0 (left) or sign fill (right); shift-right is arithmetic (all
  **Chosen**, see cob.md Unknowns).

## Threads and scheduling

- Threads live in a `Vec` in creation order; IDs are never reused.
- One `Instance::tick`:
  1. advance every piece animation by one tick;
  2. walk the threads in order. A thread is runnable if ready, its sleep
     deadline tick is `<= now`, or its awaited turn/move axis is idle. Run it
     until it sleeps, waits, finishes or errors. Threads started during the
     walk are appended and run later in the same tick.
  3. drop dead threads.
- A turn or move issued during tick T starts stepping in tick T+1.
- `signal mask` kills every thread whose signal mask ANDs non-zero with
  `mask`. **Chosen**: the signalling thread itself is spared (retail always
  does `signal S; set-signal-mask S`, so it would not match anyway).
- **Believed** (TA): `start-script` threads inherit the starter's signal mask.
- An error kills only that thread and is logged (`Instance::errors`, capped,
  with `error_count`).

## Time

- 30 ticks per second. **Chosen**, per the brief: `sleep ms` wakes on the
  first tick whose start time is at or after `now + ms`, i.e. after
  `ceil(ms * 30 / 1000)` ticks, **minimum 1** (so `sleep 0` yields one tick
  rather than spinning). Negative ms count as 0. Examples: 33 → 1, 34 → 2,
  100 → 3, 115 → 4 (the swordsman walk step), 1000 → 30. `sleep_ticks()`.

## Piece animation

Per piece, per axis: position (`Fx`, 16.16 model units) and rotation
(`Angle`, 65,536 per turn), plus render flags.

- **Known**: speeds are per second. `turn ... speed <500>` compiles to
  `500 * 182 = 91022`; `move ... speed [500]` to `500 * 163840`.
- **Believed**: linear values are already in model 16.16 units (Scriptor's
  `[1] = 163840` is a source-language scale of 2.5 model units). The
  swordsman's hip bob is −98,303 raw = −1.5 model units on a ~50-unit model,
  which looks right in the render.
- Per tick each moving axis steps `speed / 30` with the remainder carried, so
  30 ticks cover exactly `speed` units. It snaps when within one step.
- Turns take the shortest way round (**Believed**, TA). Script angles are
  truncated to 16 bits.
- **Chosen**: speed 0 snaps to the target (no retail evidence).
- `turn-now`/`move-now` set the value and cancel any turn/move/spin on that
  axis. `turn`/`turn-now` cancel a spin.
- `spin`: speed (signed, units/s) and accel. Accel 0 means full speed at
  once (all retail spins). **Chosen**: non-zero accel changes speed by `accel`
  per tick. `stop-spin decel`: 0 stops at once (all retail uses); otherwise
  slows by `decel` per tick to 0. Units of non-zero accel: **Unknown**.
- `wait-for-turn` blocks while a turn is in progress or a `stop-spin` is
  slowing down; a steady spin does not block. `wait-for-move` blocks while a
  move is in progress. Waiting on an idle axis does not block.
- `show`/`hide`, `cache`/`dont-cache`, `shade`/`dont-shade`, `dont-shadow`:
  flags only (default visible, cached, shaded, shadowed).

## Engine interface

`trait Engine`: `get_unit_value`, `get(port, [a,b,c,d])`, `set`,
`rand(low, high)` (inclusive, from the world RNG), `emit_sfx`, `explode`,
`play_sound(sound, priority) -> result`, `mission_command(cmd, args) -> result`,
plus TA's unused `attach_unit`/`drop_unit`. It must not panic and returns 0
for anything it does not understand.

`HeadlessEngine` (tools and tests): register file for ports (`set` stores,
reads return the value, `HEALTH` defaults to 100), `pin(port, value)` to fake
world state, a seeded `tak_math::Rng`, an effect log (capped), and counts of
reads and writes of ports above 20.

Ports 1–20 keep TA's names (`engine::port`), **Believed**. Port meanings from
script usage (all **Believed** at best, otherwise **Unknown**):

| Port | Use in scripts | Guess |
| ---: | --- | --- |
| 17 | `Create` loops until it reads 0 | TA `BUILD_PERCENT_LEFT` (**Believed**) |
| 28 | picks `walk_water` | in water (**Unknown**) |
| 29 | `MoveWatcher`: moving = `get-unit-value 29 > 5`; wheel spin speeds scale with it | current speed (**Believed**) |
| 32 | `StatusControl` swaps damaged pieces at > 4 and > 9 | damage or veterancy level (**Unknown**) |
| 34 | picks `walk_road` | on road (**Unknown**) |
| 21, 22, 23 | written by `AimWeapon`/`RestoreAfterDelay` (`set 22 <local>`, `set 21 0`) | aim state for the engine (**Unknown**) |
| 27, 30, 33, 40, 41, 45, 46 | read occasionally | **Unknown** |

## Limits

Per instance: 256 live threads. Per thread: 1,024 stack values, 64 call
frames, 256 locals per frame, 100,000 instructions per tick. Exceeding any
limit errors that thread (`VmError`), never the process.

### Runaway loops

- **Known**: some control loops can spin without sleeping under faked
  inputs. Example: `verlihr` `MeleeControl` calls `walk` while moving, and
  `walk` does nothing (and does not sleep) while `FireWeapon`'s
  "attacking" static is set; with `TargetCleared` arriving mid-attack, the
  loop never yields. The sweep hits this in 11 scripts (`walk`/`swim`). The
  instruction cap stops it. Whether the real engine ever produces that
  state, or guards against it, is **Unknown**.

## Walk check (how it was triggered)

`tak-cli anim arasword --set 29=10 --warmup 10 --ticks 48 --frames 12 --out DIR`:
`Create` waits for port 17 = 0 (the default), starts `MoveWatcher`, which
sees port 29 > 5 and sets the moving static; `MeleeControl` then calls
`walk` repeatedly. The strip shows a full stride (legs alternate, knees bend,
sword arm swings). Without `--set` the pose never changes. The renderer
rotates `Ry(-y)·Rx(-x)·Rz(-z)` about each piece origin, chosen so knees bend
backward; whether Kingdoms negates X/Z like TA is still **Unknown**
(3do.md).

## For the sim (WP-I)

- Map COB piece index → model piece by name once per unit type.
- Implement `Engine` per unit over world state and the world RNG; feed
  port 29 (speed), 17 (build left), 28/34 (terrain) at least.
- Call `start("Create")` on spawn, `tick` once per sim tick in a fixed unit
  order, include `Instance::state_hash()` in the world hash, and use
  `call` for queries/callbacks (`QueryWeapon`, `AimWeapon`, `Killed`, ...).
  Callback argument lists remain **Unknown**.

## Unknowns

- Ports above 20 (table above) and callback argument lists.
- Non-zero spin acceleration units; speed 0 behavior.
- Whether the signalling thread can kill itself; whether new threads inherit
  the signal mask (TA says yes).
- Real-engine handling of runaway loops.
