//! Practice, offline: the board-vision drills and the trainers that need no account.

use super::Harness;
use crate::pages::Page;

fn practice(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Practice");
    app
}

/// Open a tab of the page ("Knight paths" is 2).
fn tab(app: &mut Harness, index: usize) {
    app.click(("practice-tabs", index));
    assert_eq!(app.find(("practice-tabs", index)).selected(), Some(true));
}

fn page(app: &mut Harness) -> Option<Page> {
    app.cx.update(|cx| crate::shell::current_page(cx))
}

/// Wait until the element's accessible label is `label`.
fn wait_for_label(app: &mut Harness, id: &'static str, label: &str) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for(&format!("{id} to read {label:?}"), |window, _| {
        window
            .try_find(id)
            .is_some_and(|element| element.label() == Some(label))
    });
}

/// Scroll `list` until `id` is in view.
fn reveal(app: &mut Harness, list: &'static str, id: &'static str) {
    use gpui_kit::test::TestWindowExt as _;
    for _ in 0..30 {
        if app.shows(id) {
            return;
        }
        app.with_window(|window, cx| {
            let by = gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-300.));
            window.scroll(list, gpui_kit::ScrollDelta::Pixels(by), cx);
        });
    }
    app.find(id);
}

/// Whether a square is light, as a name like "e4" says it.
fn is_light(square: &str) -> bool {
    let bytes = square.as_bytes();
    (bytes[0] - b'a' + bytes[1] - b'1') % 2 == 1
}

#[gpui_kit::test]
fn every_trainer_opens_without_an_account(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    assert_eq!(app.find(("practice-tabs", 0usize)).selected(), Some(true));
    app.find("coordinates-start");
    tab(&mut app, 1);
    app.find("square-colors-start");
    tab(&mut app, 2);
    app.find("knight-start");
    tab(&mut app, 3);
    app.find("card-checkmate-patterns-1");
    tab(&mut app, 4);
    app.find("open-studies");
    tab(&mut app, 5);
    app.find("review-a-game");
}

#[gpui_kit::test]
fn a_coordinates_run_scores_the_clicked_square(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    app.click("coordinates-start");
    let target = app.find("coordinates-target").label().unwrap().to_string();
    assert_eq!(target.len(), 2, "a square is asked for: {target:?}");
    assert_eq!(app.find("run-score").label(), Some("0"));

    // A wrong square scores nothing.
    let wrong = if target == "a1" { "h8" } else { "a1" };
    app.click_square(wrong, true);
    assert_eq!(app.find("run-score").label(), Some("0"));

    let target = app.find("coordinates-target").label().unwrap().to_string();
    app.click_square(&target, true);
    wait_for_label(&mut app, "run-score", "1");

    app.click("coordinates-end");
    assert_eq!(app.status(), "1 correct · 50% accuracy");
    assert!(app.enabled("coordinates-start"));
}

#[gpui_kit::test]
fn a_coordinates_run_can_be_limited_to_some_files_and_ranks(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    app.click("coordinates-limited");
    assert_eq!(app.find("coordinates-limited").checked(), Some(true));
    for file in "bcdefgh".chars() {
        app.click(format!("kchess:coordinates-files-{file}"));
    }
    for rank in "2345678".chars() {
        app.click(format!("kchess:coordinates-ranks-{rank}"));
    }
    app.click("coordinates-start");
    // Only a1 is left, so every question is a1.
    assert_eq!(app.find("coordinates-target").label(), Some("a1"));
    app.click_square("a1", true);
    wait_for_label(&mut app, "run-score", "1");
    assert_eq!(app.find("coordinates-target").label(), Some("a1"));
    app.click("coordinates-end");
    assert_eq!(app.status(), "1 correct · 100% accuracy");
}

