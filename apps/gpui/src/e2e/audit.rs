//! Every page's controls at the minimum, a mid and the default window size: each has an accessible
//! name, lies inside the window across (a page scrolls only down), and overlaps no other control.
//! No control is under 24 px (WCAG 2.5.8).

use std::fmt::Write as _;

use gpui_kit::test::{ElementSnapshot, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{Role, px, size};

use super::{Harness, SIZES};
use crate::pages::Page;

const CONTROLS: [Role; 7] = [
    Role::Button,
    Role::Link,
    Role::Tab,
    Role::TextInput,
    Role::ComboBox,
    Role::Switch,
    Role::CheckBox,
];

#[derive(Default)]
struct Findings {
    problems: Vec<String>,
    small: Vec<String>,
}

#[gpui_kit::test]
fn every_page_lays_out_its_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let mut findings = Findings::default();
    let mut pages: Vec<(Page, &str)> = crate::shell::nav()
        .iter()
        .map(|(page, label, _)| (*page, *label))
        .collect();
    pages.push((Page::Settings, "Settings"));
    for (width, height) in SIZES {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(height)));
        for &(page, label) in &pages {
            app.with_window(|window, cx| crate::shell::open(page, window, cx));
            for _ in 0..20 {
                app.tick();
            }
            let controls = app.with_window(|window, _| window.elements());
            check(
                &format!("{label} at {width}×{height}"),
                width,
                &controls,
                &mut findings,
            );
        }
    }
    let mut report = String::new();
    for problem in &findings.problems {
        let _ = writeln!(report, "  {problem}");
    }
    assert!(
        findings.problems.is_empty(),
        "controls out of place:\n{report}"
    );
    assert!(
        findings.small.is_empty(),
        "controls under 24 px:\n  {}",
        findings.small.join("\n  ")
    );
}

#[gpui_kit::test]
fn the_move_box_is_one_size_on_every_board_page(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let (width, height) = SIZES[2];
    app.cx
        .simulate_window_resize(app.window, size(px(width), px(height)));
    let mut sizes = Vec::new();
    for page in [Page::Computer, Page::Analysis] {
        app.with_window(|window, cx| crate::shell::open(page, window, cx));
        for _ in 0..20 {
            app.tick();
        }
        sizes.push(app.with_window(|window, _| {
            window
                .try_find_by_label("Enter a chess move in SAN or UCI")
                .expect("the page has a move box")
                .bounds()
                .size
        }));
    }
    assert_eq!(sizes[0], sizes[1]);
}

#[gpui_kit::test]
fn a_voice_error_is_read_in_full_under_the_board(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let (width, height) = SIZES[2];
    app.cx
        .simulate_window_resize(app.window, size(px(width), px(height)));
    app.with_window(|window, cx| crate::shell::open(Page::Computer, window, cx));
    app.click_label("Voice input");
    let mut line = None;
    app.wait_for("the voice error", |window, _| {
        line = window.elements().into_iter().find(|element| {
            element.role() == Some(Role::Status)
                && element.label().is_some_and(|label| label.contains("voice"))
        });
        window.try_find_by_label("Try again").is_some() && line.is_some()
    });
    // In the move box's row it had what the box and the buttons left: a word or two.
    let line = line.expect("found");
    assert!(
        line.bounds().size.width > px(400.),
        "{:?} is {:?} wide",
        line.label(),
        line.bounds().size.width
    );
}

