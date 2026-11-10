//! Memory retention: each scenario repeats one cycle a frontend runs for hours (a client
//! connecting and leaving, an analysis started and stopped, a computer move, a core opened and
//! closed) and measures the heap the cycles leave behind. A counting global allocator gives the
//! live bytes exactly, so a few hundred bytes kept per cycle show, where RSS would hide them.
//!
//! Each scenario warms up first, so caches that are bounded by design fill before measuring, then
//! samples the live heap after every batch of cycles and fails when the cycles keep more than
//! their allowance. Each scenario prints its trail, which nextest shows for passing runs too
//! (`.config/nextest.toml`), so CI's logs record how close to the allowance every run comes;
//! `cargo test -p kchess-core --test memory -- --nocapture` prints them here.
//! Its own binary: the allocator counts the whole process, so the scenarios run one at a time.

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use kchess_core::connection::ClientInfo;
use kchess_core::{Config, Core, Host, Level};
use serde_json::{Value, json};

/// Counts the bytes live on the heap.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

// SAFETY: every call forwards to the system allocator unchanged; the counter only observes sizes.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded as received.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded as received.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded as received.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded as received.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            LIVE.fetch_add(
                new_size as isize - layout.size() as isize,
                Ordering::Relaxed,
            );
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The scenarios share the counter, so they take turns.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Drops what it is sent, so the host itself keeps nothing (`support::Recording` keeps all).
struct Quiet;

impl Host for Quiet {
    fn log(&self, _level: Level, _scope: &str, _message: &str) {}
    fn emit(&self, _event: &str, _payload: Value) {}
}

/// The live heap once background work has settled: once no more than `tasks` tasks are alive (or
/// after `TASKS_SETTLE`), the lowest of a few samples a moment apart, so a task still finishing
/// its last cycle is not counted as kept. A task that never ends still shows, in the heap and in
/// the task count `measure` reports.
async fn settled(tasks: isize) -> isize {
    let deadline = tokio::time::Instant::now() + TASKS_SETTLE;
    while alive_tasks() > tasks && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut low = isize::MAX;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(40)).await;
        low = low.min(LIVE.load(Ordering::SeqCst));
    }
    low
}

/// How long `settled` waits for the cycles' tasks to end: a task finishing its last cycle is not
/// counted as kept. A closed core leaves none, so the wait normally ends at once. When loaded
/// Windows runners left 12 and then 14 alive after 20 open/close cycles (2026-11-10, read as 964
/// and 972 B kept per cycle), the tasks were not slow but stuck: a battery refresh first polled
/// after close asked the closed host and waited 30 s for its answer (`Capabilities::close`).
const TASKS_SETTLE: Duration = Duration::from_secs(5);

/// Bytes each cycle keeps, judged from `samples` (the settled heap after each batch of
/// `per_batch` cycles, from 0): the smallest batch's growth. Memory the cycles keep grows every
/// batch by about the same; a map or buffer sized by the work grows to its working capacity in a
/// few steps, in whichever batches first need it, and leaves the others flat. The median batch
/// still landed on such a step when three of six batches held one (CI's coverage build).
fn kept_per_cycle(samples: &[isize], per_batch: usize) -> isize {
    let smallest = samples
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .min()
        .unwrap_or(0);
    smallest.max(0) / per_batch.max(1) as isize
}

/// The judging rule against the trails CI recorded and a steady leak (KiB as printed).
#[test]
fn growth_in_steps_is_capacity_and_growth_in_every_batch_is_kept() {
    let trail = |kib: &[f64]| -> Vec<isize> { kib.iter().map(|k| (k * 1024.) as isize).collect() };
    // `a_long_connection_with_calls_in_flight`, CI coverage builds: 33 and 49 B "per cycle" by
    // the median batch, though the heap only stepped up to its working size.
    assert_eq!(
        kept_per_cycle(&trail(&[0.0, 0.0, 2.2, 2.2, 3.5, 5.6, 5.6]), 40),
        0
    );
    assert_eq!(
        kept_per_cycle(&trail(&[0.0, 2.1, 4.3, 4.3, 4.3, 6.2, 6.2]), 40),
        0
    );
    // 432 B a cycle (the idle timer `6b9f89d` fixed) grows every batch of 40 by about 17 KiB.
    let leak: Vec<isize> = (0..7)
        .map(|batch| batch * 432 * 40 + batch % 2 * 300)
        .collect();
    assert!(kept_per_cycle(&leak, 40) >= 420);
}

