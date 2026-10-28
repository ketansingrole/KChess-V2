//! The live game (`pages/online.vue`'s board and `MovePanel`): players with clocks and how their
//! connections look, the board, offers and the opponent leaving, the moves, and the actions for a
//! game on and over.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::hover_card::HoverCard;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use shakmaty::Color;

use super::OnlinePage;
use crate::assets::icon;
use crate::export::GameExport;
use crate::game::view;
use crate::online::{OnlineHandle, OnlineState, Phase};
use crate::play_layout::{PANEL_MAX, PANEL_MIN};
use crate::settings::SettingsHandle;
use crate::theme::Palette;
use crate::widgets::{self, KeyboardFocus as _, Side, Tone};

/// The player may touch pieces: the game is on, at its last move, in a variant shown here.
pub fn can_play(online: &OnlineState) -> bool {
    online.phase == Phase::Playing
        && online.view_ply.is_none()
        && online.model()["onlineUnsupported"]
            .as_str()
            .is_none_or(str::is_empty)
}

fn color_name(color: Color) -> &'static str {
    if color == Color::White {
        "white"
    } else {
        "black"
    }
}

fn action(name: &'static str) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, _, cx| crate::online::handle(cx).update(cx, |online, cx| online.action(name, cx))
}

/// An offer or a notice above the moves (`.offer-banner`).
fn offer(glyph: &'static str, text: String, buttons: Vec<AnyElement>, palette: &Palette) -> Div {
    h_flex()
        .gap(px(8.))
        .flex_wrap()
        .px(px(12.))
        .py(px(8.))
        .rounded(px(10.))
        .border_1()
        .border_color(palette.border)
        .bg(palette.elevated)
        .text_size(px(13.))
        .child(icon(glyph).size(px(16.)).flex_none())
        .child(div().flex_1().min_w(px(120.)).child(text))
        .children(buttons)
}

/// Connection quality from a round trip, in the bands Lichess uses for its signal bars
/// (`signalFromLatency`).
fn signal_from_latency(ms: f64) -> usize {
    match ms {
        ms if ms < 150. => 4,
        ms if ms < 300. => 3,
        ms if ms < 500. => 2,
        _ => 1,
    }
}

const QUALITY: [&str; 5] = ["", "Poor", "Fair", "Good", "Great"];

/// The recent round trips to Lichess: the last, the average and the worst (`pingStats`).
fn ping_stats(samples: &[f64]) -> Option<(f64, f64, f64)> {
    let current = *samples.last()?;
    let average = (samples.iter().sum::<f64>() / samples.len() as f64).round();
    let worst = samples.iter().copied().fold(f64::MIN, f64::max);
    Some((current, average, worst))
}

/// The graph's points, 0 to 1 across and down from the top, against at least 200 ms.
fn ping_points(samples: &[f64]) -> Vec<(f32, f32)> {
    if samples.len() < 2 {
        return Vec::new();
    }
    let ceiling = samples.iter().copied().fold(200., f64::max);
    samples
        .iter()
        .enumerate()
        .map(|(index, ms)| {
            let x = index as f32 / (samples.len() - 1) as f32;
            let y = 1. - (ms.min(ceiling) / ceiling) as f32;
            (x, y)
        })
        .collect()
}

/// A label and its value in a connection panel's rows.
fn connection_row(label: &'static str, value: String, palette: &Palette) -> Div {
    h_flex()
        .justify_between()
        .gap(px(16.))
        .child(div().text_color(palette.text_muted).child(label))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(value))
}

