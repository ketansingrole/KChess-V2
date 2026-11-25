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

/// 2026-11-21: the board reopened its game at launch without looking up the game's review, so
/// the kept review showed only when something else looked: the review queue's first report, 10 s
/// after launch. The board finds it before the queue has said anything.
#[gpui_kit::test]
fn a_finished_review_is_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = scholars_mate(cx);
    app.click("review-start");
    wait_for_review(&mut app);

    let mut app = app.restart();
    app.open("Analysis board");
    wait_for_review(&mut app);
    let reports = app.with_window(|_, cx| {
        let reviews = cx.global::<crate::reviews::ReviewHandle>().0.clone();
        reviews.read(cx).status_events()
    });
    assert_eq!(
        reports, 0,
        "the review showed only after the queue reported"
    );
}

/// 2026-11-20: the board asked for its review while the check for a Lichess game had not
/// answered (refused 39 times in a development log), and never asked again: once the check
/// failed, the review stayed missing until the moves changed.
#[gpui_kit::test]
fn a_review_held_back_by_the_startup_check_shows_once_it_ends(cx: &mut gpui_kit::TestAppContext) {
    let (mut app, lichess) = Harness::signed_in(cx, false);
    app.wait_for("engine help", |_, cx| {
        crate::online::handle(cx).read(cx).assistance_allowed()
    });
    app.open("Analysis board");
    super::analysis::import(&mut app, SCHOLARS_MATE);
    app.click("Review");
    app.click("review-start");
    wait_for_review(&mut app);

    // The check on the next launch fails: the board's lookup is held back, not refused.
    lichess.lock().fail_check = true;
    let mut app = app.restart();
    app.wait_for("the check to fail", |_, cx| {
        crate::online::handle(cx).read(cx).pause() == Some(crate::online::Pause::Unverified)
    });
    app.open("Analysis board");
    // The review queue reports its status meanwhile (it looks for work 10 s after launch): the
    // board follows the reviews again, and asked for its review then, to be refused.
    app.with_window(|_, cx| {
        let reviews = cx.global::<crate::reviews::ReviewHandle>().0.clone();
        reviews.update(cx, |_, cx| cx.notify());
    });
    app.settle();
    assert!(
        !app.shows("practice-review-mistakes"),
        "held back until Lichess answers"
    );
    lichess.lock().fail_check = false;
    app.click("assistance-check-again");
    wait_for_review(&mut app);
}

/// A Crazyhouse game is reviewed in Crazyhouse, drops and all: the review is asked for in its
/// variant and kept under it, and its mistakes are offered for practice.
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
    let page = |cx: &gpui_kit::App| crate::shell::view::<crate::pages::analysis::AnalysisPage>(cx);
    app.wait_for("the review to finish", |window, cx| {
        window.try_find("review-stop").is_none()
            && window.try_find("review-start").is_none()
            && page(cx).is_some_and(|page| page.read(cx).finished_review(cx).is_some())
    });
    let (review, analysis) = app
        .with_window(|_, cx| page(cx).and_then(|page| page.read(cx).finished_review(cx)))
        .expect("the finished review");
    assert_eq!(review.variant.as_deref(), Some("crazyhouse"), "{review:?}");
    assert_eq!(review.moves.len(), 5, "{review:?}");
    // Every move, the drop too, is judged.
    assert_eq!(analysis.moves.len(), 5, "{analysis:?}");
    // Its mistakes may be practised, in Crazyhouse (`store::library` and `e2e::practice` play
    // them; the scripted engine's lines are not legal here, so it makes no drill).
    assert!(app.shows("practice-review-mistakes"));
}

/// With the endgame tablebases on this computer, a review judges five pieces or fewer by them:
/// leaving the rook to be taken in a won KR vs K is the blunder that draws it, whatever the
/// engine would have said.
#[gpui_kit::test]
fn an_endgame_is_reviewed_by_the_tablebases(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = Harness::start_with(cx, super::practice::with_tablebases);
    app.open("Analysis board");
    super::analysis::import(
        &mut app,
        "[SetUp \"1\"]\n[FEN \"8/8/8/8/8/8/1k6/R3K3 w - - 0 1\"]\n\n1. Ra2+ Kxa2 1/2-1/2",
    );
    app.wait_gone("import-run");
    app.click("Review");
    app.click("review-start");
    let page = |cx: &gpui_kit::App| crate::shell::view::<crate::pages::analysis::AnalysisPage>(cx);
    app.wait_for("the review to finish", |window, cx| {
        window.try_find("review-stop").is_none()
            && window.try_find("review-start").is_none()
            && page(cx).is_some_and(|page| page.read(cx).finished_review(cx).is_some())
    });
    let (review, analysis) = app
        .with_window(|_, cx| page(cx).and_then(|page| page.read(cx).finished_review(cx)))
        .expect("the finished review");
    let first = review.evals[0].clone().expect("the start's score");
    assert!(first.tablebase, "{first:?}");
    assert_eq!(
        analysis.moves[0].judgment,
        Some(kchess_domain::records::review::Judgment::Blunder),
        "{analysis:?} {review:?}"
    );
    // The move's mark says what it gave away, and the tables' move that kept the win.
    app.click("Moves");
    let path = crate::game::tree::path_of(&["a1a2".to_string()]);
    let said = app
        .find(format!("label-{path}"))
        .label()
        .map(str::to_string)
        .unwrap_or_default();
    assert!(
        said.starts_with("Blunder: throws away the win · R") && said.ends_with(" wins (tablebase)"),
        "{said}"
    );
}
