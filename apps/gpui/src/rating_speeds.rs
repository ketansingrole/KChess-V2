//! A player's rating history in every speed at once (`RatingHistoryChart.vue`, on Players): a
//! line per speed in its own colour, chips that show or hide each with its current rating, and a
//! range. Lichess's history comes first; speeds it has no points for are filled from games.

use std::collections::HashSet;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::chart::{ChartDate, LineChart, Series};
use crate::game::rules;
use crate::rating_history::{
    DAY_MS, RATING_PAD, Retry, YEARS_SPAN_MS, array_or_empty, has_points, point_of,
};
use crate::theme::Palette;
use crate::widgets;
use crate::widgets::KeyboardFocus as _;

/// Colour follows the speed, never its rank: each keeps its slot when others are hidden. One
/// variant, the most played, takes the last slot.
const FIXED_SLOT_KEYS: [&str; 7] = [
    "bullet",
    "blitz",
    "rapid",
    "classical",
    "correspondence",
    "ultraBullet",
    "chess960",
];
const VARIANT_SLOT: usize = 8;
/// The validated categorical palette, light and dark, by slot from 1.
const LIGHT: [u32; 8] = [
    0x2a78d6, 0xeb6834, 0x1baf7a, 0xeda100, 0xe87ba4, 0x008300, 0x4a3aa7, 0xe34948,
];
const DARK: [u32; 8] = [
    0x3987e5, 0xd95926, 0x199e70, 0xc98500, 0xd55181, 0x008300, 0x9085e9, 0xe66767,
];
const RANGES: [(&str, f64); 4] = [("1M", 31.), ("3M", 92.), ("1Y", 366.), ("All", 0.)];

fn slot_color(slot: usize, dark: bool) -> Hsla {
    let colors = if dark { DARK } else { LIGHT };
    rgb(colors[(slot - 1).min(7)]).into()
}

/// One speed's line.
#[derive(Clone, Debug, PartialEq)]
pub struct Speed {
    /// Its perf key (`blitz`): what hiding and current ratings go by.
    pub key: String,
    pub name: String,
    pub slot: usize,
    /// UTC days and ratings, oldest first.
    pub points: Vec<(f64, f64)>,
}

/// The speeds in a rating history with points, in slot order; puzzles are left out.
pub fn speeds(history: &Value) -> Vec<Speed> {
    let with_points: Vec<Speed> = history
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let name = entry["name"].clone();
            if name.as_str().is_none_or(str::is_empty)
                || rules("isPuzzleHistory", std::slice::from_ref(&name)).ok() == Some(json!(true))
            {
                return None;
            }
            let points: Vec<(f64, f64)> = entry["points"]
                .as_array()?
                .iter()
                .filter_map(point_of)
                .collect();
            if points.is_empty() {
                return None;
            }
            let text = |method: &str| {
                rules(method, std::slice::from_ref(&name))
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default()
            };
            Some(Speed {
                key: text("normalizeRatingKey"),
                name: text("ratingDisplayName"),
                slot: 0,
                points,
            })
        })
        .collect();
    let mut chosen: Vec<Speed> = with_points
        .iter()
        .filter_map(|speed| {
            let index = FIXED_SLOT_KEYS.iter().position(|key| *key == speed.key)?;
            Some(Speed {
                slot: index + 1,
                ..speed.clone()
            })
        })
        .collect();
    // More than eight lines cannot stay apart; the most played variant gets the last slot.
    if let Some(variant) = with_points
        .iter()
        .filter(|speed| !FIXED_SLOT_KEYS.contains(&speed.key.as_str()))
        .max_by_key(|speed| speed.points.len())
    {
        chosen.push(Speed {
            slot: VARIANT_SLOT,
            ..variant.clone()
        });
    }
    chosen.sort_by_key(|speed| speed.slot);
    chosen
}

