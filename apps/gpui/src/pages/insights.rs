//! Lichess insights (`pages/insights.vue`): an account's synced games broken down by colour,
//! speed, opponent, length, day, time of day, ending and opening, filtered by speed, period and
//! rated or casual.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use crate::assets::icon;
use crate::core::AppCore;
use crate::pages::Page;
use crate::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::theme::Palette;
use crate::widgets::{self, Choice};

const SPEEDS: [(&str, &str); 6] = [
    ("all", "All speeds"),
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("correspondence", "Correspondence"),
];
const PERIODS: [(&str, &str); 4] = [
    ("all", "All time"),
    ("30", "Last 30 days"),
    ("90", "Last 90 days"),
    ("365", "Last year"),
];
const RATED: [(&str, &str); 3] = [
    ("all", "Rated and casual"),
    ("rated", "Rated only"),
    ("casual", "Casual only"),
];
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// `ENDINGS`: how games ended, in words.
fn ending(status: &str) -> &str {
    match status {
        "mate" => "Checkmate",
        "resign" => "Resignation",
        "outoftime" => "Time out",
        "timeout" => "Left the game",
        "draw" => "Draw",
        "stalemate" => "Stalemate",
        "aborted" => "Aborted",
        "noStart" => "Never started",
        "cheat" => "Cheat detected",
        "variantEnd" => "Variant win",
        other => other,
    }
}