#[gpui_kit::test]
fn the_square_colour_drill_scores_keys_and_buttons(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 1);
    app.click("square-colors-start");
    assert_eq!(app.find("square-score").label(), Some("0 correct"));

    // The right key counts.
    let square = app.find("square-prompt").label().unwrap().to_string();
    app.press(if is_light(&square) { "l" } else { "d" });
    wait_for_label(&mut app, "square-score", "1 correct");

    // The wrong button does not.
    let square = app.find("square-prompt").label().unwrap().to_string();
    app.click(if is_light(&square) {
        "answer-dark"
    } else {
        "answer-light"
    });
    assert_eq!(app.find("square-score").label(), Some("1 correct"));

    // The right button does.
    let square = app.find("square-prompt").label().unwrap().to_string();
    app.click(if is_light(&square) {
        "answer-light"
    } else {
        "answer-dark"
    });
    wait_for_label(&mut app, "square-score", "2 correct");
}

#[gpui_kit::test]
fn a_knight_run_asks_for_a_path_and_a_skip_shows_the_shortest(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 2);
    app.click("knight-start");
    let target = app.find("knight-target").label().unwrap().to_string();
    assert!(target.starts_with("Reach "), "{target:?}");
    app.click("knight-skip");
    let notice = app.find("knight-notice").label().unwrap().to_string();
    assert!(
        notice.starts_with("Skipped — the shortest path was"),
        "{notice:?}"
    );
    app.click("knight-end");
    assert_eq!(app.status(), "0 shortest paths · 0 reached");
    assert!(app.enabled("knight-start"));
}

#[gpui_kit::test]
fn endgame_drills_open_and_return_to_the_courses(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 3);
    reveal(&mut app, "endgame-courses", "card-drills");
    app.click("card-drills");
    app.find("chess-board");
    app.find("drill-hint");
    assert!(!app.shows("card-drills"));
    app.click("drills-back");
    app.find("endgame-courses");
    assert!(!app.shows("chess-board"));
}

/// F turns an endgame drill's board, as its button does.
#[gpui_kit::test]
fn f_flips_an_endgame_drill(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 3);
    reveal(&mut app, "endgame-courses", "card-drills");
    app.click("card-drills");
    app.find("drill-hint");
    let flipped = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view_of::<crate::pages::practice::PracticePage>(Page::Practice, cx)
                .and_then(|page| page.read(cx).drills(cx))
                .map(|drills| drills.read(cx).flipped())
        })
    };
    assert_eq!(flipped(&mut app), Some(false));
    app.click("chess-board");
    app.press("f");
    assert_eq!(flipped(&mut app), Some(true));
}

/// An endgame drill's buttons are in Quick Pick, those it allows now.
#[gpui_kit::test]
fn an_endgame_drill_is_flipped_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = practice(cx);
    tab(&mut app, 3);
    reveal(&mut app, "endgame-courses", "card-drills");
    app.click("card-drills");
    app.find("drill-hint");
    app.press("secondary-k");
    app.type_text(">");
    super::palette::wait_for_first(&mut app, "Hint");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..6usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    // Nothing was played, so there is nothing to take back.
    assert_eq!(&listed[..3], ["Hint", "Flip board", "Restart"]);
    app.type_text("Flip board");
    super::palette::wait_for_first(&mut app, "Flip board");
    app.press("enter");
    super::palette::wait_until_closed(&mut app);
    let flipped = app.cx.update(|cx| {
        crate::shell::view_of::<crate::pages::practice::PracticePage>(Page::Practice, cx)
            .and_then(|page| page.read(cx).drills(cx))
            .map(|drills| drills.read(cx).flipped())
    });
    assert_eq!(flipped, Some(true));
}

