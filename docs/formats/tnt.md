# TNT v0x4000: Kingdoms map terrain

Parser: `crates/tak-formats/src/tnt.rs` (`tnt::parse`). Dump tool:
`tak-cli dump-map <vfs path> --out DIR [--palette P] [--piece N]`.

Tags: **Known** = observed in all retail files, **Believed** = strong
evidence or TA-derived, **Unknown** = not understood.

## Corpus

- **Known**: 310 playable-map TNTs (`kmap/` 181, `maps/` 55, `missions/` 74)
  and 3,153 `sections/` files (`.tnt`, 7 `.bad`, 1 `.old`) all use the same
  layout below and parse with zero errors. Every byte of all 310 maps and
  all sections is covered by a header pointer (no gaps, no trailing data).
- **Known**: sections are the same format with no large minimap
  (pointer 0), no features, sea level 0, and a 128×128 small minimap.
- Kingdoms TNT is a different layout from TA's `0x2000` TNT: there are no
  embedded tile graphics, and the cell data is split into separate planes.

## Header (52 bytes, 13 × u32 LE)

| Off | Field | Tag | Notes |
| ---: | --- | --- | --- |
| 0x00 | version | Known | `0x4000` |
| 0x04 | width | Known | attribute cells. Multiplayer maps: OTA `size=A x B` → `A*32 x B*32`. Missions use other sizes (e.g. 162×168, 430×430). Always even. |
| 0x08 | height | Known | attribute cells |
| 0x0C | sea level | Believed | Height units; cells below it are water. Values 39–78, mostly 58 or 40. On Bell Toll, Adamantine Gate, Aibel's Seaport, the cells below it are exactly the ponds, canals and sea seen in the minimap. 0 in sections. |
| 0x10 | → heights | Known | always 52 (first block) |
| 0x14 | → feature grid | Known | |
| 0x18 | → feature table | Known | 0 when count is 0 (King of the Hill); in sections equals the small-minimap pointer with count 0 |
| 0x1C | feature count | Known | records in the feature table |
| 0x20 | → tile ids | Known | |
| 0x24 | → tile columns | Known | |
| 0x28 | → tile rows | Known | |
| 0x2C | → small minimap | Known | |
| 0x30 | → large minimap | Known | 0 in sections |

Blocks are usually in the order listed but missions place the feature
table after the tile rows, so always follow the pointers (**Known**).

## Blocks

Attribute cells are `W × H`; tile cells are `(W/2) × (H/2)` (one per 2×2
attribute cells). All grids are row-major, west→east, north→south (**Known**:
dumps match the in-file minimap orientation).

| Block | Size | Tag | Contents |
| --- | --- | --- | --- |
| heights | `W*H` u8 | Known (layout), Believed (meaning) | Height per cell. Renders as coherent terrain matching the minimap; plateaus, cliffs and water basins line up. Whether the value is a cell corner or centre (TA: corner) is Unknown. |
| feature grid | `W*H` u16 | Known | `< count`: index into the feature table. `0xFFFF`: empty. `0xFFFB`: **Believed** paved ground (covers roads, plazas, castle floors exactly; 120 of 310 maps use it). `0xFFFC`: **Unknown**, sparse single cells at wall ends, canal corners and castle gates. No other values in `.tnt` files (some `.bad` sections have others). Whether a feature index marks every footprint cell or one anchor cell is Unknown; it looks like one cell per feature instance. |
| feature table | `count × 132` | Known | `u32 index` (always equals the record position) + 128-byte name, NUL-terminated, junk after the NUL. Names are feature names such as `TarSpire07`, `AraHenge01` (905 distinct). |
| tile ids | `(W/2)*(H/2)` u32 | Known | Terrain JPEG per tile cell: `terrain/<id as lowercase %08x>.jpg` (VFS is case-insensitive). Every id in all 310 maps names an existing file. Ids repeat in runs: usually 16×16 tile cells share one 512-px JPEG. |
| tile columns | `(W/2)*(H/2)` u8 | Known | Column of the piece within that JPEG. |
| tile rows | `(W/2)*(H/2)` u8 | Known | Row of the piece. |
| small minimap | `u32 w, u32 h, w*h` u8 | Known (layout) | Always 126×126 in maps, 128×128 in sections. Map image in a corner, rest padding. |
| large minimap | `u32 w, u32 h, w*h` u8 | Known (layout) | Map fitted inside about 511×432, keeping aspect (446×431 for a square map). |

### Terrain pieces (Known)

A tile cell shows one **32×32-pixel piece** of its JPEG: source rectangle
`(col*32, row*32, 32, 32)`. A JPEG of `w×h` px holds `(w/32)×(h/32)` pieces.
Retail JPEGs are mostly 512×512 (16×16 pieces; 2,678 files), also 256 (8),
1024 (32) and a few odd sizes; the maximum column/row index used matches the
JPEG size in every case (`max 31` for 1024 px). So one tile cell is 32 px
and one attribute cell is 16 px of terrain, the same scale as TA. A
pieces-in-order run (col = x mod 16, row = y mod 16) is the common case,
but pieces are reused freely (e.g. a run `0 1 2 3 4 5 6 7 8 9 2 3 4 5 14 15`).

Mosaics built this way match the in-file large minimap closely
(`dumps/*/terrain.png` vs `minimap.png`).

### Minimap palette (Believed)

The minimaps are 8-bit indices with no palette in the file. They match
`palettes/<kingdom>.pcx` (palette in the last 768 bytes), where `<kingdom>`
is the OTA's `kingdom=` value: fitting each candidate palette against the
terrain mosaic picks taros/zhon/aramon/veruna correctly for one map of each.
`dump-map` reads it through tak-content's typed OTA view (`OtaDef::load`).

## What the engine needs

- **Simulation**: heights (u8 per 16-px cell) and sea level give terrain
  height and water; the feature grid + table give feature placement (OTA
  `[Features]`/feature TDFs supply footprints and blocking). Passability is
  not stored as its own plane; derive it from slope, water, features, and
  probably the paved code. Start positions are in the OTA, not the TNT.
- **Renderer**: tile ids + piece col/row → 32×32 JPEG pieces; minimaps for
  the UI with the kingdom palette.

## Unknown

- `0xFFFC` cells; whether `0xFFFB` changes movement (roads) or only build rules.
- Height scale in world units; corner vs centre sampling.
- Why the small minimap is a fixed 126×126 canvas.
