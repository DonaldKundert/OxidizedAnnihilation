# tak-client

The Bevy client for Oxidized Annihilation. A separate Cargo workspace (excluded from the core one) so the core never builds a renderer. Build, run, controls and the headless screenshot mode: `docs/client.md`.

```sh
cd client
TAK_DATA_DIR=/path/to/install cargo run --release -- --demo
```

Builder **Clear**: select a capable unit, press **C** (or use the Clear panel/wheel button), then click a visible reclaimable tree or wreck. Hold **Shift** to append targets; **Esc** cancels targeting, and **Stop** cancels queued work. Clear is separate from weapon attack and does not target live units.
