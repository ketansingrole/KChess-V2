//! A player's rating history by mode and range (Home's activity and Players): Lichess's own
//! history, with modes it has no points for filled from games (`mergeRatingHistories`).

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::chart::{ChartPoint, LineChart};
use crate::game::rules;
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

pub type Retry = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct RatingHistory {
    /// Lichess's rating history, and one rebuilt from games for modes it has none of.
    official: Value,
    fallback: Value,
    mode: String,
    range: &'static str,
    chart: Entity<LineChart>,
    mode_select: Entity<SelectState<Vec<Choice>>>,
    range_select: Entity<SelectState<Vec<Choice>>>,
    /// Said under a chart drawn from games.
    games_hint: SharedString,
    /// Said, with a way to ask again, when Lichess sent nothing to chart.
    withheld: Option<(SharedString, Retry)>,
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

/// Whether a rating history has any points at all.
pub fn has_points(history: &Value) -> bool {
    history.as_array().into_iter().flatten().any(|entry| {
        entry["points"]
            .as_array()
            .is_some_and(|points| !points.is_empty())
    })
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

fn array_or_empty(value: &Value) -> Value {
    if value.is_array() {
        value.clone()
    } else {
        json!([])
    }
}

impl RatingHistory {
    pub fn new(
        games_hint: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> RatingHistory {
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
        RatingHistory {
            official: Value::Null,
            fallback: Value::Null,
            mode: "blitz".into(),
            range: "All",
            chart: cx.new(|_| LineChart::new(px(240.), |rating| format!("{}", rating.round()))),
            mode_select,
            range_select,
            games_hint: games_hint.into(),
            withheld: None,
            _subscriptions: subscriptions,
        }
    }

    /// Lichess's history and the one from games (either may be `null` while unknown).
    pub fn set(&mut self, official: Value, fallback: Value, cx: &mut Context<Self>) {
        self.official = official;
        self.fallback = fallback;
        self.draw(cx);
    }

    pub fn set_official(&mut self, official: Value, cx: &mut Context<Self>) {
        self.official = official;
        self.draw(cx);
    }

    pub fn set_fallback(&mut self, fallback: Value, cx: &mut Context<Self>) {
        self.fallback = fallback;
        self.draw(cx);
    }

    /// What to say, and how to ask again, when there is nothing to chart though there should be.
    pub fn set_withheld(
        &mut self,
        withheld: Option<(SharedString, Retry)>,
        cx: &mut Context<Self>,
    ) {
        self.withheld = withheld;
        cx.notify();
    }

    /// `mergeRatingHistories`: Lichess's own wins; games fill modes it has no points for.
    fn history(&self) -> Value {
        rules(
            "mergeRatingHistories",
            &[
                array_or_empty(&self.official),
                array_or_empty(&self.fallback),
            ],
        )
        .unwrap_or(json!([]))
    }

    /// The chosen mode's points come from games (`chartFromGames`).
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

impl Render for RatingHistory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let values = self.values();
        let change = (values.len() >= 2).then(|| values[values.len() - 1] - values[0]);
        let empty_anywhere = !has_points(&self.history());
        v_flex()
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
                            .child(
                                div()
                                    .w(px(150.))
                                    .child(Select::new(&self.mode_select).small()),
                            )
                            .child(
                                div()
                                    .w(px(80.))
                                    .child(Select::new(&self.range_select).small()),
                            ),
                    ),
            )
            .when(values.len() >= 2, |card| card.child(self.chart.clone()))
            .when(values.len() >= 2 && self.chart_from_games(), |card| {
                card.child(widgets::hint(self.games_hint.clone(), &palette))
            })
            .when(values.len() < 2, |card| match &self.withheld {
                Some((text, retry)) if empty_anywhere => {
                    let retry = retry.clone();
                    card.child(
                        h_flex()
                            .gap(px(6.))
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(text.clone())
                            .child(
                                Button::new("rating-history-retry")
                                    .text()
                                    .xsmall()
                                    .icon(icon("rotate-cw"))
                                    .label("Retry")
                                    .on_click(move |_, window, cx| retry(window, cx)),
                            ),
                    )
                }
                _ => card.child(widgets::empty_state(
                    icon("chart-line"),
                    "No rating points",
                    Some("Try another mode or date range.".into()),
                    &palette,
                )),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{has_points, point_of, range_start};
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

    #[test]
    fn an_empty_history_has_no_points() {
        assert!(!has_points(&json!(null)));
        assert!(!has_points(&json!([{ "name": "Blitz", "points": [] }])));
        assert!(has_points(
            &json!([{ "name": "Blitz", "points": [[2026, 0, 1, 1500]] }])
        ));
    }
}
