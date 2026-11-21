//! The Train tab drawn: its choices, the board or what stands in for it, and the status panel.

use super::*;

impl PuzzlesPage {
    /// What stands in for the board while there is no puzzle to play.
    fn render_placeholder(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let empty = |glyph: Icon, title: String, description: Option<String>| {
            widgets::empty_state(glyph, title, description.map(SharedString::from), palette)
        };
        let online = crate::network::online(cx);
        let other = |cx: &mut Context<Self>| {
            let (label, mode) = if online {
                ("Practice instead", "practice")
            } else {
                ("Use downloaded puzzles", "offline")
            };
            Button::new("other-mode")
                .outline()
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| this.choose_mode(mode, cx)))
        };
        let connect = |id: &'static str, label: &'static str, cx: &mut Context<Self>| {
            let busy = crate::accounts::handle(cx).read(cx).busy;
            // Signing in needs Lichess: offline, the downloaded puzzles stand beside it.
            widgets::connect_lichess(id, cx)
                .label(label)
                .disabled(!online || busy)
        };
        let actions = |id, label, cx: &mut Context<Self>| {
            h_flex()
                .gap(px(8.))
                .child(connect(id, label, cx))
                .child(other(cx))
        };
        let content: AnyElement = match self.session.phase() {
            Phase::NoAccount => empty(
                icon("user-round-plus"),
                "Rated puzzles need a Lichess account".into(),
                None,
            )
            .child(actions("connect-lichess", "Connect Lichess", cx))
            .into_any_element(),
            Phase::Reconnect => empty(
                icon("key-round"),
                "Lichess needs your permission for puzzles".into(),
                Some(format!(
                    "Sign in to @{} on lichess.org, then reconnect to enable rated puzzles",
                    self.account(cx)
                )),
            )
            .child(actions("reconnect-lichess", "Reconnect", cx))
            .into_any_element(),
            Phase::NoDatabase => crate::puzzle_db::card(false, cx, palette).into_any_element(),
            Phase::Exhausted => {
                let difficulty = crate::prefs::get::<String>(cx, DIFFICULTY, "normal".into());
                let label = DIFFICULTIES
                    .iter()
                    .find(|(key, _)| *key == difficulty)
                    .map_or("Normal", |(_, label)| label);
                let index = DIFFICULTIES
                    .iter()
                    .position(|(key, _)| *key == difficulty)
                    .unwrap_or(2);
                let angle = crate::prefs::get::<String>(cx, ANGLE, "mix".into());
                let mode = Self::mode(cx);
                let mut actions: Vec<Button> = Vec::new();
                if index > 0 {
                    let (key, easier) = DIFFICULTIES[index - 1];
                    actions.push(
                        Button::new("try-easier")
                            .icon(icon("chevrons-down"))
                            .label(format!("Try {easier}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_difficulty(key, window, cx)
                            })),
                    );
                }
                if angle != "mix" {
                    actions.push(
                        Button::new("try-mix")
                            .icon(icon("shuffle"))
                            .label("Try Healthy mix")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_angle("mix", window, cx)
                            })),
                    );
                }
                if mode != "offline" {
                    actions.push(
                        Button::new("try-offline")
                            .icon(icon("hard-drive"))
                            .label("Use downloaded puzzles")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.choose_mode("offline", cx)),
                            ),
                    );
                }
                let actions: Vec<Button> = actions
                    .into_iter()
                    .enumerate()
                    .map(|(order, action)| {
                        if order == 0 {
                            action.primary()
                        } else {
                            action.outline()
                        }
                    })
                    .collect();
                empty(
                    icon("puzzle"),
                    format!("No more {label} puzzles for this theme"),
                    None,
                )
                .child(
                    h_flex()
                        .gap(px(8.))
                        .flex_wrap()
                        .justify_center()
                        .children(actions),
                )
                .into_any_element()
            }
            Phase::Error => empty(
                icon("triangle-alert"),
                "Couldn’t get a puzzle".into(),
                Some(self.session.state().train_error.clone()),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        crate::widgets::try_again("try-again")
                            .on_click(cx.listener(|this, _, _, cx| this.load_next(cx))),
                    )
                    .when(Self::mode(cx) != "offline", |row| {
                        row.child(
                            Button::new("error-offline")
                                .outline()
                                .label("Use downloaded puzzles")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.choose_mode("offline", cx)),
                                ),
                        )
                    }),
            )
            .into_any_element(),
            Phase::Idle | Phase::Loading | Phase::Ready => h_flex()
                .gap(px(8.))
                .text_color(palette.text_muted)
                .child(crate::widgets::spinner(px(16.)))
                .child("Finding a puzzle…")
                .into_any_element(),
        };
        content
    }

    /// The session's tally, the day streak and the latest attempts.
    fn render_session(&self, cx: &mut Context<Self>, palette: &Palette) -> impl IntoElement {
        let tally = &self.session.state().session;
        let parts = [
            (tally.solved, "solved", palette.success, true),
            (tally.hinted, "with hint", palette.success, false),
            (tally.partial, "partial", palette.info, false),
            (tally.missed, "missed", palette.error, true),
            (tally.revealed, "revealed", palette.text, false),
        ];
        let tried = tally.moves_tried;
        let rating = self.session.state().rating;
        let day_streak = f64::from(self.progress.day_streak);
        let mut line = h_flex()
            .flex_wrap()
            .gap_x(px(12.))
            .gap_y(px(4.))
            .text_size(px(13.))
            .text_color(palette.text_muted);
        for (value, label, tone, always) in parts {
            if always || value > 0. {
                line = line.child(
                    h_flex()
                        .gap(px(4.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(tone)
                                .child(format!("{value}")),
                        )
                        .child(label),
                );
            }
        }
        if tried > 0. {
            line = line.child(
                h_flex()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.text)
                            .child(format!("{}", tally.moves_found)),
                    )
                    .child(format!("/{tried} moves right")),
            );
        }
        if let Some(rating) = rating.filter(|_| Self::mode(cx) == "rated") {
            let change = tally.rating_change;
            line = line.child(format!(
                "rating {rating}{}",
                if change != 0. {
                    format!(" ({}{change})", if change > 0. { "+" } else { "" })
                } else {
                    String::new()
                }
            ));
        }
        line = line.child(
            h_flex()
                .ml_auto()
                .gap(px(8.))
                .when(day_streak >= 2., |row| {
                    row.child(
                        h_flex()
                            .gap(px(2.))
                            .text_color(palette.warning)
                            .child(icon("flame").size(px(14.)))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.text)
                                    .child(format!("{day_streak}")),
                            ),
                    )
                })
                .child(
                    Button::new("puzzle-reset")
                        .ghost()
                        .small()
                        .label("Reset")
                        .on_click(cx.listener(|this, _, _, cx| this.reset_session(cx))),
                ),
        );
        // The latest attempts, oldest first, so the newest sits at the end of the strip.
        let recent: Vec<PuzzleAttempt> = self.progress.recent.iter().rev().cloned().collect();
        let strip = (!recent.is_empty()).then(|| {
            h_flex().flex_wrap().gap(px(4.)).mt(px(8.)).children(
                recent.into_iter().enumerate().map(|(index, attempt)| {
                    let (glyph, fg, bg) = match attempt.outcome {
                        PuzzleAttemptOutcome::Solved => {
                            (icon("check"), palette.success, tint(palette.success, 10.))
                        }
                        PuzzleAttemptOutcome::Hinted => {
                            (icon("lightbulb"), palette.success, palette.bg)
                        }
                        PuzzleAttemptOutcome::Partial => (
                            icon("chart-no-axes-column"),
                            palette.info,
                            tint(palette.info, 10.),
                        ),
                        PuzzleAttemptOutcome::Revealed => {
                            (icon("eye"), palette.toned, palette.elevated)
                        }
                        PuzzleAttemptOutcome::Missed => {
                            (icon("x"), palette.error, tint(palette.error, 10.))
                        }
                    };
                    let label = format!(
                        "{}, {}/{} moves{}, rated {}",
                        match attempt.outcome {
                            PuzzleAttemptOutcome::Solved => "Solved",
                            PuzzleAttemptOutcome::Hinted => "With hint",
                            PuzzleAttemptOutcome::Partial => "Partial",
                            PuzzleAttemptOutcome::Revealed => "Revealed",
                            PuzzleAttemptOutcome::Missed => "Missed",
                        },
                        attempt.found,
                        attempt.total,
                        match attempt.wrong {
                            0 => String::new(),
                            wrong => format!(" · {wrong} wrong"),
                        },
                        attempt.puzzle.rating
                    );
                    let (puzzle, wrong) = (
                        attempt.puzzle.clone(),
                        attempt.wrong_moves.clone().unwrap_or_default(),
                    );
                    let page = cx.entity().downgrade();
                    // Try it again, or study it on the board.
                    Button::new(("attempt", index))
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .color(bg)
                                .foreground(fg)
                                .hover(tint(fg, 22.))
                                .active(tint(fg, 30.)),
                        )
                        .size(px(24.))
                        .p_0()
                        .border_1()
                        .border_color(tint(fg, 40.))
                        .tooltip(label.clone())
                        .accessibility_label(label)
                        .child(glyph.size(px(14.)))
                        .dropdown_menu(move |menu, _, _| {
                            let (puzzle, wrong, page) =
                                (puzzle.clone(), wrong.clone(), page.clone());
                            let studied = puzzle.clone();
                            menu.item(
                                PopupMenuItem::new("Try again")
                                    .icon(icon("rotate-ccw"))
                                    .on_click(move |_, _, cx| {
                                        let _ = page
                                            .update(cx, |page, cx| page.retry(puzzle.clone(), cx));
                                    }),
                            )
                            .item(
                                PopupMenuItem::new("Analysis board")
                                    .icon(icon("microscope"))
                                    .on_click(move |_, window, cx| {
                                        kchess_board::analysis::open(
                                            kchess_board::analysis::Open::Puzzle {
                                                puzzle: studied.clone(),
                                                wrong_moves: wrong.clone(),
                                            },
                                            window,
                                            cx,
                                        )
                                    }),
                            )
                        })
                }),
            )
        });
        v_flex().child(line).children(strip)
    }

    pub(super) fn render_train(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let (board_size, panel_height) = board_and_panel(&self.layout, &self.puzzle, window, cx);
        let mode = Self::mode(cx);
        let ready = self.puzzle.read(cx).puzzle().is_some() && self.session.phase() == Phase::Ready;
        let board_column = if ready {
            div()
                .w(board_size)
                .flex_none()
                .child(self.puzzle.clone())
                .into_any_element()
        } else {
            v_flex()
                .w(board_size)
                .h(board_size)
                .flex_none()
                .items_center()
                .justify_center()
                .p(px(24.))
                .border_1()
                .border_color(palette.border)
                .rounded(radii::XL)
                .bg(palette.elevated)
                .child(self.render_placeholder(cx, palette))
                .into_any_element()
        };
        let field = |label: &'static str, control: AnyElement| {
            v_flex()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.highlighted)
                        .child(label),
                )
                .child(control)
        };
        let page = cx.entity().downgrade();
        let mode_tabs = widgets::pills(
            "puzzle-mode",
            MODES
                .iter()
                .map(|(key, label)| Pill {
                    icon: Some(match *key {
                        "practice" => icon("dumbbell"),
                        "offline" => icon("hard-drive"),
                        _ => icon("cloud-upload"),
                    }),
                    label: (*label).into(),
                    badge: None,
                    disabled: false,
                })
                .collect(),
            MODES.iter().position(|(key, _)| *key == mode).unwrap_or(0),
            move |index, _, cx| {
                let _ = page.update(cx, |page, cx| page.choose_mode(MODES[index].0, cx));
            },
            false,
            palette,
        );
        let is_retry = self.session.state().is_retry;
        let account = self.account(cx);
        let source = if is_retry {
            Some("Retry · unrated".to_string())
        } else if mode == "rated" {
            Some(if account.is_empty() {
                "Rated".to_string()
            } else {
                format!("Rated · @{account}")
            })
        } else {
            None
        };
        // A rated attempt's result goes to Lichess as it ends, and its answer comes a moment
        // later: its line is there from the end, saying so, rather than arriving under the
        // puzzle and pushing the session's tally and Reset down.
        let sending = mode == "rated"
            && !is_retry
            && self.session.state().attempt_outcome.is_some()
            && self.session.state().last_report.is_none();
        let rating_text = self.session.state().last_report.as_ref().map(|report| {
            if let Some(failed) = &report.failed {
                (failed.clone(), true)
            } else if let Some(diff) = report.rating_diff {
                let rating = self
                    .session
                    .state()
                    .rating
                    .map(|rating| format!(" → {rating}"))
                    .unwrap_or_default();
                (
                    format!(
                        "Lichess puzzle rating {}{diff}{rating}",
                        if diff >= 0. { "+" } else { "" }
                    ),
                    false,
                )
            } else {
                ("Sent to Lichess".to_string(), false)
            }
        });
        let rating_text =
            rating_text.or_else(|| sending.then(|| ("Sending to Lichess…".to_string(), false)));
        let panel = widgets::side_panel(palette)
            .h(panel_height)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(14.))
            .child(field("Mode", mode_tabs.into_any_element()))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(match &source {
                        Some(source) => source.clone(),
                        None => mode_hint(&mode).to_string(),
                    }),
            )
            .when(mode == "rated" && self.accounts(cx).len() > 1, |panel| {
                panel.child(field(
                    "Account",
                    Select::new(&self.account_select)
                        .small()
                        .icon(icon("user"))
                        .accessibility_label("Account")
                        .into_any_element(),
                ))
            })
            .child(field(
                "Theme",
                Select::new(&self.theme_select)
                    .small()
                    .accessibility_label("Theme")
                    .into_any_element(),
            ))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        div().flex_1().child(field(
                            "Difficulty",
                            Select::new(&self.difficulty_select)
                                .small()
                                .accessibility_label("Difficulty")
                                .into_any_element(),
                        )),
                    )
                    .child(
                        div().flex_1().child(field(
                            "Play as",
                            Select::new(&self.color_select)
                                .small()
                                .disabled(mode == "offline")
                                .accessibility_label("Play as")
                                .into_any_element(),
                        )),
                    ),
            )
            .when(ready, |panel| {
                panel
                    .child(div().h(px(1.)).bg(palette.border))
                    .child(status_panel(
                        &self.puzzle,
                        &self.theme_list,
                        Some(Box::new({
                            let page = cx.entity().downgrade();
                            move |_, cx| {
                                let _ = page.update(cx, |page, cx| page.load_next(cx));
                            }
                        })),
                        mode == "rated" && !is_retry,
                        cx,
                        palette,
                    ))
                    .children(rating_text.map(|(text, failed)| {
                        div()
                            .id("puzzle-rated")
                            .test_support()
                            .role(gpui_kit::Role::Status)
                            .aria_label(SharedString::from(text.clone()))
                            .text_size(px(12.))
                            .text_color(if failed {
                                palette.error
                            } else {
                                palette.text_muted
                            })
                            .child(text)
                    }))
            })
            .child(div().h(px(1.)).bg(palette.border))
            .child(self.render_session(cx, palette));
        self.layout
            .render("puzzle-train", board_column, panel, window, cx)
            .into_any_element()
    }
}
