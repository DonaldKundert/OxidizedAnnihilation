# Palettes, PCX and lookup tables

Code: `crates/tak-formats/src/gaf/palette.rs`, `crates/tak-formats/src/gaf/pcx.rs`. Tags follow `README.md`.

## PCX (`.pcx`, 220 files, plus 2 `.pal` that are really PCX)

**Known**: every one is version 5, RLE (encoding 1), 8 bpp, 1 plane, with the standard 769-byte trailer (`0x0C` + 768 bytes RGB) at the end. 212 pad `bytes_per_line` to an even width, 10 have no padding. RLE: a byte ≥ `0xC0` gives a run of `b & 0x3F` copies of the next byte; any other byte is a literal. Runs can cross scanlines, and the decoder treats the scanlines as one stream.

Roles:
- `palettes/<name>.pcx` (42) carry palettes: 40 sit beside a lookup-table set (`aid_textures` has none), and `cre_textures.pcx` and `crebipal.pcx` have no tables. The ones checked (e.g. `aiden.pcx`) are 1×1 images that exist only for the palette. **Known**.
- `anims/<name>.pcx` beside `anims/<name>.gaf` (174 pairs) carries that GAF's palette. **Known** from the dumps, e.g. `anims/actionbuttons`.
- `bitmaps/initscreen.pcx` (256×256) and a few others are plain images.
- `palettes/byclient.pal` and `byclient2.pal` are 640×480 PCX images despite the extension. **Known**.

`Pcx::parse` rejects other layouts with `PcxError::Unsupported` (none occur). The decoded size must fit 63× the body length (the RLE limit), so a bad header cannot over-allocate.

## `.pal` (7 real ones)

**Known**: 1,024 bytes = 256 × `r, g, b, 0`. Present: `anims/palette.pal`, `palettes/palette.pal`, `guipal.pal`, `gameart.pal`, `oldguipal.pal`, `aiden_features.pal`, `veruna_features.pal`. Indices 0–9 hold the Windows system colours (`80 00 00`, `00 80 00`, ...), as in TA. `Palette::from_pal`.

## Lookup tables in `palettes/` (41 sets)

Each set `<name>.{alp,lht,shd,gry,blu}` belongs to the same-named palette (`palettes/<name>.pcx`, present for 40 of 41). `palettes/makeshades.bat` (`s:\kingdoms\tools\makeshades %1`) shows the original tools generated them from the palette. All five parse in the corpus test.

| Ext | Size | Shape | Role | Status |
|---|---|---|---|---|
| `.alp` | 65,536 | 256 × 256 indices | Blend table: `[a][b]` = mix of colours a and b | **Known** symmetric with identity diagonal (41/41). **Known** for `aramon`: nearest colour to the 50/50 average (200/200 samples). `fx.alp` doesn't fit the average rule against `fx.pcx` (**Unknown** why) |
| `.shd` | 8,192 | 32 rows × 256 | Shade ramp: row 0 → all index 0 (black), row 15 ≈ identity (40/41), rows above brighten up to ~1.8× | **Known** |
| `.lht` | 8,192 | 32 rows × 256 | Light ramp: rows brighten with level (aramon: row 31 ≈ 1.8× luminance) | **Known** shape; row 0 is identity in 21/41 |
| `.gry` | 256 | index remap | Desaturate: maps each colour to a grey-ish neighbour | **Known** by sampling `aramon` |
| `.blu` | 256 | index remap | Darker, cooler colours | **Believed** a tint (fog of war, water or "frozen"?); exact use **Unknown** |

Typed wrappers: `BlendTable`, `LevelTable` (`.lht`/`.shd`), `RemapTable` (`.gry`/`.blu`). They only validate size and index. How the engine picks a level (lighting, height, fog) is **Unknown** and belongs to the renderer.

## Palette for each sprite (open question)

- `anims/X.gaf` → `anims/X.pcx` when it exists (**Known**, 174 cases).
- `textures/<side>*.gaf` → `palettes/<side>_textures.pcx` (**Believed**).
- Others: **Unknown**, probably world/feature palettes (`palettes/<world>_features.pcx`) chosen by the map. `.gui`/TDF data may name palettes; WP-A's parsers may reveal this.
