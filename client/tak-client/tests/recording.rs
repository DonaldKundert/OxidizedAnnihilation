//! Session recordings (WP-S), gated on `TAK_DATA_DIR`: a scripted gamepad
//! session driven through the real mapper on a real map produces a
//! recording whose match replays identically in `tak-headless`, and whose
//! frames give a ghost cursor for playback.

use tak_client::feel::{ghost_match, preset_profile, synth_recording};
use tak_client::game::{Session, open_vfs};
use tak_commands::CommandLog;
use tak_input::{InputEvent, Mapper, Recording};
use tak_sim::{PlayerSetup, load_match};

const MAP: &str = "kmap/Adamantine Gate";
const TICKS: u32 = 600;

#[test]
fn scripted_session_records_and_replays_identically() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("client recording") else {
        return;
    };
    let vfs = open_vfs(&dir).expect("open install");
    let mut s = Session::load(&vfs, MAP, "ARA,ZON", 1, CommandLog::new()).expect("load");
    let events: Vec<InputEvent> =
        serde_json::from_str(include_str!("../../fixtures/pad_session.json")).expect("events");
    let mut mapper = Mapper::new(preset_profile("console").unwrap());
    let rec = synth_recording(&mut s, &mut mapper, &events, TICKS, "ARA,ZON", 1);

    // The script selects the monarch, moves, stops, moves, builds.
    let sum = rec.summary();
    assert!(sum.orders.get("move").copied().unwrap_or(0) >= 2, "{sum:?}");
    assert!(sum.orders.contains_key("stop"), "{sum:?}");
    assert!(sum.orders.contains_key("build"), "{sum:?}");
    assert!(sum.selections >= 1, "{sum:?}");

    // Round trip and ghost lookups.
    let back = Recording::from_json(&rec.to_json()).expect("parse");
    assert_eq!(back, rec);
    let early = back.ghost_at(20).expect("ghost at 20");
    let late = back.ghost_at(55).expect("ghost at 55");
    assert_ne!(early.cursor_world, late.cursor_world, "cursor moved");
    assert!(!late.selection.is_empty(), "monarch selected by then");

    // The recorded match replays identically headless.
    let gm = ghost_match(&back).expect("match");
    assert_eq!(gm.ticks, TICKS);
    assert_eq!(gm.log, s.log);
    let m = load_match(&vfs, MAP, &PlayerSetup::parse_list(&gm.players), gm.seed).expect("match");
    let run = tak_headless::run_match(m, &gm.log, TICKS, true).expect("headless");
    assert_eq!(run.hashes.len(), s.hashes.len());
    if let Some((tick, a, b)) = tak_headless::first_mismatch(&run.hashes, &s.hashes) {
        panic!("hash mismatch at tick {tick}: headless {a:016x}, recorded {b:016x}");
    }
    assert_eq!(
        gm.final_hash,
        s.last_hash().map(|h| format!("{h:016x}")),
        "final hash in the recording"
    );
}
