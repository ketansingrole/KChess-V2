//! Studies: made, annotated and kept.

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

use super::Harness;
use crate::pages::studies::StudiesPage;
use gpui_kit::{InputEvent as _, px};

/// Choose the chapter at `row` (from 0) in the open study's chapter select, by keyboard.
fn choose_chapter(app: &mut Harness, row: usize) {
    app.click_label("Study chapter");
    app.settle();
    for _ in 0..row {
        app.press("down");
    }
    app.press("enter");
}

/// Wait until the Studies page lists exactly these studies, by name.
fn wait_for_listed(app: &mut Harness, expected: &[&str]) {
    app.wait_for(&format!("the studies {expected:?}"), |window, _| {
        let mut names: Vec<String> = window
            .elements()
            .iter()
            .filter_map(|element| element.label()?.strip_prefix("Actions for "))
            .map(str::to_string)
            .collect();
        names.sort();
        names == expected
    });
}

/// Wait until the labelled input holds `expected`, or the select shows it chosen.
fn wait_for_value(app: &mut Harness, label: &str, expected: &str) {
    app.wait_for(&format!("{label:?} to hold {expected:?}"), |window, _| {
        window
            .try_find_by_label(label)
            .is_some_and(|input| input.value() == Some(expected))
    });
}

/// Name a study in the dialog the app opened for it, and wait for the dialog to close: it stays
/// up until the core has the name.
fn name_in_dialog(app: &mut Harness, name: &str) {
    app.find("name-confirm");
    let mut last = None;
    app.wait_for("the dialog to stop moving", |window, _| {
        let bounds = window
            .try_find("name-confirm")
            .map(|button| button.bounds());
        let still = bounds.is_some() && bounds == last;
        last = bounds;
        still
    });
    app.press("secondary-a");
    app.type_text(name);
    app.click("name-confirm");
    app.wait_gone("name-confirm");
}

const TWO_CHAPTERS: &str = "[Event \"King pawn\"]\n\n1. e4 e5 *\n\n[Event \"Queen pawn\"]\n\n1. d4 {Offline notes} d5 (1... Nf6) *";

/// Import a study named `name` from PGN and open it, from the Studies page.
fn import_study(app: &mut Harness, name: &str, pgn: &str) {
    app.open("Studies");
    let empty = app.shows("empty-import");
    app.click(if empty { "empty-import" } else { "import-pgn" });
    app.fill_in_dialog("PGN", pgn);
    app.fill_in_dialog("Study name", name);
    app.click("import-run");
    app.find("open-imported");
}

/// A new study through the empty Studies page, which opens it on the analysis board.
/// A new study, once the core has made it: its bar shows at once, but its menu offers the study's
/// own actions only from then on. A menu opened before that had no "Delete study", and Windows
/// CI failed `a_study_deleted_from_the_board_is_gone_from_the_list` waiting for it now and then.
fn new_study(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Studies");
    app.click("empty-new-study");
    app.find("study-actions");
    wait_for_value(&mut app, "Study name", "Untitled study");
    app
}

/// ⌘N on Studies starts a study, as its New study does.
#[gpui_kit::test]
fn new_starts_a_study_on_the_studies_page(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Studies");
    app.press("secondary-n");
    app.find("study-actions");
    wait_for_value(&mut app, "Study name", "Untitled study");
}

#[gpui_kit::test]
fn a_new_study_opens_on_the_board_and_is_listed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = new_study(cx);
    wait_for_value(&mut app, "Study name", "Untitled study");
    wait_for_value(&mut app, "Study chapter", "1. Untitled study");
    assert!(!app.shows("save-as-study"));

    app.click("all-studies");
    wait_for_listed(&mut app, &["Untitled study"]);
}

