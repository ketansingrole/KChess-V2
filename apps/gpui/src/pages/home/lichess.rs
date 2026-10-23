//! Home → Your Lichess activity (`pages/index.vue`, `stores/kchess/accountProfile.ts` and
//! `gameHistory.ts`): the account's games and ratings, its rating history by mode and range, and
//! its latest synced games. What was saved last time shows at once; Lichess's answer follows.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::chart::{ChartPoint, LineChart};
use crate::core::AppCore;
use crate::game::rules;
use crate::pages::history::lichess::game_row;
use crate::theme::Palette;
use crate::widgets::{self, Choice};

/// `chartModes`, by perf key.
const MODES: [(&str, &str); 6] = [
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("correspondence", "Correspondence"),
    ("puzzle", "Puzzle"),
];
const RANGES: [&str; 6] = ["1M", "3M", "6M", "YTD", "1Y", "All"];
const DAY_MS: f64 = 86_400_000.;
/// Ratings are padded by this much above and below (`ratingYDomain`).
const RATING_PAD: f64 = 15.;
/// Longer than this, dates read as month and year (`spansRatingYears`).
const YEARS_SPAN_MS: f64 = 400. * DAY_MS;
/// Synced games shown.
const RECENT: usize = 6;

pub struct LichessActivity {
    account: String,
    profile: Value,
    /// Lichess's rating history, and one rebuilt from synced games for modes it has none of.
    official: Value,
    fallback: Value,
    /// `gameLibraryOverview`: wins, losses and draws by account.
    overview: Value,
    recent: Vec<Value>,
    mode: String,
    range: &'static str,
    chart: Entity<LineChart>,
    mode_select: Entity<SelectState<Vec<Choice>>>,
    range_select: Entity<SelectState<Vec<Choice>>>,
    epoch: u64,
    _subscriptions: Vec<Subscription>,
}

/// `[year, month0, day, rating]` as a UTC time and a rating (`toRatingPoint`).
fn point_of(raw: &Value) -> Option<(f64, f64)> {
    let parts = raw.as_array()?;
    let number = |index: usize| parts.get(index)?.as_f64().filter(|n| n.is_finite());
    let date = chrono::NaiveDate::from_ymd_opt(
        number(0)? as i32,
        number(1)? as u32 + 1,
        number(2)? as u32,
    )?;
    let millis = date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis() as f64;
    Some((millis, number(3)?))
}

/// The points of one mode in a rating history.
fn points_of(history: &Value, mode: &str) -> Vec<(f64, f64)> {
    history
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| {
            rules("normalizeRatingKey", &[entry["name"].clone()]).ok() == Some(json!(mode))
        })
        .and_then(|entry| entry["points"].as_array())
        .map(|points| points.iter().filter_map(point_of).collect())
        .unwrap_or_default()
}

/// The earliest time a range shows; 0 for all of it.
fn range_start(range: &str, now: chrono::DateTime<chrono::Utc>) -> f64 {
    let days = match range {
        "1M" => 30.,
        "3M" => 90.,
        "6M" => 180.,
        "1Y" => 365.,
        "YTD" => {
            use chrono::Datelike as _;
            return chrono::NaiveDate::from_ymd_opt(now.year(), 1, 1)
                .and_then(|day| day.and_hms_opt(0, 0, 0))
                .map_or(0., |start| start.and_utc().timestamp_millis() as f64);
        }
        _ => return 0.,
    };
    now.timestamp_millis() as f64 - days * DAY_MS
}

fn choices(items: impl IntoIterator<Item = (String, String)>) -> Vec<Choice> {
    items
        .into_iter()
        .map(|(value, label)| Choice::new(value, label))
        .collect()
}

