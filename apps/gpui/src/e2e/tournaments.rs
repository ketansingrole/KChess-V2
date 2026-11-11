//! A Lichess tournament, signed in to the stand-in Lichess.

use gpui_kit::test::TestWindowExt as _;

use super::Harness;
use super::fake_lichess::ARENA;

/// Return in the entry code joins the event with that code, as the Join button does.
#[gpui_kit::test]
fn return_in_the_entry_code_joins(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = Harness::signed_in(cx, false);
    app.with_window(|window, cx| crate::pages::tournaments::open_tournament(ARENA, window, cx));
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
    app.with_window(|window, cx| crate::pages::tournaments::open_tournament(ARENA, window, cx));
    app.find("tournament-join");
    app.press("escape");
    app.wait_gone("tournament-join");
    app.find(("tournaments-filter", 0usize));
}

/// A tournament left before its details came: they arrive late, while another is open, and are
/// not shown as that one's (each tournament opened counts its requests from 1).
#[gpui_kit::test]
fn a_late_reply_for_another_tournament_is_not_shown(cx: &mut gpui_kit::TestAppContext) {
    use super::fake_lichess::{
        HELD_ARENA, HELD_ARENA_ASKED, HELD_ARENA_B, HELD_ARENA_B_ASKED, HELD_ARENA_B_RELEASED,
        HELD_ARENA_RELEASED,
    };
    use std::sync::atomic::Ordering;
    let mut app = Harness::start(cx);
    app.online();
    let shown = |app: &mut Harness| {
        app.with_window(|_, cx| {
            crate::shell::view::<crate::pages::tournaments::TournamentsPage>(cx,
            )
            .and_then(|page| page.read(cx).shown_name())
        })
    };
    app.with_window(|window, cx| crate::pages::tournaments::open_tournament(HELD_ARENA, window, cx));
    app.wait_for("the first arena to be asked for", |_, _| {
        HELD_ARENA_ASKED.load(Ordering::SeqCst)
    });
    app.with_window(|window, cx| crate::pages::tournaments::open_tournament(HELD_ARENA_B, window, cx));
    // The first arena's details come while the second is open.
    HELD_ARENA_RELEASED.store(true, Ordering::SeqCst);
    app.wait_for("the second arena to be asked for", |_, _| {
        HELD_ARENA_B_ASKED.load(Ordering::SeqCst)
    });
    let mut frames = 0;
    app.wait_for("the first arena's reply to be handled", |_, _| {
        frames += 1;
        frames > 120
    });
    let late = shown(&mut app);
    HELD_ARENA_B_RELEASED.store(true, Ordering::SeqCst);
    assert_ne!(
        late.as_deref(),
        Some("Late Arena"),
        "the first arena shown as the second"
    );
    app.wait_for("the second arena's details", |_, cx| {
        crate::shell::view::<crate::pages::tournaments::TournamentsPage>(cx,
        )
        .and_then(|page| page.read(cx).shown_name())
        .as_deref()
            == Some("Second Arena")
    });
}
