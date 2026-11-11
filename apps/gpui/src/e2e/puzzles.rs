//! Puzzles with no Lichess account, Lichess out of reach and no puzzle database downloaded.

use super::Harness;

fn puzzles(cx: &mut gpui_kit::TestAppContext) -> Harness {
    let mut app = Harness::start(cx);
    app.open("Puzzles");
    app
}

/// Open a tab of the page.
fn tab(app: &mut Harness, tab: crate::pages::puzzles::Tab) {
    app.click(super::tab("puzzle-tabs", tab));
    assert_eq!(
        app.find(super::tab("puzzle-tabs", tab)).selected(),
        Some(true)
    );
}

/// Wait until a mode's pill is (not) the chosen one.
fn wait_for_mode(app: &mut Harness, index: usize, chosen: bool) {
    use gpui_kit::test::TestWindowExt as _;
    app.wait_for(&format!("mode {index} chosen: {chosen}"), |window, _| {
        window
            .try_find(("puzzle-mode", index))
            .is_some_and(|pill| pill.selected() == Some(chosen))
    });
}

#[gpui_kit::test]
fn training_offline_offers_a_retry_and_the_downloaded_puzzles(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    wait_for_mode(&mut app, 0, true);
    assert!(app.enabled("try-again"));
    app.click("try-again");
    app.find("try-again");

    // Without a database the downloaded puzzles ask for it.
    app.click("error-offline");
    wait_for_mode(&mut app, 1, true);
    assert!(app.enabled("db-install"));
    assert!(!app.shows("chess-board"));

    // Back to Lichess practice, which is still unreachable.
    app.click(("puzzle-mode", 0usize));
    app.find("try-again");
}

#[gpui_kit::test]
fn rated_puzzles_ask_for_an_account_and_offer_another_mode(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    // The headless text is wide: the last mode needs the room the sidebar takes to be clicked.
    app.click("sidebar-toggle");
    app.click(("puzzle-mode", 2usize));
    wait_for_mode(&mut app, 2, true);
    app.click("other-mode");
    wait_for_mode(&mut app, 2, false);
    assert!(!app.shows("other-mode"));
}

#[gpui_kit::test]
fn the_daily_puzzle_offline_can_fall_back_to_downloaded_puzzles(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, crate::pages::puzzles::Tab::Daily);
    assert!(app.enabled("daily-retry"));
    app.click("daily-downloaded");
    wait_for_mode(&mut app, 1, true);
    assert_eq!(
        app.find(super::tab("puzzle-tabs", crate::pages::puzzles::Tab::Train))
            .selected(),
        Some(true)
    );
    app.find("db-install");
}

#[gpui_kit::test]
fn storm_streak_and_rush_wait_for_the_downloaded_puzzles(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, crate::pages::puzzles::Tab::Rush);
    // A run needs the downloaded puzzles; a score is only saved to an account.
    app.find("db-install");
    assert!(!app.enabled("rush-start"));
    app.find("storm-connect");
}

#[gpui_kit::test]
fn lichess_stats_ask_for_an_account(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, crate::pages::puzzles::Tab::Stats);
    assert!(app.enabled("stats-connect"));
}

#[gpui_kit::test]
fn an_empty_history_leads_back_to_training(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, crate::pages::puzzles::Tab::History);
    app.click("puzzle-history-train");
    assert_eq!(
        app.find(super::tab("puzzle-tabs", crate::pages::puzzles::Tab::Train))
            .selected(),
        Some(true)
    );
    app.find(("puzzle-mode", 0usize));
}

#[gpui_kit::test]
fn the_chosen_tab_is_kept_when_the_app_restarts(cx: &mut gpui_kit::TestAppContext) {
    let mut app = puzzles(cx);
    tab(&mut app, crate::pages::puzzles::Tab::History);
    let mut app = app.restart();
    app.open("Puzzles");
    assert_eq!(
        app.find(super::tab(
            "puzzle-tabs",
            crate::pages::puzzles::Tab::History
        ))
        .selected(),
        Some(true)
    );
    app.find("puzzle-history-train");
}

