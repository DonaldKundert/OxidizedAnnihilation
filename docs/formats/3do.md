# 3DO: 3D object models (`.3do`)

Parser: `crates/tak-formats/src/model3do.rs` (`model3do::parse`). Export tool:
`tak-cli dump-model <path-or-name> --out DIR` (OBJ + MTL + JSON piece tree).
Reference: Nanolathe `research/formats/3do.md` (MIT, original TA). Every claim
below was checked against all 447 Kingdoms models unless tagged otherwise.

Tags: **Known** = observed in all Kingdoms files. **Believed** = TA-derived or
inferred, not contradicted. **Unknown** = open.

## Corpus

- **Known**: 447 `.3do` files under `objects3d/` (mixed-case `Objects3D/` in some
  archives; the VFS folds case). Unit prefixes: `ara` (Aramon), `ver` (Veruna),
  `tar` (Taros), `zon` (Zhon), plus `cre`, `npc`, `mon`, `lif`, `wre`. Corpses
  are `<name>_dead.3do`. A unit's FBI `objectname` names its model.
- **Known**: all 447 parse with zero errors. Max hierarchy depth 13
  (`araknigh` and others). 45 models are a single piece, 64 are two pieces;
  unit models typically have 15 to 71 pieces and 128 to 512 vertices.

## Layout

Same as TA. No file header: the root piece record sits at offset 0. All fields
little-endian; all offsets absolute file offsets.

### Piece record (52 bytes) — **Known**

| Off | Type | Field | Kingdoms observation |
| ---: | --- | --- | --- |
| 0x00 | u32 | version | 1 in all 9,631 piece records. Not validated by the parser. |
| 0x04 | i32 | vertex count | ≥ 0 |
| 0x08 | i32 | primitive count | ≥ 0 |
| 0x0C | i32 | selection primitive | Roots: `0` ×411, `-1` ×32, positive index ×4. A primitive index, as in TA. |
| 0x10 | i32 ×3 | x, y, z offset from parent | signed 16.16 fixed point |
| 0x1C | i32 | name offset | → NUL-terminated name |
| 0x20 | u32 | aux offset | 0 everywhere |
| 0x24 | i32 | vertex array offset | → count × 12 bytes (`i32 x, y, z`, 16.16) |
| 0x28 | i32 | primitive array offset | → count × 32 bytes |
| 0x2C | i32 | sibling offset | 0 = end of sibling list |
| 0x30 | i32 | child offset | 0 = leaf |

- **Known**: no root has a sibling (the parser still accepts one, giving a
  second parentless piece).

### Primitive record (32 bytes) — **Known**

| Off | Type | Field | Kingdoms observation |
| ---: | --- | --- | --- |
| 0x00 | u32 | color index | Untextured: 0..=252 (most often 0). Textured: usually 0, sometimes residue. |
| 0x04 | i32 | index count | 2 ×1,342, 3 ×4,107, 4 ×42,022, 5 ×23, 8 ×2 |
| 0x08 | u32 | aux offset | 0 everywhere |
| 0x0C | i32 | index array offset | → count × u16 indices into **this piece's** vertices; all in range |
| 0x10 | i32 | texture name offset | → NUL-terminated name, or 0 = untextured |
| 0x14 | u32 | unknown 1 | Nonzero on most primitives; values often decode as ASCII text: editor residue, as in TA. Do not validate. |
| 0x18 | u32 | unknown 2 | same |
| 0x1C | u32 | is-colored | Bit 0 set on all 3,301 untextured primitives, clear on all 44,195 textured ones. |

## Piece tree and indices

- **Known**: piece names are unique per model, case-insensitively, except in
  six models with one duplicated locator name each: `aratrans` (`emitjim`),
  `vercrus` (`swordhit`), `verfltwr` (`wake4b`), `verharp` (`emitjim`),
  `vertrans` (`emitjim`), `zongod` (`emitblood4`). `Model::find` returns the
  first in pre-order.
- **Known**: names are mixed case (`AraGP`, `Body`, `LegBUL`, `zongiant`). Match
  case-insensitively.
- **Believed** (TA): the piece index used by scripts is depth-first pre-order:
  a piece, then its children's subtrees in sibling-list order, then its next
  sibling. `Model::pieces` is in this order. WP-D/WP-G should confirm against
  COB piece-name tables.
