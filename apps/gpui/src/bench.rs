//! Frame benchmarks of the real app (`--features bench`).
//!
//! `KCHESS_BENCH=<scenario>` drives the window through the pages' own methods (no synthetic
//! input), records GPUI's foreground journal for `KCHESS_BENCH_SECONDS` after a warm-up, writes a
//! JSON summary to `KCHESS_BENCH_OUT`, then quits. A run whose `visible_fraction` is below 1 had
//! its window covered for a while and is not comparable. Run it on a throwaway profile
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
    /// The analysis board stepping through a line with the engine on, as a game is reviewed.
    AnalysisStepping,
    /// The openings grid while the shell redraws ten times a second (its network, account and
    /// update observers).
    OpeningsShell,
    /// Signed in to Lichess (`cargo xtask bench`'s stand-in), on Play on Lichess.
    LichessIdle,
    /// A live Lichess game, both sides moving every 1.5 s, clocks running.
    LichessGame,
    /// Lichess TV, its featured game moving every 1.5 s.
    LichessTv,
    /// Every page of the sidebar and Settings, signed in, a new one every second: what opening
    /// each costs (`page_open_ms`).
    PagesTour,
}

impl Scenario {
    fn of(name: &str) -> Option<Scenario> {
        Some(match name {
            "home-idle" => Scenario::HomeIdle,
            "local-clock" => Scenario::LocalClock,
            "board-animation" => Scenario::BoardAnimation,
            "analysis-engine" => Scenario::AnalysisEngine,
            "analysis-stepping" => Scenario::AnalysisStepping,
            "openings-shell" => Scenario::OpeningsShell,
            "lichess-idle" => Scenario::LichessIdle,
            "lichess-game" => Scenario::LichessGame,
            "lichess-tv" => Scenario::LichessTv,
            "pages-tour" => Scenario::PagesTour,
            _ => return None,
        })
    }
}

