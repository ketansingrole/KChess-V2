//! Broadcast alerts: the tournaments and rounds the player asked to be told about when a round
//! starts. The core keeps them (`broadcasts:alerts` in the library) and watches Lichess for them
//! (`AlertWatch`): each alerted tournament is looked at every minute around a chosen round's
//! start and every 15 minutes otherwise, less often while Lichess cannot be reached, and a round
//! that goes live is announced once (`broadcast:round-started`). An alert ends once nothing it
//! waits for is left.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kchess_contracts::CoreEvent;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::{
    BroadcastAlert, BroadcastRoundRef, BroadcastRoundStarted, BroadcastTourDetail, ProfileArea,
    ProfileChange,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::host::{Host, Level};
use crate::lichess::accounts::Lichess;
use crate::store::Database;
use crate::store::library;
use crate::time::now_ms;

/// The origin of the watch's own writes: no client made them, so every window reads them again.
const NOBODY: u64 = u64::MAX;

/// How often a tournament is looked at around a chosen round's start: from `NEAR` before it
/// until it goes live, for up to `LATE` after it was due.
pub const SOON: Duration = Duration::from_secs(60);
/// How long before a round's start the watch looks every minute.
pub const NEAR: Duration = Duration::from_secs(10 * 60);
/// How long a round may run late before the watch looks only every `LATER`.
pub const LATE: Duration = Duration::from_secs(3 * 60 * 60);
/// The longest wait between two looks at a tournament with something still to come.
pub const LATER: Duration = Duration::from_secs(15 * 60);
/// The first wait after Lichess could not be reached; each failure doubles it, up to `LATER * 2`.
pub const RETRY: Duration = Duration::from_secs(60);

/// The most alerts kept.
pub const MAX_ALERTS: usize = 100;

/// What a look at a tournament found for one alert.
#[derive(Debug, PartialEq)]
pub struct Look {
    /// The rounds that went live since the last look, to announce.
    pub started: Vec<BroadcastRoundStarted>,
    /// The alert as it is now; none once nothing it waits for is left.
    pub alert: Option<BroadcastAlert>,
    /// How long until the next look.
    pub next: Duration,
}

fn chosen(alert: &BroadcastAlert, round: &str) -> bool {
    alert.round_id.as_deref().is_none_or(|id| id == round)
}

/// What `detail`, the tournament as Lichess has it at `now` (ms), means for `alert`.
pub fn look(alert: &BroadcastAlert, detail: &BroadcastTourDetail, now: i64) -> Look {
    let mut alert = alert.clone();
    alert.tour_name = detail.name.clone();
    let mut started = Vec::new();
    let wanted = alert.clone();
    for round in detail
        .rounds
        .iter()
        .filter(|round| chosen(&wanted, &round.id))
    {
        if alert.round_id.is_some() {
            alert.round_name = Some(round.name.clone());
        }
        if round.ongoing && !alert.told.contains(&round.id) {
            alert.told.push(round.id.clone());
            started.push(BroadcastRoundStarted {
                tour_id: detail.id.clone(),
                tour_name: detail.name.clone(),
                round_id: round.id.clone(),
                round_name: round.name.clone(),
            });
        }
    }
    // Only rounds the tournament still has are remembered.
    alert
        .told
        .retain(|id| detail.rounds.iter().any(|round| &round.id == id));
    let waiting: Vec<_> = detail
        .rounds
        .iter()
        .filter(|round| chosen(&alert, &round.id))
        .filter(|round| !round.finished && !alert.told.contains(&round.id))
        .collect();
    let ended = match &alert.round_id {
        // A round's alert ends once it was told or the round ended (or left the tournament).
        Some(_) => waiting.is_empty(),
        // A tournament's ends once every round has ended; Lichess may add rounds until then.
        None => !detail.rounds.is_empty() && detail.rounds.iter().all(|round| round.finished),
    };
    let next = waiting
        .iter()
        .filter_map(|round| round.starts_at)
        .map(|at| wait_for(at, now))
        .min()
        .unwrap_or(LATER);
    Look {
        started,
        alert: (!ended).then_some(alert),
        next,
    }
}

/// How long to wait before looking again for a round due at `at` (ms), at `now`.
fn wait_for(at: i64, now: i64) -> Duration {
    let until = at - now;
    let near = NEAR.as_millis() as i64;
    if until > near {
        return Duration::from_millis((until - near) as u64).min(LATER);
    }
    if -until <= LATE.as_millis() as i64 {
        SOON
    } else {
        LATER
    }
}

