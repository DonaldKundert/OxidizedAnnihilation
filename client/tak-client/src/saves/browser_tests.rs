use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use tak_headless::checkpoint::ReplaySession;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tak-browser-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::create_dir(path.join("data")).unwrap();
        Self(path)
    }
    fn saves(&self) -> PathBuf {
        self.0.join("saves")
    }
    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
    fn write(&self, name: &str, seed: u64, ticks: u32) -> PathBuf {
        std::fs::create_dir_all(self.saves()).unwrap();
        let mut replay = ReplaySession::new(
            CheckpointSource::synthetic(tak_sim::testing::example_scenario(seed, 2)),
            None,
        )
        .unwrap();
        replay.advance(&CommandLog::new(), ticks).unwrap();
        let path = self.saves().join(name);
        replay.checkpoint().unwrap().write_atomic(&path).unwrap();
        path
    }
    fn entries(&self) -> Catalog {
        catalog(&self.saves(), &self.data()).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn opening_an_empty_browser_does_not_create_any_directory() {
    let scratch = Scratch::new();
    assert!(scratch.entries().entries.is_empty());
    assert!(!scratch.saves().exists());
}

#[test]
fn metadata_exposes_time_fingerprint_and_headless_only_source() {
    let scratch = Scratch::new();
    scratch.write("test.save.json", 1, 31);
    let entries = scratch.entries();
    let metadata = entries.entries[0].metadata.as_ref().unwrap();
    assert_eq!(metadata.next_tick, 31);
    assert_eq!(metadata.match_time(), "00:00:01");
    assert_eq!(metadata.commands, 0);
    assert!(!metadata.content_identity.is_empty());
    assert!(!metadata.state_hash.is_empty());
    assert!(!metadata.playable);
    assert!(!entries.truncated);
}

#[test]
fn corrupt_oversize_and_incompatible_entries_are_visible_without_panics() {
    let scratch = Scratch::new();
    let path = scratch.write("valid.save.json", 1, 0);
    let incompatible = std::fs::read_to_string(path).unwrap().replace(
        &format!(
            "\"replay_rules_version\": {}",
            tak_headless::checkpoint::REPLAY_RULES_VERSION
        ),
        "\"replay_rules_version\": 0",
    );
    std::fs::write(scratch.saves().join("old.save.json"), incompatible).unwrap();
    std::fs::write(scratch.saves().join("bad.save.json"), b"broken").unwrap();
    let large = std::fs::File::create(scratch.saves().join("large.save.json")).unwrap();
    large.set_len(MAX_CHECKPOINT_BYTES as u64 + 1).unwrap();
    std::fs::write(scratch.saves().join("unrelated.json"), b"ignored").unwrap();
    std::fs::create_dir(scratch.saves().join("directory.save.json")).unwrap();
    let entries = scratch.entries().entries;
    assert_eq!(entries.len(), 4);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.metadata.is_err())
            .count(),
        3
    );
}

#[test]
fn changed_or_missing_selection_must_be_refreshed_and_confirmed_again() {
    let scratch = Scratch::new();
    let path = scratch.write("test.save.json", 1, 0);
    let entry = scratch.entries().entries.remove(0);
    assert!(checked_entry(&scratch.saves(), &scratch.data(), &entry).is_ok());
    scratch.write("test.save.json", 2, 0);
    let error = checked_entry(&scratch.saves(), &scratch.data(), &entry).unwrap_err();
    assert!(error.to_string().contains("changed since it was listed"));
    std::fs::remove_file(path).unwrap();
    assert!(checked_entry(&scratch.saves(), &scratch.data(), &entry).is_err());
}

#[test]
fn selection_path_cannot_escape_the_save_directory() {
    let scratch = Scratch::new();
    scratch.write("test.save.json", 1, 0);
    let mut entry = scratch.entries().entries.remove(0);
    entry.path = scratch.data().join(&entry.name);
    assert!(checked_entry(&scratch.saves(), &scratch.data(), &entry).is_err());
    assert!(catalog(&scratch.data(), &scratch.data()).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_listed_and_a_replaced_selection_cannot_follow_one() {
    let scratch = Scratch::new();
    let path = scratch.write("test.save.json", 1, 0);
    let entry = scratch.entries().entries.remove(0);
    let target = scratch.0.join("external.json");
    std::fs::rename(&path, &target).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(scratch.entries().entries.is_empty());
    assert!(checked_entry(&scratch.saves(), &scratch.data(), &entry).is_err());
}

#[test]
fn navigation_stays_in_bounds_and_confirmation_captures_its_original_target() {
    let scratch = Scratch::new();
    for number in 0..8 {
        scratch.write(&format!("{number}.save.json"), number, 0);
    }
    let mut browser = Browser {
        catalog: scratch.entries(),
        ..Default::default()
    };
    browser.navigate(-100);
    assert_eq!(browser.selected, 0);
    browser.navigate(100);
    assert_eq!(browser.selected, 7);
    assert_eq!(browser.first_visible(), PAGE_ROWS);
    let expected = browser.selected().unwrap().path.clone();
    browser.request_selected(true).unwrap();
    assert!(!browser.confirmation.as_ref().unwrap().accept);
    browser.navigate(-100);
    assert_eq!(browser.selected, 7);
    assert!(
        matches!(browser.resolve_confirmation(true), Some(Operation::Overwrite(entry)) if entry.path == expected)
    );
    browser.request_selected(true).unwrap();
    assert!(browser.resolve_confirmation(false).is_none());
    assert!(browser.confirmation.is_none());
    browser.catalog.entries.clear();
    browser.navigate(100);
    assert_eq!(browser.selected, 0);
    assert!(browser.request_selected(false).is_err());
}

#[test]
fn synthetic_load_is_explained_before_confirmation_and_refresh_cancels_prompt() {
    let scratch = Scratch::new();
    scratch.write("test.save.json", 1, 0);
    let mut browser = Browser::default();
    browser.refresh(&scratch.saves(), &scratch.data()).unwrap();
    assert!(
        browser
            .request_selected(false)
            .unwrap_err()
            .to_string()
            .contains("headless only")
    );
    assert!(browser.confirmation.is_none());
    browser.request_selected(true).unwrap();
    browser.refresh(&scratch.saves(), &scratch.data()).unwrap();
    assert!(browser.confirmation.is_none());
}

#[test]
fn catalog_size_is_bounded() {
    let scratch = Scratch::new();
    std::fs::create_dir(scratch.saves()).unwrap();
    for index in 0..MAX_BROWSER_ENTRIES + 5 {
        std::fs::write(
            scratch.saves().join(format!("{index}.save.json")),
            b"invalid",
        )
        .unwrap();
    }
    let catalog = scratch.entries();
    assert_eq!(catalog.entries.len(), MAX_BROWSER_ENTRIES);
    assert!(catalog.truncated);
}