/// A lichess.org/training link pasted into Quick Pick opens its puzzle on the Daily tab, and
/// Today's puzzle goes back to the puzzle of the day.
#[gpui_kit::test]
fn a_pasted_puzzle_link_opens_the_puzzle(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text(&format!(
        "https://lichess.org/training/{}",
        super::fake_lichess::LINKED_PUZZLE
    ));
    {
        use gpui_kit::test::TestWindowExt as _;
        app.wait_for("the puzzle to be offered", |window, _| {
            window
                .try_find(("palette-item", 0usize))
                .is_some_and(|item| item.label() == Some("Solve this puzzle"))
        });
    }
    app.press("enter");
    app.find("daily-today");
    assert_eq!(
        app.find(super::tab("puzzle-tabs", crate::pages::puzzles::Tab::Daily))
            .selected(),
        Some(true)
    );
    // Scholar's mate: the queen takes f7.
    {
        use gpui_kit::test::TestWindowExt as _;
        app.wait_for("the puzzle's board", |window, _| {
            window.try_find("chess-board").is_some()
        });
    }
    app.click_square("h5", true);
    app.click_square("f7", true);
    app.wait_for("the puzzle to be solved", |window, _| {
        use gpui_kit::test::TestQueryExt as _;
        window.try_find_by_label("Puzzle solved").is_some()
    });

    app.click("daily-today");
    app.wait_gone("daily-today");
}

/// Quick Pick shows the solution of the puzzle on the board, offering only what the panel does.
#[gpui_kit::test]
fn quick_pick_shows_the_solution(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text(&format!(
        "https://lichess.org/training/{}",
        super::fake_lichess::LINKED_PUZZLE
    ));
    super::palette::wait_for_first(&mut app, "Solve this puzzle");
    app.press("enter");
    app.find("daily-today");
    app.wait_for("the puzzle to be playable", |window, _| {
        window
            .try_find("puzzle-hint")
            .is_some_and(|hint| hint.disabled() != Some(true))
    });
    super::palette::wait_until_closed(&mut app);
    app.press("secondary-k");
    app.type_text(">");
    super::palette::wait_for_first(&mut app, "Hint");
    let listed: Vec<String> = app.with_window(|window, _| {
        (0..6usize)
            .filter_map(|index| window.try_find(("palette-item", index)))
            .filter_map(|item| item.label().map(str::to_string))
            .collect()
    });
    // Daily puzzles have no next one: Today's puzzle is the way back.
    assert!(listed.contains(&"Solution".to_string()), "{listed:?}");
    assert!(!listed.contains(&"Next puzzle".to_string()), "{listed:?}");
    app.type_text("Solution");
    super::palette::wait_for_first(&mut app, "Solution");
    app.press("enter");
    app.wait_for("the solution to be shown", |window, _| {
        window
            .try_find_by_label("Solution shown — not counted as solved")
            .is_some()
    });
}

/// A Storm run starts, ends and goes back to the modes from Quick Pick, offering only what its
/// buttons do: Storm has no skips.
#[gpui_kit::test]
fn quick_pick_starts_and_ends_a_storm_run(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start_with(cx, |profile| {
        // A mate in one, at every rating the run's ladder climbs through.
        let rows: Vec<_> = (0..40)
            .map(|index| kchess_core::puzzles::queries::DbPuzzle {
                id: format!("fixture{index:02}"),
                fen: "6k1/5ppp/8/8/8/8/5PPP/3R2K1 b - - 0 1".into(),
                moves: "g8h8 d1d8".into(),
                rating: 800. + f64::from(index) * 50.,
                plays: Some(100.),
                themes: "mateIn1 backRankMate".into(),
                openings: String::new(),
                game_id: String::new(),
            })
            .collect();
        kchess_core::puzzles::write_database(profile, &rows).expect("the fixture's puzzles");
    });
    app.open("Puzzles");
    tab(&mut app, crate::pages::puzzles::Tab::Rush);
    app.wait_for("the downloaded puzzles", |window, _| {
        use gpui_kit::test::TestWindowExt as _;
        window
            .try_find("rush-start")
            .is_some_and(|start| start.disabled() != Some(true))
    });
    let run = |app: &mut Harness, label: &str| {
        app.press("secondary-k");
        app.type_text(&format!(">{label}"));
        super::palette::wait_for_first(app, label);
        app.press("enter");
        super::palette::wait_until_closed(app);
    };
    let cost = |app: &mut Harness| {
        app.with_window(|_, cx| crate::pages::settings::updates::restart_cost(cx))
    };
    assert_eq!(cost(&mut app), None);
    run(&mut app, "Start run");
    app.find("rush-end");
    assert!(!app.shows("rush-skip"));
    // Restarting into an update would end the run, so its question says so.
    assert_eq!(cost(&mut app), Some("Your puzzle run ends"));
    run(&mut app, "End run");
    app.find("rush-again");
    assert_eq!(cost(&mut app), None);
    run(&mut app, "Back to modes");
    app.find("rush-start");

    // Over, a run is played again with Return, as the next puzzle is.
    app.click("rush-start");
    app.click("rush-end");
    app.find("rush-again");
    app.press("enter");
    app.find("rush-end");
}

