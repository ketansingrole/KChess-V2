//! A player's rating history by mode and range (Home's activity and Players): Lichess's own
//! history, with modes it has no points for filled from games.

use std::rc::Rc;

use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::chart::{ChartPoint, LineChart};
use crate::theme::Palette;
use crate::widgets::{self, Choice};
use kchess_contracts::types::RatingSeries;
use kchess_domain::records::ratings::merge_rating_series;
use kchess_ui::icon;

/// The chart's speeds, by perf key.
const MODES: [(&str, &str); 6] = [
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("correspondence", "Correspondence"),
    ("puzzle", "Puzzle"),
];
const RANGES: [&str; 6] = ["1M", "3M", "6M", "YTD", "1Y", "All"];

/// A small select's width for `label`: its text at 12px, padding and the chevron.
fn fit_select(label: &str) -> Pixels {
    px(label.chars().count() as f32 * 7. + 40.)
}
pub const DAY_MS: f64 = 86_400_000.;
/// Ratings are padded by this much above and below.
const RATING_PAD: f64 = 15.;
/// Longer than this, dates read as month and year.
const YEARS_SPAN_MS: f64 = 400. * DAY_MS;

/// A rating chart's date, short under the axis and long in its tooltip: with the year once the
/// chart spans `first` to `last` over a year.
pub fn chart_dates(first: f64, last: f64) -> impl Fn(f64) -> (SharedString, SharedString) {
    let years = last - first > YEARS_SPAN_MS;
    move |millis| {
        let format = |pattern: &str| crate::dates::utc_format(millis, pattern).unwrap_or_default();
        (
            format(if years { "%b %y" } else { "%b %-d" }).into(),
            format("%b %-d, %Y").into(),
        )
    }
}

/// The ratings' lowest and highest, the chart's range padded around them; 0 to 1 for none.
pub fn rating_domain(ratings: impl Iterator<Item = f64>) -> (f64, f64) {
    let (low, high) = ratings.fold((f64::MAX, f64::MIN), |(low, high), rating| {
        (low.min(rating), high.max(rating))
    });
    if low > high {
        (0., 1.)
    } else {
        (low - RATING_PAD, high + RATING_PAD)
    }
}

pub type Retry = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct RatingHistory {
    /// Lichess's rating history, and one rebuilt from games for modes it has none of.
    official: Vec<RatingSeries>,
    fallback: Vec<RatingSeries>,
    mode: String,
    /// The speed is chosen here, not by the cards beside the chart (Home's).
    mode_picker: bool,
    range: &'static str,
    chart: Entity<LineChart>,
    mode_select: Entity<SelectState<Vec<Choice>>>,
    range_select: Entity<SelectState<Vec<Choice>>>,
    /// Said under a chart drawn from games.
    games_hint: SharedString,
    _subscriptions: Vec<Subscription>,
}

/// `[year, month0, day, rating]` as a UTC time and a rating.
fn point_of(&[year, month0, day, rating]: &[i64; 4]) -> Option<(f64, f64)> {
    let date = jiff::civil::Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month0 + 1).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()?;
    Some((utc_midnight_millis(date)?, rating as f64))
}

/// The UTC midnight starting `date`, in milliseconds.
fn utc_midnight_millis(date: jiff::civil::Date) -> Option<f64> {
    let start = date.to_zoned_datetime(jiff::tz::TimeZone::UTC).ok()?;
    Some(start.timestamp().as_millisecond() as f64)
}

/// A series' points as UTC times and ratings.
pub fn series_points(series: &RatingSeries) -> Vec<(f64, f64)> {
    series.points.iter().filter_map(point_of).collect()
}

/// The points of one mode in a rating history.
fn points_of(history: &[RatingSeries], mode: &str) -> Vec<(f64, f64)> {
    history
        .iter()
        .find(|series| series.key() == mode)
        .map(series_points)
        .unwrap_or_default()
}