- **Known**: the root is usually a ground-plate piece (`AraGP`, `targround`,
  or the model name), whose selection primitive is a flat quad near y = 0.

### Coordinates

- **Known**: offsets and vertices are 16.16 fixed point; composing offsets
  relative to the parent (sum up the ancestor chain) puts every piece in the
  right place. The parser keeps raw `i32`s; `Model::origin(i)` sums them with
  wrapping adds.
- **Known**: Y is up. Feet/ground plates sit at y ≈ 0. A horseman (`araknigh`)
  spans x −24..24, y −1..54, z −36..41 world units; Kingdoms units are roughly
  twice TA's size.
- **Known** (asset convention): models face **−Z**: heads, necks and lances
  sit at negative Z, tails at positive Z (`araknigh`, `tarblack`). Nanolathe
  reports that the TA engine negates X and Z at load and maps `z → −z` into
  world space; **Unknown** whether Kingdoms does the same. The exporter writes
  authored coordinates unchanged.
- **Known, Kingdoms-specific pattern**: "pivot + model-space child". 405 piece
  records in 118 models have a nonzero offset that exactly cancels their
  parent's accumulated origin, so their vertices are effectively in model
  space. 66 hold geometry (e.g. `zongiant`: `head` → `head_5`, `arml1` →
  `arml1_10`); 339 are emit points. Standard parent-relative composition
  handles this; animating the parent pivots the child about the parent's
  origin. **Believed**: the exporter that made these baked the mesh before
  setting the pivot.

### Locator / emit pieces

- **Known, differs from TA**: TA locators were one vertex and no primitives.
  In Kingdoms, emit points (`EmitBlood*`, `EmitWake*`, `wake*`, `blood*`,
  `damage*`, ...) are mostly **two vertices and one 2-index primitive** (1,333
  pieces), i.e. a line segment. Neither endpoint is at the piece origin.
  **Believed**: the segment gives an emit position and direction. **Unknown**:
  which endpoint is the source.
- **Known**: all 1,342 2-index primitives are untextured. 1,333 are in
  locator-named pieces; the other 9 are in `wingl3`/`wingr3`/`nanospray`
  pieces. **Believed**: renderers should skip 2-index primitives.

## Textures

- **Known**: a primitive's texture is a **name** (no extension), as in TA.
  3,470 distinct names across 44,195 textured primitives.
- **Known**: names match GAF entry names. 3,326 distinct names resolve in the
  88 GAFs under `textures/` (all GAF version `0x00010100`); adding the GAFs
  under `anims/` resolves 3,411. 59 names (199 primitives, e.g. `batbod*`,
  `drake*`, `deaddragwing*`) resolve nowhere in the mounted archives.
  **Unknown**: the engine's search order (`textures/` before `anims/`?) and
  what it draws for a missing name (TA used flat color 209). 44 names appear
  in more than one GAF.
- **Known**: most used entries have one frame; 125 have 10 frames, a few 5 or
  8. **Believed** (TA): multi-frame entries are team-color or animated
  variants. **Unknown** for Kingdoms.
- **Known, differs from TA**: 3,913 **textured triangles** (TA had textured
  quads only). There are no UVs in the file. **Believed** (TA): UVs are implied
  by vertex order, the four corners of the texture mapping to the quad's four
  indices. **Unknown**: how textured triangles are mapped. All 5- and
  8-index polygons (25) are untextured. This is the main open question for WP-C.
- **Known**: untextured color indices are ≤ 252, consistent with an 8-bit
  palette index. **Unknown**: which palette (Kingdoms renders in 16-bit
  color).

## Parser guarantees

- Iterative traversal (no recursion), visited-offset set (cycles and shared
  nodes are errors), depth cap 64, piece cap `len / 52`.
- Every count is checked against the bytes remaining before allocating;
  negative counts and offsets are typed errors; vertex indices are checked.
- Unit tests: synthetic tree, cycles, bad index, depth, huge counts,
  exhaustive truncation and 1-byte flips, proptest on random bytes. Corpus
  test (gated on `TAK_DATA_DIR`) parses all 447 and prints distributions.

## Unknowns (summary)

1. UV mapping for textured triangles and polygons.
2. Texture search path and fallback for missing names; palette for flat colors.
3. Whether Kingdoms applies TA's X/Z negation at load.
4. Which end of an emit segment is the source, and how its direction is used.
5. Piece index order is Believed pre-order until checked against COB files.
