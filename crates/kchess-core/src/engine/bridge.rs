//! How the engine services reach the rest of the core: the battery reading the scheduler
//! consults, the database the review queue stores through, the host the reviews talk to
//! (settings, accounts, Lichess, events), and the handoff that suspends engines while an
//! executable is replaced.
//!
//! The battery reading never makes a lease wait for the host: a refresh asks the host in the
//! background and the next lease uses whatever reading has arrived.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};

use super::analysis::Analysis;
use super::managed::{EngineGuard, EngineHandoff};
use super::review::{
    GameToReview, Pause, ReviewHost, ReviewSettings, ReviewStatus, ReviewStorage, ReviewUpdate,
    Reviews, StoredReview,
};
use super::search::Search;
use super::uci::EngineHub;
use crate::capabilities::Capabilities;
use crate::error::{CoreError, Result};
use crate::host::{Host, Level};
use crate::store::Database;

/// How often the battery reading is refreshed while the core runs.
const REFRESH_EVERY: Duration = Duration::from_secs(15);
/// How long a Lichess review lookup may take on the host (it fetches a batch of games).
/// Games a review pass looks at (the store's `gamesToReview` limit).
const REVIEW_LIMIT: usize = 300;

/// The battery reading the scheduler's on-battery function reads. A refresh asks the host once
/// at a time, in the background.
pub struct Battery {
    value: AtomicBool,
    refreshing: AtomicBool,
    closed: AtomicBool,
    host: Arc<dyn Host>,
}

impl Battery {
    pub fn new(host: Arc<dyn Host>) -> Arc<Battery> {
        Arc::new(Battery {
            value: AtomicBool::new(false),
            refreshing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            host,
        })
    }

    /// The latest reading (`CorePlatform.onBattery()`, as of the last refresh).
    pub fn value(&self) -> bool {
        self.value.load(Ordering::SeqCst)
    }

    /// Asks the host for the reading without waiting for the answer. A refresh already in flight
    /// is not repeated. Outside a runtime nothing is asked.
    pub fn refresh(self: &Arc<Self>, capabilities: &Arc<Capabilities>) {
        if self.closed.load(Ordering::SeqCst) || self.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.refreshing.store(false, Ordering::SeqCst);
            return;
        };
        let battery = Arc::clone(self);
        let capabilities = Arc::clone(capabilities);
        runtime.spawn(async move {
            match capabilities.on_battery().await {
                Ok(on_battery) => battery.value.store(on_battery, Ordering::SeqCst),
                Err(cause) => battery.host.log(
                    Level::Debug,
                    "engine",
                    &format!("Battery check failed: {}", cause.message),
                ),
            }
            battery.refreshing.store(false, Ordering::SeqCst);
        });
    }

    /// Refreshes the reading on a slow interval until the core closes or is dropped.
    pub fn start_refreshing(self: &Arc<Self>, capabilities: Arc<Capabilities>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let weak: Weak<Battery> = Arc::downgrade(self);
        runtime.spawn(async move {
            loop {
                tokio::time::sleep(REFRESH_EVERY).await;
                let Some(battery) = weak.upgrade() else {
                    return;
                };
                if battery.closed.load(Ordering::SeqCst) {
                    return;
                }
                battery.refresh(&capabilities);
            }
        });
    }

    /// Stops refreshing; the last reading stays.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

/// The review store: `kchess.db`'s review functions, through the one shared connection.
pub struct ReviewRecords(pub Arc<Database>);

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|cause| CoreError::new(cause.to_string()))
}

impl ReviewStorage for ReviewRecords {
    fn read_review(&self, key: &str) -> Result<Option<StoredReview>> {
        let value = self.0.call("store.reviewStore.readReview", &[json!(key)])?;
        if value.is_null() {
            return Ok(None);
        }
        parse(value).map(Some)
    }

    fn write_review(&self, review: StoredReview) -> Result<ReviewUpdate> {
        let value = self.0.call(
            "store.reviewStore.writeReview",
            &[serde_json::to_value(review).map_err(|cause| CoreError::new(cause.to_string()))?],
        )?;
        parse(value)
    }

