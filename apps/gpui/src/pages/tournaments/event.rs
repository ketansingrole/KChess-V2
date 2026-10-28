//! One tournament (`TournamentEvent.vue`): joining, its details, the standings, the top game,
//! the games in progress and the totals.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use shakmaty::Color;

use super::{TournamentsPage, control};
use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::game::rules;
use crate::pages::players::player_link;
use crate::theme::Palette;
use crate::widgets::{self, Look};

/// "Starts in 5 min", "12 min left", "Next round in 2 min", "Finished".
pub fn status_line(detail: &Value) -> String {
    let minutes = |key: &str| {
        detail[key]
            .as_f64()
            .filter(|seconds| *seconds > 0.)
            .map(|seconds| (seconds / 60.).ceil())
    };
    if detail["status"] == json!("finished") {
        "Finished".into()
    } else if let Some(m) = minutes("secondsToStart") {
        format!("Starts in {m} min")
    } else if let Some(m) = minutes("secondsToFinish") {
        format!("{m} min left")
    } else if let Some(m) = minutes("nextRoundIn") {
        format!("Next round in {m} min")
    } else {
        "Under way".into()
    }
}

/// "3+2 · Rated · 60 min · Berserk allowed".
pub fn facts(detail: &Value) -> Vec<String> {
    let mut facts = vec![control(&detail["clock"])];
    if detail["variant"] != json!("standard")
        && let Some(name) = detail["variantName"].as_str()
    {
        facts.push(name.to_string());
    }
    facts.push(
        if detail["rated"] == json!(true) {
            "Rated"
        } else {
            "Casual"
        }
        .into(),
    );
    if let Some(minutes) = detail["minutes"].as_f64() {
        facts.push(format!("{minutes} min"));
    } else if let Some(rounds) = detail["nbRounds"].as_f64() {
        facts.push(format!("{rounds} rounds"));
    }
    if detail["berserkable"] == json!(true) {
        facts.push("Berserk allowed".into());
    }
    facts
}

impl TournamentsPage {
    fn joined_now(&self, detail: &Value, cx: &App) -> bool {
        if detail["me"].is_object() {
            return detail["me"]["withdraw"] != json!(true);
        }
        self.is_joined(
            detail["system"].as_str().unwrap_or_default(),
            detail["id"].as_str().unwrap_or_default(),
            Self::account(cx).as_deref(),
        )
    }

