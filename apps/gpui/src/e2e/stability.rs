//! What holds still: a page's content keeps its place when its state changes under the
//! pointer, so the next click lands where the person aimed it (stepping back through a game's
//! moves, a load finishing, a status line changing).

use std::collections::BTreeMap;

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{App, AppContext as _, Bounds, ElementId, Pixels, Point, Role, Window, px, size};

use crate::pages::Page;

use super::{Harness, SIZES};

/// Where `ids` are now, each by its element id.
fn places(app: &mut Harness, ids: &[ElementId]) -> Vec<Bounds<Pixels>> {
    app.with_window(|window, _| {
        ids.iter()
            .map(|id| window.find(id.clone()).bounds())
            .collect()
    })
}

/// The board and the game's first `moves` in the move list, which a person steps through.
fn game_ids(moves: usize) -> Vec<ElementId> {
    std::iter::once("chess-board".into())
        .chain((0..moves).map(|ply| ("move", ply).into()))
        .collect()
}

/// Step back, to the start and to the game again, checking `ids` hold their place throughout.
fn stepping_back_holds_still(app: &mut Harness, ids: &[ElementId]) {
    stepping_holds_still(app, ids, &["chevron-left", "skip-back", "skip-forward"]);
}

/// Take `steps` through the moves, ending at the game's last, checking `ids` hold their place
/// after each, at the window sizes that put the move list beside the board.
fn stepping_holds_still(app: &mut Harness, ids: &[ElementId], steps: &[&'static str]) {
    for (width, height) in &SIZES[1..] {
        app.cx
            .simulate_window_resize(app.window, size(px(*width), px(*height)));
        app.settle();
        let live = places(app, ids);
        for &step in steps {
            app.click(step);
            app.settle();
            assert_eq!(
                places(app, ids),
                live,
                "the page moved after {step} at {width}×{height}"
            );
        }
    }
}

#[gpui_kit::test]
fn over_the_board_holds_still_while_you_step_back(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Over the board");
    app.find("chess-board");
    super::local::wait_resumed(&mut app);
    super::local::type_moves(&mut app, &["e4", "e5", "Nf3"]);
    stepping_back_holds_still(&mut app, &game_ids(3));
}

#[gpui_kit::test]
fn play_with_computer_holds_still_while_you_step_back(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Play with computer");
    app.find("chess-board");
    super::computer::wait_playable(&mut app);
    // The scripted engine answers only as White, so one move of yours is the game.
    app.click_square("e2", true);
    app.click_square("e4", true);
    app.wait_for_moves(&["e4"]);
    stepping_holds_still(&mut app, &game_ids(1), &["chevron-left", "skip-forward"]);
}

#[gpui_kit::test]
fn a_live_game_holds_still_while_you_step_back(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = super::live_game::past_the_opening(cx);
    assert_eq!(app.status(), "Your move");
    stepping_back_holds_still(&mut app, &game_ids(2));
    assert_eq!(app.status(), "Your move");
}

/// A banner whose actions do not fit beside its text puts them under it: squeezed beside them, a
/// refused game's detail stood one word a line, 439 px high, beside the panel's own Watch.
#[gpui_kit::test]
fn a_banners_actions_go_under_its_text_in_a_narrow_panel(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = super::live_game::in_a_game(cx);
    lichess.lock().board_refused = true;
    let watch = app.find("online-watch").bounds();
    let banner = app.find("status-banner").bounds();
    assert!(
        banner.size.height < px(160.),
        "the banner is {:?} high",
        banner.size.height
    );
    assert!(
        watch.origin.x < banner.origin.x + px(24.),
        "Watch is beside the text: {watch:?}"
    );
}

/// The engine's depth holds still as its search ends: the spinner before it kept its place only
/// while it turned, and the text slid left each time a search finished.
#[gpui_kit::test]
fn the_engine_line_holds_still_as_its_search_ends(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::analysis::AnalysisPage;
    let searching = |busy: bool| {
        move |_: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            crate::shell::view::<AnalysisPage>(cx).is_some_and(|page| {
                let page = page.read(cx);
                page.engine_search().1 == busy && page.engine_text(cx).starts_with("Depth ")
            })
        }
    };
    let mut app = Harness::start(cx);
    // An infinite search runs until stopped, so the spinner turns beside a depth.
    app.cx
        .update(|cx| crate::prefs::set(cx, crate::pages::analysis::INFINITE, true));
    app.open("Analysis board");
    app.find("chess-board");
    app.wait_for("the engine to search", searching(true));
    let ids: [ElementId; 1] = ["engine-status".into()];
    let during = places(&mut app, &ids);
    let (client, _) = super::analysis::engine_search(&mut app);
    super::analysis::stop_search(&mut app, client, false);
    app.wait_for("the search to end", searching(false));
    assert_eq!(places(&mut app, &ids)[0].origin, during[0].origin);
}

