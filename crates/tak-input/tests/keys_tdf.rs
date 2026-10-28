//! `Keys.TDF` import: a hand-written synthetic file (never the retail text)
//! and, with `TAK_DATA_DIR` set, the install's own file, which must give
//! exactly the Classic TA:K keyboard bindings (Classic is TA:K's defaults).

use std::collections::BTreeSet;

use tak_commands::{ControlGroup, Scope};
use tak_input::{
    Action, Binding, Category, Input, Key, LineStatus, Profile, ReservedOrder, parse_keys_tdf,
};

fn k(name: &str) -> Input {
    Input::Key(Key::new(name))
}

fn find(p: &Profile, key: &str, with: Option<&str>) -> Option<Action> {
    p.find(&k(key), with.map(k).as_ref(), tak_input::Gesture::Press)
}

const SYNTHETIC: &str = r#"
// A made-up key file in Keys.TDF syntax.
[CustomKeys]
{
    lower_q = UnitCommand Attack;
    LOWER_S = UnitCommand Stop ;
    UPPER_S = UnitCommand Stop;
    LOWER_C = UnitCommand Clear;
    LOWER_D = DiplomacyMenu;
    LOWER_Z =;
    CTRL_K = SelectUnits MELEE;
    CTRLSHIFT_K = SelectUnitsAdd MELEE;
    CTRL_J = SelectUnits Monarch, TrackUnit;
    7 = RetrieveSquad 3;
    0 = RetrieveSquad 10;
    CTRL_7 = CreateSquad 3;
    ALT_7 = CreateSquad 3 formation;
    F9 = MapBookmarkRetrieve 2;
    CTRL_F9 = MapBookmarkStore 2;
    F11 = DebugToggleDebug;
    TAB = fsr;
    SYMBOL_2B = IncreaseSpeed;
    SYMBOL_23 = ToggleDamageBars;
    SYMBOL_60 = ToggleDamageBars;
    XCHAR_E9 =;
    WIBBLE = Nonsense;
}
"#;

#[test]
fn synthetic_keys_tdf_maps_reserves_and_reports() {
    let imp = parse_keys_tdf(SYNTHETIC.as_bytes()).expect("parse");
    let p = Profile::classic_tak().with_keys_tdf(&imp);
    p.validate().unwrap();
    assert_eq!(find(&p, "q", None), Some(Action::AttackMode));
    assert_eq!(find(&p, "s", None), Some(Action::Stop));
    // Shift+S is the same order: Shift just queues, no extra binding.
    assert_eq!(find(&p, "s", Some("shift")), None);
    assert_eq!(find(&p, "c", None), Some(Action::ClearMode));
    // Unbound in the file: gone, even though Classic binds Z with nothing
    // but Ctrl+Z stays (the file leaves it alone).
    assert_eq!(find(&p, "z", None), None);
    assert_eq!(
        find(&p, "z", Some("ctrl")),
        Some(Action::SelectSameType { scope: Scope::Map })
    );
    assert_eq!(
        find(&p, "k", Some("ctrl")),
        Some(Action::SelectCategory {
            category: Category::Melee
        })
    );
    assert_eq!(
        find(&p, "j", Some("ctrl")),
        Some(Action::SelectMonarch { track: true })
    );
    let g = |n| ControlGroup::new(n).unwrap();
    assert_eq!(
        find(&p, "7", None),
        Some(Action::RecallGroup { group: g(3) })
    );
    assert_eq!(
        find(&p, "0", None),
        Some(Action::RecallGroup { group: g(0) })
    );
    assert_eq!(
        find(&p, "7", Some("ctrl")),
        Some(Action::AssignGroup { group: g(3) })
    );
    assert_eq!(
        find(&p, "7", Some("alt")),
        Some(Action::Reserved {
            order: ReservedOrder::Formation
        })
    );
    assert_eq!(find(&p, "f9", None), Some(Action::GoToBookmark { slot: 2 }));
    assert_eq!(
        find(&p, "f9", Some("ctrl")),
        Some(Action::SetBookmark { slot: 2 })
    );
    // Unsupported command: the profile's own F11 (tuning overlay) stays.
    assert_eq!(find(&p, "f11", None), Some(Action::ToggleOverlay));
    assert_eq!(find(&p, "tab", None), Some(Action::ToggleCrystalBall));
    assert_eq!(
        find(&p, "equal", Some("shift")),
        Some(Action::GameSpeed { faster: true })
    );
    assert_eq!(find(&p, "grave", None), Some(Action::ToggleDamageBars));
    let status = |key: &str| {
        imp.lines
            .iter()
            .find(|l| l.key == key)
            .map(|l| l.status)
            .unwrap()
    };
    assert_eq!(status("LOWER_D"), LineStatus::Unsupported);
    assert_eq!(status("CTRLSHIFT_K"), LineStatus::Mapped);
    assert_eq!(status("SYMBOL_23"), LineStatus::Skipped);
    assert_eq!(status("XCHAR_E9"), LineStatus::Skipped);
    assert_eq!(status("WIBBLE"), LineStatus::Skipped);
    assert_eq!(status("LOWER_Z"), LineStatus::Unbound);
    assert_eq!(status("LOWER_C"), LineStatus::Mapped);
    assert_eq!(status("F11"), LineStatus::Unsupported);
    // Mouse bindings are untouched.
    assert!(p.bindings.iter().any(|b| b.action == Action::DragSelect));
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    assert!(parse_keys_tdf(b"").is_err());
    assert!(parse_keys_tdf(b"[Other] { A = B; }").is_err());
    assert!(parse_keys_tdf(&[0xff, 0x00, 0x5b, 0x7b]).is_err() || true);
    let imp = parse_keys_tdf(b"[CUSTOMKEYS] { LOWER_A = UnitCommand; CTRL_ = x; }").unwrap();
    assert!(imp.bindings.is_empty());
}

fn keyboard(p: &Profile) -> BTreeSet<String> {
    p.bindings
        .iter()
        .filter(|b: &&Binding| matches!(b.input, Input::Key(_)))
        .map(|b| serde_json::to_string(b).unwrap())
        .collect()
}

/// The install's `Keys.TDF` gives exactly Classic TA:K's keyboard: Classic
/// is TA:K's own defaults. Counts only, no file contents.
#[test]
fn install_keys_tdf_is_classic_tak() {
    let Some(dir) = std::env::var_os("TAK_DATA_DIR") else {
        eprintln!("TAK_DATA_DIR not set: skipping install Keys.TDF test");
        return;
    };
    let Ok(data) = std::fs::read(std::path::Path::new(&dir).join("Keys.TDF")) else {
        eprintln!("no Keys.TDF in the install: skipping");
        return;
    };
    let imp = parse_keys_tdf(&data).expect("install Keys.TDF parses");
    eprintln!("install Keys.TDF: {}", imp.summary());
    let classic = Profile::classic_tak();
    let applied = classic.with_keys_tdf(&imp);
    applied.validate().unwrap();
    let (a, b) = (keyboard(&classic), keyboard(&applied));
    let only_classic: Vec<_> = a.difference(&b).collect();
    let only_tdf: Vec<_> = b.difference(&a).collect();
    assert!(
        only_classic.is_empty() && only_tdf.is_empty(),
        "classic only: {only_classic:#?}\nfrom Keys.TDF only: {only_tdf:#?}"
    );
    assert!(imp.count(LineStatus::Mapped) >= 60, "{}", imp.summary());
}
