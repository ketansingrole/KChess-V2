//! The review queue's surroundings: a temporary profile's `kchess.db` holding its games and
//! reviews, and a scripted `ReviewHost`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use kchess_contracts::error::Result;
use kchess_core::Config;
use kchess_core::engine::review::{
    Pause, ReviewHost, ReviewSettings, ReviewStatus, ReviewUpdate, StoredReview,
};
use kchess_core::store::{Database, games, reviews};
use rusqlite::OptionalExtension;
use serde_json::json;
use tempfile::TempDir;

use super::Recording;

/// A synced game, as a test adds it to the profile.
#[derive(Clone)]
pub struct GameRow {
    pub id: String,
    pub account: String,
    pub moves: String,
    pub pgn: Option<String>,
    pub perf: String,
    pub created_at: i64,
}

/// A fresh profile whose database the review queue stores into.
pub struct ReviewDb {
    _dir: TempDir,
    pub database: Arc<Database>,
}

impl ReviewDb {
    pub fn new() -> Arc<ReviewDb> {
        let dir = tempfile::tempdir().expect("temporary profile");
        let database = Arc::new(Database::new(
            Config::new(dir.path().to_path_buf()),
            Arc::new(Recording::default()),
        ));
        Arc::new(ReviewDb {
            _dir: dir,
            database,
        })
    }

    pub fn add_account(&self, name: &str) {
        self.database
            .write(|ctx| games::add_account(ctx.db, name, true).map(drop))
            .expect("account");
    }

    /// Adds a game of `game.account`, which is added first when it is not on this device.
    pub fn add_game(&self, game: GameRow) {
        let known = self
            .database
            .read(|ctx| reviews::has_account(ctx.db, &game.account))
            .expect("accounts");
        if !known {
            self.add_account(&game.account);
        }
        let row = json!({
            "account": game.account,
            "id": game.id,
            "moves": game.moves,
            "pgn": game.pgn,
            "perf": game.perf,
            "speed": game.perf,
            "createdAt": game.created_at,
            "lastMoveAt": game.created_at,
            "status": "mate",
            "rated": true,
            "color": "white",
            "opponent": "rival",
        });
        self.database
            .write(|ctx| games::save_games_page(ctx.host, ctx.db, &game.account, &[row], &[]))
            .expect("game");
    }

    pub fn review(&self, key: &str) -> Option<StoredReview> {
        self.database
            .read(|ctx| reviews::read_review(ctx.host, ctx.db, key))
            .expect("review read")
            .map(|value| serde_json::from_value(value).expect("a stored review"))
    }

    /// Whether a game is linked to a review with this key.
    pub fn linked(&self, game: &str, key: &str) -> bool {
        self.database
            .read(|ctx| {
                Ok(ctx
                    .db
                    .query_row(
                        "SELECT 1 FROM game_reviews WHERE gameId = ?1 AND reviewKey = ?2",
                        [game, key],
                        |_| Ok(()),
                    )
                    .optional()
                    .expect("links")
                    .is_some())
            })
            .expect("link read")
    }

    /// Stores `review` directly, as an earlier session would have.
    pub fn put(&self, review: StoredReview) {
        let value = serde_json::to_value(review).expect("review");
        self.database
            .write(|ctx| reviews::write_review(ctx.host, ctx.db, &value).map(drop))
            .expect("review write");
    }

    /// Marks these games as looked up on Lichess.
    pub async fn mark_checked(&self, ids: Vec<String>) -> Result<()> {
        let at = kchess_core::time::now_ms() as f64;
        self.database
            .write_async(move |ctx| reviews::mark_checked(ctx.db, &ids, at))
            .await
    }
}

/// A host whose settings, accounts and busy state the test sets, recording what the review sends.
pub struct ScriptedHost {
    pub settings: Mutex<ReviewSettings>,
    pub accounts: Mutex<Vec<String>>,
    pub busy: Mutex<Option<Pause>>,
    pub store: Arc<ReviewDb>,
    /// The `(account, ids)` of each Lichess lookup.
    pub fetched: Mutex<Vec<(String, Vec<String>)>>,
    pub updates: Mutex<Vec<ReviewUpdate>>,
    pub statuses: Mutex<Vec<ReviewStatus>>,
    /// When set, the next settings read waits until it is answered: a test holds the review
    /// queue inside one pass with it.
    pub hold_settings: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    /// How many settings reads have started.
    pub settings_reads: AtomicUsize,
    /// When set, the next update panics, as a broken window would.
    pub panic_on_update: AtomicBool,
}

impl ScriptedHost {
    pub fn new(store: Arc<ReviewDb>, settings: ReviewSettings) -> Arc<ScriptedHost> {
        Arc::new(ScriptedHost {
            settings: Mutex::new(settings),
            accounts: Mutex::new(vec!["tester".to_string()]),
            busy: Mutex::new(None),
            store,
            fetched: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
            statuses: Mutex::new(Vec::new()),
            hold_settings: Mutex::new(None),
            settings_reads: AtomicUsize::new(0),
            panic_on_update: AtomicBool::new(false),
        })
    }

    pub fn last_status(&self) -> Option<ReviewStatus> {
        self.statuses.lock().expect("statuses").last().cloned()
    }

    pub fn complete_updates(&self) -> Vec<StoredReview> {
        self.updates
            .lock()
            .expect("updates")
            .iter()
            .filter(|update| update.review.complete)
            .map(|update| update.review.clone())
            .collect()
    }
}

/// Settings that name the fake engine as the configured engine.
pub fn settings(auto: &str, on_battery: bool) -> ReviewSettings {
    ReviewSettings {
        review_auto: auto.to_string(),
        review_on_battery: on_battery,
        engine_path: super::fake_path(),
    }
}

impl ReviewHost for ScriptedHost {
    fn settings(&self) -> BoxFuture<'_, Result<ReviewSettings>> {
        Box::pin(async move {
            self.settings_reads.fetch_add(1, Ordering::SeqCst);
            let held = self.hold_settings.lock().expect("hold").take();
            if let Some(held) = held {
                let _ = held.await;
            }
            Ok(self.settings.lock().expect("settings").clone())
        })
    }

    fn accounts(&self) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async move { Ok(self.accounts.lock().expect("accounts").clone()) })
    }

    fn busy(&self) -> Option<Pause> {
        *self.busy.lock().expect("busy")
    }

    fn fetch_lichess(
        &self,
        account: &str,
        ids: &[String],
    ) -> BoxFuture<'_, Result<Vec<StoredReview>>> {
        let account = account.to_string();
        let ids = ids.to_vec();
        Box::pin(async move {
            self.fetched
                .lock()
                .expect("fetched")
                .push((account, ids.clone()));
            // What the real host does: the games are marked checked, nothing came back.
            self.store.mark_checked(ids).await?;
            Ok(Vec::new())
        })
    }

    fn update(&self, update: ReviewUpdate) {
        if self.panic_on_update.swap(false, Ordering::SeqCst) {
            panic!("the window failed showing {}", update.review.key);
        }
        self.updates.lock().expect("updates").push(update);
    }

    fn status(&self, status: ReviewStatus) {
        self.statuses.lock().expect("statuses").push(status);
    }
}
