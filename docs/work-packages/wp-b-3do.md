# WP-B: 3DO models (M4a)

**Owns:** `crates/tak-formats/src/model3do.rs`, `docs/formats/3do.md`, `crates/tak-cli/src/model_cmd.rs`.

## Why it matters
Models are piece trees. Unit scripts (WP-D, WP-G) animate pieces by name and index, so the piece tree is the contract between scripts, simulation, and the future renderer.

## What we know
- **Known**: 447 `.3do` in `objects3d/`, all starting with u32 `1`. Sample header (`objects3d/araarch.3do`) as i32: `1, 4, 1, 0, 0, 0, 0, 606, 0, 526, 574, 0, 612`. That matches TA's 3DO object header: version, vertex count, primitive count, selection primitive, x/y/z offset, then offsets to vertices, primitives, sibling, child, name. **Believed**: same as TA; confirm on every file.
- Reference: Nanolathe `research/formats/3do.md` (MIT). Read the doc, not the Go code.
- **Unknown**: how Kingdoms primitives reference textures. TA used a texture name per primitive resolved through GAF in `textures/` (88 `.gaf` there) and palette colors. Kingdoms may differ (16-bit color); coordinate findings with WP-C.
- Positions in TA 3DO are fixed point (16.16 or similar). Keep them as integers in the parsed struct. Floats are fine only in the export path.

## Tasks
1. `model3do::parse(&[u8]) -> Result<Model, Model3doError>`: piece tree (name, offset, vertices, primitives, children), with cycle and depth protection and offset validation.
2. Corpus test: all 447 parse; report piece-count and vertex-count distributions.
3. `tak-cli dump-model <path> --out DIR`: writes OBJ (or glTF) to a folder outside the repo, plus a JSON piece tree. Texture hookup can be stubbed until WP-C lands; flat colors per primitive are fine.
4. Document the format in `docs/formats/3do.md` with Known / Believed / Unknown tags.

## Done when
- Every model parses; piece names are unique per model (or the exceptions are documented).
- Tyler can open dumps of one unit per race (Aramon, Veruna, Taros, Zhon) in a model viewer and they look right.
- Truncation and byte-flip sweep never panics.
