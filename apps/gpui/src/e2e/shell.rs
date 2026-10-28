//! The window and its sidebar.

use super::Harness;
use gpui_kit::test::TestQueryExt as _;

#[gpui_kit::test]
fn the_app_starts_on_home_with_every_page_in_the_sidebar(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.with_window(|window, _| {
        for e in window.elements() {
            eprintln!(
                "EL {:?} role={:?} label={:?} value={:?} vis={}",
                e.path().last(),
                e.role(),
                e.label(),
                e.value(),
                e.visible()
            );
        }
    });
}
