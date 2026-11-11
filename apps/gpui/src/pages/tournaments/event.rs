//! One tournament: joining, its details, the standings, the top game,
//! the games in progress and the totals.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{
    TournamentDetail, TournamentFeatured, TournamentPlayer, TournamentStatus, TournamentSystem,
};
use kchess_domain::training::clock::format_clock;
use serde_json::Number;
use shakmaty::Color;

use super::{TournamentsPage, control};
use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::pages::players::player_link;
use crate::theme::{Palette, radii};
use crate::widgets::Pressable as _;
use crate::widgets::{self, KeyboardFocus as _, Look};

/// A number Lichess may leave out.
fn number(value: Option<&Number>) -> Option<f64> {
    value.and_then(Number::as_f64)
}

/// "Starts in 5 min", "12 min left", "Next round in 2 min", "Finished".
pub fn status_line(detail: &TournamentDetail) -> String {
    let minutes = |seconds: &Option<Number>| {
        number(seconds.as_ref())
            .filter(|seconds| *seconds > 0.)
            .map(|seconds| (seconds / 60.).ceil())
    };
    if detail.summary.status == TournamentStatus::Finished {
        "Finished".into()
    } else if let Some(m) = minutes(&detail.seconds_to_start) {
        format!("Starts in {m} min")
    } else if let Some(m) = minutes(&detail.seconds_to_finish) {
        format!("{m} min left")
    } else if let Some(m) = minutes(&detail.next_round_in) {
        format!("Next round in {m} min")
    } else {
        "Under way".into()
    }
}

/// "3+2 · Rated · 60 min · Berserk allowed".
pub fn facts(detail: &TournamentDetail) -> Vec<String> {
    let summary = &detail.summary;
    let mut facts = vec![control(&summary.clock)];
    if summary.variant != "standard" {
        facts.push(summary.variant_name.clone());
    }
    facts.push(if summary.rated { "Rated" } else { "Casual" }.into());
    if let Some(minutes) = number(summary.minutes.as_ref()) {
        facts.push(format!("{minutes} min"));
    } else if let Some(rounds) = number(summary.nb_rounds.as_ref()) {
        facts.push(format!("{rounds} rounds"));
    }
    if detail.berserkable == Some(true) {
        facts.push("Berserk allowed".into());
    }
    facts
}