/// F turns the puzzle's board, as its button does, after a click on the page.
#[gpui_kit::test]
fn f_flips_the_puzzle(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text(&format!(
        "https://lichess.org/training/{}",
        super::fake_lichess::LINKED_PUZZLE
    ));
    super::palette::wait_for_first(&mut app, "Solve this puzzle");
    app.press("enter");
    app.find("daily-today");
    app.wait_for("the puzzle to be playable", |window, _| {
        window
            .try_find("puzzle-hint")
            .is_some_and(|hint| hint.disabled() != Some(true))
    });
    super::palette::wait_until_closed(&mut app);
    let before = app.white_at_bottom();
    app.click("chess-board");
    app.press("f");
    assert_eq!(app.white_at_bottom(), !before);
}

/// Once a training puzzle is over (and Next puzzle after solving is off), Return goes on to the next one, from the (now shut) move field
/// or the page, and so does Space; while it is being solved, Return belongs to the move field.
#[gpui_kit::test]
fn return_goes_on_to_the_next_puzzle(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = Harness::start(cx);
    app.cx.update(|cx| {
        crate::prefs::set(
            cx,
            "kchess:puzzle-angle",
            super::fake_lichess::TRAINED_THEME,
        );
        // Moving on by itself after a solve would hide whether the keys do.
        crate::settings::change(cx, |settings| settings.frontend.puzzle_auto_next = false);
    });
    app.open("Puzzles");
    let key = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view::<crate::pages::puzzles::PuzzlesPage>(cx)
                .map(|page| page.read(cx).puzzle_key())
        })
    };
    let playable = |app: &mut Harness| {
        app.wait_for("the puzzle to be playable", |window, _| {
            window
                .try_find("puzzle-hint")
                .is_some_and(|hint| hint.disabled() != Some(true))
        });
    };
    let solve = |app: &mut Harness| {
        app.click_label("Enter a chess move in SAN or UCI");
        app.type_text("Qxf7#");
        app.press("enter");
        app.wait_for("the puzzle to be solved", |window, _| {
            window
                .try_find("puzzle-hint")
                .is_none_or(|hint| hint.disabled() == Some(true))
        });
    };
    playable(&mut app);
    let first = key(&mut app);
    solve(&mut app);
    // The solving Return did not skip the solved puzzle.
    assert_eq!(key(&mut app), first);
    app.press("enter");
    playable(&mut app);
    let second = key(&mut app);
    assert_ne!(second, first);

    solve(&mut app);
    app.click("chess-board");
    app.press("space");
    playable(&mut app);
    assert_ne!(key(&mut app), second);
}

/// Once a puzzle is over its moves step back and forth with the arrows, Home and End, as a game's
/// do; while it is being solved they do nothing to the board.
#[gpui_kit::test]
fn a_finished_puzzle_is_stepped_through(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = Harness::start(cx);
    app.cx.update(|cx| {
        crate::prefs::set(
            cx,
            "kchess:puzzle-angle",
            super::fake_lichess::TRAINED_THEME,
        );
        crate::settings::change(cx, |settings| settings.frontend.puzzle_auto_next = false);
    });
    app.open("Puzzles");
    app.wait_for("the puzzle to be playable", |window, _| {
        window
            .try_find("puzzle-hint")
            .is_some_and(|hint| hint.disabled() != Some(true))
    });
    let viewing = |app: &mut Harness| {
        app.cx.update(|cx| {
            crate::shell::view::<crate::pages::puzzles::PuzzlesPage>(cx)
                .and_then(|page| page.read(cx).viewing(cx))
        })
    };
    app.click("chess-board");
    app.press("left");
    assert_eq!(viewing(&mut app), None, "not while it is being solved");

    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Qxf7#");
    app.press("enter");
    app.wait_for("the puzzle to be solved", |window, _| {
        window
            .try_find("puzzle-hint")
            .is_none_or(|hint| hint.disabled() == Some(true))
    });
    app.click("chess-board");
    app.press("left");
    let back = viewing(&mut app).expect("a position before the mate");
    app.press("home");
    assert_eq!(viewing(&mut app), Some(0));
    app.press("right");
    // One on from the start: the second position, or the last (shown as none) in a two-position
    // puzzle.
    assert_eq!(viewing(&mut app), (back >= 1).then_some(1));
    app.press("end");
    assert_eq!(viewing(&mut app), None);
}