fn check(page: &str, width: f32, elements: &[ElementSnapshot], findings: &mut Findings) {
    let controls: Vec<&ElementSnapshot> = elements
        .iter()
        .filter(|e| e.visible() && e.role().is_some_and(|role| CONTROLS.contains(&role)))
        .collect();
    for (index, control) in controls.iter().enumerate() {
        let name = describe(control);
        let bounds = control.bounds();
        if control.label().is_none_or(str::is_empty) {
            findings
                .problems
                .push(format!("{page}: {name} has no name"));
        }
        if f32::from(bounds.left()) < -0.5 || f32::from(bounds.right()) > width + 0.5 {
            findings.problems.push(format!(
                "{page}: {name} is cut off at the side ({bounds:?})"
            ));
        }
        // A text input's accessible element is its editor's text line, not the box around it
        // (GPUI Kit 0.9 marks the frame presentational), so only its width says how big it is.
        let reach = if control.role() == Some(Role::TextInput) {
            bounds.size.width
        } else {
            bounds.size.width.min(bounds.size.height)
        };
        if f32::from(reach) < 24. {
            findings
                .small
                .push(format!("{page}: {name} {:?}", bounds.size));
        }
        for other in &controls[index + 1..] {
            let (a, b) = (bounds, other.bounds());
            let across = f32::from(a.right().min(b.right()) - a.left().max(b.left()));
            let down = f32::from(a.bottom().min(b.bottom()) - a.top().max(b.top()));
            let nested = contains(a, b) || contains(b, a);
            if across > 1. && down > 1. && !nested {
                findings
                    .problems
                    .push(format!("{page}: {name} overlaps {}", describe(other)));
            }
        }
    }
}

fn contains(
    outer: gpui_kit::Bounds<gpui_kit::Pixels>,
    inner: gpui_kit::Bounds<gpui_kit::Pixels>,
) -> bool {
    outer.left() <= inner.left()
        && outer.top() <= inner.top()
        && outer.right() >= inner.right()
        && outer.bottom() >= inner.bottom()
}

fn describe(control: &ElementSnapshot) -> String {
    match control.label() {
        Some(label) => format!("{:?} {label:?}", control.role()),
        None => format!("{:?} {:?}", control.role(), control.path().last()),
    }
}

/// The board editor's settings scroll under its actions, which stay inside the panel at every size
/// (they were cut off at its foot in a laptop-sized window).
#[gpui_kit::test]
fn the_board_editors_actions_stay_in_its_panel(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.with_window(|window, cx| crate::shell::open(Page::Editor, window, cx));
    for (width, height) in SIZES {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(height)));
        for _ in 0..20 {
            app.tick();
        }
        let bounds = |app: &mut Harness, id: &'static str| {
            app.with_window(|window, _| window.try_find(id).map(|found| found.bounds()))
                .unwrap_or_else(|| panic!("no {id}"))
        };
        let panel = bounds(&mut app, "editor-panel");
        for action in ["analyse-position", "play-stockfish", "play-friend"] {
            let button = bounds(&mut app, action);
            assert!(
                button.bottom() <= panel.bottom() && button.top() >= panel.top(),
                "{action} at {width}x{height}: {button:?} outside the panel {panel:?}"
            );
        }
    }
}