/// Your connection, shown when your connection line is pointed at (`ConnectionPanel.vue`).
fn own_connection_panel(cx: &App, palette: &Palette) -> Stateful<Div> {
    let online = crate::online::handle(cx).read(cx);
    let stats = ping_stats(&online.latency);
    let points = ping_points(&online.latency);
    let downloaded = crate::accounts::handle(cx)
        .read(cx)
        .downloaded(online.account());
    let line = palette.primary;
    v_flex()
        .id("own-connection-panel")
        .w(px(240.))
        .gap(px(8.))
        .text_size(px(12.))
        .aria_label("Your connection")
        .child(
            h_flex()
                .justify_between()
                .child(match stats {
                    Some((current, _, _)) => h_flex()
                        .items_baseline()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(18.))
                                .font_weight(FontWeight::BOLD)
                                .child(format!("{}", current.round())),
                        )
                        .child(div().text_color(palette.text_muted).child("ms ping")),
                    None => h_flex()
                        .text_color(palette.text_muted)
                        .child("Measuring ping…"),
                })
                .children(stats.map(|(current, _, _)| {
                    div()
                        .text_color(palette.success)
                        .child(QUALITY[signal_from_latency(current)])
                })),
        )
        .when(!points.is_empty(), |panel| {
            panel.child(
                div()
                    .id("ping-graph")
                    .role(Role::Image)
                    .aria_label("Recent ping")
                    .w_full()
                    .h(px(36.))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, (), window, _| {
                                let (width, height) =
                                    (bounds.size.width.as_f32(), bounds.size.height.as_f32());
                                let at = |(x, y): (f32, f32)| {
                                    point(
                                        bounds.origin.x + px(x * width),
                                        bounds.origin.y + px(1. + y * (height - 2.)),
                                    )
                                };
                                let mut path = PathBuilder::stroke(px(1.5));
                                path.move_to(at(points[0]));
                                for point in &points[1..] {
                                    path.line_to(at(*point));
                                }
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, line);
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
        })
        .child(connection_row(
            "Average / worst",
            stats.map_or("—".into(), |(_, average, worst)| {
                format!("{average} / {} ms", worst.round())
            }),
            palette,
        ))
        .child(connection_row(
            "Last move accepted in",
            online
                .move_ack
                .map_or("—".into(), |ms| format!("{} ms", ms.round())),
            palette,
        ))
        .child(connection_row(
            "Downloaded this account",
            widgets::format_bytes(downloaded),
            palette,
        ))
        .child(div().text_color(palette.text_muted).child(format!(
            "Measured against lichess.org every {} s.",
            crate::online::PRESENCE_EVERY.as_secs()
        )))
}