#[gpui_kit::test]
fn a_checkmate_lesson_takes_back_a_wrong_move_and_accepts_the_mate(
    cx: &mut gpui_kit::TestAppContext,
) {
    let mut app = practice(cx);
    tab(&mut app, 3);
    app.click("card-checkmate-patterns-2");
    app.click("training-cp2-balestra");
    app.click("lesson-next");
    assert_eq!(app.status(), "Your turn");
    app.drag_square("d3", "c4", true);
    wait_for_label(&mut app, "status-banner", "Not the move — try again");
    // The refused bishop is back on d3 to make the mate.
    app.drag_square("d3", "b5", true);
    wait_for_label(&mut app, "status-banner", "Checkmate");
    app.click("lesson-back");
    app.find("card-checkmate-patterns-2");
}

/// A lesson's buttons are in Quick Pick: its steps are gone through from there.
#[gpui_kit::test]
fn a_lesson_is_stepped_through_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 3);
    app.click("card-checkmate-patterns-2");
    app.click("training-cp2-balestra");
    app.find("lesson-next");
    let run = |app: &mut Harness, label: &str| {
        app.press("secondary-k");
        app.type_text(&format!(">{label}"));
        super::palette::wait_for_first(app, label);
        app.press("enter");
        super::palette::wait_until_closed(app);
    };
    run(&mut app, "Next");
    assert_eq!(app.status(), "Your turn");
    run(&mut app, "Previous step");
    app.wait_gone("lesson-previous");
    run(&mut app, "All courses");
    app.find("card-checkmate-patterns-2");
}

#[gpui_kit::test]
fn an_empty_repertoire_and_empty_mistakes_lead_elsewhere(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 4);
    assert!(app.enabled("browse-openings"));
    app.click("open-studies");
    assert_eq!(page(&mut app), Some(Page::Studies));

    app.open("Practice");
    tab(&mut app, 5);
    assert!(!app.shows("practice-due"));
    app.click("review-a-game");
    assert_eq!(page(&mut app), Some(Page::History));
}

/// A mistake from a Crazyhouse game is drilled by Crazyhouse's rules: the pockets are beside the
/// board, and the answer is a drop.
#[gpui_kit::test]
fn a_crazyhouse_mistake_is_solved_with_a_drop(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 5);
    app.find("review-a-game");
    let mistakes = app.cx.update(|cx| {
        crate::shell::view_of::<crate::pages::practice::PracticePage>(Page::Practice, cx)
            .and_then(|page| page.read(cx).mistakes())
            .expect("the mistakes trainer")
    });
    app.cx.update(|cx| {
        mistakes.update(cx, |mistakes, cx| {
            mistakes.seed(
                vec![serde_json::json!({
                    "id": "zh-drop",
                    "fen": "rnb1kbnr/ppp1pppp/8/q7/8/2N5/PPPP1PPP/R1BQKBNR[Pp] w KQkq - 2 4",
                    "solution": ["P@d4"],
                    "variant": "crazyhouse",
                    "dueAt": 0,
                })],
                cx,
            )
        });
    });
    app.click("practice-due");
    app.click("pocket-White-Pawn");
    app.click_square("d4", true);
    let status = |app: &mut Harness| {
        app.cx
            .update(|cx| mistakes.read(cx).board().read(cx).status().to_string())
    };
    app.wait_for("the drop to solve the drill", |_, cx| {
        mistakes.read(cx).board().read(cx).status() == "solved"
    });
    assert_eq!(status(&mut app), "solved");
}

/// A saved mistake is practised from Quick Pick, as the trainer's buttons do.
#[gpui_kit::test]
fn a_mistake_is_practised_from_quick_pick(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = practice(cx);
    tab(&mut app, 5);
    app.find("review-a-game");
    app.cx.update(|cx| {
        let mistakes =
            crate::shell::view_of::<crate::pages::practice::PracticePage>(Page::Practice, cx)
                .and_then(|page| page.read(cx).mistakes())
                .expect("the mistakes trainer");
        mistakes.update(cx, |mistakes, cx| {
            mistakes.seed(
                vec![serde_json::json!({
                    "id": "back-rank",
                    "fen": "6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1",
                    "solution": ["a1a8"],
                    "dueAt": 0,
                })],
                cx,
            )
        });
    });
    let run = |app: &mut Harness, label: &str| {
        app.press("secondary-k");
        app.type_text(&format!(">{label}"));
        super::palette::wait_for_first(app, label);
        app.press("enter");
        super::palette::wait_until_closed(app);
    };
    run(&mut app, "Practice due positions");
    app.find("mistake-solution");
    app.press("secondary-k");
    app.type_text(">");
    super::palette::wait_for_first(&mut app, "Hint: piece to move");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..6usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    assert_eq!(
        &listed[..4],
        [
            "Hint: piece to move",
            "Show solution",
            "Next due position",
            "Review early"
        ]
    );
}