/// The same audit with something on every list: played games of both kinds (one a variant), a
/// puzzle attempt and a study. Empty pages hide the rows whose columns crowd a narrow window.
#[gpui_kit::test]
fn every_page_with_content_lays_out_its_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    seed(&mut app);
    let mut findings = Findings::default();
    let mut pages: Vec<(Page, &str)> = crate::shell::nav()
        .iter()
        .map(|(page, label, _)| (*page, *label))
        .collect();
    pages.push((Page::Settings, "Settings"));
    for (width, height) in SIZES {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(height)));
        for &(page, label) in &pages {
            app.with_window(|window, cx| crate::shell::open(page, window, cx));
            for _ in 0..20 {
                app.tick();
            }
            let controls = app.with_window(|window, _| window.elements());
            check(
                &format!("{label} at {width}×{height}"),
                width,
                &controls,
                &mut findings,
            );
        }
    }
    assert!(
        findings.problems.is_empty() && findings.small.is_empty(),
        "with content:\n  {}",
        findings
            .problems
            .iter()
            .chain(&findings.small)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// Every tab of every page and every settings section, not only those a page opens on: the chess
/// clock's time fields once had no names, behind Over the board's second tab.
#[gpui_kit::test]
fn every_tab_and_settings_section_lays_out_its_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let mut findings = Findings::default();
    let settle = |app: &mut Harness| {
        for _ in 0..20 {
            app.tick();
        }
    };
    for (width, height) in SIZES {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(height)));
        for (page, label, _) in crate::shell::nav().iter() {
            let (page, label) = (*page, *label);
            app.with_window(|window, cx| crate::shell::open(page, window, cx));
            settle(&mut app);
            let tabs = app.with_window(|window, _| {
                window
                    .elements()
                    .into_iter()
                    .filter(|e| e.visible() && e.role() == Some(Role::Tab))
                    .filter_map(|e| Some((e.path().last()?.clone(), e.label()?.to_string())))
                    .collect::<Vec<_>>()
            });
            for (id, tab) in tabs {
                let shown = app.with_window(|window, cx| {
                    let shown = window.try_find(id.clone()).is_some();
                    if shown {
                        window.click(id.clone(), cx);
                    }
                    shown
                });
                if !shown {
                    continue;
                }
                settle(&mut app);
                let controls = app.with_window(|window, _| window.elements());
                check(
                    &format!("{label} → {tab} at {width}×{height}"),
                    width,
                    &controls,
                    &mut findings,
                );
            }
        }
        for (section, label, _) in crate::pages::settings::sections() {
            app.with_window(|window, cx| crate::shell::open_settings(section, window, cx));
            settle(&mut app);
            let controls = app.with_window(|window, _| window.elements());
            check(
                &format!("Settings → {label} at {width}×{height}"),
                width,
                &controls,
                &mut findings,
            );
        }
    }
    findings.problems.sort();
    findings.problems.dedup();
    findings.small.sort();
    findings.small.dedup();
    assert!(
        findings.problems.is_empty() && findings.small.is_empty(),
        "behind a tab:\n  {}",
        findings
            .problems
            .iter()
            .chain(&findings.small)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// Games, a puzzle attempt and a study, kept by the core before the pages are opened.
fn seed(app: &mut Harness) {
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    app.cx.update(|cx| {
        let played = cx.global::<crate::played::PlayedHandle>().0.clone();
        played.update(cx, |played, cx| {
            for (index, (variant, result)) in [
                ("standard", "1-0"),
                ("atomic", "0-1"),
                ("standard", "1/2-1/2"),
            ]
            .into_iter()
            .enumerate()
            {
                played.save(
                    serde_json::json!({
                        "id": format!("audit-{index}"),
                        "source": "computer",
                        "setup": { "variant": variant, "fen": start },
                        "moves": ["e2e4", "e7e5", "d1h5", "b8c6", "f1c4", "g8f6", "h5f7"],
                        "white": "You",
                        "black": "Stockfish · Club Player",
                        "result": result,
                        "reason": "checkmate",
                        "finished": true,
                        "timeControl": "300+3",
                        "startedAt": 1_790_000_000_000u64 + index as u64 * 90_000_000,
                        "updatedAt": 1_790_000_000_000u64 + index as u64 * 90_000_000,
                    }),
                    cx,
                );
            }
        });
    });
    let day = crate::game::puzzle::local_day();
    let calls = [
        (
            "recordPuzzleAttempt",
            serde_json::json!({
                "puzzle": {
                    "id": "aaaaa",
                    "fen": "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 2 4",
                    "solution": ["f3f7", "e8f7"],
                    "rating": 1500,
                    "themes": ["mateIn1", "short", "opening"],
                },
                "mode": "practice", "retry": false, "win": false, "revealed": false,
                "hinted": false, "found": 1, "wrong": 1, "total": 2, "day": day,
            }),
        ),
        (
            "studyCommand",
            serde_json::json!({
                "op": "saveChapters",
                "name": "A study with a fairly long name to wrap",
                "chapters": [
                    { "name": "King pawn", "pgn": "1. e4 e5 *" },
                    { "name": "Queen pawn", "pgn": "1. d4 d5 *" },
                ],
            }),
        ),
    ];
    for (method, argument) in calls {
        let done = app.cx.update(|cx| {
            cx.global::<crate::core::AppCore>()
                .call(method, vec![argument])
        });
        app.cx
            .foreground_executor()
            .block_test(done)
            .unwrap_or_else(|cause| panic!("{method}: {cause:?}"));
    }
}