impl TournamentsPage {
    pub(super) fn joined_now(&self, detail: &TournamentDetail, cx: &App) -> bool {
        if let Some(me) = &detail.me {
            return !me.withdraw;
        }
        self.is_joined(
            detail.summary.system.as_str(),
            &detail.summary.id,
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
                    .child(widgets::loading("Loading…", palette))
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

    fn event_head(
        &self,
        detail: &TournamentDetail,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Div {
        let joined = self.joined_now(detail, cx);
        let account = Self::account(cx);
        let summary = &detail.summary;
        let finished = summary.status == TournamentStatus::Finished;
        let started = summary.status == TournamentStatus::Started;
        let arena = summary.system == TournamentSystem::Arena;
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
                            .disabled(!self.may_join(cx))
                            .on_click(cx.listener(|this, _, window, cx| this.join(window, cx))),
                    ),
            )
        } else {
            let me = detail.me.as_ref();
            let game = me.and_then(|me| me.game_id.clone());
            Some(
                h_flex()
                    .gap(px(8.))
                    .child(widgets::badge("You are in", Look::Success, palette))
                    .children(
                        number(me.and_then(|me| me.rank.as_ref()))
                            .map(|rank| div().text_size(px(13.)).child(format!("Rank {rank}"))),
                    )
                    .children(game.map(|id| {
                        Button::new("tournament-game")
                            .primary()
                            .icon(icon("swords"))
                            .label("Go to your game")
                            .on_click(move |_, window, cx| {
                                crate::pages::online::open_game(&id, window, cx)
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
            summary.nb_players.as_f64().unwrap_or_default()
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
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(summary.name.clone()),
                            )
                            .child(
                                h_flex()
                                    .text_size(px(13.))
                                    .text_color(palette.text_muted)
                                    .child(line)
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .when(started, |text| text.text_color(palette.primary))
                                            .child(status_line(detail)),
                                    ),
                            ),
                    )
                    .children(actions),
            )
            .children(summary.problem.clone().map(|problem| {
                h_flex()
                    .gap(px(8.))
                    .p(px(12.))
                    .rounded(radii::MD)
                    .bg(palette.accented)
                    .text_size(px(13.))
                    .child(icon("eye").size(px(16.)))
                    .child(problem)
            }))
            .children(
                detail
                    .description
                    .clone()
                    .map(|text| div().text_size(px(13.)).child(text)),
            )
            .children(
                detail
                    .verdicts
                    .as_ref()
                    .map(|verdicts| &verdicts.list)
                    .filter(|list| !list.is_empty())
                    .map(|list| {
                        v_flex()
                            .gap(px(4.))
                            .text_size(px(13.))
                            .children(list.iter().map(|entry| {
                                let ok = entry.verdict == "ok";
                                h_flex()
                                    .gap(px(8.))
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
                                    .child(entry.condition.clone())
                                    .when(!ok, |row| {
                                        row.child(
                                            div()
                                                .text_color(palette.text_muted)
                                                .child(format!("· {}", entry.verdict)),
                                        )
                                    })
                            }))
                    }),
            )
    }

    fn standings(
        &self,
        detail: &TournamentDetail,
        page: usize,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Div {
        let pages =
            ((detail.summary.nb_players.as_f64().unwrap_or_default() / 10.).ceil() as usize).max(1);
        let me = Self::account(cx).unwrap_or_default().to_lowercase();
        let rows = &detail.standing;
        widgets::card(palette)
            .gap(px(8.))
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
                detail
                    .extras
                    .podium
                    .as_ref()
                    .filter(|podium| !podium.is_empty())
                    .map(|podium| {
                        h_flex().gap(px(8.)).children(podium.iter().enumerate().map(
                            |(index, entry)| {
                                let mut line = format!(
                                    "{} pts",
                                    number(entry.score.as_ref()).unwrap_or_default()
                                );
                                if let Some(performance) = number(entry.performance.as_ref()) {
                                    line.push_str(&format!(" · perf {performance}"));
                                }
                                v_flex()
                                    .flex_1()
                                    .items_center()
                                    .gap(px(2.))
                                    .p(px(12.))
                                    .rounded(radii::LG)
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
                                            entry.name.clone(),
                                            palette.primary,
                                        )
                                        .font_weight(FontWeight::MEDIUM),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette.text_muted)
                                            .child(line),
                                    )
                            },
                        ))
                    }),
            )
            .when(rows.is_empty(), |card| {
                card.child(widgets::hint("No players yet", palette))
            })
            .children(rows.iter().enumerate().map(|(index, row)| {
                let name = row.name.clone();
                let mine = name.to_lowercase() == me;
                h_flex()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(radii::SM)
                    .text_size(px(13.))
                    .when(mine, |row| row.bg(crate::theme::tint(palette.primary, 14.)))
                    .child(
                        div()
                            .w(px(32.))
                            .text_color(palette.text_muted)
                            .child(format!("{}", row.rank.as_f64().unwrap_or_default())),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(8.))
                            .children(row.title.clone().map(|title| {
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.primary)
                                    .child(title)
                            }))
                            .child(player_link(("standing", index), name, palette.primary))
                            .children(number(row.rating.as_ref()).map(|rating| {
                                div()
                                    .text_color(palette.text_muted)
                                    .child(rating.to_string())
                            })),
                    )
                    .children(row.sheet.clone().map(|sheet| {
                        div()
                            .max_w(px(160.))
                            .truncate()
                            .text_color(palette.text_muted)
                            .child(sheet)
                    }))
                    .child(
                        h_flex()
                            .w(px(48.))
                            .justify_end()
                            .gap(px(2.))
                            .font_weight(FontWeight::MEDIUM)
                            .when(row.fire == Some(true), |score| {
                                score
                                    .text_color(palette.warning)
                                    .child(icon("flame").size(px(14.)))
                            })
                            .child(
                                number(row.score.as_ref())
                                    .map_or(String::new(), |score| score.to_string()),
                            ),
                    )
            }))
    }

    /// The top game, watched with a click.
    fn featured(
        &mut self,
        detail: &TournamentDetail,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Option<Div> {
        let featured = detail.extras.featured.clone()?;
        let orientation = featured.orientation;
        let mut state = BoardState::shown(
            &featured.fen,
            kchess_domain::variant::Variant::Standard,
            orientation,
            featured
                .last_move
                .as_deref()
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
        let elapsed = if detail.summary.status == TournamentStatus::Finished {
            0.
        } else {
            self.open
                .as_ref()
                .map_or(0., |open| open.received.elapsed().as_secs_f64())
        };
        let side = |color: Color, index: usize| {
            let entry = player(&featured, color);
            let clock = featured_clock(&featured, color, elapsed).map(|(text, running)| {
                div()
                    .font_features(widgets::tabular())
                    .when(running, |clock| clock.font_weight(FontWeight::MEDIUM))
                    .when(!running, |clock| clock.text_color(palette.text_muted))
                    .child(text)
            });
            h_flex()
                .justify_between()
                .text_size(px(13.))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .children(number(entry.rank.as_ref()).map(|rank| {
                            div()
                                .text_color(palette.text_muted)
                                .child(format!("#{rank}"))
                        }))
                        .child(player_link(
                            ("featured-player", index),
                            entry.name.clone(),
                            palette.primary,
                        ))
                        .children(number(entry.rating.as_ref()).map(|rating| {
                            div()
                                .text_color(palette.text_muted)
                                .child(rating.to_string())
                        })),
                )
                .children(clock)
        };
        let id = featured.id.clone();
        Some(
            widgets::card(palette)
                .gap(px(8.))
                .child(widgets::heading("Top game", palette))
                .child(side(!orientation, 0))
                .child(
                    div()
                        .id("featured-board")
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label("Watch the top game")
                        .w_full()
                        .h(px(298.))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            crate::pages::watch::watch_game(&id, window, cx)
                        })
                        .child(board),
                )
                .child(side(orientation, 1)),
        )
    }
}

