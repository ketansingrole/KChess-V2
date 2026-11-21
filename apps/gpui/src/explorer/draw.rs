//! Drawing the explorer: its filters, the player's name, the moves as a list or a grid of
//! boards, and the games from the position.

use super::*;

/// A tablebase result in words, from the side to move's view or (`opponent`) the other's.
pub(super) fn category(value: &str, opponent: bool) -> String {
    let value = if opponent {
        value
            .split('-')
            .map(|part| match part {
                "win" => "loss",
                "loss" => "win",
                "blessed" => "cursed",
                "cursed" => "blessed",
                other => other,
            })
            .collect::<Vec<_>>()
            .join("-")
    } else {
        value.to_string()
    };
    value.replace('-', " ")
}

fn count(n: Option<i64>) -> u64 {
    n.map_or(0, |n| n.max(0) as u64)
}

fn games(entry: &LookupMove) -> u64 {
    count(entry.white) + count(entry.draws) + count(entry.black)
}

/// How the games from a position ended, White, draws and Black; a share is
/// labelled once it is wider than `min_label` percent.
pub fn result_bar(entry: &LookupMove, min_label: u64, palette: &Palette) -> impl IntoElement {
    let counts = [count(entry.white), count(entry.draws), count(entry.black)];
    let total: u64 = counts.iter().sum();
    let looks = [
        (rgb(0xf5f5f5), rgb(0x222222)),
        (rgb(0x9e9e9e), rgb(0xffffff)),
        (rgb(0x333333), rgb(0xeeeeee)),
    ];
    let first = counts.iter().position(|&part| part > 0);
    let last = counts.iter().rposition(|&part| part > 0);
    let title = SharedString::from(format!(
        "White {} · Draw {} · Black {}",
        counts[0], counts[1], counts[2]
    ));
    h_flex()
        .id(SharedString::from(format!(
            "result-bar-{}-{}-{}",
            counts[0], counts[1], counts[2]
        )))
        .flex_1()
        .min_w(px(40.))
        .h(px(16.))
        .rounded(radii::XS)
        .overflow_hidden()
        .border_1()
        .border_color(palette.border)
        .text_size(px(11.))
        .line_height(px(14.))
        .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
        .children(counts.into_iter().zip(looks).enumerate().map(
            move |(index, (part, (bg, fg)))| {
                let pct = if total == 0 {
                    0
                } else {
                    (100. * part as f64 / total as f64).round() as u64
                };
                let ends = (Some(index) == first, Some(index) == last);
                widgets::bar_segment(div(), ends, radii::XS - px(1.))
                    .h_full()
                    .w(relative(pct as f32 / 100.))
                    .overflow_hidden()
                    .bg(bg)
                    .text_color(fg)
                    .flex()
                    .justify_center()
                    .when(pct > min_label, |part| part.child(format!("{pct}%")))
            },
        ))
}

