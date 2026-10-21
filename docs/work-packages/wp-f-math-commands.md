# WP-F: tak-math and tak-commands (M8 prep, no game data needed)

**Owns:** `crates/tak-math/`, `crates/tak-commands/`.

## Why it matters
Every simulation number goes through `tak-math`, and every input goes through `tak-commands`. Getting determinism right here is cheap now and very expensive later. This package needs no game data, so it can run in a session that is not linked to the Mac.

## What to build (from the handoff's determinism rules)
1. `Fx`: signed 16.16 fixed point as an `i32` newtype. `from_int`, `to_int` (floor and round), add, sub, neg, mul, div (checked, returning `Option`), `abs`, `min`, `max`, `clamp`, `sqrt` (integer Newton or bit-by-bit, deterministic). Saturating and wrapping behavior chosen explicitly and documented. `Display`/`Debug` showing a decimal approximation.
2. `Angle(u16)`: 65,536 per turn, wrapping add and sub, and a table-driven `sin`/`cos` returning `Fx`. Generate the table at build time or as a `const` array. No floats at runtime, and no `f64` in table generation either if it can be avoided (if the table is generated with floats, commit the generated table and test it, so platforms cannot differ).
3. `atan2(Fx, Fx) -> Angle`, integer only.
4. `Vec2`/`Vec3` of `Fx` with dot, length, normalize.
5. `Rng`: a small, seedable, documented PRNG (e.g. xoshiro128** or PCG32) with `next_u32` and `range(lo, hi)` without modulo bias. No OS seeding anywhere.
6. `tak-commands`: `PlayerId`, `UnitId` (generational), `SimCommand` and `ClientCommand` as in the handoff's command-layer sketch, `CommandLog` (tick-stamped, ordered) with serde JSON round trip. Leave unit and spell IDs as opaque newtypes; content definitions come later.

## Done when
- `proptest` coverage: mul/div round trips and error bounds, `sin² + cos² ≈ 1` within a documented tolerance, `atan2(sin a, cos a) ≈ a`, RNG reproducibility from a seed, command-log JSON round trips.
- No `f32`/`f64` in any public type or runtime path of `tak-math` (a test or lint enforces it).
- Doc comments explain every rounding choice.