/// Start the benchmark named by `KCHESS_BENCH` on the main window, if any.
pub fn start(window: AnyWindowHandle, cx: &mut App) {
    let Ok(name) = std::env::var("KCHESS_BENCH") else {
        return;
    };
    if name == "lichess-login" {
        log_in_and_quit(window, cx);
        return;
    }
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
        // `pages-tour` opens every page once before measuring, each first opening on its own:
        // the run then opens pages already made, as most openings are.
        let mut first_opens = BTreeMap::new();
        if scenario == Scenario::PagesTour {
            for (page, label) in tour() {
                let _ = collector.collect_unseen();
                let opened = std::time::Instant::now();
                let _ = window.update(cx, |_, window, cx| shell::open(page, window, cx));
                timer(step_every(scenario)).await;
                let mut first = Totals {
                    opening: Some((label, opened)),
                    ..Totals::default()
                };
                first.add(collector.collect_unseen().entries);
                if let Some(open) = first.opens.get(label).and_then(|opens| opens.first()) {
                    first_opens.insert(label, open.as_secs_f64() * 1000.);
                }
            }
        }
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
                // What was drawn before the step belongs to the page before it.
                totals.add(collector.collect_unseen().entries);
                let stepped = std::time::Instant::now();
                let _ = window.update(cx, |_, window, cx| step(scenario, &mut ply, window, cx));
                if scenario == Scenario::PagesTour {
                    totals.opening = Some((tour()[(ply - 1) % tour().len()].1, stepped));
                }
            }
            totals.add(collector.collect_unseen().entries);
            totals.checks += 1;
            if window
                .update(cx, |_, window, _| window.is_visible())
                .unwrap_or(false)
            {
                totals.visible += 1;
            }
        }
        totals.add(collector.collect_unseen().entries);
        totals.cpu = cpu_time()
            .zip(cpu_before)
            .map(|(after, before)| after - before);
        let mut report = totals.report(&name, started.elapsed());
        // What the window showed at the end, so a run that never reached its page is seen.
        cx.update(|cx| {
            report["page"] = json!(shell::current_page(cx).map(|page| format!("{page:?}")));
            report["account"] = json!(crate::accounts::handle(cx).read(cx).active(cx));
            report["online_moves"] = json!(crate::online::handle(cx).read(cx).moves().len());
            if !first_opens.is_empty() {
                report["first_open_ms"] = json!(first_opens);
            }
        });
        if let Err(cause) = std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap()) {
            eprintln!("could not write {out}: {cause}");
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

fn step_every(scenario: Scenario) -> Duration {
    match scenario {
        Scenario::BoardAnimation | Scenario::AnalysisStepping => Duration::from_millis(400),
        Scenario::OpeningsShell => Duration::from_millis(100),
        Scenario::PagesTour => Duration::from_secs(1),
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
        Scenario::BoardAnimation | Scenario::AnalysisEngine | Scenario::AnalysisStepping => {
            shell::open(Page::Analysis, window, cx);
            if let Some(analysis) = shell::view_of::<AnalysisPage>(Page::Analysis, cx) {
                let engine = scenario != Scenario::BoardAnimation;
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
        // Signed in by an earlier launch (`lichess-login`), as someone who plays there is.
        Scenario::LichessIdle | Scenario::LichessGame => shell::open(Page::Online, window, cx),
        Scenario::LichessTv => shell::open(Page::Watch, window, cx),
        Scenario::PagesTour => {}
    }
}

/// `lichess-login`: sign in, then quit once the account is connected. `cargo xtask bench` runs
/// it before a `lichess-*` scenario on the same profile, which then starts signed in.
fn log_in_and_quit(window: AnyWindowHandle, cx: &mut App) {
    let _ = window.update(cx, |_, window, cx| sign_in(window, cx));
    cx.spawn(async move |cx| {
        for _ in 0..400 {
            if cx.update(|cx| crate::accounts::handle(cx).read(cx).active(cx).is_some()) {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Sign in with Connect, as a person would: the core opens Lichess's login page, which the
/// stand-in's app writes to `opened-urls.txt` (the core's test switch), and the "browser" here
/// returns to the core's callback with a code.
fn sign_in(window: &mut Window, cx: &mut App) {
    let Ok(dir) = crate::profile::data_dir() else {
        return;
    };
    crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.connect(window, cx));
    std::thread::spawn(move || {
        let opened = dir.join("opened-urls.txt");
        for _ in 0..200 {
            if let Some(url) = std::fs::read_to_string(&opened).ok().and_then(|text| {
                text.lines()
                    .find(|line| line.contains("/oauth"))
                    .map(str::to_string)
            }) {
                return_from_login(&url);
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        eprintln!("the login page never opened");
    });
}

/// GET the login URL's `redirect_uri` with its `state` and a code.
fn return_from_login(url: &str) {
    use std::io::{Read as _, Write as _};
    let query = url.split_once('?').map_or("", |(_, query)| query);
    let param = |name: &str| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| decode(value))
        })
    };
    let (Some(redirect), Some(state)) = (param("redirect_uri"), param("state")) else {
        return;
    };
    let rest = redirect.trim_start_matches("http://");
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let Ok(mut socket) = std::net::TcpStream::connect(host) else {
        return;
    };
    let _ = write!(
        socket,
        "GET /{path}?code=bench&state={state} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    );
    let _ = socket.read_to_end(&mut Vec::new());
}

/// A query value's percent-encoding undone.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'%' if at + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        at += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The pages `pages-tour` opens in turn: the sidebar's, then Settings.
fn tour() -> Vec<(Page, &'static str)> {
    let mut pages: Vec<(Page, &'static str)> = shell::nav()
        .iter()
        .map(|(page, label, _)| (*page, *label))
        .collect();
    pages.push((Page::Settings, "Settings"));
    // `KCHESS_BENCH_TOUR=Openings,Home`: those pages alone, in that order, so a page's first
    // opening is measured without the pages before it.
    if let Ok(only) = std::env::var("KCHESS_BENCH_TOUR") {
        let all = pages;
        pages = only
            .split(',')
            .filter_map(|label| all.iter().find(|(_, name)| *name == label).copied())
            .collect();
    }
    pages
}

fn step(scenario: Scenario, ply: &mut usize, window: &mut Window, cx: &mut App) {
    match scenario {
        Scenario::BoardAnimation | Scenario::AnalysisStepping => {
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
        Scenario::PagesTour => {
            let pages = tour();
            shell::open(pages[*ply % pages.len()].0, window, cx);
            *ply += 1;
        }
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
    /// From each frame's first invalidation to the end of its draw: how long a change takes to
    /// be drawn.
    latencies: Vec<Duration>,
    request_layout: Duration,
    prepaint: Duration,
    paint: Duration,
    busy: Duration,
    lost: u64,
    /// How many of the run's checks found the window shown on screen: a covered or minimized
    /// window draws no frames, and its run is not comparable.
    checks: u64,
    visible: u64,
    /// The process's CPU time (user and system, every thread) over the measured window.
    cpu: Option<Duration>,
    views: BTreeMap<&'static str, ViewTotals>,
    /// The page just opened (`pages-tour`) and when, whose first frame is its opening's.
    opening: Option<(&'static str, std::time::Instant)>,
    /// Each page's openings: from the open, making the page if it is new, to the end of the
    /// frame that drew it.
    opens: BTreeMap<&'static str, Vec<Duration>>,
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
                    self.latencies.extend(frame.dirty_to_draw_duration());
                    if let Some((page, opened)) = self.opening.take() {
                        let open = frame.draw_end.saturating_duration_since(opened);
                        self.opens.entry(page).or_default().push(open);
                    }
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
        self.latencies.sort();
        let latency = |p: f64| -> f64 {
            if self.latencies.is_empty() {
                return 0.;
            }
            ms(self.latencies[((self.latencies.len() - 1) as f64 * p).round() as usize])
        };
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
        let mut opens: Vec<Duration> = self.opens.values().flatten().copied().collect();
        opens.sort();
        let open = |p: f64| -> Option<f64> {
            (!opens.is_empty()).then(|| ms(opens[((opens.len() - 1) as f64 * p).round() as usize]))
        };
        let page_open_ms: serde_json::Map<String, Value> = self
            .opens
            .iter_mut()
            .map(|(page, times)| {
                times.sort();
                (page.to_string(), json!(ms(times[times.len() / 2])))
            })
            .collect();
        json!({
            "scenario": scenario,
            "page_open_ms_p50": open(0.5),
            "page_open_ms_p95": open(0.95),
            "page_open_ms": page_open_ms,
            "seconds": seconds,
            "draws_per_s": self.draws.len() as f64 / seconds,
            "draw_ms_p50": percentile(0.5),
            "draw_ms_p95": percentile(0.95),
            "draw_ms_max": percentile(1.),
            "draw_ms_per_s": ms(draw_total) / seconds,
            "latency_ms_p50": latency(0.5),
            "latency_ms_p95": latency(0.95),
            "request_layout_ms_per_s": ms(self.request_layout) / seconds,
            "prepaint_ms_per_s": ms(self.prepaint) / seconds,
            "paint_ms_per_s": ms(self.paint) / seconds,
            "main_thread_busy_fraction": self.busy.as_secs_f64() / seconds,
            "process_cpu_fraction": self.cpu.map(|cpu| cpu.as_secs_f64() / seconds),
            "max_rss_mib": max_rss_mib(),
            "journal_entries_lost": self.lost,
            "visible_fraction": self.visible as f64 / self.checks.max(1) as f64,
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
