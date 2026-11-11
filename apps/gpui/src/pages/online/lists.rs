//! Beside the form: the challenges to and from the connected accounts and
//! the games in progress.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{ChallengeInfo, ChallengeTimeControl, DeclineReason};
use serde_json::Number;

use super::OnlinePage;
use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::online::OnlineHandle;
use crate::pages::Page;
use crate::pages::players::player_link;
use crate::theme::Palette;
use crate::widgets;

/// The reasons a challenge can be declined with (`DECLINE`).
const DECLINE: [(DeclineReason, &str); 9] = [
    (DeclineReason::Generic, "Decline"),
    (DeclineReason::Later, "Not now, maybe later"),
    (DeclineReason::TooFast, "Too fast for me"),
    (DeclineReason::TooSlow, "Too slow for me"),
    (DeclineReason::TimeControl, "Not this time control"),
    (DeclineReason::Rated, "Send a rated challenge instead"),
    (DeclineReason::Casual, "Send a casual challenge instead"),
    (DeclineReason::Standard, "Standard chess only, please"),
    (DeclineReason::Variant, "Not this variant"),
];

/// "5+3", "3 days per move", "No clock".
pub fn describe_control(control: &ChallengeTimeControl) -> String {
    match control {
        ChallengeTimeControl::Clock { limit, increment } => crate::widgets::time_control(
            limit.as_f64().unwrap_or_default() / 60.,
            increment.as_f64().unwrap_or_default(),
        ),
        ChallengeTimeControl::Correspondence { days } => {
            let days = days.as_f64().unwrap_or_default();
            format!(
                "{days} {} per move",
                if days == 1. { "day" } else { "days" }
            )
        }
        ChallengeTimeControl::Unlimited => "No clock".into(),
    }
}

/// A player's rating as it reads: "1800", not "1800.0".
fn rating(rating: &Option<Number>) -> Option<f64> {
    rating.as_ref().and_then(Number::as_f64)
}

/// "you play white", "random colours": the colour is the challenger's.
pub fn color_text(challenge: &ChallengeInfo) -> String {
    let color = challenge.color.as_str();
    if color == "random" {
        return "random colours".into();
    }
    let yours = if challenge.direction == "out" {
        color
    } else if color == "white" {
        "black"
    } else {
        "white"
    };
    format!("you play {yours}")
}

/// "2d 3h left", "4h 10m left", "12m left".
pub fn time_left(seconds: Option<f64>) -> Option<String> {
    let seconds = seconds?;
    Some(if seconds >= 86_400. {
        format!(
            "{}d {}h left",
            (seconds / 86_400.).floor(),
            ((seconds % 86_400.) / 3600.).floor()
        )
    } else if seconds >= 3600. {
        format!(
            "{}h {}m left",
            (seconds / 3600.).floor(),
            ((seconds % 3600.) / 60.).floor()
        )
    } else {
        format!("{}m left", (seconds / 60.).floor().max(0.))
    })
}

impl OnlinePage {
    pub(super) fn lists(&mut self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        v_flex()
            .w(px(380.))
            .flex_none()
            .gap(px(20.))
            .child(self.challenge_list(cx, palette))
            .child(self.ongoing_list(cx, palette))
            .child(
                Button::new("online-tournaments")
                    .outline()
                    .w_full()
                    .icon(icon("trophy"))
                    .label("Tournaments")
                    .on_click(|_, window, cx| crate::shell::open(Page::Tournaments, window, cx)),
            )
    }

