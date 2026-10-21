# Bevy client (`client/`, WP-Q)

The first playable view of the core: a real Kingdoms map, animated 3DO
units driven by their own COB scripts, selection and orders through
`tak-input`, building and fighting from `tak-sim`. Correct over pretty.

`client/` is its own Cargo workspace (excluded from the core one) with one
crate, `tak-client` (library + binary). The core never depends on Bevy. The
client renders, adapts input and submits orders; it never changes sim rules.
Tags as elsewhere: **Known**, **Believed**, **Chosen**, **Unknown**.

## Build and run

Bevy 0.20.0 (`default-features = false`, features `3d` + `ui`: no audio;
gamepads through Bevy's `gilrs` backend). Bevy 0.20 declares
`rust-version = 1.97.1`.

### macOS (Apple Silicon)

```sh
rustup update                       # needs Rust >= 1.97.1
cd client
export TAK_DATA_DIR="$HOME/Games/Total Annihilation Kingdoms"   # your install
cargo run --release -- --map "kmap/Adamantine Gate" --players ARA,ZON
cargo run --release -- --demo       # scripted session: a wall, then a monarch duel
```

Metal is picked automatically. The first build compiles Bevy (several
minutes on an M-series Mac); later builds only the client. Debug builds
(`cargo run`) already optimise dependencies (`opt-level = 2`) and are
playable, but are large (about 2 GB with debug info); use `--release` to
play.

### Linux

Needs the usual Bevy system packages (udev, alsa headers are not needed
without audio, x11/xkbcommon/wayland, a Vulkan or GL driver). Same commands.
With a toolchain older than 1.97.1 (the cloud box has 1.97.0 and cannot
download another), add `--ignore-rust-version` to `cargo build/run/test/clippy`;
1.97.0 compiles Bevy 0.20 without changes.

### Headless verification (CI / cloud)

```sh
Xvfb :99 -screen 0 1280x720x24 &
export DISPLAY=:99
tak-client --demo --screenshot out/demo_{tick}.png --at-tick 45,720,1500 \
    --focus monarch:0 --zoom 0.3 --select 0
```

- `--screenshot PATH --at-tick N[,M...]` runs the sim as fast as it can to
  each tick (no wall clock), lets the frame settle, saves a PNG and exits
  after the last one. `{tick}` in the path is replaced. The first capture
  waits 150 frames so the GPU pipelines finish compiling (llvmpipe compiles
  shaders asynchronously for several seconds; earlier frames show only UI
  and gizmos), later ones 20.
- `--focus monarch:N | unit:SLOT | X,Y` centres the camera (at the point's
  terrain height); `--zoom` is world units per pixel (1 = TA's native
  640-px view scale); `--select 0,2` selects unit slots through the mapper,
  so rings and the HUD panel show; `--window WxH`; `--showcase a,b,c`
  draws those 3DO models (rest pose, render only) in a row next to the
  focus, for checking art by eye.
- Each capture prints every unit (slot, def, owner, position, health,
  build progress, first orders), the feature count and the hash.
- **Known** to work on this machine: Mesa lavapipe (Vulkan, llvmpipe),
  1280×720, about 8 fps rendering with the sim at exactly 30 ticks/s.
  `WGPU_BACKEND=gl` was not needed.

### Logs, hashes, replay

- `--log FILE` plays a command log (`tak-headless` JSON). `--demo` uses the
  built-in scripted session instead (`game::demo_log`; committed as
  `client/fixtures/demo_session.json`, kept in sync by a test).
- `--record FILE` writes the full log on exit (fixture plus every order you
  gave, stamped with the tick it executed on); `--hashes FILE` writes one
  hash per tick in `tak-headless verify` format.
- **Known**: the client's sim and `tak-headless` agree. `cargo test` runs
  `tests/replay.rs` (gated on `TAK_DATA_DIR`): the demo log through the
  client's `Session` and through `tak_headless::run_match` give identical
  hashes for 2,400 ticks. Manual CLI check, done for WP-Q:

  ```sh
  tak-client --demo --screenshot x.png --at-tick 2400 \
      --record demo.json --hashes demo_hashes.txt
  tak-headless verify --map "kmap/Adamantine Gate" --players ARA,ZON \
      --log demo.json --hashes demo_hashes.txt
  # ok: 2400 ticks match (final b2388a398e43fd46)
  ```

## Controls

Both default `tak-input` profiles are merged into one mapper, so mouse,
keyboard and a gamepad work together and share one selection.

| Intent | Mouse / keyboard | Controller |
|---|---|---|
| Select / drag-circle select | LMB click / drag (Shift adds) | A tap / A hold (brush) |
| Context order (move, attack enemy, guard friend, place building) | RMB (Shift queues) | X (RB+X queues) |
| Build | F1–F8 pick an option of the selected builder, then RMB to place | LB build wheel |
| Orders wheel | S stop, P patrol, G guard, H/R/F fire state | LT command wheel |
| Cancel / deselect | Esc | B |
| Same-type select | Z (Ctrl+Z whole map) | Y / double Y |
| Control groups | Ctrl+0–9 store, 0–9 recall | RB+D-pad store, D-pad recall |
| Centre on monarch, idle builder | Home, Period | R3, View hold |
| Pan | arrow keys | right stick |
| Cursor | mouse | left stick (virtual cursor, magnetism, RT precision) |
| Zoom | mouse wheel | |
| Pause, game speed | Pause (or Ctrl+P), `=` / `-` | |

Game speed changes ticks per wall second only, never tick length.

## How it works

### Sim loop

`game::Session` owns the `Sim` and its `CommandLog` and steps exactly as
`tak-headless` does: `Sim::with_scripts(&scenario, Box::new(scripts))`,
then `step(log.for_tick(tick))` once per tick. The match comes from
`tak_sim::load_match` (full build trees, map features, corpses since WP-P).
Orders from the mapper are recorded into the log at the next tick to run.
A manual accumulator steps the sim at 30 Hz (at most 8 ticks per frame,
then the backlog is dropped). After every tick the client takes a
`Snapshot` (positions, headings, COB piece poses, health, build progress,
projectiles); rendering lerps between the last two with
`alpha = accumulator / tick`. Floats exist only on the render side.

### Camera and coordinates

- Render space: `x` = world x (east), `y` = height, `z` = world y (south).
  World units are map pixels. Height is the raw TNT byte (**Believed**,
  as the sim uses it).
- **Camera: TA's oblique projection (WP-R).** `screen_x = (x − cam_x) /
  zoom`, `screen_y = (z − 0.5·height − cam_z) / zoom`: one pixel per world
  unit on both ground axes at zoom 1 (no foreshortening), and height
  shifts the image straight up by half its value (`view::SHEAR` = 0.5).
  **Believed**: Nanolathe's TA research (`docs/DESIGN_PRESENTATION_CLIENT.md`
  C1, `screenY = worldZ − (worldY >> 1)`), and Kingdoms agrees: drawing the
  height map's contours over the terrain mosaic at `z − h/2` puts them
  exactly on the painted cliff edges, crater rims and pond shores, while at
  `z` they miss by tens of pixels (Adamantine Gate, checked by eye). So the
  terrain art is screen-space: mosaic pixel `(x, y)` shows world `(x, y +
  h/2)`.
- Bevy has no oblique camera, so `app::ObliqueProjection` is a custom
  `CameraProjection`: the camera sits at `(cam_x, 0, cam_z)` with no
  rotation and the matrix does the shear (`ViewGeom::clip_from_view`).
  Rays run along `(0, −1, −0.5)` (down and north); depth is `y + 0.5·z`
  (reverse-Z over ±8000), so higher and more southern points are nearer.
  Frustum culling uses the full matrix.
- Terrain: vertices keep their real heights (for occlusion and picking)
  but sample the mosaic at `(x, z − h/2)`, so on screen the art shows
  exactly as the flat mosaic (chunk images carry a 128-px margin from the
  chunk to the north). With this the sea-level plane now tints exactly the
  painted water, island in the pond included.
- Models are projected with the same shear (verticals shrink to half,
  ground plan unforeshortened), as TA draws them.
- `tak-input`'s `Camera` is an orthographic ground mapping with one scale;
  with no foreshortening its pixels are screen pixels of the height-0
  plane. The client feeds pointer positions as the pixel of the terrain
  point under the mouse (ray-marched against the heightfield), so picking
  and orders land on what is under the cursor even on plateaus (`view.rs`,
  tested).

### Terrain

`terrain.rs`: the TNT's tile grid assembled into 1024-px chunk textures
exactly as `tak-cli dump-map` builds its mosaic (tile `(x, y)` shows the
32×32 piece `(col, row)` of `terrain/<id>.jpg`), on a heightfield mesh with
vertices on cell corners (mean of the touching cells), sampled with the
oblique shift (see Camera). Unlit, no tonemapping, so the JPEG colours are
shown as is. Sea level is a translucent plane; it tints exactly the ponds,
canals and sea painted in the terrain art. Rows north of the map edge
(visible because heights lift the top rows) are black.

### Map features, corpses and wrecks (WP-R)

`sprites.rs` + `app::spawn_feature_art`. A feature TDF names either
`object` (a 3DO: drawn like a unit at rest pose, static; corpses keep the
unit's heading, map features use the file's orientation) or `filename` +
`seqname` (a GAF sequence in `anims/<filename>.gaf`) with `seqnameshad`
as its shadow. Every placed feature is drawn, in two groups:

- Gameplay-relevant ones (the sim keeps them: blocking, reclaimable,
  damageable, sacred sites) and corpses/wrecks come from `World.features`
  and disappear when the sim removes them.
- Pure scenery (`FeatureDef::matters()` false) is drawn once from the
  `MapDef` (on the maps checked every placed feature matters, so this group
  is usually empty).

Findings and choices:

- **Palette (Believed)**: feature GAFs have no sibling PCX; they decode
  correctly with `palettes/<kingdom>_features.pcx` (the OTA `kingdom`):
  trees, rocks, henges, crops and huts show natural colours, while
  `<kingdom>.pcx` gives noise.
- **Placement (Believed, TA rule)**: the frame's `(x_offset, y_offset)`
  pixel sits on the footprint centre at its terrain height. Sprites are
  1:1 (one pixel per world unit at zoom 1). The quad lies in the screen
  plane (`ViewGeom::screen_up`), lifted toward the viewer along the view
  ray by `0.4·(pixels below the anchor) + 6` height units so its lower
  edge does not sink into the ground (`sprites::lift_for`); this moves
  nothing on screen, and things north of the base are hidden behind it
  while things south of it draw in front, like TA's painter order. Trees,
  henges and rocks sit on their painted spots and cast their shadows to
  the right, as drawn.
- **Shadows (Chosen level)**: `seqnameshad` frames are black silhouettes;
  they are alpha-blended at 0.85 opacity, which in Bevy's linear-light
  blending darkens the ground to roughly half brightness. TA's exact
  `shadtrans` level is Unknown. (A material alpha below about 0.6 is
  barely visible on lavapipe for that reason.)
- **Animation (Believed)**: `animating=1` sequences (lodestone glows) play
  by sim tick with each frame's duration word as ticks per frame (TA), so
  they pause with the game and are deterministic in screenshots; each
  instance gets a phase from its cell so neighbours do not pulse in step
  (**Chosen**).
- **Batching**: every instance of a sequence frame shares one quad mesh
  and one material, so Bevy batches them; Grimm Farms (1,160 features)
  draws at 7.6 fps at zoom 1 and 4.5 fps at zoom 2.5 with hundreds of crops,
  trees and huts on screen (lavapipe, 1280×720).
- 1×1-pixel sequences (`TarNoise01`, sound emitters) draw as nothing
  visible.

### Units: models and textures

`models.rs`, per (model, player):

- One mesh per 3DO piece, in the piece's local space, parented like the
  file's tree. Not drawn: 2-index primitives (emit segments) and the root
  piece's selection primitive (the ground plate). Polygons are fanned.
- Textures: every `textures/*.gaf` entry by name, decoded with
  `palettes/<first three letters of the GAF>_textures.pcx` (**Believed**),
  index 5 and the colour key transparent (**Believed** cutout), packed into
  one atlas per model with edge-padded borders. Alpha-mask material,
  double-sided (winding is not relied on), unlit with a baked per-face
  shade (`0.62 + 0.38·|n·L|`, fixed light in model space).
- **UV mapping (Chosen, looks right)**: a quad's four indices take the
  texture's corners in TA order (0,0) (1,0) (1,1) (0,1); a textured triangle
  takes the first three. On the monarchs (Elsin, Thirsha) armour plates,
  faces, capes, swords and the wall's stonework all read correctly; no
  mirrored or rotated panels were seen at close zoom. Confidence: good for
  quads (93 % of textured primitives), moderate for triangles (the
  diagonal case could be the other three corners on some faces; not seen
  wrong yet).
- **Team colour (Believed, confirmed visually)**: multi-frame texture
  entries are per-player variants; frame `player % frames` is used. Player
  0 gets blue capes and banners, player 1 red; swapping `--players` swaps
  the colours on the same models. A team ring under each unit (blue = you,
  red = enemy) is drawn too.
- Untextured faces use their colour index in the model side's texture
  palette.
- **Texture search (Chosen order)**: `textures/*.gaf` first, then
  `anims/*.gaf`. The `anims/` files are only header-scanned for entry
  names at startup (about 0.6 s for 434 files) and parsed on first use.
  Their palette: a sibling `anims/<file>.pcx` if present (none of the 19
  files that matter has one), else `<side>_textures.pcx` by the first three
  letters, `*monst*` files (`nyrmonst`, `talmonst`) on `mon_textures`, else
  `palette.pal` (**Chosen**; the `aid*` files land there). This resolves
  85 more names (3,411 of 3,470): live units `aradrag`, `tartroop`,
  `tardung`, `tarnecro`, `npcpeas`, `monpiran`, `npcbotl` and many
  `_dead` corpses.
- **Still unresolved (Known)**: 59 names, all on corpses except
  `liegeshoul2` (`vercrus`): `zonbat_dead` (`bat*`), `zondrake_dead`
  (`drake*`), `aradrag_dead`/`tardrag_dead`/`tarknigh_dead`/`npcthesh_dead`/
  `npcjor2_dead` (`deaddragwing*`), `verpar_dead` (`deadwing3/4`),
  `zongryp_dead` (`gryphwing*1`), `zonorc_dead`, `zontroll_dead`
  (`trolllimb*`). They are not untextured by design (live models have
  same-stem textures), so: a name whose stem (trailing digits dropped,
  then shortened down to six letters) prefixes a known texture uses that
  texture (`gryphwingl1` → `gryphwingL2`, `drakehead1` → `drakeheadA1`;
  **Chosen**), and the rest draw in a neutral brown (`MISSING_RGB`), not
  grey. Both are logged per model. `--showcase zondrake_dead,zonbat_dead`
  shows them.

### Units: placement and animation

- Unit root: interpolated sim position at the terrain height the sim
  reports (`Unit.ground`), rotated by the heading. Models face −Z
  (**Known**, `3do.md`); heading 0 = east, quarter = south, so the root
  rotation is `RotY(−(heading + 90°))`. Checked: walking monarchs face
  their direction of travel.
- Pieces: each model piece maps to its COB piece through
  `ScriptType::piece_map` (by name). Per frame the piece's transform is
  `translate(rest offset + script move) · RotY(−y) · RotX(−x) · RotZ(−z)`
  with the script's turn angles, the convention `tak-cli anim` renders with
  (**Believed**, TA/Spring 3DO order). Poses lerp between ticks (angles the
  short way). `hide`/`show` toggle the piece's own mesh, not its children.
  Walk cycles, the build pose and weapon swings play from the retail
  scripts. Mirroring (TA's X negation, `3do.md` Unknown 3): not applied;
  the monarchs appear to hold their swords in the right hand, which
  suggests none is needed (moderate confidence).

### HUD and overlays

Top bar: map, tick, pause/speed, every player's mana, storage and income.
Bottom-left panel: the first selected unit's name, def, health, order
count, construction progress, and its build menu with costs (the option
in placement mode is marked). Gizmos: team rings, selection rings, health
and build-progress bars, queued-order lines for selected units, the
cursor, the gamepad brush circle, the footprint of the building being
placed, projectiles (orange spheres). Radial wheels are drawn as labelled
slots next to the cursor from `Mapper::wheel()`.

## Core changes made for the client

- `ScriptHost::as_any` (default `None`; `CobScripts` returns itself),
  `Sim::scripts()` and `Sim::cob()`: read-only access so the renderer can
  read COB piece poses. Additive; the sim never calls them; hashes
  unchanged.

## Known gaps / follow-up

- Effects: no TAF/GAF sprites for weapon hits, deaths, build sparkles or
  dust yet; projectiles are spheres. Units under construction draw fully
  built (no TA:K build-up effect), with a progress bar.
- No minimap, no fog of war, no audio, no in-HUD click targets (build menu
  is keyboard/wheel), no spell UI beyond the cast mode.
- Rejected build placements (`ConstructionState.rejections`) are not yet
  shown in the HUD.
- The demo duel: the monarchs stop at range and trade ranged attacks; the
  Zhon monarch dies around tick 1,700. Behaviour is the sim's.
