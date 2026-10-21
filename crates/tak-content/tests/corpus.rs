//! Corpus tests: need `TAK_DATA_DIR`; skip otherwise. They assert counts
//! and zero errors only, never file contents.

use tak_content::{MapDef, RefKind, Ruleset, RulesetKind, StartSource, playable_maps};
use tak_vfs::testing::data_dir_or_skip;

fn unit_resolution_misses(rs: &Ruleset) -> usize {
    rs.dangling
        .iter()
        .filter(|d| {
            d.from.starts_with("unit ")
                && matches!(d.kind, RefKind::Model | RefKind::Script)
                && (d.field == "objectname" || d.field == "script")
        })
        .count()
}

#[test]
fn base_ruleset_loads() {
    let Some(dir) = data_dir_or_skip("base_ruleset_loads") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    let rs = Ruleset::load(&vfs, RulesetKind::Base);
    assert!(rs.issues.is_empty(), "{:#?}", rs.issues);
    assert_eq!(rs.files["fbi"], 203);
    assert_eq!(rs.files["feature"], 412);
    assert_eq!(rs.files["canbuild"], 196);
    assert_eq!(rs.units.len(), 203);
    // 1379 feature sections; 35 names repeat across world folders.
    assert_eq!(rs.features.len(), 1344);
    assert_eq!(rs.overrides.len(), 35);
    assert_eq!(rs.build_menus.len(), 32);
    assert_eq!(rs.sides.len(), 8);
    assert_eq!(rs.move_classes.len(), 11);
    let weapons: usize = rs.units.values().map(|u| u.weapons.len()).sum();
    let armed = rs.units.values().filter(|u| !u.weapons.is_empty()).count();
    let exploding = rs.units.values().filter(|u| u.explode_as.is_some()).count();
    let mana = rs
        .units
        .values()
        .filter(|u| u.economy.max_mana.is_some())
        .count();
    assert_eq!((weapons, armed, exploding, mana), (198, 141, 6, 34));
    assert_eq!(unit_resolution_misses(&rs), 0);
    assert!(rs.dangling.is_empty(), "{:#?}", rs.dangling);
}

#[test]
fn cb_ruleset_loads() {
    let Some(dir) = data_dir_or_skip("cb_ruleset_loads") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    let rs = Ruleset::load(&vfs, RulesetKind::Cb);
    assert!(rs.issues.is_empty(), "{:#?}", rs.issues);
    // units/ (203) then unitscb/ (197) over it.
    assert_eq!(rs.files["fbi"], 400);
    assert_eq!(rs.files["canbuild"], 171);
    assert_eq!(rs.units.len(), 203);
    let from_cb = rs
        .units
        .values()
        .filter(|u| u.source.to_ascii_lowercase().starts_with("unitscb/"))
        .count();
    assert_eq!(from_cb, 197);
    assert_eq!(rs.build_menus.len(), 27);
    assert_eq!(unit_resolution_misses(&rs), 0);
    // unitscb/arapal.fbi names a frozen feature that does not exist.
    assert_eq!(rs.dangling.len(), 1, "{:#?}", rs.dangling);
    assert_eq!(rs.dangling[0].kind, RefKind::Feature);
}

#[test]
fn all_playable_maps_build() {
    let Some(dir) = data_dir_or_skip("all_playable_maps_build") else {
        return;
    };
    let vfs = tak_vfs::open_kingdoms(&dir).unwrap();
    let rs = Ruleset::load(&vfs, RulesetKind::Base);
    let maps = playable_maps(&vfs);
    assert_eq!(maps.len(), 310);
    let (mut feature_refs, mut unit_refs, mut fallback, mut placed, mut with_crt) =
        (0, 0, 0, 0usize, 0);
    for name in &maps {
        let m = MapDef::load(&vfs, &rs, name).unwrap_or_else(|e| panic!("{e}"));
        assert!(m.issues.is_empty(), "{name}: {:#?}", m.issues);
        assert!(m.player_slots >= 1, "{name}");
        assert!(m.missing_start_slots().is_empty(), "{name}");
        for s in &m.start_positions {
            assert!(s.x >= 0 && (s.x as u32) < m.width, "{name}");
            assert!(s.z >= 0 && (s.z as u32) < m.height, "{name}");
        }
        assert!(m.kingdom.is_some(), "{name}");
        assert_eq!(m.passability.cells.len(), m.heights.len());
        fallback += m
            .start_positions
            .iter()
            .filter(|s| s.source == StartSource::FirstUnit)
            .count();
        feature_refs += m
            .dangling
            .iter()
            .filter(|d| d.kind == RefKind::Feature)
            .count();
        unit_refs += m
            .dangling
            .iter()
            .filter(|d| d.kind == RefKind::Unit)
            .count();
        placed += m.features.len();
        with_crt += usize::from(m.crt.is_some());
    }
    // takmission11_dh, takmission13_ph, takmission22_ph lack a marker for
    // their AI slot.
    assert_eq!(fallback, 3);
    // Temple of Blood lists `zonhand_dead`, which no feature file defines.
    assert_eq!(feature_refs, 1);
    assert_eq!(unit_refs, 0);
    assert_eq!(placed, 88_319);
    assert_eq!(with_crt, 214);
}
