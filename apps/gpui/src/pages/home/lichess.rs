//! Home → Your Lichess activity: the account's games and ratings, its rating history by mode and range, and
//! its latest synced games. What was saved last time shows at once; Lichess's answer follows.
//!
//! A Chess.com player's card is the same, built from the games synced here: Chess.com keeps no
//! profile KChess asks for.

use crate::widgets::KeyboardFocus as _;
use gpui_kit::component::button::Button;
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

/// Under a chart drawn from games, as each source has them.
const LICHESS_HINT: &str =
    "Built from your synced rated games. Lichess has no rating history for this account.";
const CHESSCOM_HINT: &str = "Built from your synced rated games.";

/// Where the account's games come from.
#[derive(Clone, Debug, PartialEq)]
enum Source {
    Lichess,
    /// A Chess.com player, by name: the ratings are the synced games'.
    Chesscom(String),
}

impl Source {
    /// The source of a game account (`name@chess.com` is a Chess.com player's).
    fn of(account: &str) -> Source {
        match kchess_domain::chesscom::username_of(account) {
            Some(name) => Source::Chesscom(name.to_string()),
            None => Source::Lichess,
        }
    }

    fn heading(&self) -> &'static str {
        match self {
            Source::Lichess => "Lichess",
            Source::Chesscom(_) => "Chess.com",
        }
    }

    fn games_hint(&self) -> &'static str {
        match self {
            Source::Lichess => LICHESS_HINT,
            Source::Chesscom(_) => CHESSCOM_HINT,
        }
    }

    /// Download the account's new games.
    fn sync(&self, account: &str, window: &mut Window, cx: &mut App) {
        let (source, account) = (self.clone(), account.to_string());
        crate::accounts::handle(cx).update(cx, |accounts, cx| match source {
            Source::Lichess => accounts.sync(Some(account), window, cx),
            Source::Chesscom(name) => accounts.sync_chesscom(Some(name), window, cx),
        })
    }
}

pub struct LichessActivity {
    account: String,
    source: Source,
    profile: Value,
    history: Entity<RatingHistory>,
    /// `gameLibraryOverview`: wins, losses and draws by account.
    overview: Value,
    recent: Vec<Value>,
    /// The saved games have been read once for this account, so an empty list means none.
    loaded: bool,
    epoch: u64,
    /// A sync ran when the accounts last changed: its end brings the new games.
    syncing: bool,
    _accounts: Subscription,
}

impl LichessActivity {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> LichessActivity {
        let accounts = crate::accounts::handle(cx);
        let _accounts = cx.observe(&accounts, |this, accounts, cx| {
            let syncing = accounts.read(cx).working();
            if std::mem::replace(&mut this.syncing, syncing) && !syncing && !this.account.is_empty()
            {
                let account = this.account.clone();
                this.load(&account, cx);
            }
        });
        LichessActivity {
            account: String::new(),
            source: Source::Lichess,
            profile: Value::Null,
            history: cx
                .new(|cx| RatingHistory::new(LICHESS_HINT, window, cx).without_mode_picker()),
            overview: Value::Null,
            recent: Vec::new(),
            loaded: false,
            epoch: 0,
            syncing: accounts.read(cx).working(),
            _accounts,
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
            self.source = Source::of(account);
            self.profile = Value::Null;
            let hint = self.source.games_hint();
            self.history.update(cx, |history, cx| {
                history.set_games_hint(hint, cx);
                history.set(Value::Null, Value::Null, cx)
            });
            self.recent.clear();
            self.loaded = false;
        }
        self.epoch += 1;
        let epoch = self.epoch;
        let core = cx.global::<AppCore>().clone();
        let name = json!(account);
        let overview = core.call("gameLibraryOverview", Vec::new());
        let recent = core.call(
            "gamePage",
            vec![json!({ "account": account, "offset": 0, "limit": RECENT })],
        );
        let fallback = core.call("gameRatingHistory", vec![name.clone()]);
        if let Source::Chesscom(_) = self.source {
            // No profile to ask for: each speed's games come from the synced games' report.
            let speeds = core.call("insights", vec![json!({ "account": account })]);
            cx.spawn(async move |this, cx| {
                let (overview, recent, fallback, speeds) =
                    (overview.await, recent.await, fallback.await, speeds.await);
                let _ = this.update(cx, |this, cx| {
                    if this.epoch != epoch {
                        return;
                    }
                    if let Ok(fallback) = &fallback {
                        this.profile =
                            local_profile(fallback, speeds.as_ref().unwrap_or(&Value::Null));
                    }
                    this.show_local(overview.ok(), recent.ok(), fallback.ok(), cx);
                });
            })
            .detach();
            return;
        }
        let saved = core.call("cachedProfile", vec![name.clone()]);
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
                this.show_local(overview.ok(), recent.ok(), fallback.ok(), cx);
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

