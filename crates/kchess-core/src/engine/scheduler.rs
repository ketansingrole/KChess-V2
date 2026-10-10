//! The shared engine budget (`crates/kchess-node/js/engineScheduler.ts`): one active lease at a
//! time, higher priorities preempt lower ones, and the thread budget yields on battery power.
//! Computer moves are priority 3, analysis 2, review 1. Each `Scheduler` is an independent
//! budget; a core owns one, and tests create their own.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::uci::{cancelled_or_pending, lock, search_cancelled, signal_aborted};
use crate::error::Result;

type Stop = Arc<dyn Fn() + Send + Sync>;

/// Shared by every clone of a `Scheduler` and by the leases it hands out.
struct Shared {
    budget: usize,
    on_battery: Box<dyn Fn() -> bool + Send + Sync>,
    state: Mutex<State>,
    next_id: AtomicU64,
}

#[derive(Default)]
struct State {
    active: Option<Active>,
    waiting: Vec<Waiting>,
}

struct Active {
    id: u64,
    priority: i32,
    stop: Stop,
}

struct Waiting {
    id: u64,
    priority: i32,
    stop: Stop,
    grant: oneshot::Sender<Lease>,
}

/// The engine budget for a machine with `cpus` logical CPUs: one short of them, within 1..=4.
pub fn default_budget(cpus: usize) -> usize {
    cpus.saturating_sub(1).clamp(1, 4)
}

#[derive(Clone)]
pub struct Scheduler {
    shared: Arc<Shared>,
}

impl Scheduler {
    /// `on_battery` is consulted on every `search_threads` call.
    pub fn new(on_battery: impl Fn() -> bool + Send + Sync + 'static) -> Scheduler {
        let cpus = std::thread::available_parallelism().map_or(1, |count| count.get());
        Scheduler::with_budget(default_budget(cpus), on_battery)
    }

    pub fn with_budget(
        budget: usize,
        on_battery: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Scheduler {
        Scheduler {
            shared: Arc::new(Shared {
                budget: budget.max(1),
                on_battery: Box::new(on_battery),
                state: Mutex::new(State::default()),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// Whether the machine runs on battery power (`configureEngineResources`' battery callback).
    pub fn on_battery(&self) -> bool {
        (self.shared.on_battery)()
    }

    /// `searchThreads`: the full budget, or at most two threads on battery power.
    pub fn search_threads(&self) -> usize {
        if (self.shared.on_battery)() {
            self.shared.budget.min(2)
        } else {
            self.shared.budget
        }
    }

    /// `acquireEngine`: waits for the lease. A higher priority calls the active lease's `stop`
    /// at once. Cancelling `cancel` removes the waiter and reports `search_cancelled()`.
    pub async fn acquire(
        &self,
        priority: i32,
        stop: impl Fn() + Send + Sync + 'static,
        cancel: Option<&CancellationToken>,
    ) -> Result<Lease> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(search_cancelled());
        }
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (grant, mut granted) = oneshot::channel();
        let stop: Stop = Arc::new(stop);
        let preempt = {
            let mut state = lock(&self.shared.state);
            state.waiting.push(Waiting {
                id,
                priority,
                stop,
                grant,
            });
            match &state.active {
                Some(active) if priority > active.priority => Some(Arc::clone(&active.stop)),
                _ => None,
            }
        };
        if let Some(stop) = preempt {
            stop();
        }
        Shared::pump(&self.shared);
        let _queued = Queued {
            shared: &self.shared,
            id,
        };
        tokio::select! {
            biased;
            lease = &mut granted => lease.map_err(|_| search_cancelled()),
            _ = cancelled_or_pending(cancel) => Err(search_cancelled()),
        }
    }

    /// `withEngineLease`: runs `work` while holding the lease. The lease is a guard owned by this
    /// future, so it is released on success, failure, cancellation and panic alike.
    pub async fn with_lease<T>(
        &self,
        priority: i32,
        stop: impl Fn() + Send + Sync + 'static,
        cancel: &CancellationToken,
        work: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let lease = self.acquire(priority, stop, Some(cancel)).await?;
        if cancel.is_cancelled() {
            return Err(signal_aborted());
        }
        let outcome = work.await;
        drop(lease);
        outcome
    }
}

impl Shared {
    /// Grants the next waiter when nothing is active. Never holds the lock while sending.
    fn pump(this: &Arc<Shared>) {
        loop {
            let next = {
                let mut state = lock(&this.state);
                if state.active.is_some() || state.waiting.is_empty() {
                    return;
                }
                state.waiting.sort_by(|a, b| b.priority.cmp(&a.priority));
                let next = state.waiting.remove(0);
                state.active = Some(Active {
                    id: next.id,
                    priority: next.priority,
                    stop: Arc::clone(&next.stop),
                });
                next
            };
            let lease = Lease {
                shared: Arc::clone(this),
                id: next.id,
                armed: true,
            };
            match next.grant.send(lease) {
                Ok(()) => return,
                Err(mut lease) => {
                    // The waiter left before the grant arrived: free the slot and try again.
                    lease.armed = false;
                    let mut state = lock(&this.state);
                    if state
                        .active
                        .as_ref()
                        .is_some_and(|active| active.id == next.id)
                    {
                        state.active = None;
                    }
                }
            }
        }
    }
}

/// Removes a waiter that is still queued when its acquisition ends.
struct Queued<'a> {
    shared: &'a Shared,
    id: u64,
}

impl Drop for Queued<'_> {
    fn drop(&mut self) {
        lock(&self.shared.state)
            .waiting
            .retain(|waiter| waiter.id != self.id);
    }
}

/// Exclusive use of the engine budget. Dropping it releases the budget to the next waiter.
#[must_use = "dropping the lease releases the engine budget"]
pub struct Lease {
    shared: Arc<Shared>,
    id: u64,
    armed: bool,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        {
            let mut state = lock(&self.shared.state);
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.id == self.id)
            {
                state.active = None;
            }
        }
        Shared::pump(&self.shared);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_leaves_one_cpu_and_stays_within_bounds() {
        assert_eq!(default_budget(1), 1);
        assert_eq!(default_budget(2), 1);
        assert_eq!(default_budget(6), 4);
        assert_eq!(default_budget(64), 4);
        assert_eq!(default_budget(0), 1);
    }

    #[test]
    fn battery_limits_search_threads_to_two() {
        assert_eq!(Scheduler::with_budget(4, || true).search_threads(), 2);
        assert_eq!(Scheduler::with_budget(1, || true).search_threads(), 1);
        assert_eq!(Scheduler::with_budget(4, || false).search_threads(), 4);
    }
}
