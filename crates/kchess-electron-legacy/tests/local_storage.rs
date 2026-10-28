//! `fixtures/local-storage` was written by Electron 45 itself, loading `kchess://app` as the app
//! does: a UTF-16 study list, Latin-1 view preferences, a value written twice and a removed one.

use std::path::Path;

#[test]
fn reads_what_electron_wrote_for_the_app_origin() {
    let profile = tempfile::tempdir().unwrap();
    let target = profile.path().join("Local Storage").join("leveldb");
    std::fs::create_dir_all(&target).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/local-storage");
    for entry in std::fs::read_dir(&fixture).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
    }
    let before: Vec<_> = std::fs::read_dir(&target).unwrap().collect();

    let found = kchess_electron_legacy::local_storage::read(profile.path()).unwrap();
    assert_eq!(
        found.get("kchess:studies:v1").map(String::as_str),
        Some(
            r#"{"version":2,"items":[{"id":"s1","name":"Ünïcode ♞ study","pgn":"1. e4 *","updatedAt":1,"chapters":[{"id":"c1","name":"Chapter","pgn":"1. e4 e5 *"}]}]}"#
        )
    );
    assert_eq!(found["kchess:studies-sort"], "updated", "the later write");
    assert_eq!(found["kchess:sidebar-open"], "false");
    assert_eq!(found["kchess:puzzle-angle"], r#"{"theme":"fork"}"#);
    assert!(!found.contains_key("kchess:deleted"));
    assert_eq!(found.len(), 4);

    let after: Vec<_> = std::fs::read_dir(&target).unwrap().collect();
    assert_eq!(
        before.len(),
        after.len(),
        "the profile's copy is left alone"
    );
}

#[test]
fn a_profile_without_local_storage_has_none() {
    let profile = tempfile::tempdir().unwrap();
    assert!(
        kchess_electron_legacy::local_storage::read(profile.path())
            .unwrap()
            .is_empty()
    );
}