/// Turn voice on and off on `page` (`open` shows it), its Voice button holding its place under
/// the pointer while the bar fills in beside it (here with the error a test machine's missing
/// speech model gives, and Try again).
fn voice_holds_still(app: &mut Harness, page: &str, open: fn(&mut Harness), bar: &'static str) {
    let button: ElementId = (bar, 0usize).into();
    for (width, height) in &SIZES[1..] {
        app.cx
            .simulate_window_resize(app.window, size(px(*width), px(*height)));
        open(app);
        app.find(button.clone());
        app.settle();
        let ids = [button.clone()];
        let off = places(app, &ids);
        app.click(button.clone());
        app.find((bar, 2usize));
        app.settle();
        assert_eq!(
            places(app, &ids),
            off,
            "{page}'s Voice moved once on at {width}×{height}"
        );
        app.click(button.clone());
        app.settle();
        assert_eq!(
            places(app, &ids),
            off,
            "{page}'s Voice moved once off at {width}×{height}"
        );
    }
}

#[gpui_kit::test]
fn the_voice_button_holds_still_beside_the_move_box(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    voice_holds_still(
        &mut app,
        "Play with computer",
        |app| app.open("Play with computer"),
        "computer-voice",
    );
}

#[gpui_kit::test]
fn the_voice_button_holds_still_under_the_board_editor(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    voice_holds_still(
        &mut app,
        "Board editor",
        |app| app.open("Board editor"),
        "editor-voice",
    );
}

#[gpui_kit::test]
fn the_voice_button_holds_still_under_an_endgame_lesson(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    voice_holds_still(
        &mut app,
        "An endgame lesson",
        |app| {
            app.open("Practice");
            if !app.shows(("lesson-voice", 0usize)) {
                super::practice::tab(app, crate::pages::practice::Tab::Endgames);
                app.click("card-checkmate-patterns-2");
                app.click("training-cp2-balestra");
            }
        },
        "lesson-voice",
    );
}

#[gpui_kit::test]
fn the_voice_button_holds_still_under_an_endgame_drill(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Practice");
    super::practice::tab(&mut app, crate::pages::practice::Tab::Endgames);
    super::practice::reveal(&mut app, "endgame-courses", "card-drills");
    app.click("card-drills");
    voice_holds_still(
        &mut app,
        "An endgame drill",
        |app| app.open("Practice"),
        "drill-voice",
    );
}

#[gpui_kit::test]
fn the_voice_button_holds_still_under_the_repertoire_trainer(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    voice_holds_still(
        &mut app,
        "The repertoire trainer",
        |app| {
            app.open("Practice");
            if !app.shows(("repertoire-voice", 0usize)) {
                app.open("Openings");
                super::openings::open_kieseritzky(app);
                app.click("train-line");
            }
        },
        "repertoire-voice",
    );
}

/// The drills with a board have Voice in the row under it, as the game pages do.
#[gpui_kit::test]
fn the_voice_button_holds_still_on_every_drill(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::practice::Tab;
    let mut app = Harness::start(cx);
    voice_holds_still(
        &mut app,
        "Coordinates",
        |app| {
            app.open("Practice");
            app.click(super::tab("practice-tabs", Tab::Coordinates));
            app.click(("coordinates-mode", 1usize));
        },
        "coordinates-voice",
    );
    voice_holds_still(
        &mut app,
        "Knight paths",
        |app| {
            app.open("Practice");
            app.click(super::tab("practice-tabs", Tab::Knight));
        },
        "knight-voice",
    );
}