/// Whether a rating history has any points at all.
pub fn has_points(history: &[RatingSeries]) -> bool {
    history.iter().any(|series| !series.points.is_empty())
}

/// The earliest time a range shows; 0 for all of it.
fn range_start(range: &str, now: jiff::Timestamp) -> f64 {
    let days = match range {
        "1M" => 30.,
        "3M" => 90.,
        "6M" => 180.,
        "1Y" => 365.,
        "YTD" => {
            let year = now.to_zoned_datetime(jiff::tz::TimeZone::UTC).year();
            return jiff::civil::Date::new(year, 1, 1)
                .ok()
                .and_then(utc_midnight_millis)
                .unwrap_or(0.);
        }
        _ => return 0.,
    };
    now.as_millisecond() as f64 - days * DAY_MS
}

fn choices(items: impl IntoIterator<Item = (String, String)>) -> Vec<Choice> {
    items
        .into_iter()
        .map(|(value, label)| Choice::new(value, label))
        .collect()
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
            official: Vec::new(),
            fallback: Vec::new(),
            mode: "blitz".into(),
            mode_picker: true,
            range: "All",
            chart: cx.new(|_| LineChart::new(CHART_HEIGHT, |rating| format!("{}", rating.round()))),
            mode_select,
            range_select,
            games_hint: games_hint.into(),
            _subscriptions: subscriptions,
        }
    }

    /// The mode on the chart, as chosen elsewhere (Home's rating cards).
    pub fn show_mode(&mut self, mode: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode.to_string();
        self.mode_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(mode.to_string()), window, cx)
        });
        self.draw(cx);
    }

    /// Leave choosing the speed to the page: the heading names it instead.
    pub fn without_mode_picker(mut self) -> Self {
        self.mode_picker = false;
        self
    }

    /// What is said under a chart drawn from games.
    pub fn set_games_hint(&mut self, hint: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.games_hint = hint.into();
        cx.notify();
    }

    pub fn mode(&self) -> &str {
        &self.mode
    }

    /// Lichess's history and the one from games (either empty while unknown).
    pub fn set(
        &mut self,
        official: Vec<RatingSeries>,
        fallback: Vec<RatingSeries>,
        cx: &mut Context<Self>,
    ) {
        self.official = official;
        self.fallback = fallback;
        self.draw(cx);
    }

    /// Lichess's history (empty while unknown).
    pub fn set_official(&mut self, official: Vec<RatingSeries>, cx: &mut Context<Self>) {
        self.official = official;
        self.draw(cx);
    }

    /// The history from games, for the modes Lichess has no points for.
    pub fn set_fallback(&mut self, fallback: Vec<RatingSeries>, cx: &mut Context<Self>) {
        self.fallback = fallback;
        self.draw(cx);
    }

    /// Lichess's own wins; games fill modes it has no points for.
    fn history(&self) -> Vec<RatingSeries> {
        merge_rating_series(&self.official, &self.fallback)
    }

    /// The chosen mode's points come from games.
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
        let start = range_start(self.range, jiff::Timestamp::now());
        let points: Vec<(f64, f64)> = points_of(&history, &self.mode)
            .into_iter()
            .filter(|(date, _)| *date >= start)
            .collect();
        let dates = chart_dates(
            points.first().map_or(0., |point| point.0),
            points.last().map_or(0., |point| point.0),
        );
        let chart_points: Vec<ChartPoint> = points
            .iter()
            .map(|(date, rating)| {
                let (short, long) = dates(*date);
                ChartPoint {
                    value: *rating,
                    short,
                    long,
                }
            })
            .collect();
        let domain = rating_domain(points.iter().map(|point| point.1));
        let (low, high) = points
            .iter()
            .fold((f64::MAX, f64::MIN), |(low, high), (_, rating)| {
                (low.min(*rating), high.max(*rating))
            });
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
        let start = range_start(self.range, jiff::Timestamp::now());
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
        v_flex()
            .id("rating-history")
            .test_support()
            .gap(px(8.))
            .child(
                h_flex()
                    .justify_between()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        v_flex()
                            .child(widgets::heading(
                                if self.mode_picker {
                                    "Rating history".to_string()
                                } else {
                                    format!(
                                        "{} rating history",
                                        MODES
                                            .iter()
                                            .find(|(value, _)| *value == self.mode)
                                            .map_or("", |(_, label)| label)
                                    )
                                },
                                &palette,
                            ))
                            // The change's line is kept without points, so the heading's row
                            // stays one height as they come.
                            .child(
                                h_flex()
                                    .h(px(18.))
                                    .gap(px(4.))
                                    .text_size(px(12.))
                                    .text_color(palette.text_muted)
                                    .children(change.into_iter().flat_map(|change| {
                                        change_line(change, values[values.len() - 1], &palette)
                                    })),
                            ),
                    )
                    // Each only as wide as its choice and chevron.
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .when(self.mode_picker, |selects| {
                                selects.child(
                                    div()
                                        .w(fit_select(
                                            MODES
                                                .iter()
                                                .find(|(value, _)| *value == self.mode)
                                                .map_or("", |(_, label)| label),
                                        ))
                                        .child(
                                            Select::new(&self.mode_select)
                                                .small()
                                                .accessibility_label("Speed"),
                                        ),
                                )
                            })
                            .child(
                                div().w(fit_select(self.range)).child(
                                    Select::new(&self.range_select)
                                        .small()
                                        .accessibility_label("Period"),
                                ),
                            ),
                    ),
            )
            // The chart's place keeps its height without points: the card grew by the chart as
            // the history came, and shrank again for a mode or range with none.
            .child(
                div()
                    .id("rating-history-area")
                    .test_support()
                    .h(CHART_HEIGHT)
                    .flex()
                    .items_center()
                    .justify_center()
                    .map(|area| {
                        if values.len() >= 2 {
                            area.child(self.chart.clone())
                        } else {
                            area.child(widgets::empty_state(
                                icon("chart-line"),
                                "No rating points",
                                Some("Try another mode or date range".into()),
                                &palette,
                            ))
                        }
                    }),
            )
            .when(values.len() >= 2 && self.chart_from_games(), |card| {
                card.child(widgets::hint(self.games_hint.clone(), &palette))
            })
    }
}

