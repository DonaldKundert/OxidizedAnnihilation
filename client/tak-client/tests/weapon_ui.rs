//! Runtime-only art verification. No retail pixels or metadata snapshots
//! are stored: an owned install is optional, as in the replay tests.

use std::collections::BTreeSet;

use tak_client::game::open_vfs;
use tak_client::hud::weapon_button_paths;
use tak_content::{Ruleset, RulesetKind};

#[test]
fn installed_weapon_button_pictures_decode() {
    let Some(dir) = tak_vfs::testing::data_dir_or_skip("weapon button pictures") else {
        return;
    };
    let vfs = open_vfs(&dir).expect("open install");
    let rules = Ruleset::load(&vfs, RulesetKind::Base);
    let paths: BTreeSet<String> = rules
        .units
        .values()
        .flat_map(|unit| &unit.weapons)
        .flat_map(|weapon| weapon_button_paths(&weapon.extra))
        .flatten()
        .collect();
    assert!(!paths.is_empty(), "install has no weapon button metadata");
    let mut failures = Vec::new();
    for path in &paths {
        let valid = vfs
            .read(path)
            .ok()
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .is_some_and(|image| image.width() > 0 && image.height() > 0);
        if !valid {
            failures.push(path);
        }
    }
    assert!(
        failures.is_empty(),
        "missing or invalid weapon pictures: {failures:?}"
    );
    eprintln!("{} distinct weapon button pictures decoded", paths.len());
}