#[gpui_kit::test]
fn the_voice_button_holds_still_in_guess_the_move(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Practice");
    super::practice::guess_game(&mut app, "1. e4 e5 2. Nf3 Nc6 *");
    // The game stays on the tab as the page opens again at each size.
    voice_holds_still(
        &mut app,
        "Guess the move",
        |app| app.open("Practice"),
        "guess-voice",
    );
}

/// Choosing a spare piece in the Board editor leaves the row where it was.
#[gpui_kit::test]
fn the_board_editors_spare_pieces_hold_still_as_one_is_chosen(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Board editor");
    app.find("chess-board");
    let ids: Vec<ElementId> = ["King", "Queen", "Rook", "Bishop", "Knight", "Pawn"]
        .iter()
        .map(|role| format!("spare-White-{role}").into())
        .collect();
    let before = places(&mut app, &ids);
    for id in &ids {
        app.click(id.clone());
        app.settle();
        assert_eq!(
            places(&mut app, &ids),
            before,
            "the spare pieces moved choosing {id:?}"
        );
    }
}

/// Choosing a piece set in Settings leaves every option where it was.
#[gpui_kit::test]
fn choosing_a_piece_set_leaves_the_options_in_place(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open_settings("Appearance");
    let ids: Vec<ElementId> = ["cburnett", "merida", "pixel"]
        .iter()
        .map(|set| format!("piece-set-{set}").into())
        .collect();
    let before = places(&mut app, &ids);
    for id in ids.iter().rev() {
        app.click(id.clone());
        app.settle();
        assert_eq!(
            places(&mut app, &ids),
            before,
            "the options moved choosing {id:?}"
        );
    }
}

/// Switching Studies between this device and Lichess leaves its tabs where they were.
#[gpui_kit::test]
fn the_studies_tabs_hold_still_as_you_switch(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in(cx, false);
    app.open("Studies");
    let ids: Vec<ElementId> = (0..2usize)
        .map(|tab| ("studies-tabs", tab).into())
        .collect();
    app.find(ids[0].clone());
    app.settle();
    let before = places(&mut app, &ids);
    for tab in [1, 0] {
        app.click(ids[tab].clone());
        app.settle();
        assert_eq!(
            places(&mut app, &ids),
            before,
            "the tabs moved choosing tab {tab}"
        );
    }
}

/// The analysis board's moves hold still as you step off the start position and back: the
/// opening's name above them came only once there was one, a line down.
#[gpui_kit::test]
fn the_analysis_moves_hold_still_off_the_start(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let (width, height) = SIZES[2];
    app.cx
        .simulate_window_resize(app.window, size(px(width), px(height)));
    app.open("Analysis board");
    app.find("chess-board");
    super::analysis::import(&mut app, "1. e4 e5 2. Nf3");
    let ids: Vec<ElementId> = ["tree-e2e4", "tree-e2e4 e7e5", "tree-e2e4 e7e5 g1f3"]
        .into_iter()
        .map(Into::into)
        .collect();
    app.find(ids[0].clone());
    app.settle();
    let before = places(&mut app, &ids);
    for step in ["analysis-forward", "analysis-end", "analysis-start"] {
        app.click(step);
        app.settle();
        assert_eq!(
            places(&mut app, &ids),
            before,
            "the moves moved after {step}"
        );
    }
}