/// A solved mistake's board answers the keys as a puzzle's: the arrows step back through its
/// moves, and F turns it. A click on the board gives the keys back to the page.
#[gpui_kit::test]
fn a_solved_mistake_is_stepped_through_and_turned(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 5);
    app.find("review-a-game");
    let mistakes = app.cx.update(|cx| {
        crate::shell::view_of::<crate::pages::practice::PracticePage>(Page::Practice, cx)
            .and_then(|page| page.read(cx).mistakes())
            .expect("the mistakes trainer")
    });
    app.cx.update(|cx| {
        mistakes.update(cx, |mistakes, cx| {
            mistakes.seed(
                vec![serde_json::json!({
                    "id": "mate",
                    "fen": "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
                    "solution": ["h5f7"],
                    "dueAt": 0,
                })],
                cx,
            )
        });
    });
    app.click("practice-due");
    app.click_square("h5", true);
    app.click_square("f7", true);
    app.wait_for("the mate to solve the drill", |_, cx| {
        mistakes.read(cx).board().read(cx).status() == "solved"
    });
    app.click("chess-board");
    app.press("left");
    assert_eq!(
        app.cx
            .update(|cx| mistakes.read(cx).board().read(cx).viewing()),
        Some(0)
    );
    app.press("f");
    assert!(
        app.cx
            .update(|cx| mistakes.read(cx).board().read(cx).flipped())
    );
}

/// Once a mistake is solved, Return goes on to the next due one, as on Puzzles.
#[gpui_kit::test]
fn return_goes_on_to_the_next_due_mistake(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 5);
    app.find("review-a-game");
    let mistakes = app.cx.update(|cx| {
        crate::shell::view_of::<crate::pages::practice::PracticePage>(Page::Practice, cx)
            .and_then(|page| page.read(cx).mistakes())
            .expect("the mistakes trainer")
    });
    let mate = |id: &str| {
        serde_json::json!({
            "id": id,
            "fen": "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
            "solution": ["h5f7"],
            "dueAt": 0,
        })
    };
    app.cx.update(|cx| {
        mistakes.update(cx, |mistakes, cx| {
            mistakes.seed(vec![mate("first"), mate("second")], cx)
        })
    });
    app.click("practice-due");
    app.click("chess-board");
    // Not while it is being solved.
    app.press("enter");
    assert_eq!(
        app.cx
            .update(|cx| mistakes.read(cx).board().read(cx).status().to_string()),
        "playing"
    );
    app.click_square("h5", true);
    app.click_square("f7", true);
    app.wait_for("the mate to solve the drill", |_, cx| {
        mistakes.read(cx).board().read(cx).status() == "solved"
    });
    // The answer is saved in the core, whose drills (none: these were seeded here) come back;
    // seed the other one again once they have.
    app.wait_for("the answer to be saved", |_, cx| {
        !mistakes
            .read(cx)
            .actions()
            .contains(&crate::pages::practice::mistakes::MistakeAction::Next)
    });
    app.cx
        .update(|cx| mistakes.update(cx, |mistakes, cx| mistakes.seed(vec![mate("second")], cx)));
    app.click("chess-board");
    app.press("enter");
    app.wait_for("the next mistake", |_, cx| {
        mistakes.read(cx).board().read(cx).status() == "playing"
    });
}

