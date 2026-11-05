//! A Lichess tournament, signed in to the stand-in Lichess.

use gpui_kit::test::TestWindowExt as _;

use super::Harness;
use super::fake_lichess::ARENA;

/// Return in the entry code joins the event with that code, as the Join button does.
#[gpui_kit::test]
fn return_in_the_entry_code_joins(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = Harness::signed_in(cx, false);
    app.with_window(|window, cx| crate::shell::open_tournament(ARENA, window, cx));
    app.wait_for("the arena's Join button", |window, _| {
        window
            .try_find("tournament-join")
            .is_some_and(|join| join.disabled() != Some(true))
    });
    app.click_label("Tournament entry code");
    app.type_text("open-sesame");
    // Escape in the field leaves the event open.
    app.press("escape");
    assert!(app.shows("tournament-join"));
    app.press("enter");
    let joined = format!("POST /api/tournament/{ARENA}/join");
    app.wait_for("the join to reach Lichess", |_, _| {
        lichess
            .lock()
            .writes
            .iter()
            .any(|write| write.starts_with(&joined) && write.contains("password=open-sesame"))
    });
}

/// Escape goes back from an event to the list, as All tournaments does.
#[gpui_kit::test]
fn escape_goes_back_to_all_tournaments(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.online();
    app.with_window(|window, cx| crate::shell::open_tournament(ARENA, window, cx));
    app.find("tournament-join");
    app.press("escape");
    app.wait_gone("tournament-join");
    app.find(("tournaments-filter", 0usize));
}