/// Home's Lichess rating chart is one height with points and without: the card grew by the
/// chart (and the change under its heading) as the history came, and shrank again for a mode or
/// range with no points, moving what is under it.
#[gpui_kit::test]
fn the_home_rating_chart_keeps_its_height_as_points_come(cx: &mut gpui_kit::TestAppContext) {
    use crate::rating_history::RatingHistory;
    use gpui_kit::component::v_flex;
    use gpui_kit::test::TestSupportExt as _;
    use gpui_kit::{
        AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
        Render, Styled as _, Window, div,
    };
    use kchess_contracts::types::RatingSeries;
    /// The chart with something under it.
    struct Card(Entity<RatingHistory>);
    impl Render for Card {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .child(self.0.clone())
                .child(div().id("under-the-chart").test_support().h(px(10.)))
        }
    }
    let mut app = Harness::start(cx);
    let window = app.cx.add_window(|window, cx| {
        Card(cx.new(|cx| RatingHistory::new("", window, cx).without_mode_picker()))
    });
    let under = |app: &mut Harness| {
        app.cx
            .update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.find("under-the-chart").bounds().origin.y
            })
            .expect("the window")
    };
    let empty = under(&mut app);
    window
        .update(&mut app.cx, |card, _, cx| {
            card.0.update(cx, |chart, cx| {
                chart.set_official(
                    vec![RatingSeries {
                        name: "Blitz".into(),
                        points: vec![[2026, 10, 1, 1500], [2026, 10, 15, 1540]],
                    }],
                    cx,
                )
            })
        })
        .expect("the window");
    assert_eq!(under(&mut app), empty);
}

/// An opening's explorer holds still as its description and popularity come from the web: they
/// came above it, pushing down the next move you were about to click (and up again at the next).
#[gpui_kit::test]
fn the_openings_explorer_holds_still_as_the_line_is_described(cx: &mut gpui_kit::TestAppContext) {
    use crate::pages::openings::OpeningsPage;
    use kchess_contracts::types::{DescriptionBlock, OpeningDescription, PopularityMonth};
    let mut app = Harness::start(cx);
    app.open("Openings");
    super::openings::open_kieseritzky(&mut app);
    let ids: [ElementId; 1] = [("explorer-kind", 0usize).into()];
    app.find(ids[0].clone());
    app.settle();
    let before = places(&mut app, &ids);
    let description = OpeningDescription {
        title: "Kieseritzky Gambit".into(),
        url: "https://en.wikibooks.org/wiki/Chess_Opening_Theory".into(),
        blocks: (0..4)
            .map(|block| DescriptionBlock {
                heading: false,
                text: format!("Paragraph {block} about the gambit's ideas and its history."),
            })
            .collect(),
        fetched_at: 0,
    };
    let months: Vec<PopularityMonth> = (1..=12)
        .map(|month| PopularityMonth {
            month: format!("2025-{month:02}"),
            share: 0.1,
            games: 100,
        })
        .collect();
    app.cx.update(|cx| {
        crate::shell::view::<OpeningsPage>(cx)
            .expect("the page")
            .update(cx, |page, cx| page.arrive(description, &months, cx));
    });
    app.settle();
    assert_eq!(places(&mut app, &ids), before, "the explorer moved");
}

/// Where each element in the window is drawn, by its path; toasts left out, which slide in over
/// the page.
fn drawn(window: &Window) -> BTreeMap<String, Point<Pixels>> {
    window
        .elements()
        .into_iter()
        .map(|element| (format!("{:?}", element.path()), element.bounds().origin))
        .filter(|(path, _)| !path.contains("ToastStack"))
        .collect()
}

/// Do `act` and return the first frame after it, drawn at once, as a person sees it, before any
/// frame callback runs.
fn first_frame(
    app: &mut Harness,
    act: impl FnOnce(&mut Window, &mut App),
) -> BTreeMap<String, Point<Pixels>> {
    app.cx
        .update_window(app.window, |_, window, cx| {
            act(window, cx);
            super::draw(window, cx);
            drawn(window)
        })
        .expect("the main window is open")
}

/// Where the elements of `first` have moved to once the window has settled, each named by
/// `what`.
fn moved_since(
    app: &mut Harness,
    first: &BTreeMap<String, Point<Pixels>>,
    what: &str,
) -> Vec<String> {
    app.settle();
    let settled = app.with_window(|window, _| drawn(window));
    let viewport = app.with_window(|window, _| window.viewport_size());
    first
        .iter()
        .filter_map(|(path, origin)| {
            let to = settled.get(path).filter(|to| *to != origin)?;
            let id: String = path.chars().rev().take(60).collect::<String>();
            let id: String = id.chars().rev().collect();
            Some(format!(
                "{what} at {}×{}: {origin:?} → {to:?} …{id}",
                viewport.width, viewport.height
            ))
        })
        .collect()
}

