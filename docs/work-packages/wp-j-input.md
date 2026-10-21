# WP-J: input layer (M11)

**Owns:** `crates/tak-input/`.
**Inputs on main:** `tak_math` and `tak_commands` (WP-F). Read the handoff's command-layer section (summarized in `docs/plan.md` and the controller table below). No game data is required.

## What wave 1 established (WP-F notes)
- `WorldPos = Vec2`. `ClientCommand`s take world positions, so the input layer converts screen coordinates to world through a camera abstraction it defines.
- `ControlGroup::new(0..=9)` validates group numbers. Shift-click maps to `QueueMode::Append`. `Build { at: None }` adds to a factory queue.
- `SimCommand` and `ClientCommand` serialize with a `"type"` tag.

## Controller mapping target (from the handoff)
| Input | Produces |
|---|---|
| Left stick | Cursor movement with magnetism toward units and buildings |
| Right stick / R3 | Camera pan / `CenterOnMonarch` |
| RT (hold) | Precision cursor (reduced speed, magnetism off) |
| LT (hold) | Command wheel: Stop, Patrol, Guard, fire states, CastSpell |
| A / hold A | `SelectAt` / `SelectBrush` with radius growing while held |
| X | Context action: Move, Attack, or Build depending on target |
| Y / double Y | `SelectSameType` on screen / global |
| B | Cancel or deselect |
| LB (hold) | Build wheel for the selected builder |
| D-pad / RB + D-pad | `RecallGroup` / `AssignGroup` 1–4 |
| View / hold View | `JumpToAlert` / `CycleIdleBuilder` |

## Build
1. A device-neutral event type (`ButtonDown/Up`, `Axis`, `PointerMove`, `Key`), timestamped in ticks or ms. No gamepad library in the core; a `gilrs` adapter can come later behind a feature.
2. Data-driven bindings (serde JSON) mapping events to actions, including hold, double-tap, and chord (RB + D-pad) recognition. A default controller profile matching the table, plus a mouse/keyboard profile producing the same commands.
3. A read-only `WorldView` trait (unit positions, owners, def ids, selectable, is-builder, on-screen test) so everything is testable without a sim.
4. Pure functions: cursor magnetism (snap/attraction toward the nearest selectable within a radius, disabled in precision mode), selection brush (radius growth while held, additive), context action resolution (enemy → Attack, ground → Move, build mode → Build), radial wheel state machine (open, stick angle to slot via `tak_math::Angle`, confirm, cancel).
5. Output: an ordered stream of `ClientCommand` and `SimCommand`, with sim commands stamped by the caller's tick.

## Done when
- A recorded gamepad event stream (synthetic JSON fixture) converts to the expected command sequence in a test, covering select, brush select, context move/attack, groups, both wheels, and cancel.
- Mouse/keyboard and controller profiles produce identical sim commands for equivalent intents (property test).
- Magnetism, brush and wheel math use `tak_math` types only; no floats.