/// Runs `cycle` `warmup` times, then `batches` × `per_batch` times, sampling the settled heap
/// after each batch. Reports, and fails beyond `allowance` bytes kept per cycle (the smallest
/// batch's growth, `kept_per_cycle`).
async fn measure<F, Fut>(
    name: &str,
    warmup: usize,
    batches: usize,
    per_batch: usize,
    allowance: isize,
    mut cycle: F,
) where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let _turn = TURN.lock().await;
    let mut n = 0;
    for _ in 0..warmup {
        cycle(n).await;
        n += 1;
    }
    let base = settled(isize::MAX).await;
    let tasks = alive_tasks();
    let mut samples = vec![0isize];
    for _ in 0..batches {
        for _ in 0..per_batch {
            cycle(n).await;
            n += 1;
        }
        samples.push(settled(tasks).await - base);
    }
    let cycles = (batches * per_batch) as isize;
    let kept = *samples.last().unwrap_or(&0);
    let per_cycle = kept_per_cycle(&samples, per_batch);
    let trail: Vec<String> = samples.iter().map(|bytes| kib(*bytes)).collect();
    let tasks = alive_tasks() - tasks;
    println!(
        "memory {name}: {cycles} cycles kept {} ({per_cycle} B/cycle, {tasks:+} tasks); after \
         each batch of {per_batch}: {}",
        kib(kept),
        trail.join(" → ")
    );
    // For a heap profiler: exit with everything the cycles left still live, tasks included
    // (`KCHESS_MEMORY_KEEP=1 valgrind --tool=dhat <test binary> <scenario>`).
    if std::env::var_os("KCHESS_MEMORY_KEEP").is_some() {
        std::process::exit(0);
    }
    assert!(
        per_cycle <= allowance,
        "{name} keeps {per_cycle} B per cycle (allowed {allowance}): {}",
        trail.join(" → ")
    );
    // A task still alive `TASKS_SETTLE` after the cycles is held work, whatever it weighs.
    assert!(
        tasks <= 0,
        "{name} left {tasks} tasks running after {cycles} cycles"
    );
}

async fn finish(core: Core) {
    core.close().await;
}

/// The runtime's tasks still alive: a cycle that leaves one behind keeps what it holds.
fn alive_tasks() -> isize {
    isize::try_from(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
    )
    .unwrap_or(0)
}

fn kib(bytes: isize) -> String {
    format!("{:.1} KiB", bytes as f64 / 1024.)
}

fn open(dir: &Path) -> Core {
    Core::open(Config::new(dir.to_path_buf()), Arc::new(Quiet)).expect("the profile opens")
}

/// A core whose engine is the fake engine in `search` mode, with assistance allowed.
async fn with_engine(dir: &Path) -> Core {
    let core = open(dir);
    core.call("loadData", vec![]).await.expect("load");
    core.call("online.resume", vec![]).await.expect("resume");
    let engine = support::engine_copy(dir, "search");
    core.call_sync("engine.trust", vec![json!(engine)])
        .expect("trusted engine");
    let mut settings = core
        .call_sync("store.games.getSettings", vec![])
        .expect("settings");
    settings["enginePath"] = json!(engine);
    core.call_sync("store.games.saveSettings", vec![settings])
        .expect("save settings");
    core
}

/// A client (the CLI, a second window) connects, asks a few things and leaves.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clients_connecting_and_leaving() {
    let dir = tempfile::tempdir().expect("profile");
    let core = open(dir.path());
    core.call("loadData", vec![]).await.expect("load");
    measure("client connect/leave", 50, 5, 200, 64, |_| async {
        let id = core.connect(ClientInfo::app());
        core.focus(id, true);
        let _ = core.call_client(id, "loadData", vec![]).await;
        let _ = core.call_client(id, "viewPreferences", vec![]).await;
        core.disconnect(id);
    })
    .await;
    finish(core).await;
}

/// The analysis page: a position analysed, then the next one, then stopped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn analysis_started_and_stopped() {
    let dir = tempfile::tempdir().expect("profile");
    let core = with_engine(dir.path()).await;
    measure("analysis start/stop", 10, 5, 20, 256, |_| async {
        core.call(
            "startAnalysis",
            vec![json!({"fen": START, "lines": 2, "infinite": true})],
        )
        .await
        .expect("analysis starts");
        tokio::time::sleep(Duration::from_millis(20)).await;
        core.call("stopAnalysis", vec![json!(false)])
            .await
            .expect("analysis stops");
    })
    .await;
    finish(core).await;
}

/// A clients' analysis stopped by its leaving rather than by `stopAnalysis`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn analysing_clients_leaving() {
    let dir = tempfile::tempdir().expect("profile");
    let core = with_engine(dir.path()).await;
    measure("analysing client leaves", 10, 5, 20, 256, |_| async {
        let id = core.connect(ClientInfo::app());
        core.call_client(
            id,
            "startAnalysis",
            vec![json!({"fen": START, "lines": 1, "infinite": true})],
        )
        .await
        .expect("analysis starts");
        tokio::time::sleep(Duration::from_millis(20)).await;
        core.disconnect(id);
    })
    .await;
    finish(core).await;
}

/// A game against the computer: a move asked for, answered, again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computer_moves() {
    let dir = tempfile::tempdir().expect("profile");
    let core = with_engine(dir.path()).await;
    let moves = ["e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6"];
    measure("computer move", 10, 5, 20, 256, |n| {
        let played: Vec<&str> = moves[..n % moves.len()].to_vec();
        let core = &core;
        async move {
            core.call(
                "bestMove",
                vec![json!(played), json!("beginner"), json!({ "movetime": 50 })],
            )
            .await
            .expect("a move");
        }
    })
    .await;
    finish(core).await;
}

/// A profile opened and closed in one process (the service hands a profile over, tests).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cores_opened_and_closed() {
    let dir = tempfile::tempdir().expect("profile");
    measure("core open/close", 3, 4, 5, 512, |_| async {
        let core = open(dir.path());
        core.call("loadData", vec![]).await.expect("load");
        core.close().await;
        drop(core);
    })
    .await;
}
