//! Home → Your Lichess activity (`pages/index.vue`, `stores/kchess/accountProfile.ts` and
//! `gameHistory.ts`): the account's games and ratings, its rating history by mode and range, and
//! its latest synced games. What was saved last time shows at once; Lichess's answer follows.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::core::AppCore;
use crate::pages::history::lichess::{RowDetail, game_row};
use crate::rating_history::RatingHistory;
use crate::theme::Palette;
use crate::widgets;

/// Synced games shown.
const RECENT: usize = 6;

pub struct LichessActivity {
    account: String,
    profile: Value,
    history: Entity<RatingHistory>,
    /// `gameLibraryOverview`: wins, losses and draws by account.
    overview: Value,
    recent: Vec<Value>,
    epoch: u64,
}

impl LichessActivity {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> LichessActivity {
        LichessActivity {
            account: String::new(),
            profile: Value::Null,
            history: cx.new(|cx| {
                RatingHistory::new(
                    "Built from your synced rated games. Lichess has no rating history for this account.",
                    window,
                    cx,
                )
            }),
            overview: Value::Null,
            recent: Vec::new(),
            epoch: 0,
        }
    }

    /// Show `account`, or read it again (`loadProfile` and the game history's account data).
    pub fn load(&mut self, account: &str, cx: &mut Context<Self>) {
        if self.account != account {
            self.account = account.to_string();
            self.profile = Value::Null;
            self.history
                .update(cx, |history, cx| history.set(Value::Null, Value::Null, cx));
            self.recent.clear();
        }
        self.epoch += 1;
        let epoch = self.epoch;
        let core = cx.global::<AppCore>().clone();
        let name = json!(account);
        let saved = core.call("cachedProfile", vec![name.clone()]);
        let overview = core.call("gameLibraryOverview", Vec::new());
        let recent = core.call(
            "gamePage",
            vec![json!({ "account": account, "offset": 0, "limit": RECENT })],
        );
        let fallback = core.call("gameRatingHistory", vec![name.clone()]);
        let profile = core.call("profile", vec![name.clone()]);
        let history = core.call("ratingHistory", vec![name]);
        cx.spawn(async move |this, cx| {
            // What was saved last time paints first.
            let saved = saved.await;
            let local = (overview.await, recent.await, fallback.await);
            let current = |this: &mut Self| this.epoch == epoch;
            let _ = this.update(cx, |this, cx| {
                if !current(this) {
                    return;
                }
                if let Ok(saved) = saved {
                    this.profile = saved["profile"].clone();
                    this.history.update(cx, |history, cx| {
                        history.set_official(saved["ratingHistory"].clone(), cx)
                    });
                }
                let (overview, recent, fallback) = local;
                if let Ok(overview) = overview {
                    this.overview = overview;
                }
                if let Ok(recent) = recent {
                    this.recent = recent["games"].as_array().cloned().unwrap_or_default();
                }
                if let Ok(fallback) = fallback {
                    this.history
                        .update(cx, |history, cx| history.set_fallback(fallback, cx));
                }
                cx.notify();
            });
            let (profile, history) = (profile.await, history.await);
            let _ = this.update(cx, |this, cx| {
                if !current(this) {
                    return;
                }
                match (profile, history) {
                    (Ok(profile), Ok(history)) => {
                        this.profile = profile;
                        this.history
                            .update(cx, |chart, cx| chart.set_official(history, cx));
                    }
                    (Err(cause), _) | (_, Err(cause)) => crate::log::write(
                        kchess_core::Level::Warn,
                        "account-profile",
                        &format!("loading profile failed: {}", cause.message),
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// `.stat-card`: a label, a value and a line under it.
fn stat(label: &str, value: impl IntoElement, sub: impl IntoElement, palette: &Palette) -> Div {
    widgets::card(palette)
        .flex_1()
        .min_w(px(150.))
        .gap(px(4.))
        .child(
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(label.to_string()),
        )
        .child(
            div()
                .text_size(px(26.))
                .font_weight(FontWeight::BOLD)
                .text_color(palette.highlighted)
                .child(value),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(sub),
        )
}

impl Render for LichessActivity {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let account = self.account.to_lowercase();
        let record = &self.overview["byAccount"][&account];
        let count = |key: &str| record[key].as_f64().unwrap_or_default();
        let lifetime = self.profile["count"]["all"].as_f64();
        let games = lifetime.unwrap_or_else(|| count("total"));
        let rating = |perf: &str| {
            self.profile["perfs"][perf]["rating"]
                .as_f64()
                .map_or_else(|| "—".to_string(), |rating| rating.to_string())
        };
        let progress = self.profile["perfs"]["blitz"]["prog"]
            .as_f64()
            .filter(|prog| *prog != 0.);
        // `.record`: each count with its letter small after it.
        let tally = |value: f64, letter: &'static str, color: Hsla| {
            h_flex()
                .items_baseline()
                .text_color(color)
                .child(format!("{value}"))
                .child(
                    div()
                        .ml(px(2.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .opacity(0.7)
                        .child(letter),
                )
        };
        let record_value = h_flex()
            .items_baseline()
            .gap(px(10.))
            .text_size(px(22.))
            .child(tally(count("win"), "W", palette.win))
            .child(tally(count("loss"), "L", palette.loss))
            .child(tally(count("draw"), "D", palette.text_muted));
        let stats = h_flex()
            .flex_wrap()
            .gap(px(12.))
            .child(stat(
                "Games",
                format!("{games}"),
                if lifetime.is_some() {
                    "Lichess lifetime"
                } else {
                    "Synced locally"
                },
                &palette,
            ))
            .child(stat(
                "Blitz rating",
                rating("blitz"),
                match progress {
                    Some(prog) => h_flex()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_color(if prog > 0. { palette.win } else { palette.loss })
                                .child(format!(
                                    "{} {}",
                                    if prog > 0. { "▲" } else { "▼" },
                                    prog.abs()
                                )),
                        )
                        .child("recently")
                        .into_any_element(),
                    None => div().child("Current rating").into_any_element(),
                },
                &palette,
            ))
            .child(stat(
                "Rapid rating",
                rating("rapid"),
                "Current rating",
                &palette,
            ))
            .child(stat(
                "Recent record",
                record_value,
                "From synced games",
                &palette,
            ));
        let chart_card = widgets::card(&palette)
            .flex_1()
            .min_w(px(300.))
            .child(self.history.clone());
        let account_name = self.account.clone();
        let recent_card = widgets::card(&palette)
            .flex_1()
            .min_w(px(300.))
            .gap(px(8.))
            .child(
                h_flex()
                    .justify_between()
                    .child(widgets::heading("Recent games", &palette))
                    .when(!self.recent.is_empty(), |header| {
                        header.child(
                            Button::new("home-lichess-all")
                                .text()
                                .xsmall()
                                .child(
                                    h_flex()
                                        .gap(px(4.))
                                        .child("View all")
                                        .child(icon("arrow-right").size(px(14.))),
                                )
                                .on_click(|_, window, cx| {
                                    crate::shell::open_history_lichess(window, cx)
                                }),
                        )
                    }),
            )
            .when(self.recent.is_empty(), |card| {
                let busy = crate::accounts::handle(cx).read(cx).busy;
                card.child(
                    widgets::empty_state(icon("history"), "No synced games yet", None, &palette)
                        .child(
                            h_flex().mt(px(10.)).child(
                                Button::new("home-lichess-sync")
                                    .outline()
                                    .icon(icon("refresh-cw"))
                                    .label("Sync games")
                                    .loading(busy)
                                    .disabled(busy)
                                    .on_click(move |_, window, cx| {
                                        let name = account_name.clone();
                                        crate::accounts::handle(cx).update(cx, |accounts, cx| {
                                            accounts.sync(Some(name), window, cx)
                                        })
                                    }),
                            ),
                        ),
                )
            })
            .children(self.recent.iter().enumerate().map(|(index, game)| {
                let chosen = game.clone();
                game_row(
                    ("home-lichess-game", index),
                    game,
                    None,
                    RowDetail::Brief,
                    move |window, cx| crate::shell::open_lichess_game(chosen.clone(), window, cx),
                    &palette,
                )
            }));
        let stacked = window.viewport_size().width < px(900.);
        v_flex()
            .gap(px(14.))
            .child(widgets::heading("Your Lichess activity", &palette))
            .child(stats)
            .child(
                if stacked {
                    v_flex()
                } else {
                    h_flex().items_start()
                }
                .gap(px(16.))
                .child(chart_card)
                .child(recent_card),
            )
    }
}
