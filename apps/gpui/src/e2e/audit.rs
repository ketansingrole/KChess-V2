//! Every page's controls at the minimum, a mid and the default window size: each has an accessible
//! name, lies inside the window across (a page scrolls only down), and overlaps no other control.
//! No control is under 24 px (WCAG 2.5.8). A button or select in a row with a bar of pills is the
//! bar's height: History's Export was 24 px beside a 30 px bar.

use std::fmt::Write as _;

use gpui_kit::test::{ElementSnapshot, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{Role, px, size};

use super::{Harness, SIZES};
use crate::pages::Page;

const CONTROLS: [Role; 15] = [
    Role::Button,
    Role::Link,
    Role::Tab,
    Role::TextInput,
    Role::PasswordInput,
    Role::MultilineTextInput,
    Role::SearchInput,
    Role::ComboBox,
    Role::Switch,
    Role::CheckBox,
    Role::RadioButton,
    Role::Slider,
    Role::SpinButton,
    Role::MenuItem,
    Role::ListBoxOption,
];

/// The fields among them: text a person types.
const FIELDS: [Role; 4] = [
    Role::TextInput,
    Role::PasswordInput,
    Role::MultilineTextInput,
    Role::SearchInput,
];

/// The controls a person can reach: while Settings' dialog is open, only its own, as the page
/// under its overlay takes no clicks. A notice's own Close (GPUI Kit's toast, which shows it
/// only while the pointer is over the notice, and hides the notice by itself) is not the page's.
fn in_front(elements: Vec<ElementSnapshot>) -> Vec<ElementSnapshot> {
    let toasts = gpui_kit::ElementId::from("notification-list");
    let in_toast = |e: &ElementSnapshot| {
        e.path().iter().any(|id| match id {
            gpui_kit::ElementId::NamedInteger(name, _) => {
                gpui_kit::ElementId::Name(name.clone()) == toasts
            }
            _ => false,
        })
    };
    let elements = elements.into_iter().filter(|e| !in_toast(e));
    let dialog = gpui_kit::ElementId::from("settings-dialog");
    let elements: Vec<ElementSnapshot> = elements.collect();
    if !elements.iter().any(|e| e.path().contains(&dialog)) {
        return elements;
    }
    elements
        .into_iter()
        .filter(|e| e.path().contains(&dialog))
        .collect()
}

#[derive(Default)]
struct Findings {
    problems: Vec<String>,
    small: Vec<String>,
}

/// Every page at each of the audit's window sizes, as it opens.
fn every_page(app: &mut Harness) -> Findings {
    let mut findings = Findings::default();
    let pages: Vec<(Page, &'static str)> =
        Page::all().map(|page| (page, page.info().label)).collect();
    for (width, height) in SIZES {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(height)));
        for &(page, label) in &pages {
            app.with_window(|window, cx| crate::shell::open(page, window, cx));
            app.settle();
            let controls = app.with_window(|window, _| in_front(window.elements()));
            check(
                &format!("{label} at {width}×{height}"),
                width,
                &controls,
                &mut findings,
            );
        }
    }
    findings
}

#[gpui_kit::test]
fn every_page_lays_out_its_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let findings = every_page(&mut app);
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
        app.settle();
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

/// Before the system had asked, Voice input's Allow microphone pushed Privacy settings past a
/// 640 px window's side (cut off at x 485 + 179 px); the audit saw it only on a Mac that had never
/// been asked, as it read the machine's own permission.
#[gpui_kit::test]
fn the_microphone_access_buttons_stay_in_the_minimum_window_before_the_system_asks(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = Harness::start(cx);
    app.set_microphone(crate::microphone::Host {
        access: crate::microphone::Access::NotDetermined,
        from_terminal: false,
        privacy_settings: crate::microphone::Host::TEST.privacy_settings,
    });
    let (width, height) = SIZES[0];
    app.cx
        .simulate_window_resize(app.window, size(px(width), px(height)));
    app.with_window(|window, cx| {
        crate::shell::open_settings(crate::pages::settings::Section::Voice, window, cx)
    });
    app.settle();
    let controls = app.with_window(|window, _| in_front(window.elements()));
    for label in ["Allow microphone", "Privacy settings"] {
        assert!(
            controls
                .iter()
                .any(|e| e.visible() && e.label() == Some(label)),
            "{label} is shown"
        );
    }
    let mut findings = Findings::default();
    check("Settings → Voice input", width, &controls, &mut findings);
    assert!(
        findings.problems.is_empty(),
        "{}",
        findings.problems.join("\n")
    );
}

