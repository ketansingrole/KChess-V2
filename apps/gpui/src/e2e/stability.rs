//! What holds still: a page's content keeps its place when its state changes under the
//! pointer, so the next click lands where the person aimed it (stepping back through a game's
//! moves, a load finishing, a status line changing).

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Bounds, ElementId, Pixels, px, size};

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

/// Turn voice on and off on `page`, its Voice button holding its place under the pointer while
/// the bar fills in beside it (here with the error a test machine's missing speech model gives,
/// and Try again).
fn voice_holds_still(app: &mut Harness, page: &str, bar: &'static str) {
    let button: ElementId = (bar, 0usize).into();
    for (width, height) in &SIZES[1..] {
        app.cx
            .simulate_window_resize(app.window, size(px(*width), px(*height)));
        app.open(page);
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
    voice_holds_still(&mut app, "Play with computer", "computer-voice");
}

#[gpui_kit::test]
fn the_voice_button_holds_still_under_the_board_editor(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    voice_holds_still(&mut app, "Board editor", "editor-voice");
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