/// The chart's height, which its place keeps without points too.
const CHART_HEIGHT: Pixels = px(240.);

/// "1712 now · +40 over range", the change colored by its sign.
fn change_line(change: f64, now: f64, palette: &Palette) -> [AnyElement; 3] {
    [
        div().child(format!("{now} now ·")).into_any_element(),
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
            })
            .into_any_element(),
        div().child("over range").into_any_element(),
    ]
}

#[cfg(test)]
mod tests {
    use super::{point_of, range_start};

    #[test]
    fn lichess_points_are_utc_days() {
        // Lichess counts months from 0.
        let (date, rating) = point_of(&[2026, 0, 5, 1712]).unwrap();
        assert_eq!(rating, 1712.);
        assert_eq!(
            jiff::Timestamp::from_millisecond(date as i64)
                .unwrap()
                .strftime("%Y-%m-%d")
                .to_string(),
            "2026-01-05"
        );
        assert!(point_of(&[2026, 12, 5, 1712]).is_none());
    }

    #[test]
    fn ranges_start_where_they_say() {
        let now: jiff::Timestamp = "2026-10-24T12:00:00Z".parse().unwrap();
        assert_eq!(range_start("All", now), 0.);
        let ytd = range_start("YTD", now);
        assert_eq!(
            jiff::Timestamp::from_millisecond(ytd as i64)
                .unwrap()
                .strftime("%m-%d")
                .to_string(),
            "01-01"
        );
        assert!(
            (now.as_millisecond() as f64 - range_start("1M", now) - 30. * 86_400_000.).abs() < 1.
        );
    }
}
