//! Frame benchmarks of the real app (`--features bench`, `tooling/gpui-render-benchmark.mjs`).
//!
//! `KCHESS_BENCH=<scenario>` drives the window through the pages' own methods (no synthetic
//! input), records GPUI's foreground journal for `KCHESS_BENCH_SECONDS` after a warm-up, writes a
//! JSON summary to `KCHESS_BENCH_OUT`, then quits. Run it on a throwaway profile
//! (`KCHESS_USER_DATA_DIR`).

use std::collections::BTreeMap;
use std::time::Duration;

use gpui_kit::profiler::journal::{ForegroundEvent, ForegroundJournalEntry};
use gpui_kit::profiler::{ViewPhase, set_trace_enabled};
use gpui_kit::*;
use serde_json::{Value, json};

use crate::pages::Page;
use crate::pages::analysis::AnalysisPage;
use crate::pages::local::LocalPage;
use crate::shell;

const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const WARM_UP: Duration = Duration::from_secs(5);
const DRAIN_EVERY: Duration = Duration::from_millis(250);
/// A line for the board to slide through: 24 plies of the Ruy Lopez.
const LINE: [&str; 24] = [
    "e2e4", "e7e5", "g1f3", "b8c6", "f1b5", "a7a6", "b5a4", "g8f6", "e1g1", "f8e7", "f1e1", "b7b5",
    "a4b3", "d7d6", "c2c3", "e8g8", "h2h3", "c6a5", "b3c2", "c7c5", "d2d4", "d8c7", "b1d2", "c5d4",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    /// Home, untouched: the control.
    HomeIdle,
    /// A timed game over the board, its clock running.
    LocalClock,
    /// The analysis board stepping through a line, every move sliding.
    BoardAnimation,
    /// The analysis board with the engine running on one position.
    AnalysisEngine,
    /// The openings grid while the shell redraws ten times a second (its network, account and
    /// update observers).
    OpeningsShell,
}

impl Scenario {
    fn of(name: &str) -> Option<Scenario> {
        Some(match name {
            "home-idle" => Scenario::HomeIdle,
            "local-clock" => Scenario::LocalClock,
            "board-animation" => Scenario::BoardAnimation,
            "analysis-engine" => Scenario::AnalysisEngine,
            "openings-shell" => Scenario::OpeningsShell,
            _ => return None,
        })
    }
}