    pub(super) fn event_view(&mut self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let back = h_flex().child(
            Button::new("tournament-back")
                .ghost()
                .icon(icon("arrow-left"))
                .label("All tournaments")
                .on_click(cx.listener(|this, _, _, cx| this.close_event(cx))),
        );
        let open = self.open.as_ref();
        let detail = open.and_then(|open| open.detail.clone());
        let Some(detail) = detail else {
            let body = match open.and_then(|open| open.error.clone()) {
                Some(error) => div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error)
                    .into_any_element(),
                None => widgets::card(palette)
                    .child(widgets::hint("Loading…", palette))
                    .into_any_element(),
            };
            return v_flex()
                .gap(px(16.))
                .child(back)
                .child(body)
                .into_any_element();
        };
        let page = open.map_or(1, |open| open.page);
        v_flex()
            .gap(px(16.))
            .child(back)
            .child(self.event_head(&detail, cx, palette))
            .child(
                h_flex()
                    .items_start()
                    .flex_wrap()
                    .gap(px(20.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(320.))
                            .child(self.standings(&detail, page, cx, palette)),
                    )
                    .child(
                        v_flex()
                            .w(px(340.))
                            .gap(px(20.))
                            .children(self.featured(&detail, cx, palette))
                            .children(duels(&detail, palette))
                            .children(totals(&detail, palette)),
                    ),
            )
            .into_any_element()
    }

    fn event_head(&self, detail: &Value, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let joined = self.joined_now(detail, cx);
        let account = Self::account(cx);
        let finished = detail["status"] == json!("finished");
        let started = detail["status"] == json!("started");
        let accepted = detail["verdicts"]["accepted"] != json!(false);
        let playable = detail["playable"] == json!(true);
        let arena = detail["system"] == json!("arena");
        let actions = if finished {
            None
        } else if !joined {
            Some(
                h_flex()
                    .gap(px(8.))
                    .when(account.is_some(), |row| {
                        row.child(
                            div().w(px(180.)).child(
                                Input::new(&self.password)
                                    .mask_toggle()
                                    .small()
                                    .aria_label("Tournament entry code"),
                            ),
                        )
                    })
                    .child(
                        Button::new("tournament-join")
                            .primary()
                            .icon(icon("log-in"))
                            .label("Join")
                            .loading(self.busy)
                            .disabled(!playable || !accepted || self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.join(window, cx))),
                    ),
            )
        } else {
            let game = detail["me"]["gameId"].as_str().map(str::to_string);
            Some(
                h_flex()
                    .gap(px(8.))
                    .child(widgets::badge("You are in", Look::Success, palette))
                    .children(
                        detail["me"]["rank"]
                            .as_f64()
                            .map(|rank| div().text_size(px(13.)).child(format!("Rank {rank}"))),
                    )
                    .children(game.map(|id| {
                        Button::new("tournament-game")
                            .primary()
                            .icon(icon("swords"))
                            .label("Go to your game")
                            .on_click(move |_, window, cx| {
                                crate::shell::open_online_game(&id, window, cx)
                            })
                    }))
                    .child(
                        Button::new("tournament-leave")
                            .outline()
                            .label(if arena { "Pause" } else { "Withdraw" })
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.leave(window, cx))),
                    ),
            )
        };
        let mut line = facts(detail).join(" · ");
        line.push_str(&format!(
            " · {} players · ",
            detail["nbPlayers"].as_f64().unwrap_or_default()
        ));
        widgets::card(palette)
            .gap(px(14.))
            .child(
                h_flex()
                    .items_start()
                    .justify_between()
                    .flex_wrap()
                    .gap(px(12.))
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(detail["name"].as_str().unwrap_or_default().to_string()),
                            )
                            .child(
                                h_flex()
                                    .text_size(px(13.))
                                    .text_color(palette.text_muted)
                                    .child(line)
                                    .child(
                                        div()
                                            .font_weight(FontWeight::BOLD)
                                            .when(started, |text| text.text_color(palette.primary))
                                            .child(status_line(detail)),
                                    ),
                            ),
                    )
                    .children(actions),
            )
            .children(detail["problem"].as_str().map(|problem| {
                h_flex()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(palette.accented)
                    .text_size(px(13.))
                    .child(icon("eye").size(px(16.)))
                    .child(problem.to_string())
            }))
            .children(
                detail["description"]
                    .as_str()
                    .map(|text| div().text_size(px(13.)).child(text.to_string())),
            )
            .children(
                detail["verdicts"]["list"]
                    .as_array()
                    .filter(|list| !list.is_empty())
                    .map(|list| {
                        v_flex()
                            .gap(px(4.))
                            .text_size(px(13.))
                            .children(list.iter().map(|entry| {
                                let ok = entry["verdict"] == json!("ok");
                                h_flex()
                                    .gap(px(6.))
                                    .child(
                                        if ok {
                                            icon("circle-check")
                                        } else {
                                            icon("circle-x")
                                        }
                                        .size(px(14.))
                                        .text_color(
                                            if ok { palette.success } else { palette.error },
                                        ),
                                    )
                                    .child(
                                        entry["condition"].as_str().unwrap_or_default().to_string(),
                                    )
                                    .when(!ok, |row| {
                                        row.child(div().text_color(palette.text_muted).child(
                                            format!(
                                                "· {}",
                                                entry["verdict"].as_str().unwrap_or_default()
                                            ),
                                        ))
                                    })
                            }))
                    }),
            )
    }

    fn standings(
        &self,
        detail: &Value,
        page: usize,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Div {
        let pages =
            ((detail["nbPlayers"].as_f64().unwrap_or_default() / 10.).ceil() as usize).max(1);
        let me = Self::account(cx).unwrap_or_default().to_lowercase();
        let rows = detail["standing"].as_array().cloned().unwrap_or_default();
        widgets::card(palette)
            .gap(px(10.))
            .child(
                h_flex()
                    .justify_between()
                    .child(widgets::heading("Standings", palette))
                    .when(pages > 1, |header| {
                        header.child(
                            h_flex()
                                .gap(px(4.))
                                .child(
                                    Button::new("standings-previous")
                                        .xsmall()
                                        .ghost()
                                        .icon(icon("chevron-left"))
                                        .accessibility_label("Previous page")
                                        .disabled(page <= 1)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.go_to_page(page - 1, cx)
                                        })),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(palette.text_muted)
                                        .child(format!("{page} / {pages}")),
                                )
                                .child(
                                    Button::new("standings-next")
                                        .xsmall()
                                        .ghost()
                                        .icon(icon("chevron-right"))
                                        .accessibility_label("Next page")
                                        .disabled(page >= pages)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.go_to_page(page + 1, cx)
                                        })),
                                ),
                        )
                    }),
            )
            .children(
                detail["podium"]
                    .as_array()
                    .filter(|podium| !podium.is_empty())
                    .map(|podium| {
                        h_flex()
                            .gap(px(10.))
                            .children(podium.iter().enumerate().map(|(index, entry)| {
                                let mut line =
                                    format!("{} pts", entry["score"].as_f64().unwrap_or_default());
                                if let Some(performance) = entry["performance"].as_f64() {
                                    line.push_str(&format!(" · perf {performance}"));
                                }
                                v_flex()
                                    .flex_1()
                                    .items_center()
                                    .gap(px(2.))
                                    .p(px(10.))
                                    .rounded(px(10.))
                                    .bg(palette.accented)
                                    .child(icon("trophy").size(px(20.)).text_color(rgb(
                                        match index {
                                            0 => 0xd4a017,
                                            1 => 0x9e9e9e,
                                            _ => 0xb0703c,
                                        },
                                    )))
                                    .child(
                                        player_link(
                                            ("podium", index),
                                            entry["name"].as_str().unwrap_or("?").to_string(),
                                        )
                                        .font_weight(FontWeight::SEMIBOLD),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette.text_muted)
                                            .child(line),
                                    )
                            }))
                    }),
            )
            .when(rows.is_empty(), |card| {
                card.child(widgets::hint("No players yet.", palette))
            })
            .children(rows.iter().enumerate().map(|(index, row)| {
                let name = row["name"].as_str().unwrap_or("?").to_string();
                let mine = name.to_lowercase() == me;
                h_flex()
                    .gap(px(10.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .when(mine, |row| row.bg(crate::theme::tint(palette.primary, 14.)))
                    .child(
                        div()
                            .w(px(32.))
                            .text_color(palette.text_muted)
                            .child(format!("{}", row["rank"].as_f64().unwrap_or_default())),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.))
                            .children(row["title"].as_str().map(|title| {
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(palette.primary)
                                    .child(title.to_string())
                            }))
                            .child(player_link(("standing", index), name))
                            .children(row["rating"].as_f64().map(|rating| {
                                div()
                                    .text_color(palette.text_muted)
                                    .child(rating.to_string())
                            })),
                    )
                    .children(row["sheet"].as_str().map(|sheet| {
                        div()
                            .max_w(px(160.))
                            .truncate()
                            .text_color(palette.text_muted)
                            .child(sheet.to_string())
                    }))
                    .child(
                        h_flex()
                            .w(px(48.))
                            .justify_end()
                            .gap(px(2.))
                            .font_weight(FontWeight::BOLD)
                            .when(row["fire"] == json!(true), |score| {
                                score
                                    .text_color(palette.warning)
                                    .child(icon("flame").size(px(14.)))
                            })
                            .child(
                                row["score"]
                                    .as_f64()
                                    .map_or(String::new(), |score| score.to_string()),
                            ),
                    )
            }))
    }

    /// The top game, watched with a click.
    fn featured(
        &mut self,
        detail: &Value,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Option<Div> {
        let featured = detail["featured"].as_object()?.clone();
        let featured = Value::Object(featured);
        let orientation = if featured["orientation"] == json!("black") {
            Color::Black
        } else {
            Color::White
        };
        let mut state = BoardState::shown(
            featured["fen"].as_str().unwrap_or_default(),
            "standard",
            orientation,
            featured["lastMove"]
                .as_str()
                .filter(|uci| !uci.contains('@')),
        );
        state.preview = true;
        let board = match &self.featured_board {
            Some(board) => {
                board.update(cx, |board, cx| board.set_state(state, cx));
                board.clone()
            }
            None => {
                let board = cx.new(|_| ChessBoard::new(state));
                self.featured_board = Some(board.clone());
                board
            }
        };
        let side = |color: Color, index: usize| {
            let key = if color == Color::White {
                "white"
            } else {
                "black"
            };
            let entry = &featured[key];
            let clock = featured["clocks"][key].as_f64().and_then(|seconds| {
                rules("formatClock", &[json!(seconds * 1000.)])
                    .ok()
                    .and_then(|text| text.as_str().map(str::to_string))
            });
            h_flex()
                .justify_between()
                .text_size(px(13.))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .children(entry["rank"].as_f64().map(|rank| {
                            div()
                                .text_color(palette.text_muted)
                                .child(format!("#{rank}"))
                        }))
                        .child(player_link(
                            ("featured-player", index),
                            entry["name"].as_str().unwrap_or("?").to_string(),
                        ))
                        .children(entry["rating"].as_f64().map(|rating| {
                            div()
                                .text_color(palette.text_muted)
                                .child(rating.to_string())
                        })),
                )
                .children(clock)
        };
        let id = featured["id"].as_str().unwrap_or_default().to_string();
        Some(
            widgets::card(palette)
                .gap(px(8.))
                .child(widgets::heading("Top game", palette))
                .child(side(!orientation, 0))
                .child(
                    div()
                        .id("featured-board")
                        .w_full()
                        .h(px(298.))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| crate::shell::watch_game(&id, window, cx))
                        .child(board),
                )
                .child(side(orientation, 1)),
        )
    }
}

