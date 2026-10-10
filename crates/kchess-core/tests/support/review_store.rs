//! An in-memory `ReviewStorage` that follows `crates/kchess-node/js/reviewStore.ts` (which review
//! ranking, game links, account checks and the Lichess check marks), and a scripted `ReviewHost`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use kchess_core::engine::review::{
    GameToReview, Pause, ReviewHost, ReviewSettings, ReviewStatus, ReviewStorage, ReviewUpdate,
    StoredReview,
};
use kchess_core::error::Result;
use serde_json::Value;

/// A game the store holds (`games`).
#[derive(Clone)]
pub struct GameRow {
    pub id: String,
    pub account: String,
    pub moves: String,
    pub pgn: Option<String>,
    pub perf: String,
    pub created_at: i64,
}

#[derive(Default)]
struct Inner {
    reviews: HashMap<String, StoredReview>,
    /// `game_reviews`: (game id, review key).
    links: BTreeSet<(String, String)>,
    accounts: Vec<String>,
    games: Vec<GameRow>,
    checked: HashSet<String>,
}

#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

fn rank(review: &StoredReview) -> u8 {
    u8::from(review.complete) * 2 + u8::from(review.source == "lichess")
}

impl MemoryStore {
    pub fn new() -> Arc<MemoryStore> {
        Arc::new(MemoryStore::default())
    }

    pub fn add_account(&self, name: &str) {
        self.inner
            .lock()
            .expect("store")
            .accounts
            .push(name.to_string());
    }

    pub fn remove_account(&self, name: &str) {
        self.inner
            .lock()
            .expect("store")
            .accounts
            .retain(|account| !account.eq_ignore_ascii_case(name));
    }

    pub fn add_game(&self, game: GameRow) {
        self.inner.lock().expect("store").games.push(game);
    }

    pub fn review(&self, key: &str) -> Option<StoredReview> {
        self.inner.lock().expect("store").reviews.get(key).cloned()
    }

    /// Whether a game is linked to a review with this key.
    pub fn linked(&self, game: &str, key: &str) -> bool {
        self.inner
            .lock()
            .expect("store")
            .links
            .contains(&(game.to_string(), key.to_string()))
    }

    /// Stores `review` directly, as an earlier session would have.
    pub fn put(&self, review: StoredReview) {
        self.inner
            .lock()
            .expect("store")
            .reviews
            .insert(review.key.clone(), review);
    }

    pub fn checked(&self, id: &str) -> bool {
        self.inner.lock().expect("store").checked.contains(id)
    }
}

fn summary_of(review: &StoredReview) -> Value {
    serde_json::to_value(review)
        .ok()
        .and_then(|value| kchess_domain::review::summarize(&value))
        .unwrap_or(Value::Null)
}

impl ReviewStorage for MemoryStore {
    fn read_review(&self, key: &str) -> Result<Option<StoredReview>> {
        Ok(self.review(key))
    }

    fn write_review(&self, review: StoredReview) -> Result<ReviewUpdate> {
        let mut inner = self.inner.lock().expect("store");
        let existing = inner.reviews.get(&review.key).cloned();
        if let Some(stored) = &existing
            && rank(stored) > rank(&review)
        {
            if let Some(game) = &review.game_id {
                inner.links.insert((game.clone(), review.key.clone()));
            }
            let kept = StoredReview {
                game_id: review.game_id.clone().or_else(|| stored.game_id.clone()),
                ..stored.clone()
            };
            let summary = summary_of(&kept);
            return Ok(ReviewUpdate {
                review: kept,
                summary,
            });
        }
        let merged = StoredReview {
            game_id: review
                .game_id
                .clone()
                .or_else(|| existing.and_then(|stored| stored.game_id)),
            updated_at: 1_000,
            ..review
        };
        inner.reviews.insert(merged.key.clone(), merged.clone());
        if let Some(game) = &merged.game_id {
            inner.links.insert((game.clone(), merged.key.clone()));
        }
        let summary = summary_of(&merged);
        Ok(ReviewUpdate {
            review: merged,
            summary,
        })
    }

    fn has_account(&self, username: &str) -> Result<bool> {
        Ok(self
            .inner
            .lock()
            .expect("store")
            .accounts
            .iter()
            .any(|account| account.eq_ignore_ascii_case(username)))
    }

    fn mark_checked(&self, ids: &[String]) -> Result<()> {
        self.inner
            .lock()
            .expect("store")
            .checked
            .extend(ids.iter().cloned());
        Ok(())
    }

    fn games_to_review(&self, accounts: &[String], since: i64) -> Result<Vec<GameToReview>> {
        let inner = self.inner.lock().expect("store");
        let mut games: Vec<&GameRow> = inner
            .games
            .iter()
            .filter(|game| {
                accounts
                    .iter()
                    .any(|account| account.eq_ignore_ascii_case(&game.account))
                    && game.created_at >= since
                    && !game.moves.is_empty()
            })
            .filter(|game| {
                !inner.links.iter().any(|(id, key)| {
                    id == &game.id && inner.reviews.get(key).is_some_and(|review| review.complete)
                })
            })
            .collect();
        games.sort_by_key(|game| std::cmp::Reverse(game.created_at));
        Ok(games
            .into_iter()
            .take(300)
            .map(|game| GameToReview {
                id: game.id.clone(),
                account: game.account.clone(),
                moves: game.moves.clone(),
                pgn: game.pgn.clone(),
                perf: game.perf.clone(),
                created_at: game.created_at,
                checked: inner.checked.contains(&game.id),
            })
            .collect())
    }
}

/// A host whose settings, accounts and busy state the test sets, recording what the review sends.
pub struct ScriptedHost {
    pub settings: Mutex<ReviewSettings>,
    pub accounts: Mutex<Vec<String>>,
    pub busy: Mutex<Option<Pause>>,
    pub store: Arc<MemoryStore>,
    /// The `(account, ids)` of each Lichess lookup.
    pub fetched: Mutex<Vec<(String, Vec<String>)>>,
    pub updates: Mutex<Vec<ReviewUpdate>>,
    pub statuses: Mutex<Vec<ReviewStatus>>,
}

impl ScriptedHost {
    pub fn new(store: Arc<MemoryStore>, settings: ReviewSettings) -> Arc<ScriptedHost> {
        Arc::new(ScriptedHost {
            settings: Mutex::new(settings),
            accounts: Mutex::new(vec!["tester".to_string()]),
            busy: Mutex::new(None),
            store,
            fetched: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
            statuses: Mutex::new(Vec::new()),
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
        Box::pin(async move { Ok(self.settings.lock().expect("settings").clone()) })
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
            self.store.mark_checked(&ids)?;
            Ok(Vec::new())
        })
    }

    fn update(&self, update: ReviewUpdate) {
        self.updates.lock().expect("updates").push(update);
    }

    fn status(&self, status: ReviewStatus) {
        self.statuses.lock().expect("statuses").push(status);
    }
}