/// A finished puzzle opens on the analysis board (from Quick Pick, as from its button), the wrong
/// try kept as a side line.
#[gpui_kit::test]
fn a_finished_puzzle_opens_on_the_analysis_board(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = Harness::start(cx);
    app.cx.update(|cx| {
        crate::prefs::set(
            cx,
            "kchess:puzzle-angle",
            super::fake_lichess::TRAINED_THEME,
        );
        crate::settings::change(cx, |settings| settings.frontend.puzzle_auto_next = false);
    });
    app.open("Puzzles");
    app.wait_for("the puzzle to be playable", |window, _| {
        window
            .try_find("puzzle-hint")
            .is_some_and(|hint| hint.disabled() != Some(true))
    });
    assert!(!app.shows("puzzle-analyse"), "not while it is being solved");
    let play = |app: &mut Harness, san: &str| {
        app.click_label("Enter a chess move in SAN or UCI");
        app.type_text(san);
        app.press("enter");
    };
    play(&mut app, "Qh4");
    play(&mut app, "Qxf7#");
    app.wait_for("the puzzle to be solved", |window, _| {
        window
            .try_find("puzzle-hint")
            .is_none_or(|hint| hint.disabled() == Some(true))
    });
    assert!(app.shows("puzzle-analyse"));
    // Quick Pick offers it as the page's own action, and opens it as the button does.
    app.press("secondary-k");
    app.type_text(">analysis board");
    super::palette::wait_for_first(&mut app, "Analysis board");
    app.press("enter");
    let pgn = |cx: &gpui_kit::App| {
        crate::shell::view::<crate::pages::analysis::AnalysisPage>(cx)
            .map(|page| page.read(cx).pgn())
            .unwrap_or_default()
    };
    app.wait_for("the puzzle on the analysis board", |_, cx| {
        pgn(cx).contains("Qxf7#")
    });
    let pgn = app.cx.update(|cx| pgn(cx));
    assert!(pgn.contains("Qxf7#"), "{pgn}");
    assert!(
        pgn.contains("Qh4 {Your move}") || pgn.contains("Qh4 { Your move }"),
        "{pgn}"
    );
}

/// A finished puzzle's themes train that theme, as on Lichess: back on Train, with it chosen.
#[gpui_kit::test]
fn a_finished_puzzles_theme_is_trained(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = Harness::start(cx);
    app.press("secondary-k");
    app.type_text(&format!(
        "https://lichess.org/training/{}",
        super::fake_lichess::LINKED_PUZZLE
    ));
    super::palette::wait_for_first(&mut app, "Solve this puzzle");
    app.press("enter");
    app.find("daily-today");
    app.wait_for("the puzzle to be playable", |window, _| {
        window
            .try_find("puzzle-solution")
            .is_some_and(|button| button.disabled() != Some(true))
    });
    assert!(
        !app.shows(("puzzle-theme", 0usize)),
        "not while it is being solved"
    );
    app.click("puzzle-solution");
    assert_eq!(
        app.find(("puzzle-theme", 0usize)).label(),
        Some("Train Mate in 1")
    );
    app.click(("puzzle-theme", 0usize));
    app.wait_for("Train with the theme chosen", |window, cx| {
        crate::prefs::get::<String>(cx, "kchess:puzzle-angle", String::new()) == "mateIn1"
            && window
                .try_find(super::tab("puzzle-tabs", crate::pages::puzzles::Tab::Train))
                .is_some_and(|tab| tab.selected() == Some(true))
    });
}

/// History narrows to the puzzles not solved, the ones worth another try, and keeps that choice.
#[gpui_kit::test]
fn history_shows_the_puzzles_not_solved(cx: &mut gpui_kit::TestAppContext) {
    let mut app = Harness::start(cx);
    let day = crate::game::puzzle::local_day();
    for (id, win, found) in [("aaaaa", true, 2), ("bbbbb", false, 1), ("ccccc", true, 2)] {
        let attempt = serde_json::json!({
            "puzzle": {
                "id": id,
                "fen": "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 2 4",
                "solution": ["f3f7", "e8f7"],
                "rating": 1500,
                "themes": ["mateIn1"],
            },
            "mode": "practice",
            "retry": false,
            "win": win,
            "revealed": false,
            "hinted": false,
            "found": found,
            "wrong": u64::from(!win),
            "total": 2,
            "day": day,
        });
        let saved = app.cx.update(|cx| {
            cx.global::<crate::core::AppCore>().request(
                kchess_contracts::calls::RecordPuzzleAttempt {
                    attempt: serde_json::from_value(attempt).expect("an attempt"),
                },
            )
        });
        app.cx
            .foreground_executor()
            .block_test(saved)
            .expect("the attempt is kept");
    }
    app.open("Puzzles");
    tab(&mut app, crate::pages::puzzles::Tab::History);
    app.find(("puzzle-retry", 2usize));
    app.click(("puzzle-history-result", 1usize));
    app.find(("puzzle-retry", 0usize));
    assert!(!app.shows(("puzzle-retry", 1usize)));

    let mut app = app.restart();
    app.open("Puzzles");
    app.find(("puzzle-retry", 0usize));
    assert!(!app.shows(("puzzle-retry", 1usize)));
    app.click(("puzzle-history-result", 0usize));
    app.find(("puzzle-retry", 2usize));
}