/// The games being played now, each watched with a click.
fn duels(detail: &Value, palette: &Palette) -> Option<Div> {
    let duels = detail["duels"]
        .as_array()
        .filter(|duels| !duels.is_empty())?
        .clone();
    Some(
        widgets::card(palette)
            .gap(px(6.))
            .child(widgets::heading("Playing now", palette))
            .children(duels.into_iter().enumerate().map(|(index, duel)| {
                let id = duel["id"].as_str().unwrap_or_default().to_string();
                let side = |key: &str, slot: usize| {
                    h_flex()
                        .gap(px(6.))
                        .min_w_0()
                        .children(duel[key]["rank"].as_f64().map(|rank| {
                            div()
                                .text_color(palette.text_muted)
                                .child(format!("#{rank}"))
                        }))
                        .child(player_link(
                            ("duel-player", index * 2 + slot),
                            duel[key]["name"].as_str().unwrap_or("?").to_string(),
                        ))
                };
                h_flex()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(side("white", 0))
                            .child(side("black", 1)),
                    )
                    .child(
                        Button::new(("duel-watch", index))
                            .xsmall()
                            .ghost()
                            .icon(icon("tv"))
                            .accessibility_label("Watch")
                            .on_click(move |_, window, cx| {
                                crate::shell::watch_game(&id, window, cx)
                            }),
                    )
            })),
    )
}

