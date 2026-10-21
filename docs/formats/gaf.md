# GAF / TAF / GAO sprite containers

Parser: `crates/tak-formats/src/gaf.rs`. CLI: `tak-cli dump-sprite <path> --out DIR [--palette PATH]`.
Reference for the TA layout: Nanolathe `research/formats/gaf.md` (MIT, TA only). Tags follow `README.md`.

## Answer: does Kingdoms use 16-bit colour?

**Known: both 8-bit and 16-bit art ship, split by file type.**

- `.gaf` (535), `.gao` (1) and `.gao2` (1) are 8-bit palette-indexed, exactly like TA. That covers all UI art, cursors, build-menu buttons, unit/feature sprites, fonts and every model texture in `textures/`.
- `.taf` (274) is true-colour 16-bit with alpha, stored raw and uncompressed: ARGB1555 for `*_1555.taf`, ARGB4444 for `*_4444.taf`. The 274 files are 137 pairs: same entries, frame counts and geometry in both formats. They hold effects: build sparkles, explosions, dust, blood, magic rings.

So the renderer needs two texture paths: indexed + palette (with a transparency rule) for GAF, and direct 16-bit ARGB for TAF. Converting both to RGBA8 at load time is the simplest route; `Frame::to_rgba` already does that for export. Which TAF twin the original engine picked (by display mode, by alpha quality) is **Unknown**. Both decode to the same picture, and the 4444 twin has smooth alpha edges where 1555 has hard ones.

## Container layout

All integers little-endian. Offsets are absolute file offsets.

| Part | Layout | Status |
|---|---|---|
| Header | `u32 version, u32 entry_count, u32 0`, then `entry_count × u32` entry offsets | **Known**. Version is `0x00010100` in 809 files and `0` in `anims/terrain.gaf`; the parser does not check it |
| Entry | `u16 frame_count, u16 flags (=1), u32 0, char[32] name`, then `frame_count × (u32 frame_offset, u32 duration)` | **Known** layout, `flags` = 1 in all 11,488 entries. **Believed** (TA): `flags` low byte = loop, `duration` = ticks per frame |
| Frame header (24 bytes) | `u16 w, u16 h, i16 x_off, i16 y_off, u8 color_key, u8 format, u16 subframe_word, u32 0, u32 data_offset, u32 trailing` | **Known**. `trailing` is 0 in every Kingdoms frame (TA has junk here) |
| Composite | if `subframe_word & 0xFF` > 0: `data_offset` → that many `u32` frame-header offsets, drawn in order | **Known**: max depth 1, children are always format 1 |

`anims/zonlogo.gaf` is an empty (0-byte) file. **Known**; the parser returns `GafError::Empty`.

### Pixel formats (TA's "compressed" byte)

| Code | Format | Data at `data_offset` | Transparency |
|---|---|---|---|
| 0 | Raw 8-bit indexed | `w × h` bytes | pixels equal to `color_key` (9 in every GAF frame). Model textures instead use index 5 as a cutout colour, see below |
| 1 | RLE 8-bit indexed | per row `u16 len` + TA RLE commands | skip runs only |
| 4 | Raw ARGB4444 | `w × h` u16, `AAAA RRRR GGGG BBBB` | 4-bit alpha |
| 5 | Raw ARGB1555 | `w × h` u16, `A RRRRR GGGGG BBBBB` | 1-bit alpha |

**Known**: the TAF pixel blocks are exactly `w × h × 2` bytes (every one of the 4,050 TAF frames), the channel order matches the dumps visually, and the alpha bits spread across all 16 levels in 4444. `color_key` is 0 in TAF frames and is not used.

RLE commands (TA rule, **Known** to decode every frame cleanly): bit 0 set → skip `cmd >> 1` transparent pixels; else bit 1 set → repeat the next byte `(cmd >> 2) + 1` times; else copy `(cmd >> 2) + 1` literals. Runs clamp to the row width, and the next row starts at `row + 2 + len`.

Composite placement (TA rule, **Believed**, and the cursors dump correctly with it): a child lands on the parent canvas at `(parent.x_off − child.x_off, parent.y_off − child.y_off)`.

### The `0xFF00` subframe word

**Known**: 74 of the 274 TAFs (37 pairs: `blood*`, `dustpuff*`, `ring_fx_*` and others) set `subframe_word = 0xFF00` on every frame. The count byte is 0, so these are plain pixel frames. The other 200 TAFs use 0. **Believed**: TA's engine uses a nonzero high byte to pick a tinted/blended blit, and these frames are mostly grey smoke, blood and dust, which fits art meant to be tinted or blended. **Unknown**: the exact draw mode.

## Corpus numbers (Known, from `corpus_every_sprite_and_palette_decodes`)

| | Files | Entries | Frames (top level) | Frames incl. subframes |
|---|---|---|---|---|
| all sprite containers | 811 (535 gaf, 274 taf, 1 gao, 1 gao2) | 11,488 | 43,473 | 53,843 |

Pixel-format distribution:

| Ext | Format | Depth 0 | Depth 1 (subframes) |
|---|---|---|---|
| gaf | indexed (0) | 9,899 | 0 |
| gaf | indexed-rle (1) | 29,428 (incl. composite parents) | 10,370 |
| gao | indexed-rle (1) | 48 | 0 |
| gao2 | indexed (0) | 48 | 0 |
| taf | argb1555 (5) | 2,025 | 0 |
| taf | argb4444 (4) | 2,025 | 0 |

Every file parses and decodes with zero errors (one empty file, above). The worst case is 33 decoded pixels per input byte, well under the parser's budget of 256 per byte plus a 1 Mi-pixel floor.

## Model textures (`textures/*.gaf`)

- **Known**: 88 files, 5,070 frames, all raw indexed (format 0), one frame per entry for most. Entry names are what 3DO primitives reference.
- **Believed**, from the dumps: palette index 5 is a cutout/transparency colour in model textures. It is the most common low index by far (1.06 M pixels in 711 entries) and paints the ragged edges of rubble, wreckage and wood. Index 9, the header key, covers only 78 pixels. WP-B and the renderer should treat index 5 as transparent for textures until proven otherwise.
- **Believed** palette: `palettes/<side>_textures.pcx`, where side is the first three letters of the file name (`ara`, `cre`, `tar`, `ver`, `zon`; `aid_`, `lif_`, `mis_`, `mon_`, `npc_`, `zhon_` also exist). With that mapping the dumps show plausible colours.

## Palettes for indexed art

See `palettes.md`. **Known**: 174 of the 447 non-texture GAFs have a same-named `.pcx` beside them whose trailer palette gives correct colours (e.g. `anims/actionbuttons.pcx`). **Unknown**: the palette rule for the other 273. Many are terrain features and buildings (`aracrop`, `arabldg`, ...) that probably use a world palette such as `palettes/aramon_features.pcx`. `dump-sprite` falls back to `palettes/palette.pal`.

## Dumps checked visually

Taken from `dump-sprite` and viewed: `anims/actionbuttons.gaf` (UI buttons, sibling PCX palette), `Anims/CURSORS.GAF` (composite frames), `anims/aramonbuild_1555/_4444.taf` (build sparkles), `anims/dustpuffa_4444.taf`, `anims/ring_fx_bluec_*.taf`, `textures/arabldg.gaf`, `textures/verpeople.gaf`. No skew, transparency correct, colours plausible.
