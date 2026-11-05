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

/// A Crazyhouse game is reviewed in Crazyhouse, drops and all: the review is asked for in its
/// variant and kept under it, and no standard-chess mistake drill is offered from it.
#[gpui_kit::test]
fn a_crazyhouse_game_is_reviewed_in_its_variant(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = Harness::start(cx);
    app.open("Analysis board");
    super::analysis::import(
        &mut app,
        "[Variant \"Crazyhouse\"]\n\n1. e4 d5 2. exd5 Qxd5 3. P@e3 *",
    );
    app.wait_gone("import-run");
    app.click("Review");
    app.click("review-start");
    let page = |cx: &gpui_kit::App| {
        crate::shell::view_of::<crate::pages::analysis::AnalysisPage>(
            crate::pages::Page::Analysis,
            cx,
        )
    };
    app.wait_for("the review to finish", |window, cx| {
        window.try_find("review-stop").is_none()
            && window.try_find("review-start").is_none()
            && page(cx).is_some_and(|page| page.read(cx).finished_review(cx).is_some())
    });
    let (review, analysis) = app
        .with_window(|_, cx| page(cx).and_then(|page| page.read(cx).finished_review(cx)))
        .expect("the finished review");
    assert_eq!(review["variant"], "crazyhouse", "{review}");
    assert_eq!(
        review["moves"].as_array().map(Vec::len),
        Some(5),
        "{review}"
    );
    // Every move, the drop too, is judged.
    assert_eq!(
        analysis["moves"].as_array().map(Vec::len),
        Some(5),
        "{analysis}"
    );
    assert!(!app.shows("practice-review-mistakes"));
}