fn totals(detail: &Value, palette: &Palette) -> Option<Div> {
    let stats = detail["stats"].as_object()?;
    let number = |key: &str| stats.get(key).and_then(Value::as_f64).unwrap_or_default();
    let rows = [
        ("Games", number("games").to_string()),
        (
            "White · draws · Black",
            format!(
                "{} · {} · {}",
                number("whiteWins"),
                number("draws"),
                number("blackWins")
            ),
        ),
        ("Berserks", number("berserks").to_string()),
        ("Average rating", number("averageRating").to_string()),
    ];
    Some(
        widgets::card(palette)
            .gap(px(4.))
            .child(widgets::heading("Totals", palette))
            .children(rows.into_iter().map(|(label, value)| {
                h_flex()
                    .justify_between()
                    .py(px(5.))
                    .text_size(px(13.))
                    .child(div().text_color(palette.text_muted).child(label))
                    .child(value)
            })),
    )
}

#[cfg(test)]
mod tests {
    use super::{facts, status_line};
    use serde_json::json;

    #[test]
    fn the_status_says_what_happens_next() {
        assert_eq!(status_line(&json!({ "status": "finished" })), "Finished");
        assert_eq!(
            status_line(&json!({ "status": "created", "secondsToStart": 290 })),
            "Starts in 5 min"
        );
        assert_eq!(
            status_line(&json!({ "status": "started", "secondsToFinish": 61 })),
            "2 min left"
        );
        assert_eq!(status_line(&json!({ "status": "started" })), "Under way");
    }

    #[test]
    fn facts_name_the_clock_variant_and_length() {
        let detail = json!({
            "clock": { "limit": 180, "increment": 2 }, "variant": "atomic",
            "variantName": "Atomic", "rated": true, "minutes": 60, "berserkable": true,
        });
        assert_eq!(
            facts(&detail),
            ["3+2", "Atomic", "Rated", "60 min", "Berserk allowed"]
        );
    }
}