impl PositionExplorer {
    fn render_filters(&self, prefs: &Prefs, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let toggle = |id: SharedString, label: String, on: bool| {
            Button::new(id)
                .small()
                .label(label)
                .when(on, |button| button.primary())
                .when(!on, |button| button.ghost())
        };
        v_flex()
            .gap(px(8.))
            .text_size(px(12.))
            .when(prefs.kind != PositionLookupKind::Masters, |filters| {
                filters.child(h_flex().flex_wrap().gap(px(4.)).children(
                    ExplorerSpeed::ALL.iter().map(|&speed| {
                        toggle(
                            SharedString::from(format!("speed-{}", speed.as_str())),
                            speed_label(speed).into(),
                            prefs.speeds.contains(&speed),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_prefs(cx, |prefs| {
                                prefs.speeds = toggled(&prefs.speeds, &speed)
                            });
                            this.filters_changed(cx);
                        }))
                    }),
                ))
            })
            .when(prefs.kind == PositionLookupKind::Opening, |filters| {
                filters.child(
                    h_flex()
                        .flex_wrap()
                        .gap(px(4.))
                        .children(EXPLORER_RATINGS.map(|rating| {
                            toggle(
                                SharedString::from(format!("rating-{rating}")),
                                format!("{rating}"),
                                prefs.ratings.contains(&rating),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.update_prefs(cx, |prefs| {
                                        prefs.ratings = toggled(&prefs.ratings, &rating)
                                    });
                                    this.filters_changed(cx);
                                },
                            ))
                        }))
                        .child(
                            div()
                                .text_color(palette.text_muted)
                                .child("(none selected: all ratings)"),
                        ),
                )
            })
            .child(
                h_flex().gap(px(8.)).child("Since").child(
                    div()
                        .w(px(96.))
                        .child(Input::new(&self.since).xsmall().aria_label("Since")),
                ),
            )
    }

    fn render_player(&self, prefs: &Prefs, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let colors = [Color::White, Color::Black];
        let chosen = colors
            .iter()
            .position(|color| *color == prefs.color)
            .unwrap_or_default();
        h_flex()
            .flex_wrap()
            .gap(px(8.))
            .child(
                div()
                    .w(px(144.))
                    .child(Input::new(&self.player).xsmall().aria_label("Player")),
            )
            .children(
                self.accounts
                    .iter()
                    .filter(|name| **name != prefs.player)
                    .enumerate()
                    .map(|(index, name)| {
                        let name = name.clone();
                        Button::new(("explorer-account", index))
                            .small()
                            .outline()
                            .label(name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_player(name.clone(), window, cx)
                            }))
                    }),
            )
            .child(widgets::pills(
                "explorer-color",
                vec![
                    Pill {
                        icon: None,
                        label: "as White".into(),
                        badge: None,
                        disabled: false,
                    },
                    Pill {
                        icon: None,
                        label: "as Black".into(),
                        badge: None,
                        disabled: false,
                    },
                ],
                chosen,
                {
                    let explorer = cx.entity().downgrade();
                    move |index, _, cx| {
                        let _ = explorer.update(cx, |this, cx| {
                            this.update_prefs(cx, |prefs| prefs.color = colors[index]);
                            this.filters_changed(cx);
                        });
                    }
                },
                true,
                palette,
            ))
    }

    fn render_moves(
        &mut self,
        result: &PositionLookup,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let moves = &result.saved.moves;
        let tablebase = result.saved.kind == PositionLookupKind::Tablebase;
        if self.layout == Layout::Grid {
            let fen = result.saved.fen.clone();
            let total = result
                .saved
                .total
                .map(|n| n.max(0) as u64)
                .filter(|total| *total > 0)
                .unwrap_or_else(|| moves.iter().map(games).sum());
            let width = self.area.size().width;
            let (columns, tile) = widgets::columns(width.max(TILE_MIN), TILE_MIN, TILE_GAP);
            let mut shown = std::collections::HashSet::new();
            let tiles: Vec<AnyElement> = moves
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let (uci, san) = (entry.uci.clone(), entry.san.clone());
                    let next =
                        kchess_domain::replay::replay_positions(&fen, std::slice::from_ref(&uci))
                            .get(1)
                            .map_or_else(|| fen.clone(), |position| position.fen.clone());
                    let share = if total == 0 {
                        0
                    } else {
                        (100. * games(entry) as f64 / total as f64).round() as u64
                    };
                    let opening = self.moves.as_ref().and_then(|moves| {
                        let line: Vec<String> =
                            moves.iter().cloned().chain([uci.clone()]).collect();
                        kchess_domain::training::openings::opening_at(
                            "standard",
                            STANDARD_FEN,
                            &line,
                            line.len() as f64,
                        )
                        .map(|opening| opening.name)
                    });
                    let board = self.preview(&next, &uci, cx);
                    shown.insert(format!("{next} {uci}"));
                    let label = match &opening {
                        Some(name) => format!("{san}, {name}"),
                        None => san.clone(),
                    };
                    v_flex()
                        .id(("explorer-tile", index))
                        .test_support()
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .w(tile)
                        .gap(px(4.))
                        .p(px(6.))
                        .border_1()
                        .border_color(palette.border)
                        .rounded(radii::MD)
                        .text_size(px(13.))
                        .cursor_pointer()
                        .pressable(palette)
                        .aria_label(label)
                        .on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.emit(ExplorerEvent::Play(uci.clone()))
                            }),
                        )
                        .child(div().w_full().h(tile - px(14.)).child(board))
                        .child(
                            h_flex()
                                .justify_between()
                                .items_baseline()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::MEDIUM).child(
                                    crate::notation::shown(&san, &crate::notation::figurine(cx)),
                                ))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(palette.text_muted)
                                        .child(format!("{share}%")),
                                ),
                        )
                        .children(opening.map(|name| {
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .truncate()
                                .child(name)
                        }))
                        .child(h_flex().child(result_bar(entry, 20, palette)))
                        .into_any_element()
                })
                .collect();
            // Boards of positions no longer shown are let go: each position stepped through
            // added a board per move, for as long as the page was open.
            self.previews.retain(|key| shown.contains(key));
            let mut rows = Vec::new();
            let mut tiles = tiles.into_iter().peekable();
            while tiles.peek().is_some() {
                rows.push(
                    h_flex()
                        .gap(TILE_GAP)
                        .items_start()
                        .children(tiles.by_ref().take(columns)),
                );
            }
            return v_flex()
                .w_full()
                .gap(TILE_GAP)
                .children(rows)
                .into_any_element();
        }
        let figurine = crate::notation::figurine(cx);
        let (fen, variant) = (&result.saved.fen, self.variant.unwrap_or_default());
        v_flex()
            .gap(px(4.))
            .children(moves.iter().enumerate().map(|(index, entry)| {
                let (uci, san) = (entry.uci.clone(), entry.san.clone());
                // The position the move reaches, on hover, as the grid's tiles show it.
                let next = kchess_domain::replay::replay_positions_in(
                    variant,
                    fen,
                    std::slice::from_ref(&uci),
                )
                .get(1)
                .map(|position| position.fen.clone());
                let tip = next.map(|next| {
                    crate::board::preview_tip(next, uci.clone(), variant, Color::White)
                });
                h_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .id(("explorer-move", index))
                            .test_support()
                            .role(Role::Button)
                            .keyboard_focus(palette.primary)
                            .aria_label(format!("Play {san}"))
                            .w(px(56.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.primary)
                            .cursor_pointer()
                            .pressable_link()
                            .when_some(tip, |cell, tip| cell.tooltip(tip))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ExplorerEvent::Play(uci.clone()))
                            }))
                            .child(crate::notation::shown(&san, &figurine)),
                    )
                    .when(!tablebase, |row| {
                        row.child(
                            div()
                                .w(px(64.))
                                .text_right()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(widgets::format_count(games(entry) as f64)),
                        )
                        .child(result_bar(entry, 12, palette))
                    })
                    .when(tablebase, |row| {
                        row.child(div().text_size(px(12.)).child(format!(
                            "{} for the current player",
                            category(entry.category.as_deref().unwrap_or_default(), true)
                        )))
                    })
            }))
            .into_any_element()
    }

    fn render_games(
        &self,
        result: &PositionLookup,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Option<Div> {
        let list = result
            .saved
            .games
            .as_ref()
            .filter(|games| !games.is_empty())?;
        let masters = result.saved.kind == PositionLookupKind::Masters;
        let title = if masters { "Top games" } else { "Recent games" };
        Some(
            v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_muted)
                        .child(title),
                )
                .children(list.iter().enumerate().map(|(index, game)| {
                    let rating = |rating: Option<f64>| {
                        rating.map(|rating| format!("{rating}")).unwrap_or_default()
                    };
                    let score = match game.winner {
                        Some(Color::White) => "1-0",
                        Some(Color::Black) => "0-1",
                        None => "½-½",
                    };
                    let when = game
                        .year
                        .map(|year| year.to_string())
                        .or_else(|| game.month.clone())
                        .unwrap_or_default();
                    let muted = palette.text_muted;
                    let chosen = game.clone();
                    let guessed = game.clone();
                    let row = h_flex()
                        .id(("explorer-game", index))
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label("Open game")
                        .justify_between()
                        .gap(px(8.))
                        .px(px(4.))
                        .py(px(2.))
                        .rounded(radii::SM)
                        .text_size(px(12.))
                        .cursor_pointer()
                        .pressable(palette)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_game(chosen.clone(), window, cx)
                        }))
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap(px(4.))
                                .overflow_hidden()
                                .child(game.white.clone())
                                .child(div().text_color(muted).child(rating(game.white_rating)))
                                .child("–")
                                .child(game.black.clone())
                                .child(div().text_color(muted).child(rating(game.black_rating))),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(muted)
                                .child(format!("{score} {when}")),
                        );
                    // A master game can be guessed through, move by move, on Practice.
                    h_flex()
                        .gap(px(2.))
                        .child(row.flex_1().min_w_0())
                        .when(masters, |line| {
                            line.child(
                                Button::new(("explorer-guess", index))
                                    .ghost()
                                    .small()
                                    .icon(icon("target"))
                                    .tooltip("Guess the moves")
                                    .accessibility_label("Guess the moves")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.guess_game(guessed.clone(), window, cx)
                                    })),
                            )
                        })
                })),
        )
    }
}