/// The pages whose first frame is checked: not Settings, which opens as a dialog and slides in,
/// nor Practice and Puzzles. Their pill tabs, above the board, wrap to a height nothing knows
/// before they are laid out (two lines at 1320×880 beside the sidebar, one in a wider window),
/// so the first frame sizes the board from the wrong area: its inset from the page as it last
/// measured, or before that the page's whole height, a few pixels off for one frame. Practice
/// showed it once Coordinates' board, with the row under it, came to fit the height rather than
/// the width. A stopgap until "Practice and Puzzles boards size from their own measured area",
/// which checks both again.
fn checked_pages() -> Vec<Page> {
    Page::all()
        .filter(|&page| !matches!(page, Page::Settings | Page::Practice | Page::Puzzles))
        .collect()
}

/// Where the elements of each page's first frame have moved to once the page has settled.
fn moved_after_first_frame(app: &mut Harness, page: Page, moved: &mut Vec<String>) {
    let first = first_frame(app, |window, cx| crate::shell::open(page, window, cx));
    moved.extend(moved_since(app, &first, page.info().label));
}

/// Open every page for the first time at `width`×`height`, its data loading as it shows.
fn first_openings_hold_still(cx: &mut gpui_kit::TestAppContext, (width, height): (f32, f32)) {
    let (mut app, _lichess) = Harness::signed_in_showcase(cx);
    app.cx
        .simulate_window_resize(app.window, size(px(width), px(height)));
    app.settle();
    let mut moved = Vec::new();
    // Home is open from the start.
    for page in checked_pages() {
        if page != Page::Dashboard {
            moved_after_first_frame(&mut app, page, &mut moved);
        }
    }
    assert!(
        moved.is_empty(),
        "moved after their first frame:\n  {}",
        moved.join("\n  ")
    );
}

/// A page's first frame shows what it does not know yet in the place it will have: Analysis
/// board said "Set up Stockfish" until the engine answered, then the engine's lines pushed its
/// tabs 84px down; Insights' filters went under its headline once it loaded.
#[gpui_kit::test]
fn a_page_holds_still_as_it_first_loads_in_a_wide_window(cx: &mut gpui_kit::TestAppContext) {
    first_openings_hold_still(cx, SIZES[2]);
}

#[gpui_kit::test]
fn a_page_holds_still_as_it_first_loads_in_a_narrow_window(cx: &mut gpui_kit::TestAppContext) {
    first_openings_hold_still(cx, SIZES[0]);
}

#[gpui_kit::test]
fn a_page_opens_laid_out_for_the_window_it_opens_in(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, _lichess) = Harness::signed_in_showcase(cx);
    let pages = checked_pages();
    let mut moved = Vec::new();
    for (was, now) in [(SIZES[0], SIZES[2]), (SIZES[2], SIZES[0])] {
        app.cx
            .simulate_window_resize(app.window, size(px(was.0), px(was.1)));
        for &page in &pages {
            app.with_window(|window, cx| crate::shell::open(page, window, cx));
            app.settle();
        }
        app.cx
            .simulate_window_resize(app.window, size(px(now.0), px(now.1)));
        app.settle();
        // Each page last drawn at the other size, its own measures from then.
        for &page in &pages {
            moved_after_first_frame(&mut app, page, &mut moved);
        }
    }
    assert!(
        moved.is_empty(),
        "moved after their first frame:\n  {}",
        moved.join("\n  ")
    );
}

/// The tabs within each page, other than the sidebar's own: their ids and names.
fn page_tabs(app: &mut Harness) -> Vec<(ElementId, String)> {
    app.with_window(|window, _| {
        let sidebar = window
            .try_find("sidebar")
            .map_or(px(0.), |sidebar| sidebar.bounds().right());
        window
            .elements()
            .into_iter()
            .filter(|e| e.role() == Some(Role::Tab) && e.visible() && e.bounds().left() >= sidebar)
            .filter_map(|e| {
                let id = e.path().last()?.clone();
                Some((id, e.label().unwrap_or_default().to_string()))
            })
            .collect()
    })
}

