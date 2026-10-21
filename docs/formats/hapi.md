# HAPI archives (`.hpi`, `.kmp`)

Source: M1 survey of the GOG install, 2026-10-20. 18 `.hpi` + 181 loose `.kmp`, 199 total.

## Header (32 bytes, little-endian)

| Offset | Type | Meaning | Status |
|--:|---|---|---|
| 0 | `[u8; 4]` | Magic `HAPI` | **Known** |
| 4 | u32 | Version. `0x00020000` in all 199 archives. No TA-style `0x00010000` files exist in this install. | **Known** |
| 8 | u32 | Directory block offset | **Known** (all 199 decode) |
| 12 | u32 | Directory block size | **Known** |
| 16 | u32 | Names block offset | **Known** |
| 20 | u32 | Names block size. Exact for SQSH names blocks. Unreliable for raw ones (see below). | **Known** |
| 24 | u32 | `32` in every archive: start of file data, right after the header | **Believed** |
| 28 | u32 | Equals dir offset + dir size in most `.hpi` files; `0` in `gog-mod.hpi` and all `.kmp` | **Unknown** meaning |

Layout in practice: `[header][file data chunks][names block][directory block][trailer]`.
The names block usually ends right where the directory begins, but not always (`Jersey.hpi`, 3 maps).

Trailer: free text after the directory, ignored by the format. Examples: `...Copyright 1999 Cavedog Entertainment`, `HPIPack by Joe D ... FNORD` (gog-mod), `Copyright 2000 Cavedog Entertainment` (maps). **Known**

## SQSH chunks

Directory blocks, most names blocks, and file data are stored as `SQSH` chunks:

| Offset | Type | Meaning | Status |
|--:|---|---|---|
| 0 | `[u8; 4]` | `SQSH` | **Known** |
| 4 | u8 | `2` everywhere seen | **Known** value, **Unknown** meaning |
| 5 | u8 | Compression. `2` = zlib everywhere seen | **Known** for 2 |
| 6 | u8 | Obfuscated flag. `1` on names/dir blocks, `0` on the file data sampled | **Known** |
| 7 | u32 | Compressed payload size | **Known** |
| 11 | u32 | Decompressed size | **Known** |
| 15 | u32 | Checksum: wrapping u32 sum of the stored (still obfuscated) payload bytes | **Known** |
| 19 | … | Payload | **Known** |

Obfuscation (flag 1): `plain[i] = (stored[i] - i) ^ i`, both in u8 with wrapping, `i` = index in the payload. This is TA's SQSH transform. After it, the payload is a standard zlib stream.
Verified: all 384 SQSH names/dir blocks across 199 archives decode, match their decompressed size, and pass the checksum.

## Raw names blocks

11 archives store the names block uncompressed: `terrain.hpi`, `gog-mod.hpi`, `Jersey.hpi` and 8 maps. It is a sequence of NUL-terminated Latin-1 strings starting with an empty string.
In these files the header's names size can run past the block into the next SQSH chunk (`Jersey.hpi`: +9 bytes). **Read names as NUL-terminated strings at offsets from the directory, and never trust the raw block size.** **Known**

## Directory block

Decompressed, it is a flat table of u32s that references the names block by offset (e.g. `data.hpi` starts `00000000 14000000 0e000000 14000000 ...`). Entry layout: **Unknown**, the first M2 task.

## Contents by archive (names only)

| Archive | Main contents |
|---|---|
| `data.hpi` (also identical copy at `data/data.hpi`) | tdf 609, gaf 453, 3do 349, taf 246, jpg 214, pcx 171, cob 156, fbi 155 |
| `IPData.hpi` | Iron Plague: jpg 748, tdf 295, gaf 62, 3do 59, fbi 56, cob 31 |
| `V3Rocket.hpi`, `V2Rocket.hpi` | Patch data: tdf 590, fbi 344, wav 68, 3do 39, cob 17 |
| `terrain.hpi` | jpg 2564 (terrain tiles are JPEG) |
| `sections.hpi`, `IPSections.hpi` | tnt 2546 / 600 (map sections) |
| `maps.hpi`, `Maps/*.kmp` | each map = `kmap/<name>.{tnt,ota,crt,txt}` |
| `missions.hpi`, `IPMissions.hpi` | campaign ota/tnt/txt/cob/tdf |
| `english.hpi`, `IPEnglish.hpi`, `Jersey.hpi` | wav voice/sound, tdf strings |
| `meta.hpi`, `boneyards*.hpi`, `gog-mod.hpi` | UI: gui, gaf, pcx, png, tsf |

Formats seen only inside archives, and not in TA docs: `taf` (274), `crt` (214), `gui` (139), `tsf` (8), and the 41-each set `alp`, `blu`, `gry`, `lht`, `shd`. **Unknown**.

## Mount precedence

Which archive wins on a path clash (base vs Iron Plague vs V2/V3 patches vs loose `.kmp`): **Unknown**. Needs a clash census in M2.
