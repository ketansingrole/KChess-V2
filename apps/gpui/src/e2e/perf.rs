//! How long the app takes to start and to open each page over a full profile: its maximum of
//! played games (500) and 50 studies, which the app reads to start and pages read to resume.
//! Headless, so it times the main thread's work (reads, views, layout), not the GPU's. Each page
//! is also scrolled, a wheel step at a time.
//!
//! Not part of `check`; it prints the figures:
//! `cargo test --release -p kchess-gpui e2e::perf -- --ignored --nocapture`, and
//! `KCHESS_PERF_PAGES=Studies,…` to open only those pages (to profile them).

use std::time::{Duration, Instant};

use gpui_kit::{AppContext as _, TestAppContext};
use serde_json::{Value, json};
use shakmaty::uci::UciMove;
use shakmaty::{CastlingMode, Chess, Position};

use super::{Harness, draw};
use crate::core::AppCore;
use crate::pages::Page;

/// xorshift64: the same games on every run.
fn random_game(seed: &mut u64, plies: usize) -> (Vec<String>, String) {
    let mut pos = Chess::default();
    let (mut moves, mut pgn) = (Vec::new(), String::new());
    for ply in 0..plies {
        let legal = pos.legal_moves();
        if legal.is_empty() {
            break;
        }
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        let m = legal[(*seed % legal.len() as u64) as usize];
        if ply % 2 == 0 {
            pgn.push_str(&format!("{}. ", ply / 2 + 1));
        }
        pgn.push_str(&format!("{} ", shakmaty::san::San::from_move(&pos, m)));
        moves.push(UciMove::from_move(m, CastlingMode::Standard).to_string());
        pos.play_unchecked(m);
    }
    pgn.push('*');
    (moves, pgn)
}

fn call(app: &mut Harness, method: &str, args: Vec<Value>) {
    app.cx.update(|cx| {
        cx.global::<AppCore>()
            .bridge
            .call_now(method, args)
            .unwrap_or_else(|cause| panic!("{method}: {}", cause.message));
    });
}

fn fill(app: &mut Harness) {
    let mut seed = 0x9E37_79B9_7F4A_7C15;
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    for n in 0..500 {
        let (moves, _) = random_game(&mut seed, 80);
        let game = json!({
            "id": format!("game-{n}"), "source": "computer", "startedAt": n, "updatedAt": n,
            "white": "You", "black": "Stockfish", "result": "*", "reason": "Unfinished",
            "timeControl": "-", "finished": false,
            "setup": { "variant": "standard", "fen": start }, "moves": moves,
        });
        call(app, "saveArchivedGame", vec![game]);
    }
    for n in 0..50 {
        let (_, pgn) = random_game(&mut seed, 60);
        let command = json!({ "op": "save", "name": format!("Study {n}"), "pgn": pgn });
        call(app, "studyCommand", vec![command]);
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Starts as `Harness::boot` does, timing the core's start, the models and window, and the
/// window's first frame.
fn timed_start(cx: TestAppContext, profile: tempfile::TempDir) -> (Harness, [f64; 3]) {
    let mut cx = cx;
    crate::profile::TEST_DATA_DIR.set(Some(profile.path().to_path_buf()));
    cx.executor().allow_parking();
    let dir = profile.path().to_path_buf();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::menu::init(cx);
        crate::notifications::init(cx);
        cx.set_reduce_motion(true);
    });
    let started = Instant::now();
    loop {
        match cx.update(|cx| crate::core::start(dir.clone(), cx)) {
            Ok(()) => break,
            Err(cause) if cause.contains("already running") => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(cause) => panic!("the core opens the profile: {cause:?}"),
        }
    }
    let core = started.elapsed();
    let started = Instant::now();
    cx.update(crate::startup::start);
    let models = started.elapsed();
    let window = cx
        .update(|cx| cx.windows().first().copied())
        .expect("the main window opened");
    let started = Instant::now();
    cx.update_window(window, |_, window, cx| draw(window, cx))
        .expect("the window draws");
    let frame = started.elapsed();
    cx.run_until_parked();
    let mut app = Harness {
        cx,
        window,
        profile: Some(profile),
        owns_app: true,
    };
    app.online();
    app.find("settings");
    (app, [ms(core), ms(models), ms(frame)])
}