/// The wait after `failures` looks in a row Lichess did not answer.
pub fn retry_after(failures: u32) -> Duration {
    RETRY
        .saturating_mul(1 << failures.saturating_sub(1).min(5))
        .min(LATER * 2)
}

/// Whether a round is under way or over when an alert is set: it is not announced.
fn begun(round: &BroadcastRoundRef) -> bool {
    round.ongoing || round.finished
}

/// `alerts` with the player's change: alerted (`on`) or not to `tour`'s rounds as they start,
/// every round or `round` alone. `detail` is the tournament as Lichess has it now, needed to
/// turn one on, or one round of a whole tournament's off. A tournament's alert covers its rounds'.
pub fn set(
    alerts: &mut Vec<BroadcastAlert>,
    tour: &str,
    round: Option<&str>,
    on: bool,
    detail: Option<&BroadcastTourDetail>,
) -> Result<()> {
    let whole = |alert: &BroadcastAlert| alert.tour_id == tour && alert.round_id.is_none();
    let covered = alerts.iter().any(whole);
    let needed = || CoreError::new("Lichess did not send the broadcast.");
    match (on, round) {
        (true, None) => {
            let detail = detail.ok_or_else(needed)?;
            alerts.retain(|alert| alert.tour_id != tour);
            alerts.push(BroadcastAlert {
                tour_id: tour.to_string(),
                tour_name: detail.name.clone(),
                round_id: None,
                round_name: None,
                told: detail
                    .rounds
                    .iter()
                    .filter(|round| begun(round))
                    .map(|round| round.id.clone())
                    .collect(),
            });
        }
        (true, Some(id)) => {
            if covered
                || alerts
                    .iter()
                    .any(|alert| alert.tour_id == tour && alert.round_id.as_deref() == Some(id))
            {
                return Ok(());
            }
            let detail = detail.ok_or_else(needed)?;
            let found = detail
                .rounds
                .iter()
                .find(|round| round.id == id)
                .ok_or_else(|| CoreError::new("This round is no longer in the broadcast."))?;
            alerts.push(BroadcastAlert {
                tour_id: tour.to_string(),
                tour_name: detail.name.clone(),
                round_id: Some(id.to_string()),
                round_name: Some(found.name.clone()),
                told: begun(found).then(|| id.to_string()).into_iter().collect(),
            });
        }
        (false, None) => alerts.retain(|alert| alert.tour_id != tour),
        (false, Some(id)) => {
            alerts
                .retain(|alert| !(alert.tour_id == tour && alert.round_id.as_deref() == Some(id)));
            if covered {
                // Every round but this one: the tournament's alert becomes theirs.
                let detail = detail.ok_or_else(needed)?;
                let told = alerts
                    .iter()
                    .find(|alert| whole(alert))
                    .map(|alert| alert.told.clone())
                    .unwrap_or_default();
                alerts.retain(|alert| !whole(alert));
                alerts.extend(
                    detail
                        .rounds
                        .iter()
                        .filter(|round| {
                            round.id != id && !begun(round) && !told.contains(&round.id)
                        })
                        .map(|round| BroadcastAlert {
                            tour_id: tour.to_string(),
                            tour_name: detail.name.clone(),
                            round_id: Some(round.id.clone()),
                            round_name: Some(round.name.clone()),
                            told: Vec::new(),
                        }),
                );
            }
        }
    }
    if alerts.len() > MAX_ALERTS {
        return Err(CoreError::new(
            "Too many broadcast alerts. Remove one first.",
        ));
    }
    Ok(())
}

/// The watch over the alerted tournaments: one task for the core's life, woken when the alerts
/// change, looking at each tournament when it is due (`look`).
#[derive(Default)]
pub struct AlertWatch {
    wake: Arc<Notify>,
    started: AtomicBool,
    /// Tests only: the next pass looks at every tournament now, whenever it was due.
    now: Arc<AtomicBool>,
    /// Tests only: the passes asked for (`look_now`), and the last one begun after an ask and
    /// finished, so a test waits for a pass that saw what it set up.
    asked: Arc<AtomicU64>,
    served: Arc<AtomicU64>,
}

/// What the watch reads and tells.
pub struct Watching {
    pub database: Arc<Database>,
    pub lichess: Arc<Lichess>,
    pub host: Arc<dyn Host>,
    pub lifetime: CancellationToken,
}