impl Render for PositionExplorer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let prefs = self.shown_prefs(cx);
        let kinds: Vec<PositionLookupKind> = KINDS
            .into_iter()
            .filter(|kind| self.layout.offers(*kind))
            .filter(|kind| !self.variants_only() || holds_variants(*kind))
            .collect();
        let chosen = kinds
            .iter()
            .position(|kind| *kind == prefs.kind)
            .unwrap_or_default();
        let needs_player = self.needs_player(cx);
        let choose_kind = {
            let explorer = cx.entity().downgrade();
            let kinds = kinds.clone();
            move |index: usize, window: &mut Window, cx: &mut App| {
                let kind = kinds[index];
                let _ = explorer.update(cx, |this, cx| {
                    this.lookup(Some(kind), true, Some(window), cx)
                });
            }
        };
        let head = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .child(widgets::pills(
                "explorer-kind",
                kinds
                    .iter()
                    .map(|kind| Pill {
                        icon: None,
                        label: kind_label(*kind).into(),
                        badge: None,
                        disabled: false,
                    })
                    .collect(),
                chosen,
                choose_kind,
                true,
                &palette,
            ))
            .when(
                !matches!(
                    prefs.kind,
                    PositionLookupKind::Tablebase | PositionLookupKind::Masters
                ),
                |head| {
                    head.child(
                        Button::new("explorer-filters")
                            .small()
                            .size(px(24.))
                            .ghost()
                            .icon(icon("sliders-horizontal"))
                            .selected(self.show_filters)
                            .tooltip("Explorer filters")
                            .accessibility_label("Explorer filters")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_filters = !this.show_filters;
                                cx.notify();
                            })),
                    )
                },
            )
            .child(
                div()
                    .id("explorer-about")
                    .text_color(palette.text_muted)
                    .tooltip(|window, cx| {
                        Tooltip::new(
                            "Lookups send this position to Lichess. The Lichess, Masters and \
                             Player databases use a connected account; saved results stay \
                             available offline. Statistics describe games, not move quality.",
                        )
                        .build(window, cx)
                    })
                    .child(icon("info").size(px(14.))),
            );
        let status = if self.busy {
            Some(if prefs.kind == PositionLookupKind::Player {
                "Indexing the player’s games — this can take a while the first time…"
            } else {
                "Loading position lookup…"
            })
        } else {
            None
        };
        let result = self.result.clone();
        let body = result.map(|result| {
            let total = result.saved.total.filter(|total| *total > 0);
            let fetched = crate::dates::local_format(result.saved.fetched_at as f64, "%b %-d, %Y");
            let empty = result.saved.moves.is_empty();
            v_flex()
                .gap(px(8.))
                .text_size(px(13.))
                .child(
                    h_flex()
                        .gap(px(4.))
                        .child(self.heading(&result, &prefs))
                        .children(total.map(|total| {
                            div()
                                .text_color(palette.text_muted)
                                .child(format!("· {} games", widgets::format_count(total as f64)))
                        })),
                )
                .when(result.saved.kind == PositionLookupKind::Tablebase, |body| {
                    let dtz = result
                        .saved
                        .dtz
                        .flatten()
                        .map_or_else(|| "unknown".to_string(), |dtz| dtz.to_string());
                    body.child(
                        div()
                            .id("tablebase-dtz")
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .underline()
                            .tooltip(|window, cx| {
                                Tooltip::new(
                                    "Plies to a pawn move or capture; not distance to mate. \
                                     Cursed wins and blessed losses account for the fifty-move rule.",
                                )
                                .build(window, cx)
                            })
                            .child(format!("DTZ {dtz}")),
                    )
                })
                .when(result.cached, |body| {
                    body.child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!(
                                "{} · {fetched}",
                                result.message.as_deref().unwrap_or("Saved lookup")
                            )),
                    )
                })
                .when(empty, |body| body.child("No moves found for this position"))
                .child(self.render_moves(&result, cx, &palette))
                .children(self.render_games(&result, cx, &palette))
        });
        v_flex()
            .relative()
            .w_full()
            .gap(px(8.))
            .child(self.area.probe(cx))
            .child(head)
            .when(prefs.kind == PositionLookupKind::Player, |explorer| {
                explorer.child(self.render_player(&prefs, cx, &palette))
            })
            .when(
                self.show_filters && prefs.kind != PositionLookupKind::Tablebase,
                |explorer| explorer.child(self.render_filters(&prefs, cx, &palette)),
            )
            .when(self.layout == Layout::List, |explorer| {
                explorer.child(
                    h_flex()
                        .flex_wrap()
                        .gap(px(12.))
                        .text_size(px(13.))
                        .child(
                            Button::new("explorer-lookup")
                                .xsmall()
                                .min_h(px(24.))
                                .outline()
                                .icon(icon("search"))
                                .label("Look up this position")
                                .loading(self.busy)
                                .disabled(needs_player)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.lookup(None, true, Some(window), cx)
                                })),
                        )
                        .child(widgets::labelled_switch(
                            "explorer-follow",
                            "Follow moves",
                            prefs.follow,
                            false,
                            {
                                let explorer = cx.entity().downgrade();
                                move |on, _, cx| {
                                    let on = *on;
                                    let _ = explorer.update(cx, |this, cx| {
                                        this.update_prefs(cx, |prefs| prefs.follow = on);
                                        if on {
                                            this.lookup(None, false, None, cx);
                                        }
                                    });
                                }
                            },
                        )),
                )
            })
            .when(needs_player, |explorer| {
                explorer.child(widgets::hint(
                    "Enter a Lichess username to see the moves they played here",
                    &palette,
                ))
            })
            .children(status.map(|status| widgets::loading(status, &palette)))
            .children(self.failure.as_ref().map(|failure| {
                if failure.kind == Some(ErrorKind::Login) {
                    // The account is the way out, here rather than in Settings.
                    return h_flex()
                        .id("explorer-signed-out")
                        .flex_wrap()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .text_color(palette.text_muted)
                        .child("No Lichess account connected")
                        .child(widgets::connect_lichess("explorer-connect", cx).small())
                        .into_any_element();
                }
                // The list has its own Look up beside the switch; the grid only this.
                if self.layout == Layout::List {
                    return div()
                        .text_size(px(13.))
                        .text_color(palette.error)
                        .child(failure.message.clone())
                        .into_any_element();
                }
                widgets::retry_line(
                    "explorer-retry",
                    failure.message.clone(),
                    palette.error,
                    cx.listener(|this, _, window, cx| this.lookup(None, true, Some(window), cx)),
                )
                .into_any_element()
            }))
            .children(body)
    }
}
