//! The Players page's cards: the search, the profile, the record against you, a speed's board,
//! the days played and the recent games.

use super::*;

impl PlayersPage {
    pub(super) fn search(
        &self,
        followed: &[String],
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Div {
        let online = crate::network::online(cx);
        let typed = !self.input.read(cx).value().trim().is_empty();
        let clear = Button::new("players-recent-clear")
            .ghost()
            .small()
            .icon(icon("x"))
            // gpui-lints: allow(icon_per_label) empties the recent names; Clear elsewhere deletes
            .label("Clear")
            .accessibility_label("Clear recent players")
            .on_click(cx.listener(|_, _, _, cx| {
                player_search::clear_recent(GameSite::Lichess, cx);
                cx.notify();
            }));
        // Names typed so far match these players; otherwise the ones looked up lately and
        // followed, the player open (already in the field) left out.
        let known = vec![
            NameGroup {
                action: Some(clear.into_any_element()),
                ..NameGroup::new(
                    "Recent",
                    "players-suggestion",
                    player_search::recent(GameSite::Lichess, cx),
                )
            },
            NameGroup::new("Following", "players-followed", followed.to_vec()),
        ];
        let page = cx.entity().downgrade();
        let groups = player_search::name_rows(
            "players-match",
            &self.names,
            known,
            self.username().as_deref(),
            palette,
            |name| {
                let page = page.clone();
                Box::new(move |_, window, cx| {
                    page.update(cx, |page, cx| page.show_player(&name, window, cx))
                        .ok();
                })
            },
        );
        widgets::card(palette)
            .gap(px(12.))
            .child(
                h_flex()
                    .w_full()
                    .gap(px(8.))
                    .child(
                        // A name is at most 30 characters: the field stops at 480px.
                        div().flex_1().min_w(px(192.)).max_w(px(480.)).child(
                            Input::new(&self.input)
                                .prefix(icon("user-search").size(px(16.)))
                                .cleanable(true)
                                .aria_label("Lichess username"),
                        ),
                    )
                    .child(
                        Button::new("players-lookup")
                            .primary()
                            .icon(icon("search"))
                            .label("Look up")
                            .loading(self.profile_loading.busy())
                            .disabled(!online || !typed)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.lookup(None, window, cx)),
                            ),
                    ),
            )
            .children(self.profile_loading.error().cloned().map(|error| {
                h_flex()
                    .id("players-error")
                    .test_support()
                    .aria_label(error.clone())
                    .gap(px(6.))
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(icon("circle-alert").size(px(14.)))
                    .child(error)
            }))
            .when(!groups.is_empty(), |card| {
                card.child(v_flex().gap(px(8.)).children(groups))
            })
    }

    pub(super) fn profile_card(
        &self,
        profile: &LichessUser,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Div {
        let username = profile.username.clone();
        let accounts = crate::accounts::handle(cx).read(cx);
        let me = accounts.active(cx);
        let connected = !accounts.connected().is_empty();
        let followed = accounts
            .all()
            .iter()
            .any(|account| account.username.eq_ignore_ascii_case(&username));
        let busy = accounts.busy;
        let not_me = me
            .as_deref()
            .is_none_or(|me| !me.eq_ignore_ascii_case(&username));
        let status = self.status.clone().unwrap_or_default();
        let playing_id = status.playing_id.clone();
        let title = profile.title.as_deref();
        let mut line = format!(
            "{} games · joined {}",
            widgets::format_count(
                profile
                    .count
                    .as_ref()
                    .and_then(|count| count.all)
                    .unwrap_or_default() as f64
            ),
            profile
                .created_at
                .map_or("?".into(), |at| local_date(at as f64))
        );
        if let Some(seen) = profile.seen_at {
            line.push_str(&format!(" · seen {}", local_date(seen as f64)));
        }
        let score = self
            .record
            .as_ref()
            .zip(me.as_ref())
            .and_then(|(record, me)| {
                let games = Some(record.nb_games).filter(|games| *games > 0)?;
                let of = |users: &std::collections::BTreeMap<String, f64>, name: &str| {
                    users.get(&name.to_lowercase()).copied().unwrap_or_default()
                };
                let mut text = format!(
                    "You (@{me}) {} – {} {username} over {games} games",
                    of(&record.users, me),
                    of(&record.users, &username)
                );
                if let Some(matchup) = record.matchup.as_ref().filter(|today| today.nb_games > 0) {
                    text.push_str(&format!(
                        " · today {} – {}",
                        of(&matchup.users, me),
                        of(&matchup.users, &username)
                    ));
                }
                Some(text)
            });
        let (follow_name, challenge_name) = (username.clone(), username.clone());
        widgets::card(palette)
            .gap(px(12.))
            .child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        v_flex()
                            .gap(px(2.))
                            .child(
                                h_flex()
                                    .gap(px(8.))
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .children(title.map(|title| {
                                        widgets::badge(title.to_string(), Look::Primary, palette)
                                    }))
                                    .child(username.clone())
                                    .when(status.online, |row| {
                                        row.child(widgets::badge(
                                            if status.playing { "Playing" } else { "Online" },
                                            Look::Success,
                                            palette,
                                        ))
                                    }),
                            )
                            .child(widgets::hint(line, palette)),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .when(!followed, |row| {
                                row.child(
                                    Button::new("players-follow")
                                        .small()
                                        .outline()
                                        .icon(icon("user-plus"))
                                        .label("Follow player")
                                        .disabled(busy)
                                        .on_click(move |_, window, cx| {
                                            follow(follow_name.clone(), window, cx)
                                        }),
                                )
                            })
                            .when(connected && not_me, |row| {
                                row.child(
                                    Button::new("players-challenge")
                                        .small()
                                        .primary()
                                        .icon(icon("swords"))
                                        .label("Challenge")
                                        .on_click(move |_, window, cx| {
                                            crate::pages::online::challenge(
                                                &challenge_name,
                                                window,
                                                cx,
                                            )
                                        }),
                                )
                                .child(
                                    Button::new("players-message")
                                        .small()
                                        .outline()
                                        .icon(icon("message-circle"))
                                        .label("Message")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_message(window, cx)
                                        })),
                                )
                            })
                            .children(playing_id.map(|id| {
                                Button::new("players-watch")
                                    .small()
                                    .outline()
                                    .icon(icon("tv"))
                                    .label("Watch")
                                    .on_click(move |_, window, cx| {
                                        crate::pages::watch::watch_game(&id, window, cx)
                                    })
                            }))
                            .children(self.relation.as_ref().map(|relation| {
                                let label = if relation.blocking {
                                    "Unblock"
                                } else {
                                    "Block"
                                };
                                let page = cx.entity().downgrade();
                                Button::new("players-more")
                                    .small()
                                    .ghost()
                                    .icon(icon("ellipsis"))
                                    .tooltip("More actions")
                                    .accessibility_label("More actions")
                                    .disabled(!crate::network::online(cx))
                                    .dropdown_menu(move |menu, _, _| {
                                        let page = page.clone();
                                        menu.item(
                                            PopupMenuItem::new(label).icon(icon("ban")).on_click(
                                                move |_, window, cx| {
                                                    page.update(cx, |page, cx| {
                                                        page.toggle_block(window, cx)
                                                    })
                                                    .ok();
                                                },
                                            ),
                                        )
                                    })
                            })),
                    ),
            )
            .when(
                self.relation
                    .as_ref()
                    .is_some_and(|relation| relation.blocking),
                |card| {
                    card.child(widgets::hint(
                        format!("Blocked by @{}", me.clone().unwrap_or_default()),
                        palette,
                    ))
                },
            )
            .when(!self.extras_failed.is_empty(), |card| {
                card.child(widgets::hint(
                    format!("Could not load {}", self.extras_failed.join(" or ")),
                    palette,
                ))
            })
            .children(
                profile
                    .profile
                    .as_ref()
                    .and_then(|profile| profile.bio.clone())
                    .map(|bio| div().text_size(px(13.)).child(bio)),
            )
            .children(score.map(|score| {
                h_flex()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(radii::BOX)
                    .bg(palette.elevated)
                    .text_size(px(13.))
                    .child(icon("scale").size(px(16.)))
                    .child(score)
            }))
            .child(self.history.clone())
    }

    pub(super) fn record_card(&self, palette: &Palette, cx: &mut Context<Self>) -> Div {
        let mut card = widgets::card(palette).gap(px(8.)).child(
            h_flex()
                .justify_between()
                .items_start()
                .gap(px(8.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(widgets::heading("Record", palette))
                        .children(self.stats.as_ref().map(|stats| {
                            let mut parts = Vec::new();
                            if let Some(deviation) = stats.deviation {
                                parts.push(format!("± {}", deviation.round()));
                            }
                            if let Some(rank) = stats.rank {
                                parts.push(format!("rank #{rank}"));
                            }
                            if let Some(percentile) = stats.percentile {
                                parts.push(format!("better than {percentile}% of players"));
                            }
                            h_flex()
                                .flex_wrap()
                                .gap(px(8.))
                                .items_baseline()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(
                                    div()
                                        .text_size(px(16.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(palette.highlighted)
                                        .child(
                                            stats
                                                .rating
                                                .map_or("–".into(), |r| r.round().to_string()),
                                        ),
                                )
                                .child(parts.join(" · "))
                        })),
                )
                .child(
                    div().w(px(150.)).flex_none().child(
                        Select::new(&self.perf_select)
                            .small()
                            .accessibility_label("Rating speed"),
                    ),
                ),
        );
        if let Some(error) = self.stats_loading.error() {
            return card.child(widgets::retry_line(
                "players-stats-retry",
                error.clone(),
                palette.error,
                cx.listener(|this, _, _, cx| this.load_stats(cx)),
            ));
        }
        let Some(stats) = &self.stats else {
            return card.child(widgets::loading("Loading…", palette));
        };
        let counts = stats.count.clone().unwrap_or_default();
        let count = |value: Option<i64>| value.unwrap_or_default() as f64;
        let mark = |mark: &Option<kchess_contracts::types::PerfMark>| match mark {
            Some(mark) => format!(
                "{} · {}",
                mark.rating,
                crate::dates::written_date(&mark.at)
                    .map(|at| at.strftime("%-d %b %Y").to_string())
                    .unwrap_or_default()
            ),
            None => "–".into(),
        };
        let streak = |streak: kchess_contracts::types::PerfStreak, word: &str| {
            format!("{} now · {word} {}", streak.current, streak.best)
        };
        let rows = [
            (
                "Games",
                format!("{} · {} rated", count(counts.all), count(counts.rated)),
            ),
            (
                "Opponents' average",
                counts.op_avg.unwrap_or_default().round().to_string(),
            ),
            ("Time played", hours(count(counts.seconds))),
            (
                "Tournament games",
                format!(
                    "{} · berserked {}",
                    count(counts.tour),
                    count(counts.berserk)
                ),
            ),
            ("Highest", mark(&stats.highest)),
            ("Lowest", mark(&stats.lowest)),
            ("Winning streak", streak(stats.win_streak, "best")),
            ("Losing streak", streak(stats.loss_streak, "worst")),
        ];
        card = card
            .child(widgets::result_row(
                "Results",
                None,
                (count(counts.win), count(counts.draw), count(counts.loss)),
                palette,
            ))
            .child(v_flex().children(rows.into_iter().map(|(label, value)| {
                h_flex()
                    .justify_between()
                    .gap(px(16.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(palette.border)
                    .text_size(px(13.))
                    .child(div().text_color(palette.text_muted).child(label))
                    .child(value)
            })));
        let opponents =
            |title: &'static str, list: &[PerfResult], none: &'static str, id: &'static str| {
                let entries = list.to_vec();
                v_flex()
                    .flex_1()
                    .min_w(px(160.))
                    .gap(px(4.))
                    .text_size(px(13.))
                    .child(div().font_weight(FontWeight::MEDIUM).child(title))
                    .when(entries.is_empty(), |list| {
                        list.child(div().text_color(palette.text_muted).child(none))
                    })
                    .children(entries.into_iter().enumerate().map(move |(index, entry)| {
                        let name = entry.opponent.clone();
                        h_flex()
                            .justify_between()
                            .gap(px(12.))
                            .child(player_link((id, index), name, palette.primary))
                            .child(
                                div()
                                    .text_color(palette.text_muted)
                                    .child(entry.opponent_rating.to_string()),
                            )
                    }))
            };
        card.child(
            h_flex()
                .flex_wrap()
                .items_start()
                .gap(px(16.))
                .mt(px(8.))
                .child(opponents(
                    "Best rated wins",
                    &stats.best_wins,
                    "None yet",
                    "best-win",
                ))
                .child(opponents(
                    "Worst rated losses",
                    &stats.worst_losses,
                    "None",
                    "worst-loss",
                )),
        )
    }

    pub(super) fn board_card(&self, palette: &Palette, cx: &mut Context<Self>) -> Div {
        let card = widgets::card(palette).gap(px(8.)).child(
            // On a narrow window the filters drop under the heading.
            h_flex()
                .flex_wrap()
                .gap(px(8.))
                .justify_between()
                .items_center()
                .child(widgets::heading("Leaderboard", palette))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .child(
                            div().w(px(144.)).child(
                                Select::new(&self.title_select)
                                    .small()
                                    .disabled(self.board.is_none())
                                    .accessibility_label("Leaderboard titles"),
                            ),
                        )
                        .child(
                            div().w(px(160.)).child(
                                Select::new(&self.board_select)
                                    .small()
                                    .accessibility_label("Leaderboard speed"),
                            ),
                        ),
                ),
        );
        if let Some(error) = self.board_loading.error() {
            return card.child(widgets::retry_line(
                "players-board-retry",
                error.clone(),
                palette.error,
                cx.listener(|this, _, _, cx| this.load_board(cx)),
            ));
        }
        let Some(board) = &self.board else {
            return card.child(widgets::loading("Loading…", palette));
        };
        let filter = self.board_title.as_str();
        let shown: Vec<(usize, &LeaderboardEntry)> = board
            .iter()
            .enumerate()
            .filter(|(_, entry)| title_shown(filter, entry.title.as_deref()))
            .collect();
        if shown.is_empty() && !board.is_empty() {
            let none = match filter {
                "titled" => "No titled players in this leaderboard".to_string(),
                title => format!("No {title} in this leaderboard"),
            };
            return card.child(
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .child(widgets::hint(none, palette))
                    .child(
                        widgets::text_button("players-board-all")
                            .label("Show all players")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_titles("all".to_string(), cx)
                            })),
                    ),
            );
        }
        let numeric = |width: f32| {
            div()
                .w(px(width))
                .flex_none()
                .text_right()
                .font_features(widgets::tabular())
        };
        card.child(
            v_flex()
                .id("players-board")
                .max_h(px(512.))
                .overflow_y_scroll()
                .text_size(px(13.))
                .children(shown.into_iter().map(|(index, entry)| {
                    let name = entry.username.clone();
                    let progress = entry.progress;
                    // Rank, title, name, rating and change, each in its own column, so the
                    // names and the digits line up down the board.
                    h_flex()
                        .id(("players-board-row", index))
                        .test_support()
                        .gap(px(12.))
                        .py(px(4.))
                        .border_t_1()
                        .border_color(palette.border)
                        .child(
                            numeric(28.)
                                .text_color(palette.text_muted)
                                .child((index + 1).to_string()),
                        )
                        .child(
                            div()
                                .id(("players-board-title", index))
                                .test_support()
                                .w(px(32.))
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.warning)
                                .children(entry.title.clone()),
                        )
                        .child(
                            div()
                                .id(("players-board-player", index))
                                .test_support()
                                .flex_1()
                                .min_w_0()
                                .child(player_link(
                                    ("players-board-name", index),
                                    name,
                                    palette.primary,
                                )),
                        )
                        .child(numeric(48.).child(entry.rating.round().to_string()))
                        .child(
                            numeric(40.)
                                .id(("players-board-change", index))
                                // Lichess's progress counts games, not days.
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "Rating change over the last 12 games",
                                    )
                                    .build(window, cx)
                                })
                                .text_size(px(12.))
                                .text_color(if progress > 0. {
                                    palette.success
                                } else if progress < 0. {
                                    palette.error
                                } else {
                                    palette.text_muted
                                })
                                .child(match progress {
                                    p if p > 0. => format!("+{p}"),
                                    p if p < 0. => p.to_string(),
                                    _ => String::new(),
                                }),
                        )
                })),
        )
    }

    pub(super) fn days_card(&self, narrow: bool, palette: &Palette) -> Div {
        let card = widgets::card(palette)
            .gap(px(8.))
            .child(widgets::heading("Activity", palette));
        if self.days_error {
            return card.child(widgets::hint(
                "Lichess isn’t sending this player’s activity",
                palette,
            ));
        }
        match &self.days {
            None => card.child(widgets::loading("Loading…", palette)),
            Some(days) if days.is_empty() => {
                card.child(widgets::hint("No recent activity", palette))
            }
            Some(days) => card.child(crate::activity::activity_list(
                "players-activity",
                &crate::activity::activity_groups(days),
                narrow,
                palette,
            )),
        }
    }

    pub(super) fn games_card(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let card = widgets::card(palette)
            .gap(px(8.))
            .child(widgets::heading("Recent games", palette));
        if self.games_error {
            return card.child(widgets::retry_line(
                "players-games-retry",
                "Lichess isn’t sending this player’s games right now",
                palette.text_muted,
                cx.listener(|this, _, _, cx| this.load_activity(cx)),
            ));
        }
        match &self.games {
            None => card.child(widgets::loading("Loading…", palette)),
            Some(games) if games.is_empty() => card.child(widgets::hint("No games yet", palette)),
            Some(games) => card.children(games.iter().enumerate().map(|(index, game)| {
                let chosen = game.clone();
                game_row(
                    ("players-game", index),
                    game,
                    None,
                    RowDetail::NoReview,
                    move |window, cx| {
                        kchess_board::analysis::open_lichess_game(chosen.clone(), window, cx)
                    },
                    palette,
                )
            })),
        }
    }
}