/// Puzzle history's rows fit the page at every window size: their fixed columns once squeezed the
/// themes to nothing in a 1024-wide window, and pushed each row's buttons past the list's edge in
/// the smallest one.
#[gpui_kit::test]
fn puzzle_history_rows_fit_every_window(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{px, size};
    let mut app = Harness::start(cx);
    let day = crate::game::puzzle::local_day();
    let attempt = serde_json::json!({
        "puzzle": {
            "id": "aaaaa",
            "fen": "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5Q2/PPPP1PPP/RNB1K1NR w KQkq - 2 4",
            "solution": ["f3f7", "e8f7"],
            "rating": 1500,
            "themes": ["mateIn1", "short", "opening"],
        },
        "mode": "practice",
        "retry": false,
        "win": false,
        "revealed": false,
        "hinted": false,
        "found": 1,
        "wrong": 1,
        "total": 2,
        "day": day,
    });
    let saved = app.cx.update(|cx| {
        cx.global::<crate::core::AppCore>()
            .request(kchess_contracts::calls::RecordPuzzleAttempt {
                attempt: serde_json::from_value(attempt).expect("an attempt"),
            })
    });
    app.cx
        .foreground_executor()
        .block_test(saved)
        .expect("the attempt is kept");
    app.open("Puzzles");
    tab(&mut app, crate::pages::puzzles::Tab::History);
    for (width, height) in super::SIZES {
        app.cx
            .simulate_window_resize(app.window, size(px(width), px(height)));
        for _ in 0..20 {
            app.tick();
        }
        let page = app
            .find(super::tab(
                "puzzle-tabs",
                crate::pages::puzzles::Tab::History,
            ))
            .bounds();
        let analyse = app.find(("puzzle-analyse", 0usize)).bounds();
        assert!(
            analyse.right() <= page.right(),
            "at {width}×{height}: {analyse:?} past {page:?}"
        );
    }
}

/// A puzzle failed and left unfinished when the app quits is still in the history.
#[gpui_kit::test]
fn a_puzzle_failed_before_quitting_is_kept(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let mut app = Harness::start(cx);
    app.online();
    app.cx.update(|cx| {
        crate::prefs::set(
            cx,
            "kchess:puzzle-angle",
            super::fake_lichess::TRAINED_THEME,
        );
    });
    app.open("Puzzles");
    app.wait_for("the puzzle to be playable", |window, _| {
        window
            .try_find("puzzle-hint")
            .is_some_and(|hint| hint.disabled() != Some(true))
    });
    app.click_label("Enter a chess move in SAN or UCI");
    app.type_text("Qh4");
    app.press("enter");
    let mut frames = 0;
    app.wait_for("the wrong move to be judged", |_, _| {
        frames += 1;
        frames > 60
    });
    let mut app = app.restart();
    app.open("Puzzles");
    tab(&mut app, crate::pages::puzzles::Tab::History);
    app.find(("puzzle-retry", 0usize));
}

/// The stat cards in a row are one height, a card without a line under its number too.
#[gpui_kit::test]
fn lichess_stat_cards_are_one_height(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::test::TestWindowExt as _;
    let (mut app, _session) = Harness::signed_in(cx, false);
    app.open("Puzzles");
    tab(&mut app, crate::pages::puzzles::Tab::Stats);
    app.wait_for("the stat cards", |window, _| {
        window.try_find(("puzzle-stat", 3usize)).is_some()
    });
    let heights: Vec<_> = (0..4usize)
        .map(|index| app.find(("puzzle-stat", index)).bounds())
        .collect();
    // "Puzzles played" (1) has no line under its number; its neighbours do. All four share the
    // test window's one row.
    for (index, bounds) in heights.iter().enumerate() {
        assert_eq!(
            (bounds.top(), bounds.size.height),
            (heights[0].top(), heights[0].size.height),
            "card {index} is not the height of its row: {heights:?}"
        );
    }
}
