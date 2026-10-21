# Getting the game data into a session

The game data never goes in the repo. Each cloud session needs its own copy, read from Tyler's Mac. The install is the GOG Mac build (a Wine wrapper):

```
/Applications/Total Annihilation Kingdoms.app/Contents/Resources/game/Total Annihilation Kingdoms.app/
  Contents/Resources/drive_c/Program Files/GOG.com/Total Annihilation Kingdoms
```

A pre-packed copy (everything except the 539 MB of Bink movies) sits in `~/Downloads/tak-data-chunks/` as six ~80 MB tar chunks plus `SHA256SUMS.txt`. Transfers over ~90 MB time out, which is why it is chunked.

## Steps (cloud session linked to Tyler's Mac)

1. Ask for folder access to `~/Downloads/tak-data-chunks` (the user approves on the Mac).
2. Stage the chunks, **two per call, sequentially** (parallel staging times out):
   `main.p_aa`+`main.p_ab`, then `main.p_ac`+`main.p_ad`, then `main.p_ae`+`main.p_af`+`SHA256SUMS.txt`.
   They land in `/mnt/user-data/uploads/tak-data-chunks/`.
3. Verify and extract outside the repo:
   ```sh
   U=/mnt/user-data/uploads/tak-data-chunks
   cat $U/main.p_a? | sha256sum   # must equal SHA256SUMS.txt
   mkdir -p ~/takdata && cat $U/main.p_a? | tar -xf - -C ~/takdata
   export TAK_DATA_DIR="$HOME/takdata/Total Annihilation Kingdoms"
   cargo run -q -p tak-cli -- ls units | head   # sanity check
   ```

Expected: 675 files, 465 MB, sha256 `21823dc2ae02b939d977af5c260406921afbedf93e22aa0f35a755debcc9490e`.

## Without a linked Mac

Write and test against synthetic fixtures (`tak_vfs::testing::HapiBuilder` builds archives in memory) and leave corpus tests to a session that has the data. Say so in the PR.

## Rules

- Extract to `~/takdata` or `/tmp`, never inside the repo.
- Use `tak-cli extract <prefix> --out /tmp/x` to pull just the files you need.
- Never paste file contents into docs, tests, snapshots, or PR text. Counts, sizes, header field values, and short magic bytes are fine.
