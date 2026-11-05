//! Home → Your Lichess activity: the account's games and ratings, its rating history by mode and range, and
//! its latest synced games. What was saved last time shows at once; Lichess's answer follows.

use crate::widgets::KeyboardFocus as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::core::AppCore;
use crate::pages::history::lichess::{RowDetail, game_row};
use crate::rating_history::RatingHistory;
use crate::theme::{Palette, radii};
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
    /// The saved games have been read once for this account, so an empty list means none.
    loaded: bool,
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
                .without_mode_picker()
            }),
            overview: Value::Null,
            recent: Vec::new(),
            loaded: false,
            epoch: 0,
        }
    }

    /// "No synced games yet": only once the saved games are known to be none, not while they load.
    pub(crate) fn shows_empty(&self) -> bool {
        self.loaded && self.recent.is_empty()
    }

    /// Show `account`, or read it again (`loadProfile` and the game history's account data).
    pub fn load(&mut self, account: &str, cx: &mut Context<Self>) {
        if self.account != account {
            self.account = account.to_string();
            self.profile = Value::Null;
            self.history
                .update(cx, |history, cx| history.set(Value::Null, Value::Null, cx));
            self.recent.clear();
            self.loaded = false;
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
                this.loaded = true;
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

/// The speeds a rating card can show, in Lichess's order.
const PERFS: [(&str, &str); 6] = [
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("correspondence", "Correspondence"),
    ("puzzle", "Puzzles"),
];
/// Rating cards shown: the speeds played most.
const CARDS: usize = 4;

/// "3,897": a count with thousands separated.
fn grouped(value: f64) -> String {
    let digits = format!("{}", value.round() as i64);
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The speeds to show a card for: those played, most games first; Blitz and Rapid until the
/// profile is known.
fn shown_perfs(profile: &Value) -> Vec<(&'static str, &'static str)> {
    let games = |key: &str| profile["perfs"][key]["games"].as_f64().unwrap_or_default();
    let mut played: Vec<_> = PERFS
        .into_iter()
        .filter(|(key, _)| games(key) > 0.)
        .collect();
    if played.is_empty() {
        return vec![PERFS[1], PERFS[2]];
    }
    played.sort_by(|a, b| games(b.0).total_cmp(&games(a.0)));
    played.truncate(CARDS);
    played
}

impl Render for LichessActivity {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let account = self.account.to_lowercase();
        let record = &self.overview["byAccount"][&account];
        // Lichess's lifetime counts, else the synced games'.
        let counts = if self.profile["count"]["all"].is_number() {
            &self.profile["count"]
        } else {
            record
        };
        let count = |key: &str| counts[key].as_f64().unwrap_or_default();
        let games = counts["all"].as_f64().unwrap_or_else(|| count("total"));
        let chart_mode = self.history.read(cx).mode().to_string();
        let tally = |value: f64, letter: &'static str, color: Hsla| {
            h_flex()
                .items_baseline()
                .gap(px(2.))
                .child(div().text_color(color).child(grouped(value)))
                .child(div().text_color(palette.text_muted).child(letter))
        };
        let busy = crate::accounts::handle(cx).read(cx).busy;
        let online = crate::network::online(cx);
        let sync_account = self.account.clone();
        let account_line = h_flex()
            .flex_wrap()
            .items_baseline()
            .gap_x(px(10.))
            .gap_y(px(2.))
            .text_size(px(13.))
            .text_color(palette.text_muted)
            .child(format!("@{}", self.account))
            .when(games > 0., |line| {
                line.child(format!("{} games", grouped(games))).child(
                    h_flex()
                        .id("home-record")
                        .test_support()
                        .gap(px(8.))
                        .child(tally(count("win"), "W", palette.win))
                        .child(tally(count("loss"), "L", palette.loss))
                        .child(tally(count("draw"), "D", palette.text)),
                )
            });
        let header = h_flex()
            .items_start()
            .justify_between()
            .gap(px(12.))
            .child(
                v_flex()
                    .min_w_0()
                    .gap(px(2.))
                    .child(widgets::heading("Lichess", &palette).text_size(px(16.)))
                    .child(account_line),
            )
            .child(
                Button::new("home-sync")
                    .outline()
                    .small()
                    .flex_none()
                    .icon(icon("refresh-cw"))
                    .label("Sync games")
                    .loading(busy)
                    .disabled(!online || busy)
                    .on_click(move |_, window, cx| {
                        let name = sync_account.clone();
                        crate::accounts::handle(cx)
                            .update(cx, |accounts, cx| accounts.sync(Some(name), window, cx))
                    }),
            );
        let cards = h_flex()
            .id("home-perfs")
            .role(Role::TabList)
            .aria_label("Ratings")
            .flex_wrap()
            .items_stretch()
            .gap(px(12.))
            .children(shown_perfs(&self.profile).into_iter().map(|(key, label)| {
                let perf = &self.profile["perfs"][key];
                let rating = perf["rating"].as_f64();
                let progress = perf["prog"].as_f64().filter(|prog| *prog != 0.);
                let played = perf["games"].as_f64().unwrap_or_default();
                let chosen = chart_mode == key;
                let history = self.history.clone();
                v_flex()
                    .id(SharedString::from(format!("home-perf-{key}")))
                    .test_support()
                    // The cards choose the chart's speed, as tabs would.
                    .role(Role::Tab)
                    .aria_selected(chosen)
                    .aria_label(match rating {
                        Some(rating) => format!("{label} rating {rating}"),
                        None => format!("{label} rating"),
                    })
                    .keyboard_focus(palette.primary)
                    .flex_1()
                    .flex_basis(px(150.))
                    .min_w(px(140.))
                    .gap(px(2.))
                    .px(px(16.))
                    .py(px(14.))
                    .rounded(radii::XL)
                    .border_1()
                    .bg(palette.elevated)
                    .border_color(if chosen {
                        palette.primary
                    } else {
                        palette.border
                    })
                    .cursor_pointer()
                    .hover(|card| card.border_color(palette.border_accented))
                    .on_click(move |_, window, cx| {
                        history.update(cx, |history, cx| history.show_mode(key, window, cx))
                    })
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(label),
                    )
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap(px(8.))
                            .child(
                                div()
                                    .text_size(px(26.))
                                    .line_height(px(36.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(palette.highlighted)
                                    .child(
                                        rating.map_or_else(
                                            || "—".into(),
                                            |rating| format!("{rating}"),
                                        ),
                                    ),
                            )
                            .children(progress.map(|prog| {
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if prog > 0. { palette.win } else { palette.loss })
                                    .child(format!(
                                        "{}{}",
                                        if prog > 0. { "+" } else { "−" },
                                        prog.abs()
                                    ))
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(if played > 0. {
                                format!(
                                    "{} {}",
                                    grouped(played),
                                    if key == "puzzle" { "puzzles" } else { "games" }
                                )
                            } else {
                                "\u{a0}".into()
                            }),
                    )
            }));
        let chart_card = widgets::card(&palette)
            .flex_1()
            .min_w(px(300.))
            .child(self.history.clone());
        let account_name = self.account.clone();
        let recent_card = widgets::card(&palette)
            .flex_1()
            .min_w(px(300.))
            .gap(px(4.))
            .child(
                h_flex()
                    .justify_between()
                    .mb(px(4.))
                    .child(widgets::heading("Games", &palette))
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
            .when(self.shows_empty(), |card| {
                card.child(
                    widgets::empty_state(icon("history"), "No synced games yet", None, &palette)
                        .py(px(24.))
                        .child(
                            h_flex().mt(px(8.)).child(
                                Button::new("home-lichess-sync")
                                    .outline()
                                    .small()
                                    .icon(icon("refresh-cw"))
                                    .label("Sync games")
                                    .loading(busy)
                                    .disabled(busy || !online)
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
            .child(
                v_flex()
                    .mx(px(-12.))
                    .children(self.recent.iter().enumerate().map(|(index, game)| {
                        let chosen = game.clone();
                        game_row(
                            ("home-lichess-game", index),
                            game,
                            None,
                            RowDetail::Brief,
                            move |window, cx| {
                                crate::shell::open_lichess_game(chosen.clone(), window, cx)
                            },
                            &palette,
                        )
                    })),
            );
        let stacked = window.viewport_size().width < px(900.);
        v_flex().gap(px(16.)).child(header).child(cards).child(
            if stacked {
                v_flex()
            } else {
                h_flex().items_stretch()
            }
            .gap(px(16.))
            .child(chart_card)
            .child(recent_card),
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{LichessActivity, grouped, shown_perfs};

    #[test]
    fn counts_are_grouped_by_thousands() {
        assert_eq!(grouped(7.), "7");
        assert_eq!(grouped(3897.), "3,897");
        assert_eq!(grouped(1_234_567.), "1,234,567");
    }

    #[test]
    fn the_speeds_played_most_get_the_cards() {
        let profile = json!({ "perfs": {
            "bullet": { "games": 10 },
            "blitz": { "games": 900 },
            "rapid": { "games": 300 },
            "classical": { "games": 0 },
            "puzzle": { "games": 400 },
            "correspondence": { "games": 2 },
        } });
        let keys: Vec<_> = shown_perfs(&profile)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(keys, ["blitz", "puzzle", "rapid", "bullet"]);
        let unknown: Vec<_> = shown_perfs(&json!(null))
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(unknown, ["blitz", "rapid"]);
    }

    use crate::e2e::Harness;
    use gpui_kit::AppContext as _;

    /// The empty state waits for the saved games: it does not flash before them.
    #[gpui_kit::test]
    fn no_synced_games_shows_only_once_they_are_read(cx: &mut gpui_kit::TestAppContext) {
        let mut app = Harness::start(cx);
        let activity = app.with_window(|window, cx| {
            let activity = cx.new(|cx| LichessActivity::new(window, cx));
            activity.update(cx, |activity, cx| activity.load("nobody", cx));
            assert!(
                !activity.read(cx).shows_empty(),
                "still reading the saved games"
            );
            activity
        });
        app.wait_for("the saved games to be read", |_, cx| {
            activity.read(cx).shows_empty()
        });
    }
}
