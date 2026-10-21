# CRT: map companion file

Parser: `crates/tak-formats/src/tnt/crt.rs` (`tnt::crt::parse`). Header
only; the body is kept raw.

## Corpus (Known)

214 files, one per map bundle that has one (`kmap/` 181, `Maps/` 28, plus
`IPData.hpi`/`V2Rocket.hpi` copies). Most `maps/` and all `missions/`
bundles have no CRT.

- 199 files are the same 56 bytes: `f32 1.0`, `0`, `0`, `9`, then zeros.
- 3 more have count 0 but extra bytes (e.g. Increasing Strength, 46,404 bytes;
  Sewers of Elam, 64 bytes).
- 12 carry objects: 11 with kind 0 and a non-zero count, plus Cairbray
  Coast Landing (kind 1, see below).

## Header

| Off | Type | Tag | Meaning |
| ---: | --- | --- | --- |
| 0 | f32 | Known | `1.0` in every file (used as the magic) |
| 4 | u32 | Known | `0` in 213 files, `1` in Cairbray Coast Landing, whose body has a different shape (starts `VERBALL`/`ALL`) |
| 8 | u32 | Believed | number of placed objects |

## Body (Believed / Unknown)

In the files with objects, each record starts with a unit name as a
NUL-terminated string padded to a 4-byte boundary (`ZONHUNT`, `VERPULT`,
`ARAKEEP`, `ARAAT`, `ARABUILD`, `verlode`, `NPCFARM`), followed by binary
fields beginning with `f32 0.787…`. Most records are 568 bytes apart, with
exceptions. After the records some files have a trailer containing strings
such as `Unit` and `TempleBlood`. Savannah Hunt (171 `NPCFARM`… records)
uses a different record shape.

**Believed**: CRT holds pre-placed units / creatures for the map (the name
"CRT" plausibly = creatures), serialized in an engine-internal layout that
looks like a save-game dump. Empty maps carry a placeholder.

**Unknown**: record layout (position, owner, facing, health), the trailer,
the `9` in the placeholder, and the kind-1 layout.

## What the engine needs

Pre-placed units at map start for the few maps that have them. Until the
records are decoded, the simulation should treat CRT as "no extra units"
and rely on OTA start data.