/// With the microphone allowed, Privacy settings opened a Windows page that lists no switch of
/// KChess's own, so players looked there for KChess and found nothing. Blocked, it stays.
#[gpui_kit::test]
fn privacy_settings_shows_only_while_the_microphone_is_not_allowed(
    cx: &mut gpui_kit::TestAppContext,
) {
    use crate::microphone::{Access, Host};
    let mut app = Harness::start(cx);
    for (access, shown) in [(Access::Granted, false), (Access::Denied, true)] {
        app.set_microphone(Host {
            access,
            from_terminal: false,
            privacy_settings: Host::TEST.privacy_settings,
        });
        app.with_window(|window, cx| {
            crate::shell::open_settings(crate::pages::settings::Section::Voice, window, cx)
        });
        app.settle();
        let controls = app.with_window(|window, _| in_front(window.elements()));
        assert_eq!(
            controls
                .iter()
                .any(|e| e.visible() && e.label() == Some("Privacy settings")),
            shown,
            "Privacy settings with the microphone {access:?}"
        );
    }
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
        // A field's accessible element is its editor's text line, not the box around it (GPUI
        // Kit 0.9 marks the frame presentational), so only its width says how big it is.
        let reach = if control.role().is_some_and(|role| FIELDS.contains(&role)) {
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
    check_rows(page, elements, findings);
}

/// Buttons and selects in a row with a bar of pills: centred on the bar's middle and near it.
/// Each is the bar's height. A field is left out: its element is its text line, not its box.
fn check_rows(page: &str, elements: &[ElementSnapshot], findings: &mut Findings) {
    let shown = |role: Role| {
        elements
            .iter()
            .filter(move |e| e.visible() && e.role() == Some(role))
    };
    for bar in shown(Role::TabList) {
        let row = bar.bounds();
        for control in shown(Role::Button).chain(shown(Role::ComboBox)) {
            let bounds = control.bounds();
            let middle = f32::from(bounds.center().y - row.center().y).abs();
            let apart = f32::from((bounds.left() - row.right()).max(row.left() - bounds.right()));
            if contains(row, bounds) || middle > 2. || apart > 320. {
                continue;
            }
            if f32::from(bounds.size.height - row.size.height).abs() > 0.5 {
                findings.problems.push(format!(
                    "{page}: {} is {:?} tall beside a bar of pills {:?} tall",
                    describe(control),
                    bounds.size.height,
                    row.size.height
                ));
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

/// A page's own tabs are as wide as their labels, at the page's start: stretched across a wide
/// window, Game history's three tabs were 320 px each, their labels far apart.
#[gpui_kit::test]
fn a_pages_tabs_are_as_wide_as_their_labels(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let (width, height) = SIZES[SIZES.len() - 1];
    app.cx
        .simulate_window_resize(app.window, size(px(width), px(height)));
    for (page, bar) in [
        ("Game history", "history-source"),
        ("Over the board", "local-mode"),
        ("Openings", "openings-group"),
    ] {
        app.open(page);
        app.settle();
        let bounds = app.find(bar).bounds();
        assert!(
            bounds.size.width < px(width / 2.),
            "{page}'s {bar} is {:?} wide in a {width} px window",
            bounds.size.width
        );
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
        app.settle();
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
    let findings = every_page(&mut app);
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

/// The same audit signed in to a Lichess account with ratings, a rating history and synced
/// games: what only an account shows (Home's ratings, Play on Lichess, Studies' Lichess tab,
/// Insights) was never looked at.
#[gpui_kit::test]
fn every_page_signed_in_lays_out_its_controls(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in_showcase(cx);
    let findings = every_page(&mut app);
    assert!(
        findings.problems.is_empty() && findings.small.is_empty(),
        "signed in:\n  {}",
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
/// clock's time fields once had no names, behind Over the board's second tab. One test per window
/// size, so they run side by side: in one they were the app's slowest test (46 s), which every
/// run of the suite waited for.
#[gpui_kit::test]
fn every_tab_and_settings_section_lays_out_its_controls_at_the_minimum_size(
    cx: &mut gpui_kit::TestAppContext,
) {
    every_tab_and_settings_section_lays_out_its_controls(cx, SIZES[0]);
}

#[gpui_kit::test]
fn every_tab_and_settings_section_lays_out_its_controls_at_a_mid_size(
    cx: &mut gpui_kit::TestAppContext,
) {
    every_tab_and_settings_section_lays_out_its_controls(cx, SIZES[1]);
}

#[gpui_kit::test]
fn every_tab_and_settings_section_lays_out_its_controls_at_the_default_size(
    cx: &mut gpui_kit::TestAppContext,
) {
    every_tab_and_settings_section_lays_out_its_controls(cx, SIZES[2]);
}

fn every_tab_and_settings_section_lays_out_its_controls(
    cx: &mut gpui_kit::TestAppContext,
    (width, height): (f32, f32),
) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    let mut findings = Findings::default();
    let settle = |app: &mut Harness| {
        app.settle();
    };
    {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(height)));
        for page in Page::in_sidebar() {
            let label = page.info().label;
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
                let controls = app.with_window(|window, _| in_front(window.elements()));
                check(
                    &format!("{label} → {tab} at {width}×{height}"),
                    width,
                    &controls,
                    &mut findings,
                );
            }
        }
        for (section, label) in crate::pages::settings::Section::ALL.map(|s| (s, s.label())) {
            let states = if section == crate::pages::settings::Section::Voice {
                microphone_states()
            } else {
                vec![(crate::microphone::Host::TEST, String::new())]
            };
            for (host, state) in states {
                app.set_microphone(host);
                app.with_window(|window, cx| crate::shell::open_settings(section, window, cx));
                settle(&mut app);
                let controls = app.with_window(|window, _| in_front(window.elements()));
                check(
                    &format!("Settings → {label}{state} at {width}×{height}"),
                    width,
                    &controls,
                    &mut findings,
                );
            }
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

/// Every state Settings → Voice input shows the microphone in, on every machine: the one a test
/// ran on once decided which was checked, and Allow microphone pushed Privacy settings past a
/// 640 px window's side only while macOS had not asked yet, which no CI runner reports.
fn microphone_states() -> Vec<(crate::microphone::Host, String)> {
    use crate::microphone::{Access, Host};
    let mut states = Vec::new();
    for access in [
        Access::NotDetermined,
        Access::Denied,
        Access::Granted,
        Access::Restricted,
        Access::Unknown,
    ] {
        for from_terminal in [false, true] {
            for privacy_settings in [Host::TEST.privacy_settings, None] {
                let host = Host {
                    access,
                    from_terminal,
                    privacy_settings,
                };
                let state = format!(
                    " (microphone {access:?}{}{})",
                    if from_terminal {
                        ", from a terminal"
                    } else {
                        ""
                    },
                    if privacy_settings.is_none() {
                        ", no privacy settings"
                    } else {
                        ""
                    },
                );
                states.push((host, state));
            }
        }
    }
    states
}

/// The dialogs a fresh profile opens: the computer's game options, a new game over the board,
/// the import dialogs and the shortcut list, each opened in the largest window and then checked at
/// every size while it stays open.
#[gpui_kit::test]
fn every_dialog_lays_out_its_controls(cx: &mut gpui_kit::TestAppContext) {
    // Signed in, for the dialogs only an account opens.
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    let mut findings = Findings::default();
    let settle = |app: &mut Harness| {
        app.settle();
    };
    let click = |id: &'static str| move |app: &mut Harness| app.click(id);
    /// What opens a dialog, once its page is shown.
    type Opener = Box<dyn Fn(&mut Harness)>;
    let dialogs: [(Page, &str, Opener); 11] = [
        (
            Page::Computer,
            "Game options",
            Box::new(click("game-options")),
        ),
        (
            Page::Local,
            "New game over the board",
            Box::new(click("local-new")),
        ),
        (
            Page::Analysis,
            "Import into the analysis board",
            Box::new(click("analysis-import")),
        ),
        (
            Page::Analysis,
            "Save as study",
            Box::new(click("save-as-study")),
        ),
        (
            Page::Analysis,
            "Game details",
            Box::new(|app: &mut Harness| {
                app.click("study-actions");
                app.click_label("Game details…");
            }),
        ),
        (
            Page::Studies,
            "Import a study",
            Box::new(click("empty-import")),
        ),
        (
            Page::Dashboard,
            "Keyboard shortcuts",
            Box::new(|app: &mut Harness| app.press("?")),
        ),
        (
            Page::Dashboard,
            "What's new",
            Box::new(|app: &mut Harness| app.with_window(crate::whats_new::open)),
        ),
        (
            Page::Friends,
            "Import from Lichess",
            Box::new(click("friends-import")),
        ),
        (
            Page::Tournaments,
            "Create an arena",
            Box::new(click("tournaments-create")),
        ),
        (
            Page::Players,
            "Message a player",
            Box::new(|app: &mut Harness| {
                app.cx.update(|cx| {
                    let page = crate::shell::view::<crate::pages::players::PlayersPage>(cx)
                        .expect("the Players page");
                    page.update(cx, |page, cx| {
                        page.show_profile(
                            kchess_contracts::types::LichessUser {
                                username: "Rival".into(),
                                ..Default::default()
                            },
                            cx,
                        )
                    });
                });
                app.click("players-message");
            }),
        ),
    ];
    let (widest, tallest) = SIZES[SIZES.len() - 1];
    for (page, label, open) in dialogs {
        app.cx
            .simulate_window_resize(app.window, size(px(widest), px(tallest)));
        app.with_window(|window, cx| crate::shell::open(page, window, cx));
        settle(&mut app);
        open(&mut app);
        for (width, height) in SIZES {
            app.cx
                .simulate_window_resize(app.window, size(px(width), px(height)));
            settle(&mut app);
            // Only the dialog's own controls: the page under its overlay takes no clicks.
            let controls: Vec<ElementSnapshot> = app.with_window(|window, _| {
                window
                    .elements()
                    .into_iter()
                    .filter(|e| {
                        e.path()
                            .iter()
                            .any(|part| *part == gpui_kit::ElementId::from("dialog"))
                    })
                    .collect()
            });
            assert!(!controls.is_empty(), "{label} opens a dialog");
            check(
                &format!("{label} at {width}×{height}"),
                width,
                &controls,
                &mut findings,
            );
        }
        app.press("escape");
        settle(&mut app);
    }
    findings.problems.sort();
    findings.problems.dedup();
    findings.small.sort();
    findings.small.dedup();
    assert!(
        findings.problems.is_empty() && findings.small.is_empty(),
        "in a dialog:\n  {}",
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
                    serde_json::from_value(serde_json::json!({
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
                    }))
                    .expect("a played game"),
                    cx,
                );
            }
        });
    });
    let day = crate::game::puzzle::local_day();
    let attempt = serde_json::json!({
        "puzzle": {
            "id": "aaaaa",
            "fen": "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 2 4",
            "solution": ["f3f7", "e8f7"],
            "rating": 1500,
            "themes": ["mateIn1", "short", "opening"],
        },
        "mode": "practice", "retry": false, "win": false, "revealed": false,
        "hinted": false, "found": 1, "wrong": 1, "total": 2, "day": day,
    });
    app.request(kchess_contracts::calls::RecordPuzzleAttempt {
        attempt: serde_json::from_value(attempt).expect("an attempt"),
    });
    let command = serde_json::json!({
        "op": "saveChapters",
        "name": "A study with a fairly long name to wrap",
        "chapters": [
            { "name": "King pawn", "pgn": "1. e4 e5 *" },
            { "name": "Queen pawn", "pgn": "1. d4 d5 *" },
        ],
    });
    app.request(kchess_contracts::calls::StudyCommand {
        command: serde_json::from_value(command).expect("a study command"),
        incremental: None,
        based_on: None,
    });
}

/// Every page with the longest content people realistically bring: names from imported PGNs
/// (unbounded: a club's own header, a federation title, a hyphenated surname), a study and its
/// chapters at Lichess's 100-character limit, and a 20-character Lichess username, the longest
/// it allows. Long text wraps or ends in an ellipsis; it never pushes a control out of the window
/// or onto another.
#[gpui_kit::test]
fn every_page_with_the_longest_content_lays_out_its_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    seed_longest(&mut app);
    let findings = every_page(&mut app);
    assert!(
        findings.problems.is_empty(),
        "with the longest content:\n  {}",
        findings.problems.join("\n  ")
    );
}

/// Played games, a study and an analysis line, each with the longest realistic text.
fn seed_longest(app: &mut Harness) {
    const WHITE: &str = "GM Aleksandra Wiśniewska-Kowalczyk (Northwind Chess Club)";
    const BLACK: &str = "Bartholomew Fitzgerald-Montgomery III";
    const STUDY: &str =
        "Najdorf Sicilian, English Attack with 6.Be3 e5 7.Nb3: every critical line after 8.f3 Be7";
    const CHAPTER: &str =
        "Chapter 12: 9.Qd2 O-O 10.O-O-O Nbd7 11.g4 b5 12.g5 Nh5 13.Kb1 Nb6 14.Na5 Qc7 15.Nd5";
    let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    app.cx.update(|cx| {
        let played = cx.global::<crate::played::PlayedHandle>().0.clone();
        played.update(cx, |played, cx| {
            for index in 0..3 {
                played.save(
                    serde_json::from_value(serde_json::json!({
                        "id": format!("longest-{index}"),
                        "source": "computer",
                        "setup": { "variant": "standard", "fen": start },
                        "moves": ["e2e4", "c7c5", "g1f3", "d7d6", "d2d4", "c5d4"],
                        "white": WHITE,
                        "black": BLACK,
                        "result": "1/2-1/2",
                        "reason": "agreement",
                        "finished": true,
                        "timeControl": "5400+30",
                        "startedAt": 1_790_000_000_000u64 + index as u64 * 90_000_000,
                        "updatedAt": 1_790_000_000_000u64 + index as u64 * 90_000_000,
                    }))
                    .expect("a played game"),
                    cx,
                );
            }
        });
    });
    // A study imported as a person would, three games each named at the length limit.
    let game = format!(
        "[Event \"{CHAPTER}\"]\n[White \"{WHITE}\"]\n[Black \"{BLACK}\"]\n\n1. e4 c5 2. Nf3 d6 *"
    );
    app.open("Studies");
    app.click("empty-import");
    app.fill_in_dialog("PGN", &[game.as_str(); 3].join("\n\n"));
    app.fill_in_dialog("Study name", STUDY);
    app.click("import-run");
    // Its notice stays over every page until taken up.
    app.click("open-imported");
    app.wait_gone("open-imported");
}
