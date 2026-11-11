//! The tests' in-memory `LichessStore` (`fixtures::MemoryStore`) answers as the real one over
//! `kchess.db` (`DbLichessStore`) does: the same calls, made of both, give the same answers. The
//! Lichess tests use the fake for its hooks (a held cache write) and its inspectable state; this
//! keeps its behaviour from drifting from the store the app runs.

use std::sync::Arc;

use kchess_core::Config;
use kchess_core::lichess::accounts::LichessStore;
use kchess_core::lichess::adapters::DbLichessStore;
use kchess_core::store::Database;
use serde_json::json;

use crate::fixtures::{MemoryStore, Recording};

/// What a store answered to the script below, as one comparable list.
async fn script(store: &dyn LichessStore) -> Vec<String> {
    let mut seen = Vec::new();
    let yes = || true;
    let no = || false;
    let row = |rows: Vec<kchess_core::lichess::accounts::AccountRow>| {
        rows.iter()
            .map(|row| {
                format!(
                    "{} {} {:?}",
                    row.username, row.connected, row.last_synced_at
                )
            })
            .collect::<Vec<_>>()
    };
    seen.push(format!(
        "accounts {:?}",
        row(store.accounts().await.unwrap())
    ));
    seen.push(format!(
        "dismissed {:?}",
        store.dismissed_friends().await.unwrap()
    ));

    let refused = store.save_login("Alice", "enc:a", &no).await.unwrap_err();
    seen.push(format!("cancelled login {:?}", refused.aborted));
    store.save_login("Alice", "enc:a", &yes).await.unwrap();
    seen.push(format!(
        "signed in {:?}",
        row(store.accounts().await.unwrap())
    ));

    store.save_games_cursor("Alice", 1234, &yes).await.unwrap();
    seen.push(format!("synced {:?}", row(store.accounts().await.unwrap())));
    let refused = store.save_games_cursor("Alice", 99, &no).await.unwrap_err();
    seen.push(format!("cancelled cursor {}", refused.message));
    seen.push(format!("kept {:?}", row(store.accounts().await.unwrap())));

    let refused = store
        .save_games_page("Alice", &[], &[], &no)
        .await
        .unwrap_err();
    seen.push(format!("cancelled page {}", refused.message));
    let refused = store.save_reviews("Alice", &[], &no).await.unwrap_err();
    seen.push(format!("cancelled reviews {}", refused.message));
    let refused = store
        .mark_checked(&["g1".to_string()], &no)
        .await
        .unwrap_err();
    seen.push(format!(
        "cancelled check {} {:?}",
        refused.message, refused.aborted
    ));

    seen.push(format!(
        "no cache {:?}",
        store.read_api_cache("k").await.unwrap().is_none()
    ));
    store
        .write_api_cache("k", &json!({ "a": 1 }), &yes)
        .await
        .unwrap();
    let cached = store
        .read_api_cache("k")
        .await
        .unwrap()
        .map(|cached| cached.value);
    seen.push(format!("cached {cached:?}"));
    store
        .write_api_cache("k", &json!({ "a": 2 }), &no)
        .await
        .unwrap();
    let cached = store
        .read_api_cache("k")
        .await
        .unwrap()
        .map(|cached| cached.value);
    seen.push(format!("a cancelled write is skipped {cached:?}"));
    seen
}

#[tokio::test]
async fn the_fake_store_answers_as_the_real_one() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Recording::default());
    let database = Arc::new(Database::new(Config::new(dir.path().to_path_buf()), host));
    let real = script(&DbLichessStore::new(database)).await;
    let fake = script(&*MemoryStore::with_accounts(&[])).await;
    assert_eq!(fake, real);
}
