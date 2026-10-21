# WP-E: maps: TNT, CRT, OTA (M7)

**Owns:** `crates/tak-formats/src/tnt.rs` (submodules under `src/tnt/` e.g. `crt.rs`, `ota.rs`), `docs/formats/tnt.md`, `docs/formats/crt.md`, `crates/tak-cli/src/map_cmd.rs`.

## Why it matters
The simulation needs heights, passability, features and start positions. Maps are also the first big visual test.

## What we know
- **Known**: every map is a bundle `<dir>/<name>.{tnt,ota,crt,txt}`. Multiplayer maps are in `kmap/` (181 loose `.kmp` archives, one map each) and `maps/` (55). Campaign maps are in `missions/` (74).
- **Known**: TNT starts with u32 `0x4000`; TA's TNT starts with `0x2000`. This is a different layout, not just a version bump. Sample header (`kmap/Adamantine Gate.tnt`, 378,282 bytes, a "6 x 6" map per its OTA) as i32: `16384, 192, 192, 58, 52, 36916, 110644, 32, 114868, 151732, 160948, 170164, 186048, ...`, so possibly width and height 192, then offsets to blocks.
- **Known**: `sections/` holds 3,145 `.tnt` (+7 `.bad`, 1 `.old`, same magic): map-editor building blocks, not playable maps. `terrain/` holds 3,261 JPEG tiles, so Kingdoms terrain is likely built from JPEG tiles referenced by the TNT, not TA's 8-bit 32×32 tiles. **Believed**.
- **Known**: `.crt` is 56 bytes in the sample: f32 `1.0`, then mostly zeros and a `9`. Unknown purpose (camera? creature? lighting?). 214 files.
- **Known**: `.ota` is TDF syntax (`[GlobalHeader]`, `missionname`, `kingdom=taros`, `numplayers`, `size=6 x 6`, nested `[Map Data]` sections with per-player start data).
- Reference: Nanolathe `research/formats/tnt.md`, `ota.md` (TA only; Kingdoms differs).

## Tasks
1. Map out the TNT v0x4000 header and blocks by comparing maps of different sizes (the OTA `size` field gives the expected dimensions). Heights, tile references into `terrain/`, features, sea level, minimap if present.
2. `tnt::parse` for whatever is understood, with every offset validated; unknown blocks kept as raw byte ranges with their offsets.
3. CRT: compare all 214 files, tabulate which fields vary, and guess their role.
4. OTA: once WP-A merges, add a typed OTA view on top of `tak_formats::tdf`. Until then, skip it.
5. `tak-cli dump-map <path> --out DIR`: height map PNG, and a terrain mosaic PNG if the tile references are understood. Outside the repo.

## Done when
- All 310 playable-map TNTs parse (header + heights at minimum); sections parse or are documented as different.
- Height dumps for a few maps match their in-game minimaps (Tyler checks).
- `docs/formats/tnt.md` lists each header field as Known, Believed, or Unknown.
