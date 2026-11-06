//! Memory retention over the socket: a window that stays connected for hours and makes call after
//! call, and windows that connect and leave. A counting global allocator gives the live heap
//! exactly; each scenario warms up, then fails when its cycles keep more than their allowance.
//! `cargo test -p kchess-service --test memory -- --nocapture` prints the figures. Its own binary,
//! since the allocator counts the whole process; the scenarios take turns.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use kchess_core::Config;
use kchess_service::server::{ServeOptions, serve};
use kchess_service::{Capability, Client, ClientEvent, ConnectError, Hello};
use tokio::sync::mpsc::UnboundedReceiver;

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

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const WAIT: Duration = Duration::from_secs(10);

fn window() -> Hello {
    Hello {
        kind: "gpui".into(),
        capabilities: vec![Capability::Browser, Capability::Focus],
    }
}

async fn connect(profile: &Path) -> (Client, UnboundedReceiver<ClientEvent>) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        match Client::connect(profile, &window()).await {
            Ok(connected) => return connected,
            Err(ConnectError::NotRunning) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => panic!("could not connect: {error}"),
        }
    }
}

/// The settled live heap: the lowest of a few samples a moment apart.
async fn settled() -> isize {
    let mut low = isize::MAX;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(40)).await;
        low = low.min(LIVE.load(Ordering::SeqCst));
    }
    low
}

/// Runs `cycle` `warmup` times, then `batches` × `per_batch` times, sampling the settled heap
/// after each batch; fails beyond `allowance` bytes kept per cycle (the lower quartile of the
/// batches' growth).
async fn measure<F, Fut>(
    name: &str,
    warmup: usize,
    batches: usize,
    per_batch: usize,
    allowance: isize,
    mut cycle: F,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    for _ in 0..warmup {
        cycle().await;
    }
    let base = settled().await;
    let mut samples = vec![0isize];
    for _ in 0..batches {
        for _ in 0..per_batch {
            cycle().await;
        }
        samples.push(settled().await - base);
    }
    let cycles = (batches * per_batch) as isize;
    let kept = *samples.last().unwrap_or(&0);
    // Judged by the lower quartile of a batch's growth: memory the cycles keep grows by about the
    // same in every batch, while a map or buffer sized by the work grows to its working capacity
    // in steps, in whichever batches first need it (seen under CI's coverage build). Under load
    // the steps (2 KiB each) came in half the batches, enough to move the median: Windows and
    // Linux CI failed `a_long_connection_with_calls_in_flight` at 42-55 B a cycle with growth
    // 0.0 → 0.0 → 4.1 → 6.2 → 6.2 → 10.2 → 10.2 KiB, flat once at capacity.
    let mut growth: Vec<isize> = samples.windows(2).map(|pair| pair[1] - pair[0]).collect();
    growth.sort_unstable();
    let per_cycle = growth.get(growth.len() / 4).copied().unwrap_or(0) / per_batch as isize;
    let trail: Vec<String> = samples
        .iter()
        .map(|bytes| format!("{:.1} KiB", *bytes as f64 / 1024.))
        .collect();
    println!(
        "memory {name}: {cycles} cycles kept {:.1} KiB ({per_cycle} B/cycle); after each batch \
         of {per_batch}: {}",
        kept as f64 / 1024.,
        trail.join(" → ")
    );
    assert!(
        per_cycle <= allowance,
        "{name} keeps {per_cycle} B per cycle (allowed {allowance}): {}",
        trail.join(" → ")
    );
}

fn spawn_service(profile: &Path) -> tokio::task::JoinHandle<Result<(), String>> {
    tokio::spawn(serve(ServeOptions {
        config: Config::new(profile.to_path_buf()),
        idle: Duration::from_secs(600),
    }))
}

/// One window, connected all along, calling the core again and again: each call's bookkeeping
/// must go when its answer does, not when the window disconnects.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_connection_making_calls() {
    let _turn = TURN.lock().await;
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path());
    let (client, _events) = connect(profile.path()).await;
    client.call("loadData", vec![]).await.expect("load");
    measure("calls on one connection", 200, 5, 1000, 32, || async {
        let _ = client.call("viewPreferences", vec![]).await;
    })
    .await;
    drop(client);
    service.abort();
}

/// The same with many calls in flight at once, as a window opening a page sends them: workers
/// finish calls while the connection is still reading the next ones.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_long_connection_with_calls_in_flight() {
    let _turn = TURN.lock().await;
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path());
    let (client, _events) = connect(profile.path()).await;
    client.call("loadData", vec![]).await.expect("load");
    measure(
        "calls in flight on one connection",
        60,
        10,
        40,
        32,
        || async {
            let calls: Vec<_> = (0..64)
                .map(|_| tokio::spawn(client.call("viewPreferences", vec![])))
                .collect();
            for call in calls {
                let _ = call.await;
            }
        },
    )
    .await;
    drop(client);
    service.abort();
}

/// Windows connecting, asking once and leaving.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn windows_connecting_and_leaving() {
    let _turn = TURN.lock().await;
    let profile = tempfile::tempdir().expect("profile");
    let service = spawn_service(profile.path());
    drop(connect(profile.path()).await);
    measure("window connect/leave", 30, 5, 40, 256, || async {
        let (client, events) = connect(profile.path()).await;
        let _ = client.call("viewPreferences", vec![]).await;
        drop(client);
        drop(events);
    })
    .await;
    service.abort();
}
