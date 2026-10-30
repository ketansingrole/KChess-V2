//! The arena schedule: one row per series, a bar per arena coloured by
//! kind, the hours along the top and a line for now, scrolled to now when it opens.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{TournamentsPage, control};
use crate::assets::icon;
use crate::theme::{Palette, radii};
use crate::widgets;
use crate::widgets::KeyboardFocus as _;

const MINUTE: f64 = 60_000.;
/// About this much of the schedule fits across the box, so the scale follows the window.
const VISIBLE_SPAN: f64 = 3. * 60.;
const MIN_PX_PER_MINUTE: f64 = 9.;
const MAX_PX_PER_MINUTE: f64 = 14.;
const BEFORE_NOW: f64 = 60. * MINUTE;
const MIN_AHEAD: f64 = 3. * 60. * MINUTE;
const MAX_AHEAD: f64 = 8. * 60. * MINUTE;
const AXIS: f32 = 30.;
const ROW: f32 = 46.;
const ROW_GAP: f32 = 8.;

const SPEEDS: [&str; 5] = ["bullet", "blitz", "rapid", "classical", "ultraBullet"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Standard,
    Special,
    Capped,
    Variant,
}

const CATEGORIES: [(Category, &str, u32); 4] = [
    (Category::Standard, "Hourly & player-made", 0x3a8a30),
    (Category::Special, "Special events", 0x1a6fb8),
    (Category::Capped, "Rating-limited", 0x6152e0),
    (Category::Variant, "Variants", 0x7f5232),
];

pub fn category_of(arena: &Value) -> Category {
    if arena["maxRating"].as_f64().is_some() {
        return Category::Capped;
    }
    if let Some(perf) = arena["perf"].as_str()
        && !SPEEDS.contains(&perf)
    {
        return Category::Variant;
    }
    match arena["freq"].as_str() {
        Some(
            "eastern" | "daily" | "weekly" | "weekend" | "monthly" | "yearly" | "shield"
            | "marathon" | "unique",
        ) => Category::Special,
        _ => Category::Standard,
    }
}

/// Speeds first (as Lichess orders them), then rating-limited events, then variants.
fn group_of(arena: &Value) -> u8 {
    match category_of(arena) {
        Category::Capped => 1,
        Category::Variant => 2,
        _ => 0,
    }
}

fn perf_order(arena: &Value) -> usize {
    arena["perf"]
        .as_str()
        .and_then(|perf| SPEEDS.iter().position(|speed| *speed == perf))
        .unwrap_or(10)
}

fn start_of(arena: &Value) -> f64 {
    arena["startsAt"].as_f64().unwrap_or_default()
}

fn end_of(arena: &Value) -> f64 {
    arena["finishesAt"]
        .as_f64()
        .unwrap_or_else(|| start_of(arena) + arena["minutes"].as_f64().unwrap_or(60.) * MINUTE)
}

/// Second-level differences do not shift bars or split lanes, as on Lichess.
fn minute_of(time: f64) -> f64 {
    (time / MINUTE).floor() * MINUTE
}

fn series_of(arena: &Value) -> String {
    format!(
        "{}:{}:{}",
        arena["perf"].as_str().unwrap_or_default(),
        arena["maxRating"].as_f64().unwrap_or_default(),
        arena["freq"].as_str().unwrap_or_default()
    )
}

/// The times the schedule spans: an hour before now to the last arena (3 to 8 hours ahead),
/// on ten-minute marks.
pub fn range(arenas: &[Value], now: f64) -> (f64, f64) {
    let ten = 10. * MINUTE;
    let start = ((now - BEFORE_NOW) / ten).floor() * ten;
    let latest = arenas.iter().map(end_of).fold(now + MIN_AHEAD, f64::max);
    let end = latest.min(now + MAX_AHEAD);
    (start, (end / ten).ceil() * ten)
}

