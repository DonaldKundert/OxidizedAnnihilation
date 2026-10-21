# tak-client

The Bevy client for Oxidized Annihilation. A separate Cargo workspace (excluded from the core one) so the core never builds a renderer. Build, run, controls and the headless screenshot mode: `docs/client.md`.

```sh
cd client
TAK_DATA_DIR=/path/to/install cargo run --release -- --demo
```