#[gpui_kit::test]
fn an_imported_pgn_becomes_a_study_with_a_chapter_for_each_game(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Pawn openings", TWO_CHAPTERS);
    wait_for_listed(&mut app, &["Pawn openings"]);
    app.click("open-imported");
    wait_for_value(&mut app, "Study name", "Pawn openings");
    wait_for_value(&mut app, "Study chapter", "1. Chapter 1");
    app.find("tree-e2e4");
    assert!(!app.shows("tree-d2d4"));

    choose_chapter(&mut app, 1);
    wait_for_value(&mut app, "Study chapter", "2. Chapter 2");
    app.find("tree-d2d4");
    // The chapter's own comment and variation come with it.
    assert_eq!(app.find("comment-d2d4").label(), Some("Offline notes"));
    app.find("tree-d2d4 g8f6");
    assert!(!app.shows("tree-e2e4"));
}

/// A file of several games dropped on any page is offered as a study: the Studies import opens
/// holding it, and imports a chapter for each game.
#[gpui_kit::test]
fn a_dropped_file_of_several_games_is_offered_as_a_study(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Puzzles");
    let _file = app.drop_file("openings.pgn", TWO_CHAPTERS.as_bytes());
    wait_for_value(&mut app, "PGN", TWO_CHAPTERS);
    assert_eq!(
        app.cx.update(|cx| crate::shell::current_page(cx)),
        Some(crate::pages::Page::Studies)
    );
    app.fill_in_dialog("Study name", "Dropped openings");
    app.click("import-run");
    app.find("open-imported");
    wait_for_listed(&mut app, &["Dropped openings"]);
    app.click("open-imported");
    wait_for_value(&mut app, "Study name", "Dropped openings");
    wait_for_value(&mut app, "Study chapter", "1. Chapter 1");
    app.find("tree-e2e4");
}

#[gpui_kit::test]
fn a_pgn_that_cannot_be_read_is_refused(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Studies");
    app.click("empty-import");
    app.fill_in_dialog("PGN", "1. e4 e5 2. Ke5 *");
    app.click("import-run");
    app.settle();
    assert!(app.shows("import-run"), "the dialog stays for a correction");
    app.click("import-cancel");
    app.settle();
    assert!(app.shows("empty-new-study"), "no study was made");
}

#[gpui_kit::test]
fn an_annotated_study_is_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = new_study(cx);
    app.click_label("Enter a chess move in SAN or UCI");
    for text in ["e4", "e7e5"] {
        app.type_text(text);
        app.press("enter");
    }
    app.find("tree-e2e4 e7e5");
    app.click("add-comment");
    app.type_text("Keep this annotation.");
    app.click("glyph-2");
    app.click("study-actions");
    app.click_label("Game details…");
    app.fill_in_dialog("White", "Alice");
    app.press("escape");
    app.settle();

    let mut app = app.restart();
    app.open("Studies");
    app.click_label("Actions for Untitled study");
    app.click_label("Open");
    app.find("study-actions");
    wait_for_value(&mut app, "Study name", "Untitled study");
    assert_eq!(
        app.find("comment-e2e4 e7e5").label(),
        Some("Keep this annotation.")
    );
    assert_eq!(app.find("glyph-2").checked(), Some(true));
    app.click("study-actions");
    app.click_label("Game details…");
    wait_for_value(&mut app, "White", "Alice");
}

#[gpui_kit::test]
fn a_study_is_renamed_from_the_list(cx: &mut gpui_kit::TestAppContext) {
    let mut app = new_study(cx);
    app.click("all-studies");
    app.click_label("Actions for Untitled study");
    app.click_label("Rename…");
    name_in_dialog(&mut app, "Sicilian lines");
    wait_for_listed(&mut app, &["Sicilian lines"]);

    let mut app = app.restart();
    app.open("Studies");
    wait_for_listed(&mut app, &["Sicilian lines"]);
}

#[gpui_kit::test]
fn a_study_is_duplicated_and_a_deleted_one_comes_back_with_undo(cx: &mut gpui_kit::TestAppContext) {
    let mut app = new_study(cx);
    app.click("all-studies");
    app.click_label("Actions for Untitled study");
    app.click_label("Duplicate");
    let both = ["Untitled study", "Untitled study (copy)"];
    wait_for_listed(&mut app, &both);

    app.click_label("Actions for Untitled study (copy)");
    app.click_label("Delete");
    wait_for_listed(&mut app, &["Untitled study"]);
    app.click("undo-delete");
    wait_for_listed(&mut app, &both);
}