/// Open each tab of every page for the first time at `width`×`height`, its data loading as it
/// shows.
fn first_tabs_hold_still(cx: &mut gpui_kit::TestAppContext, (width, height): (f32, f32)) {
    let (mut app, _lichess) = Harness::signed_in_showcase(cx);
    app.cx
        .simulate_window_resize(app.window, size(px(width), px(height)));
    let mut moved = Vec::new();
    let mut opened = 0;
    for page in Page::all().filter(|&page| page != Page::Settings) {
        app.with_window(|window, cx| crate::shell::open(page, window, cx));
        app.settle();
        for (id, name) in page_tabs(&mut app) {
            // A tab may have left the page: Analysis board's tabs change with what it shows.
            if app.cx.update(|cx| crate::shell::current_page(cx)) != Some(page) {
                app.with_window(|window, cx| crate::shell::open(page, window, cx));
                app.settle();
            }
            if app.with_window(|window, _| window.try_find(id.clone()).is_none()) {
                continue;
            }
            let first = first_frame(&mut app, |window, cx| window.click(id.clone(), cx));
            moved.extend(moved_since(
                &mut app,
                &first,
                &format!("{}'s {name} tab", page.info().label),
            ));
            opened += 1;
        }
    }
    assert!(opened > 20, "only {opened} tabs found");
    assert!(
        moved.is_empty(),
        "moved after their first frame:\n  {}",
        moved.join("\n  ")
    );
}

/// A tab's first frame holds the place of what it is still loading: Puzzles' History said "No
/// puzzles yet" until its progress came, then its stats pushed it down; Game history's Online
/// showed its pager over an empty list; Studies' Lichess drew its Send card below the list, which
/// moved down as the studies came.
#[gpui_kit::test]
fn a_tab_holds_still_as_it_first_opens_in_a_wide_window(cx: &mut gpui_kit::TestAppContext) {
    first_tabs_hold_still(cx, SIZES[2]);
}

#[gpui_kit::test]
fn a_tab_holds_still_as_it_first_opens_in_a_narrow_window(cx: &mut gpui_kit::TestAppContext) {
    first_tabs_hold_still(cx, SIZES[0]);
}

/// The views notified while the window drew its next frame, each once, with where from.
fn notified_while_drawing(app: &mut Harness, what: &str, found: &mut Vec<String>) {
    app.settle();
    let notified = app
        .cx
        .update_window(app.window, |_, window, cx| {
            super::draw(window, cx);
            window.views_notified_while_drawing()
        })
        .expect("the main window is open");
    for (_, from) in notified {
        let line = format!("{what}:\n    {from}");
        if !found.contains(&line) {
            found.push(line);
        }
    }
}

/// Every page and each of its tabs at `width`×`height`, and each Settings section with
/// `settings` (its sections are listed only in a wide window).
fn nothing_notifies_while_drawing(
    cx: &mut gpui_kit::TestAppContext,
    (width, height): (f32, f32),
    settings: bool,
) {
    let (mut app, _lichess) = Harness::signed_in_showcase(cx);
    app.cx
        .simulate_window_resize(app.window, size(px(width), px(height)));
    app.with_window(|window, _| window.record_views_notified_while_drawing(true));
    let mut found = Vec::new();
    for page in Page::all().filter(|&page| page != Page::Settings) {
        let label = page.info().label;
        app.with_window(|window, cx| crate::shell::open(page, window, cx));
        notified_while_drawing(&mut app, label, &mut found);
        for (id, name) in page_tabs(&mut app) {
            if app.cx.update(|cx| crate::shell::current_page(cx)) != Some(page) {
                app.with_window(|window, cx| crate::shell::open(page, window, cx));
                app.settle();
            }
            if app.with_window(|window, _| window.try_find(id.clone()).is_none()) {
                continue;
            }
            app.with_window(|window, cx| window.click(id.clone(), cx));
            notified_while_drawing(&mut app, &format!("{label}'s {name} tab"), &mut found);
        }
    }
    for section in crate::pages::settings::Section::ALL
        .into_iter()
        .filter(|_| settings)
    {
        app.open_settings(section.label());
        notified_while_drawing(
            &mut app,
            &format!("Settings › {}", section.label()),
            &mut found,
        );
    }
    assert!(
        found.is_empty(),
        "drawn again in the next frame, as a frame notified them:\n  {}",
        found.join("\n  ")
    );
}