/// Return goes on through a lesson: past a step to read, and past a step once it is passed, but
/// not while it is being played.
#[gpui_kit::test]
fn return_goes_on_through_a_lesson(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 3);
    app.click("card-checkmate-patterns-2");
    app.click("training-cp2-balestra");
    app.find("lesson-next");
    app.click("chess-board");
    app.press("enter");
    assert_eq!(app.status(), "Your turn");
    app.press("enter");
    assert_eq!(app.status(), "Your turn", "not while it is being played");
    app.drag_square("d3", "b5", true);
    wait_for_label(&mut app, "status-banner", "Checkmate");
    app.click("chess-board");
    app.press("enter");
    app.wait_for("the next chapter", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("status-banner")
            .is_none_or(|banner| banner.label() != Some("Checkmate"))
    });
}

/// Guess the move: a pasted game is guessed through as White, each guess scored against the
/// move played and otherwise by the scripted engine (+31 for whoever is to move, so a guess it
/// does not list loses 62 centipawns: playable, one point), with a total at the end.
#[gpui_kit::test]
fn a_pasted_game_is_guessed_through_and_scored(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    tab(&mut app, 6);
    assert!(!app.enabled("guess-start"));
    app.click_label("Game PGN");
    app.type_text("[White \"Morphy\"]\n[Black \"Duke\"]\n\n1. e4 e5 2. Nf3 Nc6 3. Bb5 a6 *");
    app.click("guess-start");
    app.find("chess-board");
    wait_for_label(&mut app, "guess-score", "0 points · 0 of 0 matched");

    // The move played scores full points, and Black's reply follows.
    app.drag_square("e2", "e4", true);
    wait_for_label(&mut app, "status-banner", "e4 ✓");
    app.wait_for_moves(&["e4", "e5"]);
    wait_for_label(&mut app, "guess-score", "5 points · 1 of 1 matched");

    // Another move is judged by the engine.
    app.drag_square("d2", "d4", true);
    wait_for_label(&mut app, "status-banner", "Played Nf3");
    app.wait_for_moves(&["e4", "e5", "Nf3", "Nc6"]);
    wait_for_label(&mut app, "guess-score", "6 points · 1 of 2 matched");

    // The last guess ends the game with how close it came.
    app.drag_square("f1", "b5", true);
    app.wait_for_moves(&["e4", "e5", "Nf3", "Nc6", "Bb5", "a6"]);
    wait_for_label(&mut app, "status-banner", "Close to the master");
    wait_for_label(&mut app, "guess-score", "11 points · 2 of 3 matched");

    // Again starts the same game over; Another game goes back to choosing one.
    app.click("guess-restart");
    wait_for_label(&mut app, "guess-score", "0 points · 0 of 0 matched");
    app.click("guess-another");
    app.find("guess-start");
}

/// A game opened for guessing elsewhere (the Masters explorer) lands on the tab, as the side asked
/// for, and a PGN that is not a game is refused.
#[gpui_kit::test]
fn a_game_opened_for_guessing_starts_as_its_side(cx: &mut gpui_kit::TestAppContext) {
    let mut app = practice(cx);
    app.with_window(|window, cx| {
        crate::shell::guess_moves("1. d4 d5 2. c4 *", Some(shakmaty::Color::Black), window, cx)
    });
    assert_eq!(app.find(("practice-tabs", 6usize)).selected(), Some(true));
    // White's first move is played for Black to answer.
    app.wait_for_moves(&["d4"]);
    app.drag_square("d7", "d5", false);
    wait_for_label(&mut app, "status-banner", "d5 ✓");
    app.wait_for_moves(&["d4", "d5", "c4"]);
    wait_for_label(&mut app, "status-banner", "Played like the master");

    app.click("guess-another");
    app.click_label("Game PGN");
    app.type_text("not a game");
    app.click("guess-start");
    wait_for_label(&mut app, "guess-error", "That game has no moves");
}