/// The page keeps each study's summary between frames (summarizing every study twice a frame was
/// most of drawing it): a study saved with other moves is summarized from them, found by its new
/// event, and the summary of what it held before is let go.
#[gpui_kit::test]
fn an_edited_study_is_summarized_again(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Alpha", "[Event \"Old event\"]\n\n1. e4 e5 *");
    app.open("Studies");
    wait_for_listed(&mut app, &["Alpha"]);
    let kept = |app: &mut Harness| {
        app.with_window(|_, cx| {
            let page = crate::shell::view::<StudiesPage>(cx).expect("the page");
            let pgns: Vec<String> = cx
                .global::<crate::studies::StudyHandle>()
                .0
                .read(cx)
                .items()
                .iter()
                .filter_map(|study| study["pgn"].as_str().map(str::to_string))
                .collect();
            (page.read(cx).kept_summaries(), pgns)
        })
    };
    let (before, listed) = kept(&mut app);
    assert_eq!(before, listed, "the listed study's summary is kept");
    let id = app.cx.update(|cx| {
        cx.global::<crate::studies::StudyHandle>()
            .0
            .read(cx)
            .items()[0]["id"]
            .as_str()
            .expect("id")
            .to_string()
    });
    let saved = std::rc::Rc::new(std::cell::Cell::new(false));
    app.cx.update(|cx| {
        let save = cx
            .global::<crate::studies::StudyHandle>()
            .0
            .read(cx)
            .command(kchess_contracts::types::StudyCommand::Save {
                name: "Alpha".into(),
                pgn: "[Event \"New event\"]\n\n1. d4 d5 *".into(),
                id: Some(id.clone()),
                chapter_id: None,
            });
        let saved = saved.clone();
        cx.spawn(async move |_| {
            save.await.expect("the save");
            saved.set(true);
        })
        .detach();
    });
    app.wait_for("the save", |_, _| saved.get());
    app.wait_for("the study's new summary", |window, cx| {
        window.draw(cx).clear(cx);
        let Some(page) = crate::shell::view::<StudiesPage>(cx) else {
            return false;
        };
        let kept = page.read(cx).kept_summaries();
        kept.len() == 1 && kept[0].contains("1. d4")
    });
    app.click_label("Search studies");
    app.type_text("new event");
    wait_for_listed(&mut app, &["Alpha"]);
}

