# WP-Q: Bevy client, first playable view

**Owns:** everything under `client/` (a separate Cargo workspace, excluded from the core one), `docs/client.md`.
**Inputs on main:** the whole core. Read `docs/sim.md`, `docs/pathfinding.md`, `docs/combat.md`, `docs/construction.md`, `docs/economy.md`, `docs/formats/3do.md`, `docs/formats/gaf.md`, `docs/formats/palettes.md`, `docs/formats/tnt.md`, `docs/formats/cob-vm.md`, and `crates/tak-input/` (the controller layer, built to be driven by a client).

## Goal
Open a real Kingdoms map in a window and play the core with a controller or mouse/keyboard: terrain, animated units, selection, orders, building, fighting. Correct over pretty, and verifiable on a headless machine.

## Rules
- The client never changes sim rules. It owns rendering, audio (later), camera, input adapters and UI. Gameplay lives in `tak-sim`; if the client needs a sim change, make it minimal in its own commit and call it out, or note it as follow-up.
- The sim runs at exactly 30 ticks per second on a fixed timestep (Bevy `FixedUpdate` or a manual accumulator). Rendering interpolates between the last two sim states using floats only on the render side. Commands from `tak-input` are stamped with the sim tick and fed through the sim's command path, so a recorded session replays to identical hashes in `tak-headless`.
- Use current Bevy (0.20 on crates.io now). Its API changes between versions: check docs.rs and the Bevy examples for 0.20 rather than relying on memory. Pin exact versions in `client/Cargo.toml`. Disable Bevy's audio feature for now (no audio yet). Keep `gilrs` gamepad support on.

## Build, in order (commit after each step that works)
1. **Window and loop**: `client/` workspace with a `tak-client` binary; loads an install (`--data`, default `TAK_DATA_DIR`), a map (`--map "kmap/Adamantine Gate"`), sides (`--players ARA,ZON`), builds the match with build trees (`tak_sim::construction::content::load_match_with_builds` or whatever `load_match` becomes), runs the sim at 30 Hz.
2. **Headless verification mode**: `--screenshot <path> --at-tick N [--log commands.json]` runs to tick N, renders, saves a PNG and exits. This is how you and the orchestrator check work on the headless machine: run under `Xvfb :99 -screen 0 1280x720x24` with `DISPLAY=:99`; Mesa's lavapipe Vulkan (llvmpipe) is installed, and `WGPU_BACKEND=gl` is a fallback if Vulkan misbehaves. Look at every screenshot with the Read tool.
3. **Terrain**: a heightfield mesh from the TNT heights (16-px cells, sea level as a water plane), textured with the terrain JPEG tiles exactly as `tak-cli dump-map` assembles them. Features (trees, rocks) can start as simple markers. TA-style camera: fixed downward angle, pan, zoom.
4. **Units**: 3DO piece trees as meshes, textured from `textures/*.gaf` with the side's texture palette (see `docs/formats/gaf.md`; index 5 is Believed transparent; texture-to-face mapping is Unknown, so start with the TA convention of mapping the texture's corners to the face's first four vertices and document what looks right). Piece transforms each frame from the unit's COB instance (`CobScripts::instance`, `ScriptType::piece_map`, pieces matched by name). Position and heading from the sim. Team colour if the data supports it (otherwise a ring under the unit).
5. **Input and UI**: feed `tak-input`'s `Mapper` from gilrs, mouse and keyboard; set its `Camera` each frame; draw the cursor, selection rings, brush circle and radial wheels from the mapper's state; submit `Output::Sim` to the sim and mirror `Output::Client`. Minimal HUD: player mana and storage, selected unit name and health, build menu for a selected builder (via the controller's build wheel or a simple panel).
6. **Effects**: TAF/GAF effect sprites for weapon hits and deaths if time allows; otherwise note as follow-up.

## Done when
- `tak-client --screenshot` on Adamantine Gate shows textured terrain at the right scale, both monarchs as recognisable textured, animated models at their start positions, and the HUD. Include the screenshot paths in your report.
- A short scripted session (command log fixture) shows a monarch walking, building a structure, and units fighting, in 3–4 screenshots at different ticks.
- The same command log replayed in `tak-headless` gives the same final hash as the client's sim (a test or a documented manual check).
- Core workspace checks still pass unchanged; `client/` passes its own `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`.
- `docs/client.md`: how to build and run on macOS (Tyler's machine, Apple Silicon) and on Linux, controls, verification mode, known gaps.