/// Time `shell::open` of `page` and the frame that shows it, once the page before has settled.
fn open(app: &mut Harness, page: Page) -> f64 {
    app.settle();
    opening(app, page)
}

/// The opening alone: never inlined, so a profile can pick out its samples.
#[inline(never)]
fn opening(app: &mut Harness, page: Page) -> f64 {
    let started = Instant::now();
    app.cx
        .update_window(app.window, |_, window, cx| {
            crate::shell::open(page, window, cx);
            draw(window, cx);
        })
        .expect("the window is open");
    ms(started.elapsed())
}

/// Six steps of the wheel down the open page, each timed with the frame that shows it.
fn scroll(app: &mut Harness) -> Vec<f64> {
    app.settle();
    (0..6).map(|_| scrolling(app)).collect()
}

#[inline(never)]
fn scrolling(app: &mut Harness) -> f64 {
    use gpui_kit::InputEvent as _;
    let started = Instant::now();
    app.cx
        .update_window(app.window, |_, window, cx| {
            // Over the page: the sidebar is at the window's left.
            let size = window.viewport_size();
            let wheel = gpui_kit::ScrollEvent {
                position: gpui_kit::point(size.width * 0.6, size.height * 0.5),
                delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-300.),
                )),
                ..Default::default()
            };
            window.dispatch_event(wheel.to_platform_input(), cx);
            draw(window, cx);
        })
        .expect("the window is open");
    ms(started.elapsed())
}

#[gpui_kit::test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn start_and_open_every_page_over_a_full_profile(cx: &mut TestAppContext) {
    let mut app = Harness::start(cx);
    fill(&mut app);
    let mut starts: Vec<[f64; 3]> = Vec::new();
    let mut pages: Vec<(Page, &'static str)> = crate::shell::nav()
        .iter()
        .map(|(page, label, _)| (*page, *label))
        .collect();
    pages.push((Page::Settings, "Settings"));
    // `KCHESS_PERF_PAGES=Studies,Game history`: those pages alone, to profile their openings.
    if let Ok(only) = std::env::var("KCHESS_PERF_PAGES") {
        pages.retain(|(_, label)| only.split(',').any(|name| name == *label));
    }
    let mut first: Vec<Vec<f64>> = vec![Vec::new(); pages.len()];
    let mut again: Vec<Vec<f64>> = vec![Vec::new(); pages.len()];
    let mut scrolls: Vec<Vec<f64>> = vec![Vec::new(); pages.len()];
    for _ in 0..5 {
        let profile = app.profile.take().expect("the profile");
        let next = app.cx.new_app();
        drop(app);
        let (started, times) = timed_start(next, profile);
        app = started;
        starts.push(times);
        for (n, (page, _)) in pages.iter().enumerate() {
            first[n].push(open(&mut app, *page));
        }
        for (n, (page, _)) in pages.iter().enumerate() {
            again[n].push(open(&mut app, *page));
            scrolls[n].extend(scroll(&mut app));
        }
    }
    let phase = |n: usize| median(starts.iter().map(|times| times[n]).collect());
    println!(
        "start: core {:.1} ms, models and window {:.1} ms, first frame {:.1} ms",
        phase(0),
        phase(1),
        phase(2)
    );
    for (n, (_, label)) in pages.iter().enumerate() {
        println!(
            "open {label}: first {:.1} ms, again {:.1} ms, scroll step {:.1} ms",
            median(first[n].clone()),
            median(again[n].clone()),
            median(scrolls[n].clone())
        );
    }
}