impl LichessActivity {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> LichessActivity {
        let mode_select = cx.new(|cx| {
            SelectState::new(
                choices(MODES.map(|(value, label)| (value.to_string(), label.to_string()))),
                None,
                window,
                cx,
            )
        });
        let range_select = cx.new(|cx| {
            SelectState::new(
                choices(RANGES.map(|range| (range.to_string(), range.to_string()))),
                None,
                window,
                cx,
            )
        });
        mode_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from("blitz"), window, cx)
        });
        range_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from("All"), window, cx)
        });
        let subscriptions = vec![
            cx.subscribe(
                &mode_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.mode = value.to_string();
                        this.draw(cx);
                    }
                },
            ),
            cx.subscribe(
                &range_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.range = RANGES
                            .iter()
                            .find(|range| **range == value.as_ref())
                            .copied()
                            .unwrap_or("All");
                        this.draw(cx);
                    }
                },
            ),
        ];
        LichessActivity {
            account: String::new(),
            profile: Value::Null,
            official: Value::Null,
            fallback: Value::Null,
            overview: Value::Null,
            recent: Vec::new(),
            mode: "blitz".into(),
            range: "All",
            chart: cx.new(|_| LineChart::new(px(240.), |rating| format!("{}", rating.round()))),
            mode_select,
            range_select,
            epoch: 0,
            _subscriptions: subscriptions,
        }
    }

    /// Show `account`, or read it again (`loadProfile` and the game history's account data).
    pub fn load(&mut self, account: &str, cx: &mut Context<Self>) {
        if self.account != account {
            self.account = account.to_string();
            self.profile = Value::Null;
            self.official = Value::Null;
            self.fallback = Value::Null;
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
                    this.official = saved["ratingHistory"].clone();
                }
                let (overview, recent, fallback) = local;
                if let Ok(overview) = overview {
                    this.overview = overview;
                }
                if let Ok(recent) = recent {
                    this.recent = recent["games"].as_array().cloned().unwrap_or_default();
                }
                if let Ok(fallback) = fallback {
                    this.fallback = fallback;
                }
                this.draw(cx);
            });
            let (profile, history) = (profile.await, history.await);
            let _ = this.update(cx, |this, cx| {
                if !current(this) {
                    return;
                }
                match (profile, history) {
                    (Ok(profile), Ok(history)) => {
                        this.profile = profile;
                        this.official = history;
                    }
                    (Err(cause), _) | (_, Err(cause)) => crate::log::write(
                        kchess_core::Level::Warn,
                        "account-profile",
                        &format!("loading profile failed: {}", cause.message),
                    ),
                }
                this.draw(cx);
            });
        })
        .detach();
    }

    /// `mergeRatingHistories`: Lichess's own wins; games fill modes it has no points for.
    fn history(&self) -> Value {
        rules(
            "mergeRatingHistories",
            &[
                if self.official.is_array() {
                    self.official.clone()
                } else {
                    json!([])
                },
                if self.fallback.is_array() {
                    self.fallback.clone()
                } else {
                    json!([])
                },
            ],
        )
        .unwrap_or(json!([]))
    }

    /// The chosen mode's points come from synced games (`chartFromGames`).
    fn chart_from_games(&self) -> bool {
        points_of(&self.official, &self.mode).is_empty()
            && !points_of(&self.history(), &self.mode).is_empty()
    }

    /// The chosen mode and range on the chart; a mode without points gives way to one with them.
    fn draw(&mut self, cx: &mut Context<Self>) {
        let history = self.history();
        if points_of(&history, &self.mode).is_empty()
            && let Some(mode) = MODES
                .iter()
                .map(|(mode, _)| *mode)
                .find(|mode| !points_of(&history, mode).is_empty())
        {
            self.mode = mode.to_string();
        }
        let start = range_start(self.range, chrono::Utc::now());
        let points: Vec<(f64, f64)> = points_of(&history, &self.mode)
            .into_iter()
            .filter(|(date, _)| *date >= start)
            .collect();
        let years = points.len() > 1
            && points.last().map(|p| p.0).unwrap_or_default()
                - points.first().map(|p| p.0).unwrap_or_default()
                > YEARS_SPAN_MS;
        let format = |millis: f64, pattern: &str| {
            chrono::DateTime::from_timestamp_millis(millis as i64)
                .map(|date| date.format(pattern).to_string())
                .unwrap_or_default()
        };
        let chart_points: Vec<ChartPoint> = points
            .iter()
            .map(|(date, rating)| ChartPoint {
                value: *rating,
                short: format(*date, if years { "%b %y" } else { "%b %-d" }).into(),
                long: format(*date, "%b %-d, %Y").into(),
            })
            .collect();
        let (low, high) = points
            .iter()
            .fold((f64::MAX, f64::MIN), |(low, high), (_, rating)| {
                (low.min(*rating), high.max(*rating))
            });
        let domain = if points.is_empty() {
            (0., 1.)
        } else {
            (low - RATING_PAD, high + RATING_PAD)
        };
        let label = MODES
            .iter()
            .find(|(mode, _)| *mode == self.mode)
            .map_or(self.mode.clone(), |(_, label)| label.to_string());
        let summary = format!(
            "{label} rating history from {} to {}",
            low.round(),
            high.round()
        );
        self.chart
            .update(cx, |chart, cx| chart.set(chart_points, domain, summary, cx));
        cx.notify();
    }

    fn values(&self) -> Vec<f64> {
        let start = range_start(self.range, chrono::Utc::now());
        points_of(&self.history(), &self.mode)
            .into_iter()
            .filter(|(date, _)| *date >= start)
            .map(|(_, rating)| rating)
            .collect()
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
        let record_value = h_flex()
            .gap(px(10.))
            .child(
                div()
                    .text_color(palette.win)
                    .child(format!("{}W", count("win"))),
            )
            .child(
                div()
                    .text_color(palette.loss)
                    .child(format!("{}L", count("loss"))),
            )
            .child(
                div()
                    .text_color(palette.text_muted)
                    .child(format!("{}D", count("draw"))),
            );
        let stats = h_flex()
            .flex_wrap()
            .gap(px(12.))
            .child(stat(
                "Games",
                widgets::format_count(games),
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
        let values = self.values();
        let change = (values.len() >= 2).then(|| values[values.len() - 1] - values[0]);
        let chart_card = widgets::card(&palette)
            .flex_1()
            .min_w(px(300.))
            .gap(px(10.))
            .child(
                h_flex()
                    .justify_between()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        v_flex()
                            .child(widgets::heading("Rating history", &palette))
                            .children(change.map(|change| {
                                h_flex()
                                    .gap(px(4.))
                                    .text_size(px(12.))
                                    .text_color(palette.text_muted)
                                    .child(format!("{} now ·", values[values.len() - 1]))
                                    .child(
                                        div()
                                            .text_color(if change >= 0. {
                                                palette.success
                                            } else {
                                                palette.error
                                            })
                                            .child(if change >= 0. {
                                                format!("+{change}")
                                            } else {
                                                format!("{change}")
                                            }),
                                    )
                                    .child("over range")
                            })),
                    )
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .child(div().w(px(150.)).child(Select::new(&self.mode_select).small()))
                            .child(div().w(px(80.)).child(Select::new(&self.range_select).small())),
                    ),
            )
            .when(values.len() >= 2, |card| card.child(self.chart.clone()))
            .when(values.len() >= 2 && self.chart_from_games(), |card| {
                card.child(widgets::hint(
                    "Built from your synced rated games. Lichess has no rating history for this account.",
                    &palette,
                ))
            })
            .when(values.len() < 2, |card| {
                card.child(widgets::empty_state(
                    icon("chart-line"),
                    "No rating points",
                    Some("Try another mode or date range.".into()),
                    &palette,
                ))
            });
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
                    false,
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

#[cfg(test)]
mod tests {
    use super::{point_of, range_start};
    use serde_json::json;

    #[test]
    fn lichess_points_are_utc_days() {
        // Lichess counts months from 0.
        let (date, rating) = point_of(&json!([2026, 0, 5, 1712])).unwrap();
        assert_eq!(rating, 1712.);
        assert_eq!(
            chrono::DateTime::from_timestamp_millis(date as i64)
                .unwrap()
                .format("%Y-%m-%d")
                .to_string(),
            "2026-01-05"
        );
        assert!(point_of(&json!([2026, 0, 5])).is_none());
    }

    #[test]
    fn ranges_start_where_they_say() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-24T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(range_start("All", now), 0.);
        let ytd = range_start("YTD", now);
        assert_eq!(
            chrono::DateTime::from_timestamp_millis(ytd as i64)
                .unwrap()
                .format("%m-%d")
                .to_string(),
            "01-01"
        );
        assert!(
            (now.timestamp_millis() as f64 - range_start("1M", now) - 30. * 86_400_000.).abs() < 1.
        );
    }
}