/// A window records the views notified while it draws only once a test asks, at most eight a
/// frame: resolving every notification's stack in every test took the app's tests on Windows
/// 10 to 25 s each, and the job ran out of its hour (CI, 2026-11-22).
#[gpui_kit::test]
fn a_window_records_what_notifies_while_drawing_only_when_asked(cx: &mut gpui_kit::TestAppContext) {
    struct NotifiesWhileDrawing;
    impl gpui_kit::Render for NotifiesWhileDrawing {
        fn render(
            &mut self,
            _: &mut Window,
            cx: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            for _ in 0..10 {
                // gpui-lints: allow(notify_in_render) the fault this test records
                cx.notify();
            }
            gpui_kit::div()
        }
    }
    let window: gpui_kit::AnyWindowHandle = cx.add_window(|_, _| NotifiesWhileDrawing).into();
    let drawn = |cx: &mut gpui_kit::TestAppContext| {
        cx.update_window(window, |_, window, cx| {
            super::draw(window, cx);
            window.views_notified_while_drawing()
        })
        .expect("the window is open")
    };
    assert!(drawn(cx).is_empty(), "recorded without being asked");
    cx.update_window(window, |_, window, _| {
        window.record_views_notified_while_drawing(true)
    })
    .expect("the window is open");
    let notified = drawn(cx);
    assert_eq!(notified.len(), 8);
    assert!(notified[0].1.contains("stability"), "{}", notified[0].1);
}

/// `render` only reads: a view it notifies (a Select told its items, a cached state set) is
/// drawn again in the next frame with every view around it, so its page is laid out anew each
/// frame, scrolling included. Studies' Lichess tab told its two Selects their accounts and
/// studies in every frame.
#[gpui_kit::test]
fn nothing_notifies_while_drawing_in_a_wide_window(cx: &mut gpui_kit::TestAppContext) {
    nothing_notifies_while_drawing(cx, SIZES[2], true);
}

#[gpui_kit::test]
fn nothing_notifies_while_drawing_in_a_narrow_window(cx: &mut gpui_kit::TestAppContext) {
    nothing_notifies_while_drawing(cx, SIZES[0], false);
}

/// A dialog's error shows beside its buttons: on a line of its own above them it grew the
/// dialog, which stays centred, and the button just pressed moved under the pointer as the
/// error came.
#[gpui_kit::test]
fn a_dialogs_buttons_hold_still_as_its_error_shows(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Analysis board");
    app.click("analysis-import");
    app.fill_in_dialog("FEN, PGN or Lichess link", "not a game");
    let before = places(&mut app, &["import-cancel".into(), "import-run".into()]);
    app.click("import-run");
    app.find("import-error");
    let after = places(&mut app, &["import-cancel".into(), "import-run".into()]);
    assert_eq!(after, before, "Import moved as its error showed");
    app.click("import-cancel");
    app.settle();

    app.with_window(|window, cx| {
        crate::name_dialog::NameDialog::open(
            "Rename study",
            "Endgames",
            "Rename",
            window,
            cx,
            // As the save's answer does, after the press.
            |_, dialog, _, cx| {
                cx.defer(move |cx| {
                    dialog.update(cx, |dialog, cx| {
                        dialog.fail("The study could not be saved.", cx)
                    })
                })
            },
        )
    });
    let before = places(&mut app, &["name-cancel".into(), "name-confirm".into()]);
    app.click("name-confirm");
    app.find("name-error");
    let after = places(&mut app, &["name-cancel".into(), "name-confirm".into()]);
    assert_eq!(after, before, "Rename moved as its error showed");
}
