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

Decompressed, the directory is a tree of fixed-size little-endian records. Offsets are byte offsets into the decompressed directory block; name offsets point into the names block. **Known**: all 13,174 files in all 198 mounted archives resolve, extract, and match their sizes.

Directory entry, 20 bytes. The root is at offset 0 and has the empty name (names offset 0).

| Offset | Type | Meaning |
|--:|---|---|
| 0 | u32 | Name offset |
| 4 | u32 | Offset of the first subdirectory entry (subdirectories are contiguous) |
| 8 | u32 | Number of subdirectories |
| 12 | u32 | Offset of the first file entry (files are contiguous) |
| 16 | u32 | Number of files |

File entry, 24 bytes:

| Offset | Type | Meaning | Status |
|--:|---|---|---|
| 0 | u32 | Name offset | **Known** |
| 4 | u32 | Absolute offset of the file data in the archive | **Known** |
| 8 | u32 | Decompressed size | **Known** |
| 12 | u32 | Stored size: the whole SQSH chunk including its 19-byte header, or `0` when the file is stored raw | **Known** |
| 16 | u32 | Unix modification time (1999 for Cavedog files, 2013 for GOG's mod) | **Known** |
| 20 | u32 | Checksum. Not CRC-32, Adler-32, byte sum, MD5 prefix, or CRC-32/MPEG-2 of either form | **Unknown** (not needed: the SQSH chunk has its own checksum) |

Each compressed file is exactly one SQSH chunk, even for files over 500 KB. TA's per-file chunk tables do not exist here. 3,477 files are stored raw. **Known**

Names are Latin-1 and keep their original case (`Anims/activity_fires.GAF` next to `anims/actionbuttons.gaf`); lookups must be case-insensitive.

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

12,442 distinct paths (case-insensitive) across 13,174 files; 521 paths come from more than one archive, and 276 of those differ in content. Main overlaps: `data.hpi` < `V3Rocket.hpi` (248 paths), `data.hpi` < `V3Rocket.hpi` < `IPData.hpi` (174), `english.hpi` < `V3Rocket.hpi` < `IPEnglish.hpi` (17), `V2Rocket` < `V3Rocket` (12 + 4).

The original executable's load order is **Unknown**. Archive timestamps give a clean chronology: base data (1999) < V2Rocket < V3Rocket (2000-01-05) < Iron Plague (2000-01-14) < gog-mod (2013). `tak_vfs::kingdoms_mount_order` uses that order, later winning, then loose `.kmp` maps on top. **Believed**. Confirm by finding a shadowed file whose effect is visible in game (e.g. `ai/default.txt`, a unit `.fbi` stat) and checking which version the original uses.

`data/data.hpi` is a byte-identical copy of `data.hpi` and is not mounted. `tak-cli shadowed` lists every overlap.
