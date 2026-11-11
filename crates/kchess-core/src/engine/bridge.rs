//! How the engine services reach the rest of the core: the battery reading the scheduler
//! consults, the database the review queue stores through, the host the reviews talk to
//! (settings, accounts, Lichess, events), and the handoff that suspends engines while an
//! executable is replaced.
//!
//! The battery reading never makes a lease wait for the host: a refresh asks the host in the
//! background and the next lease uses whatever reading has arrived.

use kchess_contracts::CoreEvent;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures_util::future::BoxFuture;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::analysis::Analysis;
use super::managed::{EngineGuard, EngineHandoff};
use super::review::{
    Pause, ReviewHost, ReviewSettings, ReviewStatus, ReviewUpdate, Reviews, StoredReview, parse,
};
use super::scheduler::Power;
use super::search::Search;
use super::uci::EngineHub;
use crate::capabilities::Capabilities;
use crate::error::Result;
use crate::host::{Host, Level};
use crate::store::{Database, chesscom, games};

/// How often the battery reading is refreshed while the core runs.
const REFRESH_EVERY: Duration = Duration::from_secs(15);

/// The battery reading the scheduler consults. A refresh asks the host once at a time, in the
/// background.
///
/// Intentionally stale-tolerant: refreshed at most every 15 s and before each lease without
/// waiting, never blocking lease admission. A plug/unplug flip mid-queue uses the last reading
/// (at most 2 threads on battery until refresh).
pub struct Battery {
    value: AtomicBool,
    refreshing: AtomicBool,
    closed: AtomicBool,
    /// Ends the refresh loop the moment the core closes, so it lets go of the capabilities (and
    /// the connections they reach) then rather than at its next tick.
    stopped: CancellationToken,
    host: Arc<dyn Host>,
    capabilities: Arc<Capabilities>,
}

impl Battery {
    pub fn new(host: Arc<dyn Host>, capabilities: Arc<Capabilities>) -> Arc<Battery> {
        Arc::new(Battery {
            value: AtomicBool::new(false),
            refreshing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            stopped: CancellationToken::new(),
            host,
            capabilities,
        })
    }

    /// The latest reading.
    pub fn value(&self) -> bool {
        self.value.load(Ordering::SeqCst)
    }

    /// Asks the host for the reading without waiting for the answer. A refresh already in flight
    /// is not repeated. Outside a runtime nothing is asked.
    pub fn refresh(self: &Arc<Self>) {
        if self.closed.load(Ordering::SeqCst) || self.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.refreshing.store(false, Ordering::SeqCst);
            return;
        };
        let battery = Arc::clone(self);
        runtime.spawn(async move {
            match battery.capabilities.on_battery().await {
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
    pub fn start_refreshing(self: &Arc<Self>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let weak: Weak<Battery> = Arc::downgrade(self);
        let stopped = self.stopped.clone();
        runtime.spawn(async move {
            loop {
                tokio::select! {
                    () = stopped.cancelled() => return,
                    () = tokio::time::sleep(REFRESH_EVERY) => {}
                }
                let Some(battery) = weak.upgrade() else {
                    return;
                };
                if battery.closed.load(Ordering::SeqCst) {
                    return;
                }
                battery.refresh();
            }
        });
    }

    /// Stops refreshing; the last reading stays.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.stopped.cancel();
    }
}

impl Drop for Battery {
    fn drop(&mut self) {
        self.stopped.cancel();
    }
}

impl Power for Arc<Battery> {
    fn on_battery(&self) -> bool {
        self.value()
    }

    fn refresh(&self) {
        Battery::refresh(self);
    }
}

/// The review queue's host: settings and accounts from the store, the busy
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

impl ReviewHost for ReviewBridge {
    fn settings(&self) -> BoxFuture<'_, Result<ReviewSettings>> {
        Box::pin(async move {
            let settings = self
                .database
                .read_async(|ctx| games::get_settings(ctx.db))
                .await?
                .core;
            Ok(ReviewSettings {
                review_auto: settings.review_auto.as_str().to_string(),
                review_on_battery: settings.review_on_battery,
                engine_path: settings.engine_path,
            })
        })
    }

    fn accounts(&self) -> BoxFuture<'_, Result<Vec<String>>> {
        Box::pin(async move {
            let data = self
                .database
                .read_async(|ctx| games::load_app_data(ctx.db))
                .await?;
            // Your own accounts: the connected ones, or the first one added when none is, and
            // every Chess.com player added (they are added by name, as your own).
            let mut own: Vec<String> = data
                .accounts
                .iter()
                .filter(|account| account.connected)
                .map(|account| account.username.clone())
                .collect();
            if own.is_empty() {
                own.extend(
                    data.accounts
                        .into_iter()
                        .take(1)
                        .map(|account| account.username),
                );
            }
            let chesscom = self
                .database
                .read_async(|ctx| chesscom::accounts(ctx.db))
                .await?;
            own.extend(
                chesscom
                    .iter()
                    .map(|row| kchess_domain::chesscom::account_of(&row.username)),
            );
            Ok(own)
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
        self.emit(CoreEvent::ReviewUpdate(Box::new(update)));
    }

    fn status(&self, status: ReviewStatus) {
        self.emit(CoreEvent::ReviewStatus(status));
    }
}

impl ReviewBridge {
    fn emit(&self, event: CoreEvent) {
        self.host.emit(event);
    }
}

/// How long an engine replacement, or closing the core, waits for a stopped analysis to end after
/// its engine exited.
pub(crate) const SETTLE_DEADLINE: Duration = super::uci::EXIT_DEADLINE;

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
            // The engines have exited; the stopped analysis also ends (its final `interrupted`
            // update sent, no longer running) before the executable changes. Waiting only for
            // the processes left it still winding down when a slow CI run published the
            // replacement. Bounded: with its engine gone it ends at once.
            let _ = tokio::time::timeout(SETTLE_DEADLINE, self.analysis.settled()).await;
            Ok(Box::new(hold) as EngineGuard)
        })
    }
}
