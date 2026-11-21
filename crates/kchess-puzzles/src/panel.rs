//! The status panel beside a puzzle: what to do, how it went, the coach, and the buttons that help.
//! The Daily tab shows it too.

use super::*;

/// What the status panel's Next button does.
pub(crate) type NextPuzzle = Box<dyn Fn(&mut Window, &mut App)>;

/// The puzzle's idea, shown with the hint.
fn clue(puzzle: &Puzzle) -> Option<AnyElement> {
    let clue = kchess_domain::coach::puzzle_clue(&puzzle.fen, &puzzle.solution)?;
    Some(div().text_size(px(13.)).child(clue).into_any_element())
}

/// Whether a coach may speak: outside help on a puzzle that counts for a rating is cheating, so a
/// rated attempt has none until it is over.
pub(super) fn coach_allowed(rated: bool, status: PuzzleStatus) -> bool {
    !rated || status.is_over()
}

/// What a coach can add: a nudge while solving, why a wrong move fails, why the solution works.
fn coach_block(
    board_entity: &Entity<PuzzleBoard>,
    puzzle: &Puzzle,
    status: PuzzleStatus,
    playing: bool,
    rated: bool,
    cx: &App,
    palette: &Palette,
) -> Option<AnyElement> {
    if !coach_allowed(rated, status) {
        return None;
    }
    let shown = board_entity.read(cx);
    let on_board = shown.puzzle()?;
    let solution = &puzzle.solution;
    // While solving, from where the board stands: a long puzzle may be several moves in.
    let current = (
        on_board.fen().to_string(),
        solution.get(on_board.index()..)?.to_vec(),
    );
    let whole = (puzzle.fen.clone(), solution.clone());
    let merged = |(fen, moves): &(String, Vec<String>), kind, played: Option<&str>| AiRequest {
        kind,
        fen: fen.clone(),
        played: played.map(str::to_string),
        moves: moves.clone(),
        best: Vec::new(),
        refutation: Vec::new(),
        mate_before: None,
        mate_after: None,
    };
    let (id, label, request, on_answer): (_, _, _, Option<crate::ai::OnAnswer>) =
        if let (true, Some(wrong)) = (playing, shown.last_wrong()) {
            (
                "puzzle-wrong",
                "Why not?",
                merged(&current, AiKind::Wrong, Some(wrong)),
                None,
            )
        } else if playing {
            let board = board_entity.downgrade();
            let nudged: crate::ai::OnAnswer = std::rc::Rc::new(move |cx| {
                let _ = board.update(cx, |board, _| board.nudged());
            });
            (
                "puzzle-nudge",
                "Nudge · not counted",
                merged(&current, AiKind::Nudge, None),
                Some(nudged),
            )
        } else if status.is_over() {
            (
                "puzzle-why",
                "Explain",
                merged(&whole, AiKind::Solution, None),
                None,
            )
        } else {
            return None;
        };
    crate::ai::explain_block(
        id,
        label,
        shown.explainer(),
        request,
        shown.orientation(),
        on_answer,
        cx,
        palette,
    )
}

