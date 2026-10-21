# WP-C: GAF/TAF sprites, PCX, palettes and lookup tables (M4b)

**Owns:** `crates/tak-formats/src/gaf.rs` (add submodules under `src/gaf/` as needed, e.g. `pcx.rs`, `palette.rs`), `docs/formats/gaf.md`, `docs/formats/palettes.md`, `crates/tak-cli/src/sprite_cmd.rs`.

## Why it matters
All 2D art: UI, build pictures, unit animations, effects, model textures. The **Believed** "Kingdoms uses 16-bit color" claim gets settled here, which decides the renderer's texture path.

## What we know
- **Known**: `.gaf` (535) and `.taf` (274) share the magic `0x00010100`, TA's GAF version word. `.gao`, `.gao2` (1 each) also do.
- **Known**: GAF header `u32 version, u32 entry count, u32 0`, then a table of u32 entry offsets. Entry: `u16 frame count, u16 (1), u32 (0), char[32] name`, then frame pointers. Matches TA so far.
- **Known**: TAF file names end in `_1555` or `_4444` (`anims/aramonbuild_1555.taf`), very likely ARGB1555 and ARGB4444 pixel formats. In a `_1555` TAF the frame header reads `width 42, height 45, x 19, y 21, 0, compression byte 5, subframes 0, 0, data offset 440`. **Believed**: the byte TA used as a compression flag is a pixel-format code here.
- `palettes/` holds 41 sets of `.alp` (64 KB: 256×256 blend table?), `.lht` and `.shd` (8 KB: 32×256 light/shade tables?), `.blu`, `.gry` (256 bytes: remap tables?), plus 8 `.pal` (1,024 bytes = 256 × 4-byte entries) and 42 `.pcx`. `.pcx` (220) are standard PCX (`0a 05 01 08`).
- `textures/` has 88 `.gaf` (model textures). `terrain/` and build pictures are plain JPEG and need nothing from this package.
- Reference: Nanolathe `research/formats/gaf.md`, `pal.md` (MIT), for TA only.

## Tasks
1. GAF/TAF container parser: entries, frames, subframes, all pixel formats found (8-bit indexed with a palette, 1555, 4444, and whatever else the census shows). Decoders return RGBA8 for export only.
2. PCX decoder (8-bit RLE) and `.pal` reader.
3. Identify the lookup tables: document sizes, shapes, and a best guess at their role. Decoding them fully can wait.
4. `tak-cli dump-sprite <path> --out DIR`: every frame as PNG, outside the repo.
5. Corpus test: every `.gaf`, `.taf`, `.gao*`, `.pcx`, `.pal` parses and decodes.

## Done when
- Every sprite file decodes; frame counts and pixel-format distribution are in `docs/formats/gaf.md`.
- Tyler confirms a handful of dumps look right (a build animation, a UI button sheet, a model texture).
- The 16-bit color question is answered in the doc as Known.