/// Only the cards in view are made, laid out and drawn: fifty studies' cards (the limit) were most
/// of opening the page. Scrolled down with the wheel, the cards coming into view are made and those
/// left behind are let go, and the grid keeps its height, so the oldest study can be reached;
/// scrolled back by the page itself, which asks for no frame, the first cards come back.
#[gpui_kit::test]
fn only_the_studies_in_view_are_made(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let saved = std::rc::Rc::new(std::cell::Cell::new(0));
    app.cx.update(|cx| {
        for n in 0..50 {
            let save = cx
                .global::<crate::studies::StudyHandle>()
                .0
                .read(cx)
                .command(kchess_contracts::types::StudyCommand::Save {
                    name: format!("Study {n:02}"),
                    pgn: "1. e4 *".into(),
                    id: None,
                    chapter_id: None,
                });
            let saved = saved.clone();
            cx.spawn(async move |_| {
                save.await.expect("the save");
                saved.set(saved.get() + 1);
            })
            .detach();
        }
    });
    app.wait_for("fifty studies", |_, _| saved.get() == 50);
    app.open("Studies");
    let listed = |app: &mut Harness| {
        app.with_window(|window, _| {
            window
                .elements()
                .iter()
                .filter_map(|element| element.label()?.strip_prefix("Actions for "))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
    };
    // Newest first: the last study made heads the list.
    app.wait_for("the first cards", |window, _| {
        window
            .elements()
            .iter()
            .any(|element| element.label() == Some("Actions for Study 49"))
    });
    let first = listed(&mut app);
    assert!(
        first.len() < 50,
        "only the cards in view are made, not {}",
        first.len()
    );
    // The cards made are a run of the list, in its order: the newest first.
    let run: Vec<String> = (0..first.len())
        .map(|n| format!("Study {:02}", 49 - n))
        .collect();
    assert_eq!(first, run);
    // Each row made is measured, its first-frame estimate is near what it measures, and the grid
    // is as tall as every row, made or not, and the gaps between them.
    let grid = |app: &mut Harness| {
        app.with_window(|_, cx| {
            crate::shell::view::<StudiesPage>(cx)
                .expect("the page")
                .read(cx)
                .grid_layout()
        })
    };
    let (heights, height) = grid(&mut app);
    let row = heights[0];
    assert!(row > px(0.), "the first row is measured");
    // Cards side by side are a card's width and the 16 px gap apart.
    let card = app.with_window(|window, _| {
        let at = |label: &str| {
            window
                .try_find_by_label(label)
                .expect("a card's actions")
                .bounds()
                .origin
        };
        let (first, second) = (at("Actions for Study 49"), at("Actions for Study 48"));
        assert_eq!(first.y, second.y, "the first two cards share a row");
        second.x - first.x - px(16.)
    });
    let estimate = crate::pages::studies::estimated_row(card);
    assert!(
        estimate > row * 0.85 && estimate < row * 1.15,
        "estimated {estimate:?}, measured {row:?}"
    );
    let rows = heights.len() as f32;
    let whole = row * rows + px(16.) * (rows - 1.);
    assert!(
        (height - whole).abs() < px(1.),
        "the grid is {height:?}, its rows {whole:?}"
    );

    // The middle of the window, over the page (the sidebar is at its left).
    let middle = app.with_window(|window, _| {
        let size = window.viewport_size();
        gpui_kit::point(size.width * 0.6, size.height * 0.5)
    });
    let mut turns = 0;
    while !listed(&mut app).contains(&"Study 00".to_string()) {
        turns += 1;
        assert!(turns < 200, "the oldest study is reached by scrolling");
        app.with_window(|window, cx| {
            let wheel = gpui_kit::ScrollEvent {
                position: middle,
                delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-300.))),
                ..Default::default()
            };
            window.dispatch_event(wheel.to_platform_input(), cx);
        });
    }
    let last = listed(&mut app);
    assert!(
        !last.contains(&"Study 49".to_string()),
        "the cards scrolled past are let go"
    );

    app.cx.update(|cx| {
        crate::shell::view::<StudiesPage>(cx)
            .expect("the page")
            .read(cx)
            .scroll_to_top();
    });
    app.wait_for("the first cards again", |window, cx| {
        window.draw(cx).clear(cx);
        window
            .elements()
            .iter()
            .any(|element| element.label() == Some("Actions for Study 49"))
    });
}

#[gpui_kit::test]
fn studies_are_found_by_name(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Alpha", "1. e4 e5 *");
    // The import's notice sits over the page's buttons, so the second one is made on the board.
    app.open("Analysis board");
    app.click_square("d2", true);
    app.click_square("d4", true);
    app.click("save-as-study");
    name_in_dialog(&mut app, "Beta");
    app.open("Studies");
    wait_for_listed(&mut app, &["Alpha", "Beta"]);
    app.click_label("Search studies");
    app.type_text("alp");
    wait_for_listed(&mut app, &["Alpha"]);
}

#[gpui_kit::test]
fn an_analysis_is_saved_as_a_study(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Analysis board");
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.click("save-as-study");
    name_in_dialog(&mut app, "Opening study");
    wait_for_value(&mut app, "Study name", "Opening study");
    assert!(!app.shows("save-as-study"));

    app.open("Studies");
    wait_for_listed(&mut app, &["Opening study"]);
}

#[gpui_kit::test]
fn opening_a_study_asks_before_replacing_an_unsaved_analysis(cx: &mut gpui_kit::TestAppContext) {
    let mut app = new_study(cx);
    app.click("study-actions");
    app.click_label("Close study");
    app.find("save-as-study");
    app.click_square("e2", true);
    app.click_square("e4", true);

    app.open("Studies");
    app.click_label("Actions for Untitled study");
    app.click_label("Open");
    app.click("cancel");
    app.settle();
    assert!(app.shows("new-study"), "still on the Studies page");

    app.click_label("Actions for Untitled study");
    app.click_label("Open");
    app.click("ok");
    wait_for_value(&mut app, "Study name", "Untitled study");
    assert!(
        !app.shows("tree-e2e4"),
        "the study's empty board replaced the analysis"
    );
}

