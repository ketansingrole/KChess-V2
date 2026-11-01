//! Game review on the analysis board, by the scripted engine: started, finished and kept.

use super::Harness;

const SCHOLARS_MATE: &str = "1. e4 e5 2. Qh5 Nc6 3. Bc4 Nf6 4. Qxf7# 1-0";

/// The analysis board on Scholar's mate, from an import as a person would load it.
fn scholars_mate(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Analysis board");
    super::analysis::import(&mut app, SCHOLARS_MATE);
    app.click("Review");
    app
}

/// Wait until the review has run to its end: nothing left to start, nothing left to stop.
fn wait_for_review(app: &mut Harness) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for("the review to finish", |window, _| {
        window.try_find("practice-review-mistakes").is_some()
            && window.try_find("review-stop").is_none()
            && window.try_find("review-start").is_none()
    });
}

#[gpui_kit::test]
fn a_game_is_reviewed_to_its_end(cx: &mut gpui_kit::TestAppContext) {
    let mut app = scholars_mate(cx);
    assert!(!app.shows("practice-review-mistakes"));
    app.click("review-start");
    wait_for_review(&mut app);
}

#[gpui_kit::test]
fn a_finished_review_is_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = scholars_mate(cx);
    app.click("review-start");
    wait_for_review(&mut app);

    let mut app = app.restart();
    app.open("Analysis board");
    wait_for_review(&mut app);
}