    fn challenge_list(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let online = cx.global::<OnlineHandle>().0.read(cx);
        let receive = cx
            .global::<crate::settings::SettingsHandle>()
            .0
            .read(cx)
            .settings
            .core
            .receive_challenges;
        let several = crate::accounts::handle(cx).read(cx).connected().len() > 1;
        let (status, tone) = match &online.lobby {
            _ if !receive => (
                "Not listening for challenges (Settings → Online play)".to_string(),
                palette.text_muted,
            ),
            None => (
                "Listening for challenges while a game is open".to_string(),
                palette.text_muted,
            ),
            Some(lobby) => match lobby["phase"].as_str() {
                Some("connected") => (
                    format!(
                        "Listening for challenges to @{}",
                        lobby["account"].as_str().unwrap_or_default()
                    ),
                    palette.success,
                ),
                Some("auth-required") => (
                    lobby["message"]
                        .as_str()
                        .unwrap_or("Sign in to Lichess again in Settings")
                        .to_string(),
                    palette.error,
                ),
                Some("disconnected") => (
                    lobby["message"]
                        .as_str()
                        .unwrap_or("Not connected to Lichess")
                        .to_string(),
                    palette.error,
                ),
                _ => ("Connecting to Lichess…".to_string(), palette.text_muted),
            },
        };
        let busy = online.busy_challenge.clone();
        let challenges = online.challenges.clone();
        let row = |glyph: &'static str| {
            h_flex()
                .gap(px(8.))
                .py(px(8.))
                .border_t_1()
                .border_color(palette.border)
                .text_size(px(13.))
                .child(icon(glyph).size(px(16.)).flex_none())
        };
        widgets::card(palette)
            .gap(px(8.))
            .child(
                v_flex()
                    .child(widgets::heading("Challenges", palette))
                    .child(div().text_size(px(12.)).text_color(tone).child(status)),
            )
            .when(challenges.is_empty(), |card| {
                card.child(widgets::hint("No open challenges", palette))
            })
            .children(
                challenges
                    .iter()
                    .filter(|c| c.direction == "in")
                    .enumerate()
                    .map(|(index, challenge)| {
                        let opponent = &challenge.opponent;
                        let mut sub = String::new();
                        if challenge.rematch_of.is_some() {
                            sub.push_str("Rematch · ");
                        }
                        sub.push_str(&format!(
                            "{} · {} · {} · {}",
                            challenge.variant_name,
                            describe_control(&challenge.time_control),
                            if challenge.rated { "Rated" } else { "Casual" },
                            color_text(challenge)
                        ));
                        if several {
                            sub.push_str(&format!(" · to @{}", challenge.account));
                        }
                        let id = challenge.id.clone();
                        let (accept, decline) = (challenge.clone(), challenge.clone());
                        row(if challenge.rematch_of.is_some() {
                            "repeat"
                        } else {
                            "swords"
                        })
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    h_flex()
                                        .gap(px(4.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .children(opponent.title.clone())
                                        .child(player_link(
                                            ("challenge-from", index),
                                            opponent.name.clone(),
                                            palette.primary,
                                        ))
                                        .children(rating(&opponent.rating).map(|rating| {
                                            div().text_color(palette.text_muted).child(format!(
                                                "({rating}{})",
                                                if opponent.provisional == Some(true) {
                                                    "?"
                                                } else {
                                                    ""
                                                }
                                            ))
                                        })),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(palette.text_muted)
                                        .child(sub),
                                )
                                .children(challenge.problem.clone().map(|problem| {
                                    div()
                                        .text_size(px(12.))
                                        .text_color(palette.warning)
                                        .child(problem)
                                })),
                        )
                        .child(
                            Button::new(("challenge-accept", index))
                                .xsmall()
                                .primary()
                                .icon(icon("check"))
                                .label("Accept")
                                .loading(busy == id)
                                .disabled(!challenge.playable)
                                .on_click(move |_, window, cx| {
                                    let challenge = accept.clone();
                                    let opened = crate::online::handle(cx)
                                        .update(cx, |online, cx| online.accept(&challenge, cx));
                                    window
                                        .spawn(cx, async move |cx| {
                                            if opened.await {
                                                let _ = cx.update(|window, cx| {
                                                    crate::shell::open(Page::Online, window, cx)
                                                });
                                            }
                                        })
                                        .detach();
                                }),
                        )
                        .child(
                            Button::new(("challenge-decline", index))
                                .xsmall()
                                .outline()
                                .icon(icon("x"))
                                .label("Decline")
                                .disabled(busy == id)
                                .dropdown_menu(move |menu, _, _| {
                                    DECLINE.iter().fold(menu, |menu, (reason, label)| {
                                        let challenge = decline.clone();
                                        menu.item(PopupMenuItem::new(*label).on_click(
                                            move |_, _, cx| {
                                                let challenge = challenge.clone();
                                                crate::online::handle(cx).update(
                                                    cx,
                                                    |online, cx| {
                                                        online.decline(&challenge, *reason, cx)
                                                    },
                                                )
                                            },
                                        ))
                                    })
                                }),
                        )
                    }),
            )
            .children(
                challenges
                    .iter()
                    .filter(|c| c.direction == "out")
                    .enumerate()
                    .map(|(index, challenge)| {
                        let id = challenge.id.clone();
                        let withdraw = challenge.clone();
                        row("send")
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        h_flex()
                                            .gap(px(4.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("To")
                                            .child(player_link(
                                                ("challenge-to", index),
                                                challenge.opponent.name.clone(),
                                                palette.primary,
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(palette.text_muted)
                                            .child(format!(
                                                "{} · {} · {} · {} · waiting",
                                                challenge.variant_name,
                                                describe_control(&challenge.time_control),
                                                if challenge.rated { "Rated" } else { "Casual" },
                                                color_text(challenge)
                                            )),
                                    ),
                            )
                            .child(
                                Button::new(("challenge-withdraw", index))
                                    .xsmall()
                                    .outline()
                                    .icon(icon("x"))
                                    .label("Withdraw")
                                    .loading(busy == id)
                                    .on_click(move |_, _, cx| {
                                        let challenge = withdraw.clone();
                                        crate::online::handle(cx).update(cx, |online, cx| {
                                            online.withdraw(&challenge, cx)
                                        })
                                    }),
                            )
                    }),
            )
    }

    fn ongoing_list(&mut self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let online = cx.global::<OnlineHandle>().0.read(cx);
        let games = online.ongoing.clone();
        let loaded = online.ongoing_loaded;
        let error = online.ongoing_error.clone();
        let current =
            (online.phase == crate::online::Phase::Playing).then(|| online.id().to_string());
        let several = crate::accounts::handle(cx).read(cx).connected().len() > 1;
        // Boards of games no longer ongoing are let go.
        self.previews
            .retain(|id, _| games.iter().any(|game| game.game_id == *id));
        let rows: Vec<AnyElement> = games
            .iter()
            .enumerate()
            .map(|(index, game)| {
                let id = game.game_id.clone();
                let orientation = if game.color == "black" {
                    shakmaty::Color::Black
                } else {
                    shakmaty::Color::White
                };
                let mut state = BoardState::shown(
                    &game.fen,
                    kchess_domain::variant::Variant::Standard,
                    orientation,
                    game.last_move.as_deref().filter(|uci| uci.len() >= 4),
                );
                state.preview = true;
                let board = match self.previews.get(&id) {
                    Some(board) => {
                        board.update(cx, |board, cx| board.set_state(state, cx));
                        board.clone()
                    }
                    None => {
                        let board = cx.new(|_| ChessBoard::new(state));
                        self.previews.insert(id.clone(), board.clone());
                        board
                    }
                };
                let mine = game.is_my_turn;
                let speed = game.speed.as_str();
                let mut sub = format!(
                    "{} · {} · you play {}",
                    if speed == "correspondence" {
                        "Correspondence"
                    } else {
                        speed
                    },
                    if game.rated { "Rated" } else { "Casual" },
                    game.color
                );
                if several {
                    sub.push_str(&format!(" · @{}", game.account));
                }
                // Lichess lets apps play only some games; the others are watched here.
                let playable = game.playable;
                let mut turn = if !playable {
                    "Played elsewhere".to_string()
                } else if mine {
                    "Your move".to_string()
                } else {
                    "Waiting for opponent".to_string()
                };
                if let Some(left) = time_left(game.seconds_left.as_ref().and_then(Number::as_f64)) {
                    turn.push_str(&format!(" · {left}"));
                }
                let open_now = current.as_deref() == Some(id.as_str());
                let account = game.account.clone();
                h_flex()
                    .gap(px(8.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(palette.border)
                    .text_size(px(13.))
                    .child(div().size(px(56.)).flex_none().child(board))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap(px(4.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(player_link(
                                        ("ongoing-opponent", index),
                                        game.opponent.name.clone(),
                                        palette.primary,
                                    ))
                                    .children(rating(&game.opponent.rating).map(|rating| {
                                        div()
                                            .text_color(palette.text_muted)
                                            .child(format!("({rating})"))
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette.text_muted)
                                    .child(sub),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .when(mine, |text| {
                                        text.text_color(palette.primary)
                                            .font_weight(FontWeight::MEDIUM)
                                    })
                                    .when(!mine, |text| text.text_color(palette.text_muted))
                                    .child(turn),
                            ),
                    )
                    .child(
                        Button::new(("ongoing-play", index))
                            .xsmall()
                            .when(mine && playable, |button| button.primary())
                            .when(!mine || !playable, |button| button.outline())
                            .label(if !playable {
                                "Watch"
                            } else if open_now {
                                "Open"
                            } else {
                                "Play"
                            })
                            .on_click(move |_, window, cx| {
                                if !playable {
                                    crate::pages::watch::watch_game(&id, window, cx);
                                    return;
                                }
                                if !open_now {
                                    let (account, id) = (account.clone(), id.clone());
                                    crate::online::handle(cx)
                                        .update(cx, |online, cx| online.open(&account, &id, cx));
                                }
                                crate::shell::open(Page::Online, window, cx);
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        widgets::card(palette)
            .gap(px(8.))
            .child(
                h_flex()
                    .justify_between()
                    .child(widgets::heading("Your games", palette))
                    .child(
                        Button::new("ongoing-refresh")
                            .xsmall()
                            .ghost()
                            .icon(icon("refresh-cw"))
                            .accessibility_label("Refresh games")
                            .on_click(|_, _, cx| {
                                crate::online::handle(cx)
                                    .update(cx, |online, cx| online.refresh_ongoing(cx))
                            }),
                    ),
            )
            .children(error.map(|error| {
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .when(loaded && games.is_empty(), |card| {
                card.child(widgets::hint("No games in progress", palette))
            })
            .children(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::{color_text, describe_control, time_left};
    use kchess_contracts::types::{ChallengeInfo, ChallengeTimeControl};
    use serde_json::{Number, json};

    fn challenge(color: &str, direction: &str) -> ChallengeInfo {
        serde_json::from_value(json!({
            "id": "c1", "account": "me", "direction": direction, "opponent": { "name": "Bob" },
            "variant": "standard", "variantName": "Standard", "rated": false, "speed": "blitz",
            "timeControl": { "type": "unlimited" }, "color": color, "playable": true,
            "receivedAt": 0,
        }))
        .expect("a challenge")
    }

    fn clock(limit: u64, increment: u64) -> ChallengeTimeControl {
        ChallengeTimeControl::Clock {
            limit: Number::from(limit),
            increment: Number::from(increment),
        }
    }

    #[test]
    fn controls_read_like_lichess() {
        assert_eq!(describe_control(&clock(300, 3)), "5+3");
        assert_eq!(describe_control(&clock(90, 0)), "1.5+0");
        let days = ChallengeTimeControl::Correspondence {
            days: Number::from(1),
        };
        assert_eq!(describe_control(&days), "1 day per move");
        assert_eq!(
            describe_control(&ChallengeTimeControl::Unlimited),
            "No clock"
        );
    }

    #[test]
    fn an_incoming_challenges_colour_is_the_other_one() {
        assert_eq!(color_text(&challenge("white", "in")), "you play black");
        assert_eq!(color_text(&challenge("white", "out")), "you play white");
        assert_eq!(color_text(&challenge("random", "in")), "random colours");
    }

    #[test]
    fn time_left_reads_in_days_hours_or_minutes() {
        assert_eq!(time_left(Some(90_000.)).unwrap(), "1d 1h left");
        assert_eq!(time_left(Some(4000.)).unwrap(), "1h 6m left");
        assert_eq!(time_left(Some(59.)).unwrap(), "0m left");
        assert!(time_left(None).is_none());
    }
}