#[gpui_kit::test]
fn chapters_are_added_renamed_duplicated_and_deleted(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Pawn openings", TWO_CHAPTERS);
    app.click("open-imported");
    wait_for_value(&mut app, "Study chapter", "1. Chapter 1");

    app.click("chapter-actions");
    app.click_label("Rename chapter");
    name_in_dialog(&mut app, "King pawn");
    wait_for_value(&mut app, "Study chapter", "1. King pawn");

    app.click("chapter-actions");
    app.click_label("Duplicate chapter");
    wait_for_value(&mut app, "Study chapter", "3. King pawn (copy)");
    app.find("tree-e2e4");

    // Deleting asks nothing: its notice's Undo puts the chapter back in its place, opened.
    app.click("chapter-actions");
    app.click_label("Delete chapter");
    wait_for_value(&mut app, "Study chapter", "2. Chapter 2");
    app.click("undo-delete-chapter");
    wait_for_value(&mut app, "Study chapter", "3. King pawn (copy)");
    app.find("tree-e2e4");
    app.click("chapter-actions");
    app.click_label("Delete chapter");
    wait_for_value(&mut app, "Study chapter", "2. Chapter 2");
}

/// Another window changed the chapter on the board: switching chapters keeps the board's edits
/// as an unsaved copy rather than dropping them for the next chapter.
#[gpui_kit::test]
fn a_refused_save_keeps_its_edits_when_another_chapter_is_chosen(
    cx: &mut gpui_kit::TestAppContext,
) {
    refused_save_then_another_chapter(cx, false);
}

/// The same, with the edit's delayed save already on its way when the chapter is chosen.
#[gpui_kit::test]
fn a_refused_autosave_keeps_its_edits_when_another_chapter_is_chosen(
    cx: &mut gpui_kit::TestAppContext,
) {
    refused_save_then_another_chapter(cx, true);
}

fn refused_save_then_another_chapter(cx: &mut gpui_kit::TestAppContext, autosaving: bool) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Pawn openings", TWO_CHAPTERS);
    app.click("open-imported");
    wait_for_value(&mut app, "Study chapter", "1. Chapter 1");
    app.find("tree-e2e4 e7e5");
    // Another window writes the chapter (this connection's own write is not shown here).
    let (study, chapter) = app.cx.update(|cx| {
        let page = crate::shell::view::<crate::pages::analysis::AnalysisPage>(cx)
            .expect("the analysis board");
        let page = page.read(cx);
        (page.study_id().to_string(), page.chapter_id().to_string())
    });
    let written = std::rc::Rc::new(std::cell::Cell::new(false));
    app.cx.update(|cx| {
        let save =
            cx.global::<crate::core::AppCore>()
                .request(kchess_contracts::calls::StudyCommand {
                    command: kchess_contracts::types::StudyCommand::Save {
                        name: "Pawn openings".into(),
                        pgn: "1. c4 *".into(),
                        id: Some(study.clone()),
                        chapter_id: Some(chapter.clone()),
                    },
                    incremental: None,
                    based_on: None,
                });
        let written = written.clone();
        cx.spawn(async move |_| {
            save.await.expect("the other window's save");
            written.set(true);
        })
        .detach();
    });
    app.wait_for("the other window's save", |_, _| written.get());
    // The edit and the chapter switch come before the edit's own delayed save.
    app.with_window(|_, cx| {
        let page = crate::shell::view::<crate::pages::analysis::AnalysisPage>(cx)
            .expect("the analysis board");
        let next = cx
            .global::<crate::studies::StudyHandle>()
            .0
            .read(cx)
            .study(&study)
            .and_then(|found| found["chapters"][1]["id"].as_str().map(str::to_string))
            .expect("the second chapter");
        assert!(page.update(cx, |page, cx| page.play_for_test("g1f3", cx)));
        if autosaving {
            page.update(cx, |page, cx| page.autosave_for_test(cx));
        }
        let bar = page.read(cx).study_bar();
        bar.update(cx, |bar, cx| bar.select_chapter(next, cx));
    });
    app.settle();
    app.find("tree-g1f3");
    assert!(
        !app.shows("tree-d2d4"),
        "the edits were dropped for chapter 2"
    );
}