/// What to do, how it went, and the buttons that help.
pub(crate) fn status_panel(
    board_entity: &Entity<PuzzleBoard>,
    themes: &[Theme],
    next: Option<NextPuzzle>,
    rated: bool,
    cx: &App,
    palette: &Palette,
) -> AnyElement {
    let shown = board_entity.read(cx);
    let Some(board) = shown.puzzle() else {
        return div().into_any_element();
    };
    let puzzle = board.puzzle.clone();
    let player = board.player_color();
    let status = shown.status();
    let (outcome, feedback, waiting, hinted) = (
        shown.outcome(),
        shown.feedback(),
        shown.waiting(),
        shown.hinted(),
    );
    let color = if player == Color::White {
        "white"
    } else {
        "black"
    };
    let (tone, glyph, title) = match status {
        PuzzleStatus::Revealed => (
            Tone::Loss,
            icon("eye"),
            "Solution shown — not counted as solved".to_string(),
        ),
        PuzzleStatus::Solved if outcome == Some(true) => {
            (Tone::Win, icon("trophy"), "Puzzle solved".to_string())
        }
        PuzzleStatus::Solved => (
            Tone::Plain,
            icon("check"),
            if shown.was_nudged() {
                "Solved with a nudge — not counted".to_string()
            } else {
                "Solved after a mistake — not counted".to_string()
            },
        ),
        PuzzleStatus::Failed => (
            Tone::Loss,
            icon("x"),
            "Not the move — puzzle over".to_string(),
        ),
        _ if feedback == Feedback::Wrong => (
            Tone::Loss,
            icon("x"),
            "That’s not the move — try again".to_string(),
        ),
        _ if waiting => (Tone::Plain, icon("hourglass"), "Good move…".to_string()),
        _ => (
            Tone::Plain,
            icon("mouse-pointer-click"),
            format!("Find the best move for {color}"),
        ),
    };
    let playing = status == PuzzleStatus::Playing;
    let finished = status.is_over();
    let facts = finished.then(|| {
        v_flex()
            .gap(px(8.))
            .text_size(px(13.))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(64.))
                            .text_color(palette.text_muted)
                            .child("Rating"),
                    )
                    .child(format!("{}", puzzle.rating)),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(64.))
                            .text_color(palette.text_muted)
                            .child("Themes"),
                    )
                    .child(h_flex().flex_1().flex_wrap().gap(px(4.)).children(
                        puzzle.themes.iter().map(String::as_str).enumerate().map(
                            |(index, theme)| {
                                let name = theme_name(theme, themes);
                                // A theme Train offers is a way to train it, as on Lichess.
                                let trainable = themes.iter().any(|known| known.key == theme);
                                let key = theme.to_string();
                                div()
                                    .id(("puzzle-theme", index))
                                    .test_support()
                                    .px(px(6.))
                                    .py(px(1.))
                                    .rounded(radii::SM)
                                    .border_1()
                                    .border_color(palette.border_accented)
                                    .bg(palette.elevated)
                                    .text_size(px(12.))
                                    .when(trainable, |pill| {
                                        pill.role(gpui_kit::Role::Button)
                                            .aria_label(format!("Train {name}"))
                                            .cursor_pointer()
                                            .pressable(palette)
                                            .on_click(move |_, window, cx| {
                                                train_theme(key.clone(), window, cx)
                                            })
                                    })
                                    .child(name)
                            },
                        ),
                    )),
            )
    });
    v_flex()
        .gap(px(12.))
        .child(widgets::StatusBanner::new(tone, glyph, title.into()).announce())
        .children(hinted.then(|| clue(&puzzle)).flatten())
        .when(rated && playing && shown.hint_used(), |panel| {
            panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child("Unrated: a hint was used"),
            )
        })
        .children(coach_block(
            board_entity,
            &puzzle,
            status,
            playing,
            rated,
            cx,
            palette,
        ))
        // Hint, Solution, Flip and Next share the row; once the puzzle is over,
        // Hint and Solution give way to the analysis board, its wrong tries as side lines.
        .child(widgets::panel_actions(
            false,
            (if finished {
                let (studied, wrong) = (puzzle.clone(), shown.wrong_moves().to_vec());
                vec![
                    Button::new("puzzle-analyse")
                        .w_full()
                        .outline()
                        .icon(icon("microscope"))
                        .label("Analyse")
                        .tooltip("Analysis board")
                        .on_click(move |_, window, cx| {
                            kchess_board::analysis::open(
                                kchess_board::analysis::Open::Puzzle {
                                    puzzle: studied.clone(),
                                    wrong_moves: wrong.clone(),
                                },
                                window,
                                cx,
                            )
                        })
                        .into_any_element()
                        .into(),
                ]
            } else {
                vec![
                    Button::new("puzzle-hint")
                        .w_full()
                        .when(hinted, |button| button.primary())
                        .when(!hinted, |button| button.outline())
                        .icon(icon("lightbulb"))
                        .label("Hint")
                        .when(rated, |button| button.tooltip("Makes this puzzle unrated"))
                        .disabled(!playing)
                        .on_click({
                            let board = board_entity.downgrade();
                            move |_, _, cx| {
                                let _ = board.update(cx, |board, cx| board.hint(cx));
                            }
                        })
                        .into_any_element()
                        .into(),
                    Button::new("puzzle-solution")
                        .w_full()
                        .outline()
                        .icon(icon("eye"))
                        .label("Solution")
                        .disabled(!playing)
                        .on_click({
                            let board = board_entity.downgrade();
                            move |_, _, cx| {
                                let _ = board.update(cx, |board, cx| board.show_solution(cx));
                            }
                        })
                        .into_any_element()
                        .into(),
                ]
            })
            .into_iter()
            .chain([widgets::fit(
                Button::new("puzzle-flip")
                    .ghost()
                    .icon(icon("arrow-up-down"))
                    .tooltip(kchess_board::flip_label(cx))
                    .accessibility_label("Flip board")
                    .on_click({
                        let board = board_entity.downgrade();
                        move |_, _, cx| {
                            let _ = board.update(cx, |board, cx| board.flip(cx));
                        }
                    }),
            )])
            .chain(next.map(|next| {
                widgets::trailing_icon(
                    Button::new("puzzle-next")
                        .w_full()
                        .when(finished, |button| button.primary())
                        .when(!finished, |button| button.outline()),
                    "Next",
                    "arrow-right",
                )
                .on_click(move |_, window, cx| next(window, cx))
                .into_any_element()
                .into()
            })),
        ))
        .children(facts)
        .into_any_element()
}