/// The opponent's connection as Lichess reports it.
fn opponent_connection_panel(cx: &App, palette: &Palette) -> Stateful<Div> {
    let online = crate::online::handle(cx).read(cx);
    let model = online.model();
    let name = model["onlineOpponent"]
        .as_str()
        .or(model["opponentId"].as_str())
        .unwrap_or_default()
        .to_string();
    let them = model["opponentId"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let entry = online
        .presence
        .as_ref()
        .map(|report| &report["users"][&them]);
    let (status, signal) = if model["opponentGone"] == json!(true) {
        ("Disconnected".to_string(), "—".to_string())
    } else {
        match entry.filter(|entry| entry.is_object()) {
            None => ("Checking…".to_string(), "—".to_string()),
            Some(entry) => (
                if entry["online"] == json!(true) {
                    "Online".to_string()
                } else {
                    "Offline".to_string()
                },
                entry["signal"]
                    .as_u64()
                    .filter(|signal| (1..=4).contains(signal))
                    .map_or("—".into(), |signal| {
                        format!("{} · {signal} of 4", QUALITY[signal as usize])
                    }),
            ),
        }
    };
    v_flex()
        .id("opponent-connection-panel")
        .w(px(220.))
        .gap(px(8.))
        .text_size(px(12.))
        .aria_label(format!("{name}'s connection"))
        .child(div().font_weight(FontWeight::BOLD).child(name))
        .child(connection_row("Status", status, palette))
        .child(connection_row("Signal", signal, palette))
        .child(
            div()
                .text_color(palette.text_muted)
                .child("As reported by Lichess."),
        )
}

/// "Online · 84 ms", or how the opponent's connection looks, with its panel when pointed at
/// (`ConnectionPanel.vue`).
fn connection(online: &OnlineState, mine: bool, palette: &Palette) -> Option<AnyElement> {
    if online.phase != Phase::Playing || online.correspondence() {
        return None;
    }
    let report = online.presence.as_ref();
    let (text, color) = if mine {
        let me = online.account().to_lowercase();
        match online.latency.last() {
            None => ("Measuring…".to_string(), palette.text_muted),
            Some(ms) => {
                let offline = report.is_some_and(|r| r["users"][&me]["online"] == json!(false));
                (
                    format!(
                        "{} · {} ms",
                        if offline { "Offline" } else { "Online" },
                        ms.round()
                    ),
                    if offline {
                        palette.error
                    } else {
                        palette.success
                    },
                )
            }
        }
    } else if online.model()["opponentGone"] == json!(true) {
        ("Away".to_string(), palette.warning)
    } else {
        let them = online.model()["opponentId"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        match report.map(|r| &r["users"][&them]) {
            Some(entry) if entry.is_object() => {
                if entry["online"] == json!(true) {
                    ("Online".to_string(), palette.success)
                } else {
                    ("Offline".to_string(), palette.error)
                }
            }
            _ => ("Checking…".to_string(), palette.text_muted),
        }
    };
    let palette = *palette;
    Some(
        HoverCard::new(if mine {
            "own-connection"
        } else {
            "opponent-connection"
        })
        .open_delay(std::time::Duration::from_millis(200))
        .trigger(
            h_flex()
                .gap(px(6.))
                .text_size(px(12.))
                .font_weight(FontWeight::NORMAL)
                .text_color(color)
                .child(div().size(px(8.)).rounded_full().bg(color))
                .child(text),
        )
        .content(move |_, _, cx| {
            if mine {
                own_connection_panel(cx, &palette)
            } else {
                opponent_connection_panel(cx, &palette)
            }
        })
        .into_any_element(),
    )
}

impl OnlinePage {
    pub(super) fn game_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let palette = *cx.global::<Palette>();
        let online_entity = cx.global::<OnlineHandle>().0.clone();
        let online = online_entity.read(cx);
        let model = online.model().clone();
        let settings = cx.global::<SettingsHandle>().0.read(cx).settings.clone();
        let setup = online.setup();
        let moves = online.moves();
        let ply = online.ply();
        let color = online.color();
        let orientation = online.orientation();
        let playing = online.phase == Phase::Playing;
        let correspondence = online.correspondence();
        let has_clock = model["onlineClockConfig"].is_object() || correspondence;
        let position = view::position_at(&setup, &moves, ply);
        let live_turn = view::turn(view::position_at(&setup, &moves, moves.len()).as_ref());
        let fen = position
            .as_ref()
            .map(kchess_domain::rules::make_fen)
            .unwrap_or_default();
        let opponent = model["onlineOpponent"]
            .as_str()
            .unwrap_or("Opponent")
            .to_string();
        let opponent_id = model["opponentId"].as_str().unwrap_or_default().to_string();
        let line = |side: Color| {
            let mine = side == color;
            let rating = model[if mine {
                "onlineRating"
            } else {
                "onlineOpponentRating"
            }]
            .as_f64();
            let name = match (mine, rating) {
                (true, Some(rating)) => format!("You ({rating})"),
                (true, None) => "You".into(),
                (false, Some(rating)) => format!("{opponent} ({rating})"),
                (false, None) => opponent.clone(),
            };
            let shown: AnyElement = if !mine && !opponent_id.is_empty() {
                let link = opponent.clone();
                div()
                    .id("online-opponent-name")
                    .role(Role::Link)
                    .keyboard_focus(palette.primary)
                    .aria_label(link.clone())
                    .cursor_pointer()
                    .hover(|name| name.underline())
                    .on_click(move |_, window, cx| crate::shell::open_player(&link, window, cx))
                    .child(name)
                    .into_any_element()
            } else {
                div().child(name).into_any_element()
            };
            let running = playing && live_turn == side;
            widgets::player_line(
                h_flex()
                    .gap(px(10.))
                    .child(widgets::avatar(
                        icon("user"),
                        if side == Color::White {
                            Side::White
                        } else {
                            Side::Black
                        },
                        running,
                        &palette,
                    ))
                    .child(shown)
                    .children(widgets::material(
                        &fen,
                        side == Color::White,
                        &settings.frontend.piece_set,
                        &palette,
                    )),
                connection(online, mine, &palette),
                has_clock.then(|| {
                    widgets::clock(
                        online.clock_text(color_name(side)).into(),
                        running,
                        &palette,
                    )
                    .into_any_element()
                }),
            )
        };
        let board_size = self.layout.board_size(window);
        let mut board_column = v_flex().w(board_size).flex_none().gap(widgets::STACK_GAP);
        if online.phase == Phase::Disconnected {
            board_column = board_column.child(self.disconnected(online, &palette));
        }
        board_column = board_column
            .child(line(!orientation))
            .child(crate::board::cached(&self.board))
            .child(line(orientation));

        let panel = self
            .game_panel(online, &model, cx, &palette)
            .h(self.layout.panel_height(board_size));
        self.layout
            .render("online-game", board_column, panel, window, cx)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.focus.is_focused(window) || event.keystroke.modifiers.modified() {
                    return;
                }
                if crate::shell::zen_key(&event.keystroke.key, cx) {
                    cx.stop_propagation();
                    return;
                }
                let online = crate::online::handle(cx);
                let (ply, count) = {
                    let online = online.read(cx);
                    (online.ply(), online.moves().len())
                };
                if let Some(next) = view::navigate(&event.keystroke.key, ply, count) {
                    online.update(cx, |online, cx| online.view_ply(next, cx));
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
            )
            .children(self.chat_dock(cx, &palette))
    }

    fn disconnected(&self, online: &OnlineState, palette: &Palette) -> Div {
        let connection = online.model()["onlineConnection"]["phase"]
            .as_str()
            .unwrap_or_default();
        let title = match connection {
            "checking" => "Checking game status",
            "auth-required" => "Sign in again",
            _ => "Connection interrupted",
        };
        let status = online.model()["onlineStatus"].as_str().unwrap_or_default();
        let has_game = !online.id().is_empty();
        widgets::card(palette)
            .gap(px(8.))
            .child(widgets::heading(title, palette))
            .child(widgets::hint(
                format!("{status} Clocks are paused until a fresh server update arrives."),
                palette,
            ))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        Button::new("online-reconnect")
                            .primary()
                            .label("Reconnect")
                            .loading(online.busy)
                            .on_click(|_, _, cx| {
                                crate::online::handle(cx)
                                    .update(cx, |online, cx| online.reconnect(cx))
                            }),
                    )
                    .child(
                        Button::new("online-accounts")
                            .outline()
                            .label("Account settings")
                            .on_click(|_, window, cx| {
                                crate::shell::open_settings(
                                    crate::pages::settings::Section::Accounts,
                                    window,
                                    cx,
                                )
                            }),
                    )
                    .when(!has_game, |row| {
                        row.child(
                            Button::new("online-another")
                                .ghost()
                                .label("Find another game")
                                .on_click(|_, _, cx| {
                                    crate::online::handle(cx)
                                        .update(cx, |online, cx| online.stop(cx))
                                }),
                        )
                    }),
            )
    }

    fn game_panel(&self, online: &OnlineState, model: &Value, cx: &App, palette: &Palette) -> Div {
        let playing = online.phase == Phase::Playing;
        let finished = online.phase == Phase::Finished;
        let moves = online.moves();
        let setup = online.setup();
        let ply = online.ply();
        let sans = view::sans(&setup, &moves);
        let opponent = model["onlineOpponent"]
            .as_str()
            .unwrap_or("Opponent")
            .to_string();
        let unsupported = model["onlineUnsupported"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let reviewing = playing && !can_play(online) && unsupported.is_empty();
        let variant = setup["variant"].as_str().unwrap_or("standard");
        let variant_name = (variant != "standard").then(|| {
            super::variants()
                .into_iter()
                .find(|(key, _, _)| key == variant)
                .map_or(variant.to_string(), |(_, label, _)| label)
        });
        let rated = model["onlineGameRated"] == json!(true);
        let status = model["onlineStatus"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let banner: Option<(&'static str, String, String)> = if !unsupported.is_empty() {
            Some((
                "triangle-alert",
                format!("{unsupported} is not supported yet"),
                "KChess cannot show this variant’s board. You can still resign or abort it here."
                    .into(),
            ))
        } else if reviewing {
            Some((
                "history",
                "Reviewing an earlier position".into(),
                "Go to the last move to keep playing.".into(),
            ))
        } else if playing {
            None
        } else if online.phase == Phase::Disconnected {
            Some((
                "triangle-alert",
                "Connection interrupted".into(),
                status.clone(),
            ))
        } else {
            Some((
                "flag",
                if status.is_empty() {
                    "Game over".into()
                } else {
                    status.clone()
                },
                String::new(),
            ))
        };
        let now = crate::online::now();
        let seconds_until = |key: &str| {
            model[key]
                .as_f64()
                .map(|at| ((at - now) / 1000.).ceil().max(0.))
        };
        let first_move_in = seconds_until("firstMoveBy").filter(|_| playing);
        let claim_in = (playing && model["opponentGone"] == json!(true))
            .then(|| seconds_until("claimAt"))
            .flatten();
        let draw = model["drawOffer"].as_str().unwrap_or("none");
        let takeback = model["takebackOffer"].as_str().unwrap_or("none");
        let mut notices: Vec<AnyElement> = Vec::new();
        if let Some(seconds) = first_move_in {
            notices.push(
                offer(
                    "timer",
                    format!("Move within {seconds}s or the game is aborted."),
                    Vec::new(),
                    palette,
                )
                .into_any_element(),
            );
        }
        let small = |id: &'static str, label: &'static str, primary: bool, run: &'static str| {
            Button::new(id)
                .xsmall()
                .when(primary, |button| button.primary())
                .when(!primary, |button| button.outline())
                .label(label)
                .on_click(action(run))
                .into_any_element()
        };
        if playing {
            match draw {
                "theirs" => notices.push(
                    offer(
                        "handshake",
                        format!("{opponent} offers a draw."),
                        vec![
                            small("draw-accept", "Accept", true, "acceptDraw"),
                            small("draw-decline", "Decline", false, "declineDraw"),
                        ],
                        palette,
                    )
                    .into_any_element(),
                ),
                "mine" => notices.push(
                    offer(
                        "handshake",
                        "You offered a draw.".into(),
                        vec![small("draw-cancel", "Cancel offer", false, "declineDraw")],
                        palette,
                    )
                    .into_any_element(),
                ),
                _ => {}
            }
            match takeback {
                "theirs" => notices.push(
                    offer(
                        "undo-2",
                        format!("{opponent} asks to take back a move."),
                        vec![
                            small("takeback-accept", "Accept", true, "takeback"),
                            small("takeback-decline", "Decline", false, "declineTakeback"),
                        ],
                        palette,
                    )
                    .into_any_element(),
                ),
                "mine" => notices.push(
                    offer(
                        "undo-2",
                        "Takeback requested.".into(),
                        vec![small("takeback-cancel", "Cancel", false, "declineTakeback")],
                        palette,
                    )
                    .into_any_element(),
                ),
                _ => {}
            }
        }
        if let Some(seconds) = claim_in {
            notices.push(
                offer(
                    "wifi-off",
                    if seconds > 0. {
                        format!("{opponent} left the game. You can claim the win in {seconds}s.")
                    } else {
                        format!("{opponent} left the game.")
                    },
                    if seconds > 0. {
                        Vec::new()
                    } else {
                        vec![
                            small("claim-win", "Claim victory", true, "claimVictory"),
                            small("claim-draw", "Call it a draw", false, "claimDraw"),
                        ]
                    },
                    palette,
                )
                .into_any_element(),
            );
        }
        if !playing && let Some(rematch) = online.rematch_offer() {
            let (accept, decline) = (rematch.clone(), rematch.clone());
            notices.push(
                offer(
                    "repeat",
                    format!(
                        "{} wants a rematch.",
                        rematch["opponent"]["name"]
                            .as_str()
                            .unwrap_or("Your opponent")
                    ),
                    vec![
                        Button::new("rematch-accept")
                            .xsmall()
                            .primary()
                            .label("Accept")
                            .on_click(move |_, _, cx| {
                                let challenge = accept.clone();
                                crate::online::handle(cx)
                                    .update(cx, |online, cx| online.accept(&challenge, cx).detach())
                            })
                            .into_any_element(),
                        Button::new("rematch-decline")
                            .xsmall()
                            .outline()
                            .label("Decline")
                            .on_click(move |_, _, cx| {
                                let challenge = decline.clone();
                                crate::online::handle(cx).update(cx, |online, cx| {
                                    online.decline(&challenge, "generic", cx)
                                })
                            })
                            .into_any_element(),
                    ],
                    palette,
                )
                .into_any_element(),
            );
        }
        let select = move |ply: usize, _: &mut Window, cx: &mut App| {
            crate::online::handle(cx).update(cx, |online, cx| online.view_ply(ply, cx));
        };
        let flip: widgets::ClickHandler = Box::new(move |_, _, cx| {
            crate::online::handle(cx).update(cx, |online, cx| {
                online.flipped = !online.flipped;
                cx.notify();
            });
        });
        let can_takeback = playing && sans.len() >= 2;
        let tournament = model["onlineTournament"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let can_berserk = playing
            && !tournament.is_empty()
            && model["onlineClockConfig"].is_object()
            && model["berserked"] != json!(true)
            && online.view()["ownMoves"].as_f64() == Some(0.);
        let last_game = model["lastGame"].is_object();
        let actions = if playing {
            h_flex()
                .flex_wrap()
                .gap(px(8.))
                .when(can_berserk, |row| {
                    row.child(
                        Button::new("online-berserk")
                            .warning()
                            .icon(icon("flame"))
                            .label("Berserk")
                            .on_click(action("berserk")),
                    )
                })
                .when(can_takeback && takeback == "none", |row| {
                    row.child(
                        Button::new("online-takeback")
                            .outline()
                            .icon(icon("undo-2"))
                            .label("Takeback")
                            .on_click(action("takeback")),
                    )
                })
                .when(can_takeback && draw == "none", |row| {
                    row.child(
                        Button::new("online-draw")
                            .outline()
                            .icon(icon("handshake"))
                            .label("Offer draw")
                            .on_click(action("offerDraw")),
                    )
                })
                .child(if can_takeback {
                    Button::new("online-resign")
                        .outline()
                        .danger()
                        .icon(icon("flag"))
                        .label("Resign")
                        .on_click(|_, window, cx| {
                            window.open_alert_dialog(cx, |dialog: AlertDialog, _, _| {
                                dialog
                                    .title("Resign this game?")
                                    .description("Your opponent will be awarded the win.")
                                    .ok_text("Resign")
                                    .ok_variant(ButtonVariant::Danger)
                                    .cancel_text("Cancel")
                                    .show_cancel(true)
                                    .on_ok(|_, _, cx| {
                                        crate::online::handle(cx)
                                            .update(cx, |online, cx| online.action("resign", cx));
                                        true
                                    })
                            })
                        })
                } else {
                    Button::new("online-abort")
                        .outline()
                        .danger()
                        .icon(icon("ban"))
                        .label("Abort")
                        .on_click(action("abort"))
                })
                .when(online.correspondence(), |row| {
                    row.child(
                        Button::new("online-close")
                            .ghost()
                            .icon(icon("x"))
                            .label("Close game")
                            .on_click(|_, _, cx| {
                                crate::online::handle(cx).update(cx, |online, cx| online.stop(cx))
                            }),
                    )
                })
        } else {
            let can_analyse = finished && variant == "standard";
            let export = finished.then(|| self.game_export(online, model));
            h_flex()
                .flex_wrap()
                .gap(px(8.))
                .when(last_game && finished && tournament.is_empty(), |row| {
                    row.child(
                        Button::new("online-rematch")
                            .primary()
                            .icon(icon("repeat"))
                            .label("Rematch")
                            .loading(online.busy)
                            .on_click(|_, _, cx| {
                                crate::online::handle(cx)
                                    .update(cx, |online, cx| online.rematch(cx))
                            }),
                    )
                })
                .when(can_analyse, |row| {
                    row.child(
                        Button::new("online-analyse")
                            .outline()
                            .icon(icon("microscope"))
                            .label("Analyse")
                            .on_click(|_, window, cx| analyse(window, cx)),
                    )
                })
                .children(export.map(|export| {
                    crate::export::export_menu(
                        "online-export",
                        &self.exporter,
                        export,
                        Anchor::BottomLeft,
                        cx,
                    )
                }))
                .when(finished && !tournament.is_empty(), |row| {
                    let tournament = tournament.clone();
                    row.child(
                        Button::new("online-tournament")
                            .outline()
                            .icon(icon("trophy"))
                            .label("Tournament")
                            .on_click(move |_, window, cx| {
                                crate::shell::open_tournament(&tournament, window, cx)
                            }),
                    )
                })
                .child(
                    Button::new("online-new")
                        .when(last_game, |button| button.outline())
                        .when(!last_game, |button| button.primary())
                        .icon(icon("plus"))
                        .label("New game")
                        .on_click(|_, _, cx| {
                            crate::online::handle(cx).update(cx, |online, cx| online.stop(cx))
                        }),
                )
        };
        widgets::side_panel(palette)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .child(
                h_flex()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!(
                        "{}{}",
                        variant_name
                            .map(|name| format!("{name} · "))
                            .unwrap_or_default(),
                        if rated { "Rated" } else { "Casual" }
                    ))
                    .child(div().flex_1())
                    .child(widgets::view_toggles(
                        cx.global::<SettingsHandle>()
                            .0
                            .read(cx)
                            .settings
                            .frontend
                            .blindfold,
                        cx.global::<SettingsHandle>()
                            .0
                            .read(cx)
                            .settings
                            .frontend
                            .zen_mode,
                    )),
            )
            .children(banner.map(|(glyph, title, detail)| {
                widgets::StatusBanner::new(Tone::Plain, icon(glyph), title.into())
                    .announce()
                    .when_some(
                        (!detail.is_empty()).then(|| detail.into()),
                        |banner, value| banner.detail(value),
                    )
            }))
            .children(notices)
            .child(div().flex_1().min_h(px(120.)).child(widgets::moves_list(
                "online-moves",
                &sans,
                ply,
                view::black_first(&setup),
                "No moves yet.".into(),
                select,
                palette,
            )))
            .child(widgets::move_controls(
                ply,
                moves.len(),
                select,
                ("Flip board".into(), flip),
            ))
            .child(
                div()
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(actions),
            )
    }

    fn game_export(&self, online: &OnlineState, model: &Value) -> GameExport {
        let setup = online.setup();
        let me = online.account().to_string();
        let opponent = model["onlineOpponent"]
            .as_str()
            .unwrap_or("Opponent")
            .to_string();
        let (white, black) = if online.color() == Color::White {
            (me, opponent)
        } else {
            (opponent, me)
        };
        GameExport {
            variant: setup["variant"].as_str().unwrap_or("standard").to_string(),
            fen: setup["fen"].as_str().unwrap_or_default().to_string(),
            moves: online.moves(),
            orientation: online.orientation(),
            white,
            black,
            ply: Some(online.ply()),
            result: Some(
                model["onlineResult"]
                    .as_str()
                    .filter(|result| !result.is_empty())
                    .unwrap_or("*")
                    .to_string(),
            ),
            lichess_id: Some(online.id().to_string()).filter(|id| !id.is_empty()),
        }
    }
}

