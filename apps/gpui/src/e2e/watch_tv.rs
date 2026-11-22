//! Watch → TV: a featured game's moves show as soon as Lichess's export answers, though the
//! export leaves out the last three plies (`fake_lichess::tv` does as Lichess does).

use super::Harness;
use super::fake_lichess::TV_CLASSICAL_EXPORTED;
use std::sync::atomic::Ordering;

fn watch(app: &mut Harness, channel: &'static str) {
    app.with_window(|window, cx| crate::pages::watch::open_channel(channel.into(), window, cx));
    app.find("watch-stop");
}

/// Blitz's export stops three plies short of its board, as Lichess's always does for a game
/// being played: the moves in between are worked out from the board, so the whole list shows
/// at once instead of "Loading moves…" until three more moves were played.
#[gpui_kit::test]
fn a_tv_game_lists_every_move_though_the_export_leaves_out_the_last_three(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = Harness::start(cx);
    watch(&mut app, "blitz");
    // 1. e4 d5 2. exd5 Qxd5 3. Nc3: five moves, the last one on the board.
    app.find(("move", 4usize));
    assert!(!app.shows(("move", 5usize)));
    assert!(!app.shows("watch-moves-behind"));
    assert!(app.enabled("watch-analyse"));
}

/// Bullet's last three plies could have come in either order: the moves before them show, with
/// a line saying the last ones come after the next move.
#[gpui_kit::test]
fn a_tv_game_whose_last_moves_are_unknown_lists_the_rest(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    watch(&mut app, "bullet");
    app.find("watch-moves-behind");
    app.find(("move", 2usize));
    assert!(!app.shows(("move", 3usize)));
    // No listed move stands for the board, which is ahead of them; stepping back shows them.
    assert_eq!(app.find(("move", 2usize)).selected(), Some(false));
    app.click(("move", 2usize));
    assert_eq!(app.find(("move", 2usize)).selected(), Some(true));
    app.find("watch-back-live");
}

/// An export that fails says so where the moves go, with Try again, never "Loading moves…".
#[gpui_kit::test]
fn a_tv_export_that_fails_says_so_and_can_be_tried_again(cx: &mut gpui_kit::TestAppContext) {
    TV_CLASSICAL_EXPORTED.store(false, Ordering::SeqCst);
    let mut app = Harness::start(cx);
    watch(&mut app, "classical");
    app.find("watch-moves-error");
    TV_CLASSICAL_EXPORTED.store(true, Ordering::SeqCst);
    app.click("watch-moves-retry");
    app.find(("move", 1usize));
    assert!(!app.shows("watch-moves-error"));
}