/// Packs arenas into rows the way Lichess does: by group, in start order, each on the row that
/// last held the same series when it fits, otherwise on the first row with room.
pub fn rows(arenas: &[Value], (start, end): (f64, f64)) -> Vec<Vec<Value>> {
    let mut visible: Vec<&Value> = arenas
        .iter()
        .filter(|arena| end_of(arena) > start && start_of(arena) < end)
        .collect();
    visible.sort_by(|a, b| {
        group_of(a)
            .cmp(&group_of(b))
            .then(
                minute_of(start_of(a))
                    .max(start)
                    .total_cmp(&minute_of(start_of(b)).max(start)),
            )
            .then(perf_order(a).cmp(&perf_order(b)))
            .then(
                a["maxRating"]
                    .as_f64()
                    .unwrap_or_default()
                    .total_cmp(&b["maxRating"].as_f64().unwrap_or_default()),
            )
    });
    struct Lane {
        group: u8,
        end: f64,
        series: String,
        items: Vec<Value>,
    }
    let mut lanes: Vec<Lane> = Vec::new();
    for arena in visible {
        let group = group_of(arena);
        let begins = minute_of(start_of(arena)).max(start);
        let series = series_of(arena);
        let free = |lane: &&mut Lane| lane.group == group && lane.end <= begins;
        let index = lanes
            .iter_mut()
            .position(|lane| free(&lane) && lane.series == series)
            .or_else(|| lanes.iter_mut().position(|lane| free(&lane)));
        match index {
            Some(index) => {
                let lane = &mut lanes[index];
                lane.items.push(arena.clone());
                lane.end = minute_of(end_of(arena));
                lane.series = series;
            }
            None => lanes.push(Lane {
                group,
                end: minute_of(end_of(arena)),
                series,
                items: vec![arena.clone()],
            }),
        }
    }
    lanes.into_iter().map(|lane| lane.items).collect()
}

/// The colour key, beside the page's controls.
pub fn legend(palette: &Palette) -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .gap(px(16.))
        .text_size(px(12.))
        .text_color(palette.text_muted)
        .children(CATEGORIES.iter().map(|(_, label, color)| {
            h_flex()
                .gap(px(6.))
                .child(div().size(px(10.)).rounded(radii::XS).bg(rgb(*color)))
                .child(*label)
        }))
}

fn perf_icon(arena: &Value) -> gpui_kit::component::Icon {
    if arena["freq"] == json!("shield") {
        return icon("shield");
    }
    match arena["perf"].as_str() {
        Some(perf) => crate::pages::watch::channel_icon(perf),
        None => icon("trophy"),
    }
}

/// "Hourly Blitz Arena, 3+0 rated, 120 players, running".
fn describe(arena: &Value) -> String {
    use chrono::TimeZone as _;
    let state = match arena["status"].as_str() {
        Some("finished") => "finished".to_string(),
        Some("started") => "running".to_string(),
        _ => format!(
            "starts {}",
            chrono::Local
                .timestamp_millis_opt(start_of(arena) as i64)
                .single()
                .map(|time| time.format("%H:%M").to_string())
                .unwrap_or_default()
        ),
    };
    format!(
        "{}, {} {}, {} players, {state}{}",
        arena["name"].as_str().unwrap_or_default(),
        control(&arena["clock"]),
        if arena["rated"] == json!(true) {
            "rated"
        } else {
            "casual"
        },
        arena["nbPlayers"].as_f64().unwrap_or_default(),
        if arena["playable"] == json!(true) {
            ""
        } else {
            ", watch only in KChess"
        }
    )
}