    /// What is kept here: the record by account, the latest games and the history from games.
    fn show_local(
        &mut self,
        overview: Option<Value>,
        recent: Option<Value>,
        fallback: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        if let Some(overview) = overview {
            self.overview = overview;
        }
        if let Some(recent) = recent {
            self.recent = recent["games"].as_array().cloned().unwrap_or_default();
        }
        self.loaded = true;
        if let Some(fallback) = fallback {
            self.history
                .update(cx, |history, cx| history.set_fallback(fallback, cx));
        }
        cx.notify();
    }
}

/// A profile's `perfs` from the games synced here (a Chess.com player's): each speed's latest
/// rating from `gameRatingHistory`, and its games from the `insights` report's `bySpeed`.
fn local_profile(history: &Value, report: &Value) -> Value {
    let mut perfs = serde_json::Map::new();
    for (key, _) in PERFS {
        let rating = history
            .as_array()
            .into_iter()
            .flatten()
            .find(|series| series["name"] == key)
            .and_then(|series| series["points"].as_array()?.last()?.get(3).cloned());
        let games = report["bySpeed"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|speed| speed["speed"] == key)
            .map(|speed| speed["record"]["total"].clone());
        if rating.is_some() || games.is_some() {
            perfs.insert(
                key.to_string(),
                json!({ "rating": rating, "games": games.unwrap_or(json!(0)) }),
            );
        }
    }
    json!({ "perfs": perfs })
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

impl LichessActivity {
    /// The games, wins, losses and draws: Lichess's lifetime counts, else the synced games'.
    fn counts(&self) -> &Value {
        if self.profile["count"]["all"].is_number() {
            &self.profile["count"]
        } else {
            &self.overview["byAccount"][self.account.to_lowercase()]
        }
    }

    fn games(&self) -> f64 {
        let counts = self.counts();
        counts["all"]
            .as_f64()
            .or_else(|| counts["total"].as_f64())
            .unwrap_or_default()
    }

    /// The name shown: a Chess.com player's without the account's suffix.
    fn handle(&self) -> &str {
        match &self.source {
            Source::Lichess => &self.account,
            Source::Chesscom(name) => name,
        }
    }

    /// The heading, the name shown and the games counted.
    #[cfg(test)]
    pub(crate) fn summary(&self) -> (&'static str, String, f64) {
        (
            self.source.heading(),
            format!("@{}", self.handle()),
            self.games(),
        )
    }
}

impl Render for LichessActivity {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let counts = self.counts();
        let count = |key: &str| counts[key].as_f64().unwrap_or_default();
        let games = self.games();
        let chart_mode = self.history.read(cx).mode().to_string();
        let tally = |value: f64, letter: &'static str, color: Hsla| {
            h_flex()
                .items_baseline()
                .gap(px(2.))
                .child(div().text_color(color).child(widgets::format_count(value)))
                .child(div().text_color(palette.text_muted).child(letter))
        };
        let busy = {
            let accounts = crate::accounts::handle(cx).read(cx);
            match self.source {
                Source::Lichess => accounts.busy,
                Source::Chesscom(_) => accounts.working(),
            }
        };
        let online = crate::network::online(cx);
        let (sync_source, sync_account) = (self.source.clone(), self.account.clone());
        let account_line = h_flex()
            .flex_wrap()
            .items_baseline()
            .gap_x(px(10.))
            .gap_y(px(2.))
            .text_size(px(13.))
            .text_color(palette.text_muted)
            .child(format!("@{}", self.handle()))
            .when(games > 0., |line| {
                line.child(widgets::format_counted(games, "game", "games"))
                    .child(
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
                    .child(widgets::heading(self.source.heading(), &palette).text_size(px(16.)))
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
                    .on_click(move |_, window, cx| sync_source.sync(&sync_account, window, cx)),
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
                                    widgets::format_count(played),
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
        let (source, account_name) = (self.source.clone(), self.account.clone());
        let all_source = source.clone();
        let all_account = account_name.clone();
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
                            crate::widgets::text_button("home-lichess-all")
                                .child(
                                    h_flex()
                                        .gap(px(4.))
                                        .child("View all")
                                        .child(icon("arrow-right").size(px(14.))),
                                )
                                .on_click(move |_, window, cx| match &all_source {
                                    Source::Lichess => {
                                        crate::shell::open_history_lichess(window, cx)
                                    }
                                    Source::Chesscom(_) => {
                                        crate::shell::open_history_account(&all_account, window, cx)
                                    }
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
                                        source.sync(&account_name, window, cx)
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

    use super::{LichessActivity, local_profile, shown_perfs};

    /// A Chess.com player's cards: each speed's latest rating, its games, most played first.
    #[test]
    fn games_kept_here_make_the_cards() {
        let history = json!([
            { "name": "rapid", "points": [[2020, 0, 11, 1480], [2020, 0, 12, 1500]] },
            { "name": "blitz", "points": [[2020, 0, 11, 1300]] },
        ]);
        let report = json!({ "bySpeed": [
            { "speed": "rapid", "record": { "total": 2 } },
            { "speed": "blitz", "record": { "total": 5 } },
            { "speed": "bullet", "record": { "total": 1 } },
        ] });
        let profile = local_profile(&history, &report);
        assert_eq!(
            profile["perfs"],
            json!({
                "bullet": { "rating": null, "games": 1 },
                "blitz": { "rating": 1300, "games": 5 },
                "rapid": { "rating": 1500, "games": 2 },
            })
        );
        let keys: Vec<_> = shown_perfs(&profile)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(keys, ["blitz", "rapid", "bullet"]);
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
