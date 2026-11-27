//! History's list and the game open in it: the source tabs, the rows, their pages, and an empty
//! list's message.

use super::*;

/// The page's top row: where the games come from.
pub(super) fn source_tabs(
    source: Source,
    cx: &mut Context<HistoryPage>,
    palette: &Palette,
) -> impl IntoElement {
    let connected = !crate::accounts::handle(cx).read(cx).connected().is_empty();
    let mut sources = vec![Source::Computer, Source::Board, Source::Lichess];
    let mut labels: Vec<(Option<Icon>, SharedString)> = vec![
        (None, "Computer".into()),
        (None, "Over the board".into()),
        (None, "Online".into()),
    ];
    if connected {
        sources.push(Source::Bookmarks);
        labels.push((None, "Bookmarks".into()));
    }
    let page = cx.entity().downgrade();
    widgets::pill_tabs(
        "history-source",
        labels,
        sources.iter().position(|item| *item == source).unwrap_or(0),
        move |index, _, cx| {
            page.update(cx, |page, cx| page.choose_source(sources[index], cx))
                .ok();
        },
        palette,
    )
}

/// An empty list's message and the action that fills it.
pub(super) fn empty(
    glyph: Icon,
    title: &'static str,
    action: Button,
    palette: &Palette,
) -> impl IntoElement {
    widgets::card(palette)
        .items_center()
        .py(px(28.))
        .child(widgets::empty_state(glyph, title, None, palette).child(action))
}