impl AlertWatch {
    /// The alerts changed: look again.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Starts the watch once, on `tracker` so the core's close waits for it.
    pub fn start(&self, tracker: &TaskTracker, watching: Watching) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let signals = Signals {
            wake: Arc::clone(&self.wake),
            now: Arc::clone(&self.now),
            asked: Arc::clone(&self.asked),
            served: Arc::clone(&self.served),
        };
        tracker.spawn(async move { run(watching, signals).await });
    }

    /// Tests only: look at every alerted tournament now. Answers the pass's number, which
    /// `served` reaches once a pass begun after this ask has finished.
    pub fn look_now(&self) -> u64 {
        let ticket = self.asked.fetch_add(1, Ordering::SeqCst) + 1;
        self.now.store(true, Ordering::SeqCst);
        self.wake.notify_one();
        ticket
    }

    /// Tests only: the last pass asked for that has finished.
    pub fn served(&self) -> u64 {
        self.served.load(Ordering::SeqCst)
    }
}

/// How the watch is woken, and how it tells the tests what it has done.
struct Signals {
    wake: Arc<Notify>,
    now: Arc<AtomicBool>,
    asked: Arc<AtomicU64>,
    served: Arc<AtomicU64>,
}

async fn run(watching: Watching, signals: Signals) {
    let Watching {
        database,
        lichess,
        host,
        lifetime,
    } = watching;
    let Signals {
        wake,
        now,
        asked,
        served,
    } = signals;
    let mut due: HashMap<String, Instant> = HashMap::new();
    let mut failures: HashMap<String, u32> = HashMap::new();
    loop {
        let serving = asked.load(Ordering::SeqCst);
        if now.swap(false, Ordering::SeqCst) {
            due.clear();
        }
        let tours = match database
            .read_async(|ctx| library::broadcast_alerts(ctx.host, ctx.db))
            .await
        {
            Ok(alerts) => {
                let mut tours: Vec<String> =
                    alerts.into_iter().map(|alert| alert.tour_id).collect();
                tours.sort();
                tours.dedup();
                tours
            }
            Err(cause) => {
                host.log(
                    Level::Warn,
                    "broadcasts",
                    &format!("Broadcast alerts unreadable: {}", cause.message),
                );
                Vec::new()
            }
        };
        due.retain(|tour, _| tours.contains(tour));
        failures.retain(|tour, _| tours.contains(tour));
        for tour in &tours {
            if lifetime.is_cancelled() {
                return;
            }
            if due.get(tour).is_some_and(|at| *at > Instant::now()) {
                continue;
            }
            let next = match super::broadcast_tour(&lichess, tour).await {
                Ok(detail) => {
                    failures.remove(tour);
                    let at = now_ms();
                    let looked = database
                        .write_async(move |ctx| {
                            library::look_at_broadcast(ctx.host, ctx.db, &detail, at)
                        })
                        .await;
                    match looked {
                        Ok(seen) => {
                            for started in seen.started {
                                host.emit(CoreEvent::BroadcastRoundStarted(started));
                            }
                            if seen.changed {
                                host.emit(CoreEvent::ProfileChanged(ProfileChange {
                                    areas: vec![ProfileArea::Broadcasts],
                                    origin: NOBODY,
                                }));
                            }
                            seen.next
                        }
                        Err(cause) => {
                            host.log(
                                Level::Warn,
                                "broadcasts",
                                &format!("Broadcast alerts not saved: {}", cause.message),
                            );
                            RETRY
                        }
                    }
                }
                Err(cause) => {
                    let failed = failures.entry(tour.clone()).or_default();
                    *failed += 1;
                    if !cause.aborted {
                        host.log(
                            Level::Warn,
                            "broadcasts",
                            &format!(
                                "Broadcast unavailable for its alert: {tour} {}",
                                cause.message
                            ),
                        );
                    }
                    retry_after(*failed)
                }
            };
            due.insert(tour.clone(), Instant::now() + next);
        }
        served.store(serving, Ordering::SeqCst);
        let wait = due
            .values()
            .min()
            .map(|at| at.saturating_duration_since(Instant::now()));
        tokio::select! {
            () = lifetime.cancelled() => return,
            () = wake.notified() => {}
            () = async {
                match wait {
                    Some(wait) => tokio::time::sleep(wait).await,
                    None => std::future::pending().await,
                }
            } => {}
        }
    }
}