impl TournamentsPage {
    pub(super) fn schedule_view(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let mut body = v_flex().gap(px(12.));
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error.clone()),
            );
        }
        let Some(list) = &self.list else {
            return body
                .child(widgets::hint("Loading…", palette))
                .into_any_element();
        };
        let arenas: Vec<Value> = list["arenas"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|arena| !self.playable_only || arena["playable"] == json!(true))
            .cloned()
            .collect();
        if arenas.is_empty() {
            return body
                .child(widgets::hint("No arenas to show right now.", palette))
                .into_any_element();
        }
        let now = chrono::Utc::now().timestamp_millis() as f64;
        let (start, end) = range(&arenas, now);
        let box_width = f64::from(self.schedule_area.size().width.as_f32());
        let per_minute = (box_width / VISIBLE_SPAN).clamp(MIN_PX_PER_MINUTE, MAX_PX_PER_MINUTE);
        let x = move |time: f64| ((minute_of(time) - start) / MINUTE * per_minute) as f32;
        let width = x(end);
        let lanes = rows(&arenas, (start, end));
        let account = Self::account(cx);
        // Open on now, once the box's width (and so the scale) is known.
        if !self.scrolled_to_now && box_width > 0. {
            self.scrolled_to_now = true;
            let offset = (x(now) - 30. * per_minute as f32).max(0.);
            self.schedule.set_offset(point(px(-offset), px(0.)));
        }
        let mut ticks = Vec::new();
        let mut time = start;
        while time <= end {
            use chrono::{TimeZone as _, Timelike as _};
            let local = chrono::Local.timestamp_millis_opt(time as i64).single();
            let hour = local.is_some_and(|local| local.minute() == 0);
            let label = local
                .map(|local| {
                    if hour {
                        local.format("%-I:%M %p").to_string()
                    } else {
                        format!(":{:02}", local.minute())
                    }
                })
                .unwrap_or_default();
            ticks.push((x(time), hour, label));
            time += 10. * MINUTE;
        }
        let height = AXIS + 8. + lanes.len() as f32 * (ROW + ROW_GAP) + 10.;
        // How far the schedule is scrolled: a bar that began before it keeps its name in view
        // (`.timeline-label { position: sticky; left: 10px }`).
        let scrolled = f32::from(-self.schedule.offset().x).max(0.);
        let mut index = 0usize;
        let bars: Vec<AnyElement> = lanes
            .iter()
            .enumerate()
            .flat_map(|(row, lane)| {
                lane.iter()
                    .map(|arena| {
                        index += 1;
                        let left = x(start_of(arena)).max(0.);
                        let right = x(end_of(arena).min(end));
                        let id = arena["id"].as_str().unwrap_or_default().to_string();
                        let color = CATEGORIES
                            .iter()
                            .find(|(category, _, _)| *category == category_of(arena))
                            .map_or(0x3a8a30, |(_, _, color)| *color);
                        let joined = self.is_joined("arena", &id, account.as_deref());
                        let finished = arena["status"] == json!("finished");
                        let players = arena["nbPlayers"].as_f64().unwrap_or_default();
                        let sub = format!(
                            "{} {}",
                            control(&arena["clock"]),
                            if arena["rated"] == json!(true) {
                                "Rated"
                            } else {
                                "Casual"
                            }
                        );
                        let pinned = (scrolled - left).max(0.).min((right - left - 40.).max(0.));
                        h_flex()
                            .id(("arena", index))
                            .role(Role::Button)
                            .keyboard_focus(palette.primary)
                            .aria_label(describe(arena))
                            .absolute()
                            .left(px(left))
                            .top(px(AXIS + 8. + row as f32 * (ROW + ROW_GAP)))
                            .w(px((right - left).max(1.)))
                            .h(px(ROW))
                            .pl(px(10. + pinned))
                            .pr(px(10.))
                            .gap(px(8.))
                            .overflow_hidden()
                            .rounded(radii::SM)
                            .bg(rgb(color))
                            .text_color(rgb(0xffffff))
                            .when(finished, |bar| bar.opacity(0.42))
                            .cursor_pointer()
                            .hover(|bar| bar.opacity(0.9))
                            .tooltip({
                                let text = SharedString::from(describe(arena));
                                move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(text.clone())
                                        .build(window, cx)
                                }
                            })
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.show_event("arena", &id, cx)
                                }),
                            )
                            .child(perf_icon(arena).size(px(22.)).flex_none())
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(
                                                arena["name"]
                                                    .as_str()
                                                    .unwrap_or_default()
                                                    .to_string(),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap(px(4.))
                                            .text_size(px(13.))
                                            .opacity(0.9)
                                            .child(div().truncate().child(sub))
                                            // "· 👤 23": the count after its icon.
                                            .when(players > 0., |line| {
                                                line.child("·")
                                                    .child(icon("user").size(px(12.)))
                                                    .child(format!("{players}"))
                                            })
                                            .when(joined, |line| {
                                                line.child(icon("check").size(px(12.)))
                                            }),
                                    ),
                            )
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let canvas = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(width))
                    .h(px(AXIS))
                    .border_b_1()
                    .border_color(palette.border)
                    .children(ticks.into_iter().map(|(at, hour, label)| {
                        div()
                            .absolute()
                            .left(px(at))
                            .top_0()
                            .h(px(height))
                            .pl(px(6.))
                            .pt(px(7.))
                            .border_l_1()
                            .border_color(if hour {
                                palette.border_accented
                            } else {
                                palette.border
                            })
                            .text_size(px(12.))
                            .text_color(if hour {
                                palette.highlighted
                            } else {
                                palette.dimmed
                            })
                            .when(hour, |tick| tick.font_weight(FontWeight::SEMIBOLD))
                            .child(label)
                    })),
            )
            .children(bars)
            .child(
                div()
                    .absolute()
                    .left(px(x(now)))
                    .top_0()
                    .w(px(2.))
                    .h(px(height))
                    .bg(palette.error),
            );
        body.child(
            div().relative().child(self.schedule_area.probe(cx)).child(
                div()
                    .id("arena-schedule")
                    .track_scroll(&self.schedule)
                    .overflow_x_scroll()
                    // A sideways scroll here moves the schedule, not between pages.
                    .on_scroll_wheel(cx.listener(|_, event: &ScrollWheelEvent, _, cx| {
                        let delta = event.delta.pixel_delta(px(16.));
                        if delta.x.abs() > delta.y.abs() {
                            cx.stop_propagation();
                        }
                        // Redraw so the pinned names follow the scroll.
                        cx.notify();
                    }))
                    .rounded(radii::LG)
                    .border_1()
                    .border_color(palette.border)
                    .bg(palette.bg)
                    .child(canvas),
            ),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Category, category_of, range, rows};
    use serde_json::json;

    const HOUR: f64 = 3_600_000.;

    #[test]
    fn arenas_are_coloured_by_kind() {
        assert_eq!(
            category_of(&json!({ "perf": "blitz", "freq": "hourly" })),
            Category::Standard
        );
        assert_eq!(
            category_of(&json!({ "perf": "blitz", "freq": "weekly" })),
            Category::Special
        );
        assert_eq!(
            category_of(&json!({ "perf": "blitz", "maxRating": 1700 })),
            Category::Capped
        );
        assert_eq!(category_of(&json!({ "perf": "atomic" })), Category::Variant);
    }

    #[test]
    fn a_series_keeps_its_row_and_overlaps_get_another() {
        let now = 100. * HOUR;
        let arena = |id: &str, perf: &str, start: f64| json!({ "id": id, "perf": perf, "startsAt": now + start * HOUR, "minutes": 60 });
        let arenas = [
            arena("b1", "blitz", 0.),
            arena("u1", "bullet", 0.),
            arena("b2", "blitz", 1.),
            arena("v1", "atomic", 0.),
        ];
        let lanes = rows(&arenas, range(&arenas, now));
        let ids: Vec<Vec<&str>> = lanes
            .iter()
            .map(|lane| lane.iter().map(|a| a["id"].as_str().unwrap()).collect())
            .collect();
        // Bullet comes before blitz at the same start; the next blitz follows on blitz's row;
        // variants get rows of their own after the speeds.
        assert_eq!(ids, vec![vec!["u1"], vec!["b1", "b2"], vec!["v1"]]);
    }

    #[test]
    fn the_schedule_spans_an_hour_back_to_at_least_three_ahead() {
        let now = 100. * HOUR + 7. * 60_000.;
        let (start, end) = range(&[], now);
        assert!(start <= now - HOUR && start > now - HOUR - 10. * 60_000.);
        assert!(end >= now + 3. * HOUR);
    }
}