fn player(featured: &TournamentFeatured, color: Color) -> &TournamentPlayer {
    match color {
        Color::White => &featured.white,
        Color::Black => &featured.black,
    }
}

/// The side whose clock runs in the top game: the side to move, once the game has a move.
pub(super) fn featured_running(featured: &TournamentFeatured) -> Option<Color> {
    featured.last_move.as_ref()?;
    featured.clocks.as_ref()?;
    Some(if featured.fen.split(' ').nth(1) == Some("b") {
        Color::Black
    } else {
        Color::White
    })
}

/// One side's clock in the top game, `elapsed` seconds after Lichess reported it, and whether it
/// is the one running.
fn featured_clock(
    featured: &TournamentFeatured,
    color: Color,
    elapsed: f64,
) -> Option<(String, bool)> {
    let clocks = featured.clocks.as_ref()?;
    let reported = match color {
        Color::White => &clocks.white,
        Color::Black => &clocks.black,
    }
    .as_f64()?;
    let running = featured_running(featured) == Some(color);
    let left = if running {
        reported - elapsed
    } else {
        reported
    };
    Some((format_clock(left.max(0.) * 1000.), running))
}

/// The games being played now: a row each, watched with a click anywhere on it.
fn duels(detail: &TournamentDetail, palette: &Palette) -> Option<Div> {
    let duels = detail
        .extras
        .duels
        .clone()
        .filter(|duels| !duels.is_empty())?;
    Some(
        widgets::card(palette)
            .gap(px(4.))
            .child(widgets::heading("Playing now", palette))
            .children(duels.into_iter().enumerate().map(|(index, duel)| {
                let id = duel.id.clone();
                let label = format!("Watch {} against {}", duel.white.name, duel.black.name);
                let side = |player: &TournamentPlayer, slot: usize| {
                    h_flex()
                        .gap(px(8.))
                        .min_w_0()
                        // Ranks in a column of their own, so the names line up.
                        .child(
                            div()
                                .w(px(40.))
                                .flex_none()
                                .font_features(widgets::tabular())
                                .text_color(palette.text_muted)
                                .children(
                                    number(player.rank.as_ref()).map(|rank| format!("#{rank}")),
                                ),
                        )
                        .child(player_link(
                            ("duel-player", index * 2 + slot),
                            player.name.clone(),
                            palette.primary,
                        ))
                };
                h_flex()
                    .id(("duel-watch", index))
                    .role(Role::Button)
                    .keyboard_focus(palette.primary)
                    .aria_label(label)
                    .gap(px(8.))
                    .mx(px(-8.))
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(radii::MD)
                    .text_size(px(13.))
                    .cursor_pointer()
                    .pressable(palette)
                    .on_click(move |_, window, cx| crate::pages::watch::watch_game(&id, window, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(side(&duel.white, 0))
                            .child(side(&duel.black, 1)),
                    )
                    .child(
                        icon("tv")
                            .size(px(16.))
                            .flex_none()
                            .text_color(palette.text_muted),
                    )
            })),
    )
}

fn totals(detail: &TournamentDetail, palette: &Palette) -> Option<Div> {
    let stats = detail.extras.stats.as_ref()?;
    let number = |value: &Number| value.as_f64().unwrap_or_default();
    let rows = [
        ("Games", number(&stats.games).to_string()),
        (
            "White · draws · Black",
            format!(
                "{} · {} · {}",
                number(&stats.white_wins),
                number(&stats.draws),
                number(&stats.black_wins)
            ),
        ),
        ("Berserks", number(&stats.berserks).to_string()),
        ("Average rating", number(&stats.average_rating).to_string()),
    ];
    Some(
        widgets::card(palette)
            .gap(px(4.))
            .child(widgets::heading("Totals", palette))
            .children(rows.into_iter().map(|(label, value)| {
                h_flex()
                    .justify_between()
                    .py(px(4.))
                    .text_size(px(13.))
                    .child(div().text_color(palette.text_muted).child(label))
                    .child(value)
            })),
    )
}

#[cfg(test)]
mod tests {
    use super::{facts, featured_clock, featured_running, status_line};
    use kchess_contracts::types::{TournamentDetail, TournamentFeatured};
    use serde_json::{Value, json};
    use shakmaty::Color;

    /// `fields` over a waiting arena with no players.
    fn detail(fields: Value) -> TournamentDetail {
        let mut detail = json!({
            "id": "a", "system": "arena", "name": "Hourly", "status": "created",
            "variant": "standard", "variantName": "Standard", "rated": false,
            "clock": { "limit": 180, "increment": 0 }, "nbPlayers": 0, "startsAt": 0,
            "playable": true, "standing": [],
        });
        for (key, value) in fields.as_object().expect("fields") {
            detail[key] = value.clone();
        }
        serde_json::from_value(detail).expect("a tournament")
    }

    fn featured(fields: Value) -> TournamentFeatured {
        let mut featured = json!({
            "id": "g", "fen": "", "orientation": "white",
            "white": { "name": "A" }, "black": { "name": "B" },
        });
        for (key, value) in fields.as_object().expect("fields") {
            featured[key] = value.clone();
        }
        serde_json::from_value(featured).expect("a featured game")
    }

    #[test]
    fn the_top_games_side_to_move_loses_time_between_refreshes() {
        let featured = featured(json!({
            "fen": "8/8/8/8/8/8/8/8 b - - 0 1", "lastMove": "e2e4",
            "clocks": { "white": 255, "black": 239 },
        }));
        assert_eq!(
            featured_clock(&featured, Color::Black, 30.),
            Some(("3:29".to_string(), true))
        );
        assert_eq!(
            featured_clock(&featured, Color::White, 30.),
            Some(("4:15".to_string(), false))
        );
        // It stops at zero.
        assert_eq!(
            featured_clock(&featured, Color::Black, 400.),
            Some(("0:00.0".to_string(), true))
        );
    }

    #[test]
    fn the_top_games_clocks_wait_for_the_first_move() {
        let featured = featured(json!({
            "fen": "8/8/8/8/8/8/8/8 w - - 0 1", "clocks": { "white": 180, "black": 180 },
        }));
        assert_eq!(featured_running(&featured), None);
        assert_eq!(
            featured_clock(&featured, Color::White, 30.),
            Some(("3:00".to_string(), false))
        );
    }

    #[test]
    fn the_status_says_what_happens_next() {
        assert_eq!(
            status_line(&detail(json!({ "status": "finished" }))),
            "Finished"
        );
        assert_eq!(
            status_line(&detail(
                json!({ "status": "created", "secondsToStart": 290 })
            )),
            "Starts in 5 min"
        );
        assert_eq!(
            status_line(&detail(
                json!({ "status": "started", "secondsToFinish": 61 })
            )),
            "2 min left"
        );
        assert_eq!(
            status_line(&detail(json!({ "status": "started" }))),
            "Under way"
        );
    }

    #[test]
    fn facts_name_the_clock_variant_and_length() {
        let detail = detail(json!({
            "clock": { "limit": 180, "increment": 2 }, "variant": "atomic",
            "variantName": "Atomic", "rated": true, "minutes": 60, "berserkable": true,
        }));
        assert_eq!(
            facts(&detail),
            ["3+2", "Atomic", "Rated", "60 min", "Berserk allowed"]
        );
    }
}
