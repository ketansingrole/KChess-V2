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

#[gpui_kit::test]
fn the_sidebar_collapses_and_is_kept_collapsed(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    assert_eq!(app.find("sidebar-toggle").label(), Some("Hide sidebar"));
    app.click("sidebar-toggle");
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
    let mut app = app.restart();
    assert_eq!(app.find("sidebar-toggle").label(), Some("Show sidebar"));
}
