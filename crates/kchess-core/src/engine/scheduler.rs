//! The shared engine budget: one active lease at a time, higher priorities preempt lower ones,
//! and the thread budget yields on battery power. Each `Scheduler` is an independent budget; a
//! core owns one, and tests create their own.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::uci::search_cancelled;
use crate::tasks::{aborted as signal_aborted, lock};
use kchess_contracts::error::Result;

/// Who holds the engine budget. A higher priority preempts a lower one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Review,
    Analysis,
    Computer,
}

/// Whether the machine runs on battery power. `refresh` runs before every lease and must not
/// wait for the answer: the lease uses the latest reading.
pub trait Power: Send + Sync {
    fn on_battery(&self) -> bool;
    fn refresh(&self) {}
}

impl<F: Fn() -> bool + Send + Sync> Power for F {
    fn on_battery(&self) -> bool {
        self()
    }
}

/// One review engine per this much of the computer's memory: an engine takes about 160 MiB
/// while it searches, so the engines of a review stay under a tenth of the memory.
const ENGINE_MEMORY: u64 = 2 << 30;

/// Shared by every clone of a `Scheduler` and by the leases it hands out.
struct Shared {
    budget: usize,
    /// The computer's memory in bytes, when it is known.
    memory: Option<u64>,
    power: Box<dyn Power>,
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
    priority: Priority,
    stop: CancellationToken,
}

struct Waiting {
    id: u64,
    priority: Priority,
    stop: CancellationToken,
    grant: oneshot::Sender<Lease>,
}

/// The engine budget for a machine with `cpus` logical CPUs: one short of them, within 1..=4.
pub fn default_budget(cpus: usize) -> usize {
    cpus.saturating_sub(1).clamp(1, 4)
}

/// How many engines review a game at once on `threads` search threads: one for each, but no
/// more than one per `ENGINE_MEMORY` of `memory`, and at least one. A computer whose memory is
/// not known gets one for each thread.
pub fn engines_within(threads: usize, memory: Option<u64>) -> usize {
    let room = memory.map_or(threads, |bytes| {
        usize::try_from(bytes / ENGINE_MEMORY).unwrap_or(threads)
    });
    threads.min(room).max(1)
}

#[derive(Clone)]
pub struct Scheduler {
    shared: Arc<Shared>,
}

impl Scheduler {
    /// `power` is consulted on every `search_threads` call.
    pub fn new(power: impl Power + 'static) -> Scheduler {
        let cpus = std::thread::available_parallelism().map_or(1, |count| count.get());
        Scheduler::with_memory(default_budget(cpus), crate::memory::physical(), power)
    }

    /// A budget on a computer whose memory is not known.
    pub fn with_budget(budget: usize, power: impl Power + 'static) -> Scheduler {
        Scheduler::with_memory(budget, None, power)
    }