/// One row per day any shown speed changed, each speed with the rating it had then (a rating
/// holds until the next rated game; before its first game a speed has none). From `since` on,
/// with the ratings at `since` first so the lines begin at the left edge; 0 for all of them.
pub fn rows(shown: &[&Speed], since: f64) -> Vec<(f64, Vec<Option<f64>>)> {
    let mut dates: Vec<f64> = shown
        .iter()
        .flat_map(|speed| speed.points.iter().map(|(date, _)| *date))
        .collect();
    dates.sort_by(f64::total_cmp);
    dates.dedup();
    let mut cursors = vec![0; shown.len()];
    let mut last: Vec<Option<f64>> = vec![None; shown.len()];
    let all: Vec<(f64, Vec<Option<f64>>)> = dates
        .into_iter()
        .map(|date| {
            for (index, speed) in shown.iter().enumerate() {
                while let Some((at, rating)) = speed.points.get(cursors[index]) {
                    if *at > date {
                        break;
                    }
                    last[index] = Some(*rating);
                    cursors[index] += 1;
                }
            }
            (date, last.clone())
        })
        .collect();
    if since <= 0. {
        return all;
    }
    let before = all.iter().rfind(|(date, _)| *date < since).cloned();
    let inside = all.into_iter().filter(|(date, _)| *date >= since);
    before
        .map(|(_, values)| (since, values))
        .into_iter()
        .chain(inside)
        .collect()
}

pub struct RatingSpeeds {
    official: Value,
    fallback: Value,
    /// The player's ratings now by perf key (`perfs`): `rating`, `prov`, `prog`.
    current: Value,
    hidden: HashSet<String>,
    range: &'static str,
    chart: Entity<LineChart>,
    /// Said under a chart drawn from games.
    games_hint: SharedString,
    /// Said, with a way to ask again, when Lichess sent nothing to chart.
    withheld: Option<(SharedString, Retry)>,
}

impl RatingSpeeds {
    pub fn new(games_hint: impl Into<SharedString>, cx: &mut Context<Self>) -> RatingSpeeds {
        RatingSpeeds {
            official: Value::Null,
            fallback: Value::Null,
            current: Value::Null,
            hidden: HashSet::new(),
            range: "All",
            chart: cx.new(|_| LineChart::new(px(260.), |rating| format!("{}", rating.round()))),
            games_hint: games_hint.into(),
            withheld: None,
        }
    }

    /// Lichess's history and the one from games (either may be `null` while unknown). Another
    /// player's history shows every speed again.
    pub fn set(&mut self, official: Value, fallback: Value, cx: &mut Context<Self>) {
        self.official = official;
        self.fallback = fallback;
        self.hidden.clear();
        self.draw(cx);
    }

    pub fn set_current(&mut self, current: Value, cx: &mut Context<Self>) {
        self.current = current;
        cx.notify();
    }

    pub fn set_withheld(
        &mut self,
        withheld: Option<(SharedString, Retry)>,
        cx: &mut Context<Self>,
    ) {
        self.withheld = withheld;
        cx.notify();
    }

    /// `mergeRatingHistories`: Lichess's own wins; games fill speeds it has no points for.
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

    fn since(&self) -> f64 {
        let days = RANGES
            .iter()
            .find(|(label, _)| *label == self.range)
            .map_or(0., |(_, days)| *days);
        if days == 0. {
            0.
        } else {
            chrono::Utc::now().timestamp_millis() as f64 - days * DAY_MS
        }
    }

    fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.hidden.remove(&key) {
            self.hidden.insert(key);
        }
        self.draw(cx);
    }

    fn choose_range(&mut self, range: &'static str, cx: &mut Context<Self>) {
        self.range = range;
        self.draw(cx);
    }

    fn draw(&mut self, cx: &mut Context<Self>) {
        let all = speeds(&self.history());
        let shown: Vec<&Speed> = all
            .iter()
            .filter(|speed| !self.hidden.contains(&speed.key))
            .collect();
        let rows = rows(&shown, self.since());
        let (first, last) = (
            rows.first().map_or(0., |row| row.0),
            rows.last().map_or(0., |row| row.0),
        );
        let years = last - first > YEARS_SPAN_MS;
        let format = |millis: f64, pattern: &str| {
            chrono::DateTime::from_timestamp_millis(millis as i64)
                .map(|date| date.format(pattern).to_string())
                .unwrap_or_default()
        };
        let dates = rows
            .iter()
            .map(|(date, _)| ChartDate {
                short: format(*date, if years { "%b %y" } else { "%b %-d" }).into(),
                long: format(*date, "%b %-d, %Y").into(),
            })
            .collect();
        let dark = cx.global::<Palette>().dark;
        let series = shown
            .iter()
            .enumerate()
            .map(|(index, speed)| Series {
                name: speed.name.clone().into(),
                color: Some(slot_color(speed.slot, dark)),
                values: rows.iter().map(|(_, values)| values[index]).collect(),
                area: false,
            })
            .collect();
        let values = rows.iter().flat_map(|(_, values)| values.iter().flatten());
        let (low, high) = values.fold((f64::MAX, f64::MIN), |(low, high), rating| {
            (low.min(*rating), high.max(*rating))
        });
        let domain = if low > high {
            (0., 1.)
        } else {
            (low - RATING_PAD, high + RATING_PAD)
        };
        let summary = format!(
            "Rating history: {}",
            shown
                .iter()
                .map(|speed| {
                    let last = speed.points.last().map(|point| point.1);
                    format!(
                        "{} {}",
                        speed.name,
                        last.map_or(String::new(), |r| r.to_string())
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        );
        self.chart.update(cx, |chart, cx| {
            chart.set_series(dates, series, domain, summary, cx)
        });
        cx.notify();
    }

    /// A speed's chip: its colour (an outline while hidden), name, rating now and its change.
    fn chip(&self, speed: &Speed, cx: &mut Context<Self>, palette: &Palette) -> impl IntoElement {
        let hidden = self.hidden.contains(&speed.key);
        let current = &self.current[&speed.key];
        let rating = current["rating"]
            .as_f64()
            .or(speed.points.last().map(|point| point.1));
        let provisional = current["prov"] == json!(true);
        let progress = current["prog"].as_f64().filter(|change| *change != 0.);
        let key = speed.key.clone();
        let color = slot_color(speed.slot, palette.dark);
        let action = if hidden { "Show" } else { "Hide" };
        h_flex()
            .id(SharedString::from(format!("speed-{}", speed.key)))
            .gap(px(6.))
            .px(px(10.))
            .py(px(4.))
            .rounded_full()
            .border_1()
            .border_color(palette.border)
            .bg(palette.bg)
            .text_size(px(13.))
            .cursor_pointer()
            .when(hidden, |chip| chip.opacity(0.5))
            .hover(|chip| chip.border_color(palette.border_accented))
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(speed.name.clone())
            .aria_toggled(if hidden {
                Toggled::False
            } else {
                Toggled::True
            })
            .tooltip({
                let text = SharedString::from(format!("{action} {}", speed.name));
                move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
            .child(div().size(px(10.)).rounded(px(3.)).map(|swatch| {
                if hidden {
                    swatch.border_1().border_color(palette.dimmed)
                } else {
                    swatch.bg(color)
                }
            }))
            .child(speed.name.clone())
            .children(rating.map(|rating| {
                div().font_weight(FontWeight::BOLD).child(format!(
                    "{}{}",
                    rating.round(),
                    if provisional { "?" } else { "" }
                ))
            }))
            .children(progress.map(|change| {
                div()
                    .text_size(px(12.))
                    .text_color(if change > 0. {
                        palette.success
                    } else {
                        palette.error
                    })
                    .child(if change > 0. {
                        format!("+{change}")
                    } else {
                        format!("{change}")
                    })
            }))
    }
}

impl Render for RatingSpeeds {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let history = self.history();
        let all = speeds(&history);
        let shown = all
            .iter()
            .filter(|speed| !self.hidden.contains(&speed.key))
            .count();
        let enough = rows(
            &all.iter()
                .filter(|speed| !self.hidden.contains(&speed.key))
                .collect::<Vec<_>>(),
            self.since(),
        )
        .len()
            >= 2;
        let from_games = !has_points(&self.official) && has_points(&history);
        let chips: Vec<AnyElement> = all
            .iter()
            .map(|speed| self.chip(speed, cx, &palette).into_any_element())
            .collect();
        let ranges = h_flex().children(RANGES.iter().map(|(label, _)| {
            let chosen = self.range == *label;
            Button::new(SharedString::from(format!("speeds-range-{label}")))
                .xsmall()
                .label(*label)
                .map(|button| {
                    if chosen {
                        button.primary()
                    } else {
                        button.outline()
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| this.choose_range(label, cx)))
        }));
        let empty = |text: &'static str| {
            div()
                .h(px(260.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(palette.text_muted)
                .child(text)
        };
        v_flex()
            .gap(px(12.))
            .child(widgets::heading("Rating history", &palette))
            .child(
                h_flex()
                    .flex_wrap()
                    .items_start()
                    .justify_between()
                    .gap(px(10.))
                    .child(h_flex().flex_1().flex_wrap().gap(px(6.)).children(chips))
                    .child(ranges),
            )
            .map(|card| {
                if all.is_empty() {
                    let (text, retry) = match &self.withheld {
                        Some((text, retry)) => (text.clone(), Some(retry.clone())),
                        None => ("No rated games yet.".into(), None),
                    };
                    card.child(
                        h_flex()
                            .flex_wrap()
                            .gap(px(6.))
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(text)
                            .children(retry.map(|retry| {
                                Button::new("rating-speeds-retry")
                                    .text()
                                    .xsmall()
                                    .icon(icon("rotate-cw"))
                                    .label("Retry")
                                    .disabled(!crate::network::online(cx))
                                    .on_click(move |_, window, cx| retry(window, cx))
                            })),
                    )
                } else if shown == 0 {
                    card.child(empty("Pick a speed to show."))
                } else if !enough {
                    card.child(empty("Not enough rated games in this range."))
                } else {
                    card.child(self.chart.clone()).when(from_games, |card| {
                        card.child(widgets::hint(self.games_hint.clone(), &palette))
                    })
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{Speed, rows, speeds};
    use serde_json::json;

    fn speed(key: &str, points: &[(f64, f64)]) -> Speed {
        Speed {
            key: key.into(),
            name: key.into(),
            slot: 1,
            points: points.to_vec(),
        }
    }

    #[test]
    fn colours_follow_the_speed_and_puzzles_stay_out() {
        let history = json!([
            { "name": "Rapid", "points": [[2026, 0, 1, 1500]] },
            { "name": "Bullet", "points": [[2026, 0, 2, 1400]] },
            { "name": "Puzzles", "points": [[2026, 0, 2, 2000]] },
            { "name": "Atomic", "points": [[2026, 0, 3, 1300]] },
            { "name": "Horde", "points": [[2026, 0, 3, 1200], [2026, 0, 4, 1210]] },
            { "name": "Classical", "points": [] },
        ]);
        let slots: Vec<(String, usize)> = speeds(&history)
            .into_iter()
            .map(|speed| (speed.key, speed.slot))
            .collect();
        assert_eq!(
            slots,
            vec![
                ("bullet".into(), 1),
                ("rapid".into(), 3),
                ("horde".into(), 8)
            ]
        );
    }

    #[test]
    fn a_rating_holds_until_the_next_game_and_ranges_start_at_their_edge() {
        let blitz = speed("blitz", &[(1., 1500.), (3., 1510.)]);
        let rapid = speed("rapid", &[(2., 1600.)]);
        let shown = [&blitz, &rapid];
        assert_eq!(
            rows(&shown, 0.),
            vec![
                (1., vec![Some(1500.), None]),
                (2., vec![Some(1500.), Some(1600.)]),
                (3., vec![Some(1510.), Some(1600.)]),
            ]
        );
        assert_eq!(
            rows(&shown, 2.5),
            vec![
                (2.5, vec![Some(1500.), Some(1600.)]),
                (3., vec![Some(1510.), Some(1600.)]),
            ]
        );
    }
}