/// The finished game on the analysis board, at the move looked at, reviewed (`analyse`).
fn analyse(window: &mut Window, cx: &mut App) {
    let online = crate::online::handle(cx).read(cx);
    let model = online.model().clone();
    let setup = online.setup();
    let moves = online.moves();
    let ply = online.ply();
    let color = online.color();
    let opponent = model["onlineOpponent"].clone();
    let (white, black) = if color == Color::White {
        (json!("You"), opponent)
    } else {
        (opponent, json!("You"))
    };
    let origin = json!({
        "gameId": model["onlineId"],
        "account": model["onlineAccount"],
        "white": white,
        "black": black,
        "player": color_name(color),
    });
    let Ok(Value::String(pgn)) = crate::game::rules("setupPgn", &[setup, json!(moves), json!({})])
    else {
        return;
    };
    crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
        analysis.open_game(&pgn, ply, Some(color), Some(origin.clone()), true, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::{ping_points, ping_stats, signal_from_latency};

    #[test]
    fn the_ping_summary_reads_as_electrons() {
        assert_eq!(ping_stats(&[]), None);
        assert_eq!(ping_stats(&[80., 120., 101.]), Some((101., 100., 120.)));
        let quality: Vec<usize> = [149., 150., 299., 300., 499., 500.]
            .into_iter()
            .map(signal_from_latency)
            .collect();
        assert_eq!(quality, vec![4, 3, 3, 2, 2, 1]);
    }

    #[test]
    fn the_graph_needs_two_round_trips_and_scales_to_at_least_200_ms() {
        assert!(ping_points(&[50.]).is_empty());
        assert_eq!(ping_points(&[0., 100.]), vec![(0., 1.), (1., 0.5)]);
        assert_eq!(ping_points(&[400., 200.]), vec![(0., 0.), (1., 0.5)]);
    }
}