#[gpui_kit::test]
fn the_chosen_chapter_is_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Pawn openings", TWO_CHAPTERS);
    app.click("open-imported");
    choose_chapter(&mut app, 1);
    wait_for_value(&mut app, "Study chapter", "2. Chapter 2");

    let mut app = app.restart();
    app.open("Analysis board");
    wait_for_value(&mut app, "Study name", "Pawn openings");
    wait_for_value(&mut app, "Study chapter", "2. Chapter 2");
    app.find("tree-d2d4");
}

/// Closing a study saved it, then emptied the board whatever it held by then: a position opened
/// while the study saved was wiped.
#[gpui_kit::test]
fn a_position_opened_while_a_study_closes_stays_on_the_board(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::analysis::AnalysisPage;
    let mut app = Harness::start(cx);
    import_study(&mut app, "Pawn openings", TWO_CHAPTERS);
    app.click("open-imported");
    app.find("tree-e2e4");
    let closed = std::rc::Rc::new(std::cell::Cell::new(None));
    let opened = app.with_window(|_, cx| {
        let page = crate::shell::view::<AnalysisPage>(cx).expect("the analysis board");
        page.update(cx, |page, cx| {
            let close = page.close_study(cx);
            let closed = closed.clone();
            cx.spawn(async move |_, _| closed.set(Some(close.await)))
                .detach();
            page.load(Some("8/8/8/4k3/8/8/4K3/8 w - - 0 1"), cx);
            page.shown_fen()
        })
    });
    app.wait_for("the study to close", |_, _| closed.get().is_some());
    assert_eq!(closed.get(), Some(true), "the close went through");
    let shown = app.with_window(|_, cx| {
        crate::shell::view::<AnalysisPage>(cx)
            .expect("the analysis board")
            .read(cx)
            .shown_fen()
    });
    assert_eq!(shown, opened);
}

#[gpui_kit::test]
fn a_study_closed_from_the_board_leaves_an_unsaved_analysis(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Pawn openings", TWO_CHAPTERS);
    app.click("open-imported");
    app.find("tree-e2e4");
    app.click("study-actions");
    app.click_label("Close study");
    app.find("save-as-study");
    assert!(!app.shows("chapter-actions"));
}

#[gpui_kit::test]
fn a_study_deleted_from_the_board_is_gone_from_the_list(cx: &mut gpui_kit::TestAppContext) {
    let mut app = new_study(cx);
    app.click("study-actions");
    app.click_label("Delete study");
    app.find("undo-delete-study");
    app.find("save-as-study");
    app.open("Studies");
    app.find("empty-new-study");
}

/// Quick Pick makes a study and opens the import dialog, as the header's buttons do.
#[gpui_kit::test]
fn quick_pick_makes_and_imports_studies(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Studies");
    app.find("empty-new-study");
    app.press("secondary-k");
    app.type_text(">Import PGN");
    super::palette::wait_for_first(&mut app, "Import PGN");
    app.press("enter");
    app.find("import-run");
    app.click("import-cancel");
    app.settle();

    app.press("secondary-k");
    app.type_text(">New study");
    super::palette::wait_for_first(&mut app, "New study");
    app.press("enter");
    app.find("study-actions");
    wait_for_value(&mut app, "Study name", "Untitled study");
}