/// Start the benchmark named by `KCHESS_BENCH` on the main window, if any.
pub fn start(window: AnyWindowHandle, cx: &mut App) {
    let Ok(name) = std::env::var("KCHESS_BENCH") else {
        return;
    };
    let Some(scenario) = Scenario::of(&name) else {
        eprintln!("unknown KCHESS_BENCH scenario: {name}");
        cx.quit();
        return;
    };
    let seconds = std::env::var("KCHESS_BENCH_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);
    let out = std::env::var("KCHESS_BENCH_OUT").unwrap_or_else(|_| "kchess-bench.json".into());
    set_trace_enabled(true);
    cx.spawn(async move |cx| {
        let executor = cx.background_executor().clone();
        let timer = |after: Duration| executor.timer(after);
        let _ = window.update(cx, |_, window, cx| set_up(scenario, window, cx));
        timer(WARM_UP).await;
        let mut collector = cx.update(|cx| cx.foreground_journal().collector());
        let mut totals = Totals::default();
        let cpu_before = cpu_time();
        let started = std::time::Instant::now();
        let length = Duration::from_secs(seconds);
        let mut ply = 0;
        let mut next_step = Duration::ZERO;
        let tick = step_every(scenario).min(DRAIN_EVERY);
        while started.elapsed() < length {
            timer(tick).await;
            let elapsed = started.elapsed();
            if elapsed >= next_step {
                next_step = elapsed + step_every(scenario);
                let _ = window.update(cx, |_, window, cx| step(scenario, &mut ply, window, cx));
            }
            totals.add(collector.collect_unseen().entries);
        }
        totals.add(collector.collect_unseen().entries);
        totals.cpu = cpu_time()
            .zip(cpu_before)
            .map(|(after, before)| after - before);
        let report = totals.report(&name, started.elapsed());
        if let Err(cause) = std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap()) {
            eprintln!("could not write {out}: {cause}");
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

fn step_every(scenario: Scenario) -> Duration {
    match scenario {
        Scenario::BoardAnimation => Duration::from_millis(400),
        Scenario::OpeningsShell => Duration::from_millis(100),
        _ => Duration::from_secs(3600),
    }
}

fn set_up(scenario: Scenario, window: &mut Window, cx: &mut App) {
    match scenario {
        Scenario::HomeIdle => {}
        Scenario::LocalClock => {
            shell::open(Page::Local, window, cx);
            if let Some(local) = shell::view_of::<LocalPage>(Page::Local, cx) {
                local.update(cx, |local, cx| {
                    let clock = json!({
                        "white": { "minutes": 5, "increment": 3 },
                        "black": { "minutes": 5, "increment": 3 },
                    });
                    let setup = json!({ "variant": "standard", "fen": START_FEN });
                    local.replace_game(setup, clock, window, cx);
                    local.play("e2e4", window, cx);
                });
            }
        }
        Scenario::BoardAnimation | Scenario::AnalysisEngine => {
            shell::open(Page::Analysis, window, cx);
            if let Some(analysis) = shell::view_of::<AnalysisPage>(Page::Analysis, cx) {
                let engine = scenario == Scenario::AnalysisEngine;
                // A search that never finishes, so the lines keep streaming for the whole run.
                crate::prefs::set(cx, "kchess:analysis-infinite", engine);
                analysis.update(cx, |analysis, cx| {
                    let line: Vec<String> = LINE.iter().map(|uci| uci.to_string()).collect();
                    analysis.load_line(&line, cx);
                    if AnalysisPage::engine_on(cx) != engine {
                        analysis.toggle_engine(cx);
                    }
                    analysis.go_to_start(cx);
                });
            }
        }
        Scenario::OpeningsShell => shell::open(Page::Openings, window, cx),
    }
}

fn step(scenario: Scenario, ply: &mut usize, _: &mut Window, cx: &mut App) {
    match scenario {
        Scenario::BoardAnimation => {
            if let Some(analysis) = shell::view_of::<AnalysisPage>(Page::Analysis, cx) {
                analysis.update(cx, |analysis, cx| {
                    if *ply == LINE.len() {
                        *ply = 0;
                        analysis.go_to_start(cx);
                    } else {
                        *ply += 1;
                        analysis.forward(cx);
                    }
                });
            }
        }
        Scenario::OpeningsShell => shell::notify_shell(cx),
        _ => {}
    }
}

#[derive(Default)]
struct ViewTotals {
    count: u64,
    request_layout: Duration,
    prepaint: Duration,
    paint: Duration,
}

#[derive(Default)]
struct Totals {
    draws: Vec<Duration>,
    request_layout: Duration,
    prepaint: Duration,
    paint: Duration,
    busy: Duration,
    lost: u64,
    /// The process's CPU time (user and system, every thread) over the measured window.
    cpu: Option<Duration>,
    views: BTreeMap<&'static str, ViewTotals>,
}

impl Totals {
    fn add(&mut self, entries: Vec<ForegroundJournalEntry>) {
        for entry in entries {
            let event = match entry {
                ForegroundJournalEntry::Event(event) => event,
                ForegroundJournalEntry::Discontinuity { lost } => {
                    self.lost += lost;
                    continue;
                }
                _ => continue,
            };
            match event {
                ForegroundEvent::Draw(frame) => {
                    self.draws.push(frame.draw_duration());
                    self.request_layout += frame.phases.request_layout;
                    self.prepaint += frame.phases.prepaint;
                    self.paint += frame.phases.paint;
                    self.busy += event.duration();
                }
                ForegroundEvent::View(view) => {
                    let name = view.name.rsplit("::").next().unwrap_or(view.name);
                    let totals = self.views.entry(name).or_default();
                    match view.phase {
                        ViewPhase::RequestLayout => {
                            totals.count += 1;
                            totals.request_layout += view.self_duration;
                        }
                        ViewPhase::Prepaint => totals.prepaint += view.self_duration,
                        ViewPhase::Paint => totals.paint += view.self_duration,
                    }
                }
                ForegroundEvent::SmallPolls(flush) => self.busy += flush.summary.total,
                _ => self.busy += event.duration(),
            }
        }
    }

    fn report(mut self, scenario: &str, length: Duration) -> Value {
        let seconds = length.as_secs_f64();
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.;
        self.draws.sort();
        let percentile = |p: f64| -> f64 {
            if self.draws.is_empty() {
                return 0.;
            }
            let index = ((self.draws.len() - 1) as f64 * p).round() as usize;
            ms(self.draws[index])
        };
        let draw_total: Duration = self.draws.iter().sum();
        let views: serde_json::Map<String, Value> = self
            .views
            .iter()
            .map(|(name, view)| {
                (
                    name.to_string(),
                    json!({
                        "renders_over_floor_per_s": view.count as f64 / seconds,
                        "request_layout_ms_per_s": ms(view.request_layout) / seconds,
                        "prepaint_ms_per_s": ms(view.prepaint) / seconds,
                        "paint_ms_per_s": ms(view.paint) / seconds,
                    }),
                )
            })
            .collect();
        json!({
            "scenario": scenario,
            "seconds": seconds,
            "draws_per_s": self.draws.len() as f64 / seconds,
            "draw_ms_p50": percentile(0.5),
            "draw_ms_p95": percentile(0.95),
            "draw_ms_max": percentile(1.),
            "draw_ms_per_s": ms(draw_total) / seconds,
            "request_layout_ms_per_s": ms(self.request_layout) / seconds,
            "prepaint_ms_per_s": ms(self.prepaint) / seconds,
            "paint_ms_per_s": ms(self.paint) / seconds,
            "main_thread_busy_fraction": self.busy.as_secs_f64() / seconds,
            "process_cpu_fraction": self.cpu.map(|cpu| cpu.as_secs_f64() / seconds),
            "max_rss_mib": max_rss_mib(),
            "journal_entries_lost": self.lost,
            "views": views,
        })
    }
}

/// This process's CPU time so far, user and system, across its threads.
#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "getrusage has no safe wrapper in std; benchmark builds only"
)]
fn cpu_time() -> Option<Duration> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` fills the struct it is given.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: filled above.
    let usage = unsafe { usage.assume_init() };
    let time = |value: libc::timeval| {
        Duration::from_secs(value.tv_sec as u64) + Duration::from_micros(value.tv_usec as u64)
    };
    Some(time(usage.ru_utime) + time(usage.ru_stime))
}

#[cfg(not(target_os = "macos"))]
fn cpu_time() -> Option<Duration> {
    None
}

/// The process's peak resident memory (macOS reports bytes).
#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "getrusage has no safe wrapper in std; benchmark builds only"
)]
fn max_rss_mib() -> Option<f64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` fills the struct it is given.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: filled above.
    Some(unsafe { usage.assume_init() }.ru_maxrss as f64 / (1024. * 1024.))
}

#[cfg(not(target_os = "macos"))]
fn max_rss_mib() -> Option<f64> {
    None
}