    fn has_account(&self, username: &str) -> Result<bool> {
        Ok(self
            .0
            .call("store.reviewStore.hasAccount", &[json!(username)])?
            .as_bool()
            == Some(true))
    }

    fn mark_checked(&self, ids: &[String]) -> Result<()> {
        let at = super::now_ms();
        self.0
            .call("store.reviewStore.markChecked", &[json!(ids), json!(at)])
            .map(|_| ())
    }

    fn games_to_review(&self, accounts: &[String], since: i64) -> Result<Vec<GameToReview>> {
        let value = self.0.call(
            "store.reviewStore.gamesToReview",
            &[json!(accounts), json!(since), json!(REVIEW_LIMIT)],
        )?;
        parse(value)
    }
}

/// The review queue's host in the Rust core: settings and accounts from the store, the busy
/// state from the engines and online play, Lichess through the host, and updates and status as
/// `review:update` and `review:status` events.
pub struct ReviewBridge {
    pub database: Arc<Database>,
    pub host: Arc<dyn Host>,
    pub capabilities: Arc<Capabilities>,
    /// The Lichess service, which fetches the analysis of games to review.
    pub lichess: Arc<crate::lichess::accounts::Lichess>,
    pub search: Search,
    pub analysis: Analysis,
    /// Whether a live online game (or its recovery) is in progress, read when the queue works.
    pub online: Arc<dyn Fn() -> bool + Send + Sync>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppAccount {
    username: String,
    connected: bool,
}

#[derive(Deserialize)]
struct AppData {
    accounts: Vec<AppAccount>,
}

impl ReviewHost for ReviewBridge {
    fn settings(&self) -> BoxFuture<'_, Result<ReviewSettings>> {
        Box::pin(async move { parse(self.database.call("store.games.getSettings", &[])?) })
    }

    fn accounts(&self) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async move {
            let data: AppData = parse(self.database.call("store.games.loadData", &[])?)?;
            // Your own accounts: the connected ones, or the first one added when none is.
            let own: Vec<String> = data
                .accounts
                .iter()
                .filter(|account| account.connected)
                .map(|account| account.username.clone())
                .collect();
            if !own.is_empty() {
                return Ok(own);
            }
            Ok(data
                .accounts
                .into_iter()
                .take(1)
                .map(|account| account.username)
                .collect())
        })
    }

    fn busy(&self) -> Option<Pause> {
        if (self.online)() {
            Some(Pause::Online)
        } else if self.analysis.analysis_running() || self.search.computer_playing(60_000) {
            // The computer opponent counts as busy for a minute after its move: the game goes on.
            Some(Pause::Engine)
        } else {
            None
        }
    }

    fn fetch_lichess(
        &self,
        account: &str,
        ids: &[String],
    ) -> BoxFuture<'_, Result<Vec<StoredReview>>> {
        let account = account.to_string();
        let ids = ids.to_vec();
        Box::pin(async move {
            let reviews =
                crate::lichess::reviews::fetch_lichess_reviews(&self.lichess, &account, &ids)
                    .await?;
            parse(Value::Array(reviews))
        })
    }

    fn update(&self, update: ReviewUpdate) {
        self.emit("review:update", json!(update));
    }

    fn status(&self, status: ReviewStatus) {
        self.emit("review:status", json!(status));
    }
}

impl ReviewBridge {
    fn emit(&self, event: &str, payload: Value) {
        self.host.emit(event, payload);
    }
}

/// The engine owners a replacement of the executable stops: the computer's search, the analysis
/// engine and the review engine. Their processes must have exited before the file changes.
pub struct EngineOwners {
    pub hub: Arc<EngineHub>,
    pub search: Search,
    pub analysis: Analysis,
    pub reviews: Reviews,
}

impl EngineHandoff for EngineOwners {
    fn suspend(&self) -> Pin<Box<dyn Future<Output = Result<EngineGuard>> + Send + '_>> {
        Box::pin(async move {
            let hold = self
                .hub
                .suspend(|| {
                    self.search.stop_engine(true);
                    self.analysis.stop_analysis(true);
                    self.reviews.restart_review_engine();
                })
                .await?;
            Ok(Box::new(hold) as EngineGuard)
        })
    }
}
