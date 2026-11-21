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