    /// A budget on a computer with `memory` bytes of memory.
    pub fn with_memory(
        budget: usize,
        memory: Option<u64>,
        power: impl Power + 'static,
    ) -> Scheduler {
        Scheduler {
            shared: Arc::new(Shared {
                budget: budget.max(1),
                memory,
                power: Box::new(power),
                state: Mutex::new(State::default()),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// Whether the machine runs on battery power.
    pub fn on_battery(&self) -> bool {
        self.shared.power.on_battery()
    }

    /// The full budget, or at most two threads on battery power.
    pub fn search_threads(&self) -> usize {
        if self.shared.power.on_battery() {
            self.shared.budget.min(2)
        } else {
            self.shared.budget
        }
    }

    /// How many engines review a game someone asked for at once: the search threads' worth,
    /// within what the memory has room for (`engines_within`).
    pub fn review_engines(&self) -> usize {
        engines_within(self.search_threads(), self.shared.memory)
    }

    /// Waits for the lease. A higher priority cancels the active lease's `token` at once, which
    /// stops its search. Cancelling `token` while waiting removes the waiter and reports
    /// `search_cancelled()`.
    pub async fn acquire(&self, priority: Priority, token: &CancellationToken) -> Result<Lease> {
        if token.is_cancelled() {
            return Err(search_cancelled());
        }
        self.shared.power.refresh();
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (grant, mut granted) = oneshot::channel();
        let preempt = {
            let mut state = lock(&self.shared.state);
            state.waiting.push(Waiting {
                id,
                priority,
                stop: token.clone(),
                grant,
            });
            match &state.active {
                Some(active) if priority > active.priority => Some(active.stop.clone()),
                _ => None,
            }
        };
        if let Some(stop) = preempt {
            stop.cancel();
        }
        Shared::pump(&self.shared);
        let _queued = Queued {
            shared: &self.shared,
            id,
        };
        tokio::select! {
            biased;
            lease = &mut granted => lease.map_err(|_| search_cancelled()),
            () = token.cancelled() => Err(search_cancelled()),
        }
    }

    /// Runs `work` while holding the lease. The lease is a guard owned by this future, so it is
    /// released on success, failure, cancellation and panic alike.
    pub async fn with_lease<T>(
        &self,
        priority: Priority,
        token: &CancellationToken,
        work: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let lease = self.acquire(priority, token).await?;
        if token.is_cancelled() {
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
                state.waiting.sort_by_key(|a| std::cmp::Reverse(a.priority));
                let next = state.waiting.remove(0);
                state.active = Some(Active {
                    id: next.id,
                    priority: next.priority,
                    stop: next.stop.clone(),
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

    #[test]
    fn a_review_gets_an_engine_per_thread_that_the_memory_has_room_for() {
        const GIB: u64 = 1 << 30;
        assert_eq!(engines_within(4, None), 4);
        assert_eq!(engines_within(4, Some(64 * GIB)), 4);
        assert_eq!(engines_within(4, Some(8 * GIB)), 4);
        assert_eq!(engines_within(4, Some(6 * GIB)), 3);
        assert_eq!(engines_within(4, Some(4 * GIB)), 2);
        assert_eq!(engines_within(4, Some(4 * GIB - 1)), 1);
        assert_eq!(engines_within(4, Some(GIB)), 1);
        assert_eq!(engines_within(2, Some(64 * GIB)), 2);
        assert_eq!(engines_within(1, None), 1);
    }

    #[test]
    fn a_scheduler_counts_review_engines_by_its_threads_and_memory() {
        const GIB: u64 = 1 << 30;
        let engines = |memory, battery: bool| {
            Scheduler::with_memory(4, memory, move || battery).review_engines()
        };
        assert_eq!(engines(None, false), 4);
        assert_eq!(engines(Some(6 * GIB), false), 3);
        assert_eq!(engines(Some(64 * GIB), true), 2);
        assert_eq!(Scheduler::with_budget(3, || false).review_engines(), 3);
    }

    /// Only a higher priority stops the active lease; an equal one waits its turn.
    #[tokio::test]
    async fn only_a_higher_priority_preempts() {
        let scheduler = Scheduler::with_budget(1, || false);
        let first = CancellationToken::new();
        let lease = scheduler.acquire(Priority::Analysis, &first).await.unwrap();
        let (same, higher) = (CancellationToken::new(), CancellationToken::new());
        let waiting = {
            let scheduler = scheduler.clone();
            let same = same.clone();
            tokio::spawn(
                async move { scheduler.acquire(Priority::Analysis, &same).await.map(drop) },
            )
        };
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        assert!(!first.is_cancelled(), "an equal priority preempted");
        let computer = {
            let scheduler = scheduler.clone();
            let higher = higher.clone();
            tokio::spawn(async move {
                scheduler
                    .acquire(Priority::Computer, &higher)
                    .await
                    .map(drop)
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), first.cancelled())
            .await
            .expect("a higher priority preempts");
        drop(lease);
        computer.await.unwrap().unwrap();
        waiting.await.unwrap().unwrap();
    }
}