/// The study bar's menus are in Quick Pick over the analysis board: an analysis is saved as a
/// study, a chapter added to it, and the study closed again.
#[gpui_kit::test]
fn quick_pick_saves_a_study_adds_a_chapter_and_closes_it(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let run = |app: &mut Harness, label: &str| {
        app.press("secondary-k");
        app.type_text(&format!(">{label}"));
        super::palette::wait_for_first(app, label);
        app.press("enter");
        super::palette::wait_until_closed(app);
    };
    app.open("Analysis board");
    app.click_square("e2", true);
    app.click_square("e4", true);
    run(&mut app, "Save as study");
    name_in_dialog(&mut app, "Opening study");
    wait_for_value(&mut app, "Study name", "Opening study");

    run(&mut app, "Add chapter");
    name_in_dialog(&mut app, "Endgames");
    wait_for_value(&mut app, "Study chapter", "2. Endgames");

    run(&mut app, "Close study");
    app.find("save-as-study");
    assert!(!app.shows("chapter-actions"));
}

#[gpui_kit::test]
fn lichess_studies_open_from_quick_pick_with_an_account(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.press("secondary-k");
    app.type_text("Lichess studies");
    super::palette::wait_for_first(&mut app, "Lichess studies");
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    assert_eq!(app.find(("studies-tabs", 1usize)).selected(), Some(true));
}

/// Quick Pick steps to the next and previous chapter, offering each only where there is one.
#[gpui_kit::test]
fn quick_pick_steps_through_the_chapters(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Pawn openings", TWO_CHAPTERS);
    wait_for_listed(&mut app, &["Pawn openings"]);
    app.click("open-imported");
    wait_for_value(&mut app, "Study chapter", "1. Chapter 1");
    let run = |app: &mut Harness, command: &str| {
        app.press("shift-secondary-p");
        app.type_text(command);
        super::palette::wait_for_first(app, command);
        app.press("enter");
        super::palette::wait_until_closed(app);
    };
    run(&mut app, "Next chapter");
    wait_for_value(&mut app, "Study chapter", "2. Chapter 2");
    app.find("tree-d2d4");
    // The last chapter has no next one.
    app.press("shift-secondary-p");
    app.type_text("chapter");
    super::palette::wait_for_first(&mut app, "Add chapter");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..12usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    assert!(
        listed.contains(&"Previous chapter".to_string()),
        "{listed:?}"
    );
    assert!(!listed.contains(&"Next chapter".to_string()), "{listed:?}");
    app.press("escape");
    super::palette::wait_until_closed(&mut app);
    run(&mut app, "Previous chapter");
    wait_for_value(&mut app, "Study chapter", "1. Chapter 1");
    app.find("tree-e2e4");
}

/// A chapter moves down from its menu and back up from Quick Pick, and stays open meanwhile.
#[gpui_kit::test]
fn chapters_move_up_and_down(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Pawn openings", TWO_CHAPTERS);
    wait_for_listed(&mut app, &["Pawn openings"]);
    app.click("open-imported");
    wait_for_value(&mut app, "Study chapter", "1. Chapter 1");

    app.click("chapter-actions");
    app.click_label("Move down");
    wait_for_value(&mut app, "Study chapter", "2. Chapter 1");
    app.find("tree-e2e4");
    // The last chapter cannot move further down.
    app.press("shift-secondary-p");
    app.type_text("Move chapter");
    super::palette::wait_for_first(&mut app, "Move chapter up");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..12usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    assert!(
        !listed.contains(&"Move chapter down".to_string()),
        "{listed:?}"
    );
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    wait_for_value(&mut app, "Study chapter", "1. Chapter 1");
    app.find("tree-e2e4");
}

/// A study's menu saves its PGN, every chapter with its notes, as a file.
#[gpui_kit::test]
fn a_study_is_saved_as_a_pgn_file(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    import_study(&mut app, "Pawn openings", TWO_CHAPTERS);
    wait_for_listed(&mut app, &["Pawn openings"]);
    app.click_label("Actions for Pawn openings");
    app.click_label("Save PGN…");
    app.wait_for_save_dialog();
    let folder = tempfile::tempdir().expect("a folder to save in");
    let path = folder.path().join("Pawn openings.pgn");
    let chosen = path.clone();
    app.cx.simulate_new_path_selection(move |_| Some(chosen));
    app.wait_for("the file", |_, _| path.exists());
    let saved = std::fs::read_to_string(&path).expect("the saved PGN");
    assert!(saved.contains("Offline notes"), "{saved}");
    assert_eq!(saved.matches("[Event ").count(), 2, "{saved}");
}
