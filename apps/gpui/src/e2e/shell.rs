//! The window and its sidebar.

use super::Harness;

#[gpui_kit::test]
fn the_app_starts_on_home(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    for action in [
        "home-history",
        "home-local",
        "home-practice",
        "home-studies",
    ] {
        app.find(action);
    }
    assert!(app.enabled("connect-lichess"));
}

#[gpui_kit::test]
fn every_sidebar_page_opens(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    for (page, label, _) in crate::shell::nav() {
        app.open(label);
        let shown = app.cx.update(|cx| crate::shell::current_page(cx));
        assert_eq!(shown, Some(page), "{label} opened another page");
    }
}

/// The window hides the system title bar, so on Windows the top bar draws its controls.
#[cfg(target_os = "windows")]
#[gpui_kit::test]
fn the_top_bar_has_the_window_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    for control in ["minimize", "maximize", "close"] {
        app.find(control);
    }
}

#[gpui_kit::test]
fn the_sidebar_collapses_and_is_kept_collapsed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Hide sidebar"));
    app.click("sidebar-toggle");
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
    let mut app = app.restart();
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
}

/// Tab and Shift-Tab walk a page's controls in opposite orders, also after frames in which only
/// the shell redrew and the page's last frame, its tab stops included, was reused.
#[gpui_kit::test]
fn tab_and_shift_tab_walk_the_same_controls(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.open("Openings");
    let step = |app: &mut Harness, forward: bool| {
        app.with_window(|window, cx| {
            if forward {
                window.focus_next(cx);
            } else {
                window.focus_prev(cx);
            }
        });
        app.with_window(|window, cx| window.focused(cx))
            .expect("a control has the keys")
    };
    let forward: Vec<_> = (0..6).map(|_| step(&mut app, true)).collect();
    for (ix, focus) in forward.iter().enumerate() {
        assert!(
            !forward[..ix].contains(focus),
            "Tab came back to a control after {ix} steps"
        );
    }
    app.cx.update(crate::shell::notify_shell);
    let back: Vec<_> = (0..5).map(|_| step(&mut app, false)).collect();
    let expected: Vec<_> = forward[..5].iter().rev().cloned().collect();
    assert_eq!(back, expected, "Shift-Tab retraces Tab");
    app.cx.update(crate::shell::notify_shell);
    let again: Vec<_> = (0..5).map(|_| step(&mut app, true)).collect();
    assert_eq!(again, forward[1..], "Tab after the shell redrew");
}