/// Lichess's speed keys, as titles.
fn speed_name(key: &str) -> String {
    if key == "ultraBullet" {
        return "UltraBullet".into();
    }
    let mut chars = key.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn record(value: &Value) -> (f64, f64, f64) {
    let n = |key: &str| value[key].as_f64().unwrap_or_default();
    (n("win"), n("draw"), n("loss"))
}

/// Four-hour blocks read better than 24 thin rows.
fn by_time(hours: &Value) -> Vec<(String, (f64, f64, f64))> {
    (0..24)
        .step_by(4)
        .map(|start| {
            let mut sum = (0., 0., 0.);
            for hour in start..start + 4 {
                let (win, draw, loss) = record(&hours[hour]);
                sum = (sum.0 + win, sum.1 + draw, sum.2 + loss);
            }
            (format!("{start:02}:00–{:02}:00", start + 4), sum)
        })
        .collect()
}

/// The best and weakest openings of at least five games, by score.
fn best_and_weakest(openings: &[Value]) -> Option<(String, String)> {
    let score = |entry: &Value| {
        let (win, draw, _) = record(&entry["record"]);
        let total = entry["record"]["total"].as_f64().unwrap_or_default();
        (win + draw / 2.) / total
    };
    let mut enough: Vec<&Value> = openings
        .iter()
        .filter(|entry| entry["record"]["total"].as_f64().unwrap_or_default() >= 5.)
        .collect();
    enough.sort_by(|a, b| score(b).total_cmp(&score(a)));
    let (top, bottom) = (enough.first()?, enough.last()?);
    (!std::ptr::eq(*top, *bottom)).then(|| {
        (
            top["name"].as_str().unwrap_or_default().to_string(),
            bottom["name"].as_str().unwrap_or_default().to_string(),
        )
    })
}

fn choices(items: &[(&str, &str)]) -> Vec<Choice> {
    items
        .iter()
        .map(|(value, label)| Choice::new(*value, *label))
        .collect()
}

type Choices = Entity<SelectState<Vec<Choice>>>;

pub struct InsightsPage {
    /// The breakdowns' width at the last layout.
    area: Measured,
    account: String,
    account_select: Choices,
    speed: Choices,
    period: Choices,
    rated: Choices,
    report: Option<Value>,
    error: Option<SharedString>,
    loading: bool,
    epoch: u64,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for InsightsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl InsightsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> InsightsPage {
        let mut select = |items: Vec<Choice>| {
            let state = cx.new(|cx| SelectState::new(items, None, window, cx));
            state.update(cx, |state, cx| {
                state.set_selected_value(&SharedString::from("all"), window, cx)
            });
            state
        };
        let speed = select(choices(&SPEEDS));
        let period = select(choices(&PERIODS));
        let rated = select(choices(&RATED));
        let account_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let accounts = crate::accounts::handle(cx);
        let mut subscriptions: Vec<Subscription> = [&speed, &period, &rated]
            .into_iter()
            .map(|select| {
                cx.subscribe(select, |this, _, _: &SelectEvent<Vec<Choice>>, cx| {
                    this.load(cx)
                })
            })
            .collect();
        subscriptions.push(cx.subscribe(
            &account_select,
            |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(account)) = event {
                    this.account = account.to_string();
                    this.load(cx);
                }
            },
        ));
        subscriptions.push(cx.observe_in(&accounts, window, |this, _, window, cx| {
            this.sync_accounts(window, cx)
        }));
        let mut page = InsightsPage {
            area: Measured::default(),
            account: String::new(),
            account_select,
            speed,
            period,
            rated,
            report: None,
            error: None,
            loading: false,
            epoch: 0,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        page.sync_accounts(window, cx);
        page
    }

    fn names(cx: &App) -> Vec<String> {
        crate::accounts::handle(cx)
            .read(cx)
            .all()
            .iter()
            .filter_map(|account| account["username"].as_str().map(str::to_string))
            .collect()
    }

    /// The accounts offered; the active one first time, or the first.
    fn sync_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names = Self::names(cx);
        let items: Vec<Choice> = names
            .iter()
            .map(|name| Choice::new(name.clone(), format!("@{name}")))
            .collect();
        self.account_select
            .update(cx, |select, cx| select.set_items(items, window, cx));
        if !names.contains(&self.account) {
            self.account = crate::accounts::handle(cx)
                .read(cx)
                .active(cx)
                .or_else(|| names.first().cloned())
                .unwrap_or_default();
            let chosen = SharedString::from(self.account.clone());
            self.account_select.update(cx, |select, cx| {
                select.set_selected_value(&chosen, window, cx)
            });
            self.load(cx);
        }
        cx.notify();
    }

    fn chosen(select: &Choices, cx: &App) -> String {
        select
            .read(cx)
            .selected_value()
            .map_or("all".into(), |value| value.to_string())
    }

    /// `InsightsQuery`: unset filters are left out.
    fn query(&self, cx: &App) -> Value {
        let mut query = json!({ "account": self.account });
        let speed = Self::chosen(&self.speed, cx);
        if speed != "all" {
            query["speed"] = json!(speed);
        }
        match Self::chosen(&self.rated, cx).as_str() {
            "rated" => query["rated"] = json!(true),
            "casual" => query["rated"] = json!(false),
            _ => {}
        }
        if let Ok(days) = Self::chosen(&self.period, cx).parse::<f64>() {
            query["days"] = json!(days);
        }
        query
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.account.is_empty() {
            self.report = None;
            return;
        }
        self.epoch += 1;
        let epoch = self.epoch;
        self.loading = true;
        self.error = None;
        cx.notify();
        let call = cx
            .global::<AppCore>()
            .call("insights", vec![self.query(cx)]);
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                this.loading = false;
                match answer {
                    Ok(report) => this.report = Some(report),
                    Err(cause) => this.error = Some(cause.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The page was shown: games may have been synced since.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        if self.report.is_some() {
            self.load(cx);
        }
    }
}

/// A headline figure: its name, value and a line under it.
fn figure(label: &str, value: String, sub: Option<String>, palette: &Palette) -> Div {
    v_flex()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(label.to_string()),
        )
        .child(
            div()
                .text_size(px(22.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
        .children(sub.map(|sub| {
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(sub)
        }))
}

fn headline(report: &Value, palette: &Palette) -> Div {
    let total = report["total"].as_f64().unwrap_or_default();
    let (win, draw, loss) = record(&report["record"]);
    let score = if total > 0. {
        (100. * (win + draw / 2.) / total).round()
    } else {
        0.
    };
    let current = report["streaks"]["current"].as_f64().unwrap_or_default();
    let now = if current > 0. {
        format!(
            "{current} {} in a row now",
            if current == 1. { "win" } else { "wins" }
        )
    } else if current < 0. {
        format!(
            "{} {} in a row now",
            -current,
            if current == -1. { "loss" } else { "losses" }
        )
    } else {
        "No streak now".into()
    };
    let accuracy = report["accuracy"].as_object().map(|accuracy| {
        let games = accuracy
            .get("games")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let mut sub = format!(
            "{} reviewed {}",
            widgets::format_count(games),
            if games == 1. { "game" } else { "games" }
        );
        if let Some(acpl) = accuracy.get("acpl").and_then(Value::as_f64) {
            sub.push_str(&format!(" · ACPL {}", acpl.round()));
        }
        let average = accuracy
            .get("average")
            .and_then(Value::as_f64)
            .map_or("–".into(), |average| format!("{average:.1}%"));
        (average, sub)
    });
    h_flex()
        .flex_wrap()
        .items_start()
        .gap(px(32.))
        .child(figure(
            "Games",
            widgets::format_count(total),
            Some(format!(
                "{} won · {} drawn · {} lost",
                widgets::format_count(win),
                widgets::format_count(draw),
                widgets::format_count(loss)
            )),
            palette,
        ))
        .child(figure("Score", format!("{score}%"), None, palette))
        .child(figure(
            "Streaks",
            format!(
                "{} / {}",
                report["streaks"]["longestWin"].as_f64().unwrap_or_default(),
                report["streaks"]["longestLoss"]
                    .as_f64()
                    .unwrap_or_default()
            ),
            Some(now),
            palette,
        ))
        .children(accuracy.map(|(average, sub)| figure("Accuracy", average, Some(sub), palette)))
}

/// `USelect size="sm"`: a filter only as wide as its choice, so the four fit beside the figures.
fn fit(select: &Choices, choices: &[(&str, &str)], cx: &App) -> Div {
    let chosen = select.read(cx).selected_value().cloned();
    let label = choices
        .iter()
        .find(|(value, _)| Some(*value) == chosen.as_deref())
        .map_or(choices[0].1, |(_, label)| label);
    div().w(px(label.chars().count() as f32 * 7. + 40.))
}

/// `.insight-sections { columns: 3 300px }`: sections kept whole and in order, split into as many
/// columns as fit (at most three) at the least height that holds them, as CSS balances columns.
fn balance(heights: &[f32], columns: usize) -> Vec<usize> {
    let fits = |limit: f32| {
        let mut used = 1;
        let mut height = 0.;
        let mut starts = vec![0];
        for (index, section) in heights.iter().enumerate() {
            if height > 0. && height + section > limit {
                used += 1;
                height = 0.;
                starts.push(index);
            }
            height += section;
        }
        (used <= columns).then_some(starts)
    };
    let mut limits: Vec<f32> = (0..heights.len())
        .flat_map(|start| {
            (start + 1..=heights.len()).map(move |end| heights[start..end].iter().sum())
        })
        .collect();
    limits.sort_by(f32::total_cmp);
    limits.into_iter().find_map(fits).unwrap_or_else(|| vec![0])
}

/// A breakdown's entries: each label with its wins, draws and losses.
type Rows = Vec<(String, (f64, f64, f64))>;

/// One breakdown: its title and a result row per entry.
fn section(title: &'static str, rows: Rows, palette: &Palette) -> Div {
    v_flex()
        .w_full()
        .gap(px(6.))
        .child(div().mb(px(4.)).child(widgets::heading(title, palette)))
        .children(
            rows.into_iter()
                .map(|(label, record)| widgets::result_row(label, None, record, palette)),
        )
}

impl Render for InsightsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let names = Self::names(cx);
        let filters = h_flex()
            .gap(px(8.))
            .flex_wrap()
            .when(names.len() > 1, |row| {
                row.child(
                    div()
                        .w(px(160.))
                        .child(Select::new(&self.account_select).small()),
                )
            })
            .child(fit(&self.speed, &SPEEDS, cx).child(Select::new(&self.speed).small()))
            .child(fit(&self.period, &PERIODS, cx).child(Select::new(&self.period).small()))
            .child(fit(&self.rated, &RATED, cx).child(Select::new(&self.rated).small()));
        let report = self.report.clone();
        let total = report
            .as_ref()
            .and_then(|report| report["total"].as_f64())
            .unwrap_or_default();
        let header = h_flex()
            .items_start()
            .justify_between()
            .flex_wrap()
            .gap(px(16.))
            .child(
                div().children(
                    report
                        .as_ref()
                        .filter(|_| total > 0. && self.error.is_none())
                        .map(|report| headline(report, &palette)),
                ),
            )
            .child(filters);
        let body: AnyElement = if names.is_empty() {
            widgets::card(&palette)
                .child(
                    widgets::empty_state(
                        icon("chart-pie"),
                        "No saved Lichess games to explore",
                        None,
                        &palette,
                    )
                    .child(
                        h_flex()
                            .justify_center()
                            .gap(px(8.))
                            .mt(px(10.))
                            .child(
                                Button::new("insights-follow")
                                    .primary()
                                    .icon(icon("user-plus"))
                                    .label("Follow a player")
                                    .on_click(|_, window, cx| {
                                        crate::shell::open(Page::Friends, window, cx)
                                    }),
                            )
                            .child(
                                Button::new("insights-analyse")
                                    .outline()
                                    .label("Analyze a PGN")
                                    .on_click(|_, window, cx| {
                                        crate::shell::open(Page::Analysis, window, cx)
                                    }),
                            ),
                    ),
                )
                .into_any_element()
        } else if let Some(error) = self.error.clone() {
            widgets::card(&palette)
                .gap(px(12.))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(palette.error)
                        .child(error),
                )
                .child(
                    h_flex().child(
                        Button::new("insights-retry")
                            .primary()
                            .icon(icon("rotate-cw"))
                            .label("Try again")
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    ),
                )
                .into_any_element()
        } else if self.loading && report.is_none() {
            widgets::hint("Loading insights…", &palette).into_any_element()
        } else if let Some(report) = report.filter(|_| total <= 0.) {
            let _ = report;
            widgets::card(&palette)
                .child(
                    widgets::empty_state(
                        icon("chart-pie"),
                        "No finished games match",
                        Some("Sync games from Lichess history, or widen the filters.".into()),
                        &palette,
                    )
                    .child(
                        h_flex().justify_center().mt(px(10.)).child(
                            Button::new("insights-history")
                                .outline()
                                .label("Open Lichess history")
                                .on_click(|_, window, cx| {
                                    crate::shell::open_history_lichess(window, cx)
                                }),
                        ),
                    ),
                )
                .into_any_element()
        } else if let Some(report) = self.report.clone() {
            self.breakdowns(&report, self.area.size().width, &palette)
                .into_any_element()
        } else {
            div().into_any_element()
        };
        div()
            .id("insights-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap(px(24.))
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .child(header)
                    // Measured for the breakdowns' column count (`columns: 3 300px`).
                    .child(div().relative().child(self.area.probe(cx)).child(body)),
            )
    }
}