impl HistoryPage {
    pub(super) fn render_open(
        &self,
        game: &PlayedGame,
        replay: &Entity<GameReplay>,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let exported = game.clone();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .child(
                        Button::new("all-games")
                            .ghost()
                            .icon(icon("arrow-left"))
                            .label("All games")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_game(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("previous-game")
                            .ghost()
                            .icon(icon("chevron-left"))
                            .tooltip("Previous game")
                            .accessibility_label("Previous game")
                            .disabled(self.neighbour(-1, cx).is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.step_game(-1, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("next-game")
                            .ghost()
                            .icon(icon("chevron-right"))
                            .tooltip("Next game")
                            .accessibility_label("Next game")
                            .disabled(self.neighbour(1, cx).is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.step_game(1, window, cx)),
                            ),
                    )
                    .child(badge(&self.outcome_of(game, cx), None, palette))
                    .child(format!(
                        "{} vs {}",
                        game.white.as_str(),
                        game.black.as_str()
                    ))
                    .child(
                        Button::new("export-open")
                            .outline()
                            .label("Export PGN")
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.export(&exported, cx)),
                            ),
                    )
                    .child({
                        let opened = game.clone();
                        Button::new("open-analysis")
                            .primary()
                            .icon(icon("chart-line"))
                            .label("Open analysis")
                            .on_click(move |_, window, cx| analyse(&opened, window, cx))
                    })
                    .children(self.message.clone().map(|message| {
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(message)
                    })),
            )
            .child(div().flex_1().min_h_0().child(replay.clone()))
            .into_any_element()
    }

    pub(super) fn render_list(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let games = self.games(cx);
        if games.is_empty() && self.all_games(cx).is_empty() {
            let (label, page) = if self.source == Source::Computer {
                ("Play the computer", Page::Computer)
            } else {
                ("Play over the board", Page::Local)
            };
            return empty(
                icon("history"),
                "No saved games yet",
                Button::new("history-play")
                    .primary()
                    .label(label)
                    .on_click(move |_, window, cx| kchess_nav::shell::open(page, window, cx)),
                palette,
            )
            .into_any_element();
        }
        let outcomes: Vec<String> = games.iter().map(|game| self.outcome_of(game, cx)).collect();
        let count = |filter: Filter| match filter {
            Filter::All => games.len(),
            filter => outcomes
                .iter()
                .filter(|outcome| filter_of(outcome) == Some(filter))
                .count(),
        };
        let filters: Vec<(Filter, &str)> = if self.source == Source::Computer {
            vec![
                (Filter::All, "All"),
                (Filter::Win, "Wins"),
                (Filter::Loss, "Losses"),
                (Filter::Draw, "Draws"),
                (Filter::Unfinished, "Unfinished"),
            ]
        } else {
            vec![
                (Filter::All, "All"),
                (Filter::White, "White won"),
                (Filter::Black, "Black won"),
                (Filter::Draw, "Draws"),
                (Filter::Unfinished, "Unfinished"),
            ]
        };
        let page = cx.entity().downgrade();
        let choices: Vec<Filter> = filters.iter().map(|(filter, _)| *filter).collect();
        let filter_tabs = widgets::pills(
            "history-filter",
            filters
                .iter()
                .map(|(filter, label)| {
                    let count = count(*filter);
                    Pill {
                        icon: None,
                        label: (*label).into(),
                        badge: Some(count.to_string().into()),
                        // An empty filter cannot lead anywhere; the selected one stays reachable.
                        disabled: count == 0 && self.filter != *filter,
                    }
                })
                .collect(),
            choices
                .iter()
                .position(|filter| *filter == self.filter)
                .unwrap_or(0),
            move |index, _, cx| {
                page.update(cx, |page, cx| page.choose_filter(choices[index], cx))
                    .ok();
            },
            true,
            palette,
        );
        let shown: Vec<(&PlayedGame, &String)> = games
            .iter()
            .copied()
            .zip(&outcomes)
            .filter(|(_, outcome)| {
                self.filter == Filter::All || filter_of(outcome) == Some(self.filter)
            })
            .collect();
        let nothing_shown = shown.is_empty();
        let pages = shown.len().div_ceil(PAGE_SIZE).max(1);
        let current = self.page.min(pages - 1);
        let narrow = self.narrow();
        let columns = |row: Stateful<Div>, cells: [AnyElement; 5]| {
            let [result, players, game, date, actions] = cells;
            row.test_support()
                .gap(px(12.))
                .child(div().w(BADGE_WIDTH).flex_none().child(result))
                .child(div().flex_1().min_w_0().child(players))
                .when(!narrow, |row| {
                    row.child(div().w(px(112.)).flex_none().child(game))
                        .child(div().w(px(104.)).flex_none().child(date))
                })
                .child(div().w(px(32.)).flex_none().child(actions))
        };
        let head = |label: &'static str| {
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(label)
                .into_any_element()
        };
        let header = columns(
            h_flex()
                .id("history-head")
                .pl(px(12.))
                .pr(px(8.))
                .pb(px(8.))
                .border_b_1()
                .border_color(palette.border),
            [
                head("Result"),
                head(if self.source == Source::Computer {
                    "Opponent"
                } else {
                    "Players"
                }),
                head("Game"),
                head("Date"),
                div().into_any_element(),
            ],
        );
        // The rows light what the search found, as the search reads it (in lower case).
        let searched = self.search.to_lowercase();
        let rows = shown
            .iter()
            .skip(current * PAGE_SIZE)
            .take(PAGE_SIZE)
            .enumerate()
            .map(|(index, (game, outcome))| {
                // Read in place: a row's actions find the game by its id when chosen, so a frame
                // copies no game (four copies of each listed game's moves were made each frame).
                let game: &PlayedGame = game;
                let id = game.id.clone();
                let clock = game.source == HistoryKind::Clock;
                let side = game.player_side();
                let variant = game.setup.variant;
                // Why it ended, and the opening it reached, as a Lichess game's line has; in a
                // narrow list, which leaves the game's column out, its variant first.
                let details = [
                    (narrow && variant != Variant::Standard).then(|| variant.label().to_string()),
                    Some(details(game)),
                    self.opening(game).filter(|_| opening_names_shown(cx)),
                ]
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                let mut kind = time_label(game.time_control.as_str());
                if variant != Variant::Standard {
                    kind.push_str(&format!(" · {}", variant.label()));
                }
                // What the search found in the row, lit: the opponent, or the opening.
                let lit = |text: String| {
                    let found = quick_pick::found(&text, &[searched.as_str()], quick_pick::lower);
                    widgets::highlighted(text, &found, palette)
                };
                let players = v_flex()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .min_w_0()
                            .font_weight(FontWeight::MEDIUM)
                            .children(side.map(widgets::side_dot))
                            .child(div().min_w_0().truncate().child(lit(opponent(game)))),
                    )
                    .when(!details.is_empty(), |cell| {
                        cell.child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(lit(details)),
                        )
                    });
                let game_cell = v_flex()
                    .child(if clock {
                        "Clock only".to_string()
                    } else {
                        move_count(game.moves.len())
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(kind),
                    );
                let (exported, deleted) = (id.clone(), id.clone());
                let finished = game.finished;
                let page = cx.entity().downgrade();
                let actions = Button::new(SharedString::from(format!("actions-{id}")))
                    .ghost()
                    .small()
                    .icon(icon("ellipsis"))
                    .accessibility_label(format!(
                        "Actions for the game from {}",
                        crate::game::game_date(game.started_at)
                    ))
                    // Aligned to the button's end, so it stays in the window.
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                        let (exported, deleted) = (exported.clone(), deleted.clone());
                        let (export_page, delete_page) = (page.clone(), page.clone());
                        menu.item(
                            PopupMenuItem::new(if clock { "Copy summary" } else { "Export PGN" })
                                .icon(if clock {
                                    icon("copy")
                                } else {
                                    icon("download")
                                })
                                .on_click(move |_, _, cx| {
                                    if let Some(game) = played_game(&exported, cx) {
                                        let _ = export_page
                                            .update(cx, |page, cx| page.export(&game, cx));
                                    }
                                }),
                        )
                        .when(finished, |menu| {
                            menu.separator().item(
                                PopupMenuItem::new("Delete").icon(icon("trash-2")).on_click(
                                    move |_, window, cx| {
                                        if let Some(game) = played_game(&deleted, cx) {
                                            delete_page
                                                .update(cx, |page, cx| {
                                                    page.delete(game, window, cx)
                                                })
                                                .ok();
                                        }
                                    },
                                ),
                            )
                        })
                    });
                let opened = id.clone();
                columns(
                    h_flex()
                        .id(("history-row", index))
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label(format!(
                            "{} vs {}",
                            game.white.as_str(),
                            game.black.as_str()
                        ))
                        .pl(px(12.))
                        .pr(px(8.))
                        .py(px(12.))
                        .when(index > 0, |row| {
                            row.border_t_1().border_color(palette.border)
                        })
                        .when(!clock, |row| {
                            row.cursor_pointer()
                                .pressable(palette)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(game) = played_game(&opened, cx) {
                                        this.open_game(game, window, cx)
                                    }
                                }))
                        }),
                    [
                        badge(outcome, Some(BADGE_WIDTH), palette).into_any_element(),
                        players.into_any_element(),
                        game_cell.into_any_element(),
                        div()
                            .text_color(palette.text_muted)
                            .child(crate::game::game_date(game.started_at))
                            .into_any_element(),
                        actions.into_any_element(),
                    ],
                )
                .into_any_element()
            })
            .collect::<Vec<_>>();
        let list = widgets::card(palette)
            .text_size(px(14.))
            .child(header)
            .children(rows)
            .when(nothing_shown, |card| {
                card.child(
                    v_flex()
                        .items_center()
                        .gap(px(8.))
                        .py(px(20.))
                        .child(
                            icon("search-x")
                                .size(px(20.))
                                .text_color(palette.text_muted),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.highlighted)
                                .child("No games match"),
                        )
                        .child(
                            Button::new("show-all")
                                .outline()
                                .label("Show all games")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.clear_search(window, cx);
                                    this.choose_filter(Filter::All, cx)
                                })),
                        ),
                )
            })
            .when(pages > 1, |card| {
                card.child(div().pt(px(16.)).child(widgets::pager(
                    "history",
                    current,
                    pages,
                    cx.listener(|this, page: &usize, _, cx| {
                        this.page = *page;
                        cx.notify();
                    }),
                )))
            });
        v_flex()
            .gap(px(16.))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(12.))
                    .child(filter_tabs)
                    .child(
                        div().w(px(200.)).child(
                            Input::new(&self.search_input)
                                .small()
                                .cleanable(true)
                                .prefix(icon("search").size(px(14.)))
                                .aria_label("Search by opponent or opening"),
                        ),
                    )
                    .child(
                        Button::new("history-export-all")
                            .outline()
                            .small()
                            .icon(icon("download"))
                            .label("Export")
                            .tooltip("Save the games listed as one PGN file")
                            .disabled(nothing_shown)
                            .on_click(cx.listener(|this, _, _, cx| this.export_shown(cx))),
                    ),
            )
            .child(list)
            .into_any_element()
    }
}