impl InsightsPage {
    fn breakdowns(&self, report: &Value, width: Pixels, palette: &Palette) -> Div {
        let list =
            |key: &str, label: &dyn Fn(&Value) -> String| -> Vec<(String, (f64, f64, f64))> {
                report[key]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|entry| (label(entry), record(&entry["record"])))
                    .collect()
            };
        let text = |field: &'static str| {
            move |entry: &Value| entry[field].as_str().unwrap_or_default().to_string()
        };
        let parts: Vec<(&'static str, Rows)> = vec![
            (
                "By colour",
                vec![
                    ("White".into(), record(&report["byColor"]["white"])),
                    ("Black".into(), record(&report["byColor"]["black"])),
                ],
            ),
            (
                "By speed",
                list("bySpeed", &|entry| {
                    speed_name(entry["speed"].as_str().unwrap_or_default())
                }),
            ),
            ("By opponent rating", list("byOpponent", &text("label"))),
            ("By game length", list("byLength", &text("label"))),
            (
                "By day",
                report["byWeekday"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(day, entry)| (WEEKDAYS[day % 7].to_string(), record(entry)))
                    .collect(),
            ),
            ("By time of day", by_time(&report["byHour"])),
            (
                "How games ended",
                list("endings", &|entry| {
                    ending(entry["status"].as_str().unwrap_or_default()).to_string()
                }),
            ),
        ];
        let openings = report["byOpening"].as_array().cloned().unwrap_or_default();
        let best = best_and_weakest(&openings);
        // Each section's height for balancing: its title and a row per entry.
        let heights: Vec<f32> = parts
            .iter()
            .map(|(_, rows)| 30. + rows.len() as f32 * 34. + 28.)
            .collect();
        let columns = ((f32::from(width) + 40.) / 340.).floor().clamp(1., 3.) as usize;
        let mut starts = balance(&heights, columns);
        starts.push(parts.len());
        let mut parts = parts.into_iter();
        let column_list: Vec<Div> = starts
            .windows(2)
            .map(|range| {
                v_flex().flex_1().min_w_0().gap(px(28.)).children(
                    parts
                        .by_ref()
                        .take(range[1] - range[0])
                        .map(|(title, rows)| section(title, rows, palette)),
                )
            })
            .collect();
        v_flex()
            .gap(px(28.))
            .child(h_flex().items_start().gap(px(40.)).children(column_list))
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(widgets::heading("Openings", palette))
                    .children(best.map(|(top, bottom)| {
                        widgets::hint(
                            format!("Best: {top} · weakest: {bottom} (at least 5 games)"),
                            palette,
                        )
                    }))
                    .when(openings.is_empty(), |list| {
                        list.child(widgets::hint(
                            "Lichess names openings of synced games; none have two games yet.",
                            palette,
                        ))
                    })
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_x(px(40.))
                            .children(openings.iter().map(|entry| {
                                div().flex_1().min_w(px(360.)).child(widgets::result_row(
                                    entry["name"].as_str().unwrap_or_default().to_string(),
                                    Some(format!(
                                        "{} as White",
                                        entry["asWhite"].as_f64().unwrap_or_default()
                                    )),
                                    record(&entry["record"]),
                                    palette,
                                ))
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{balance, best_and_weakest, by_time, speed_name};
    use serde_json::json;

    /// Electron's columns put colour, speed and opponent rating first; length and day next;
    /// time of day and endings last.
    #[test]
    fn sections_balance_into_columns_as_css_does() {
        let rows = [2., 6., 5., 4., 7., 6., 9.];
        let heights: Vec<f32> = rows.iter().map(|rows| 30. + rows * 34. + 28.).collect();
        assert_eq!(balance(&heights, 3), vec![0, 3, 5]);
        assert_eq!(balance(&heights, 1), vec![0]);
    }

    #[test]
    fn hours_group_into_four_hour_blocks() {
        let mut hours = vec![json!({ "win": 0, "draw": 0, "loss": 0 }); 24];
        hours[1] = json!({ "win": 2, "draw": 1, "loss": 0 });
        hours[3] = json!({ "win": 1, "draw": 0, "loss": 4 });
        let blocks = by_time(&json!(hours));
        assert_eq!(blocks.len(), 6);
        assert_eq!(blocks[0], ("00:00–04:00".to_string(), (3., 1., 4.)));
        assert_eq!(blocks[5].0, "20:00–24:00");
    }

    #[test]
    fn the_best_and_weakest_need_five_games() {
        let opening = |name: &str, win: f64, total: f64| json!({ "name": name, "record": { "win": win, "draw": 0, "loss": total - win, "total": total } });
        let openings = [
            opening("Sicilian", 8., 10.),
            opening("French", 1., 6.),
            opening("Rare", 3., 3.),
        ];
        assert_eq!(
            best_and_weakest(&openings),
            Some(("Sicilian".into(), "French".into()))
        );
        assert_eq!(best_and_weakest(&openings[..1]), None);
        assert_eq!(speed_name("ultraBullet"), "UltraBullet");
        assert_eq!(speed_name("blitz"), "Blitz");
    }
}
