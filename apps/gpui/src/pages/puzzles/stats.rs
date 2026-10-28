//! Puzzles → Lichess stats: the
//! account's puzzle numbers as Lichess reports them — rating, games, the dashboard's first-try
//! share and performance by theme over a period, the rating history, and the Storm and Streak
//! results. Read-only.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{Theme, theme_name};
use crate::assets::icon;
use crate::chart::{ChartPoint, LineChart};
use crate::core::AppCore;
use crate::game::rules;
use crate::theme::Palette;
use crate::widgets::{self, Choice};

const DAYS: &str = "kchess:puzzle-dashboard-days";
const PERIODS: [(u32, &str); 4] = [
    (7, "Last 7 days"),
    (30, "Last 30 days"),
    (90, "Last 90 days"),
    (365, "Last year"),
];
const SORTS: [(&str, &str); 3] = [
    ("weak", "Weakest first"),
    ("strong", "Strongest first"),
    ("played", "Most played"),
];
/// Themes tried fewer times say little about strength.
const MIN_ATTEMPTS: u64 = 3;

pub enum StatsEvent {
    /// Train a theme (`startTheme`).
    Train(String),
    Connect,
    Reconnect,
    /// Another connected account's numbers (`setAccount`).
    Account(String),
}

impl EventEmitter<StatsEvent> for PuzzleStats {}

/// One theme of the dashboard: key, attempts, first-try share, performance.
struct ThemeRow {
    key: String,
    attempts: u64,
    first_try: u64,
    performance: f64,
}

fn theme_rows(dashboard: &Value, sort: &str) -> Vec<ThemeRow> {
    let mut rows: Vec<ThemeRow> = dashboard["themes"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, entry)| {
            let results = &entry["results"];
            let attempts = results["nb"].as_u64().unwrap_or_default();
            let wins = results["firstWins"].as_u64().unwrap_or_default();
            ThemeRow {
                key: key.clone(),
                attempts,
                first_try: (wins * 100 + attempts / 2)
                    .checked_div(attempts)
                    .unwrap_or(0),
                performance: results["performance"].as_f64().unwrap_or_default(),
            }
        })
        .collect();
    if sort == "played" {
        rows.sort_by_key(|row| std::cmp::Reverse(row.attempts));
    } else {
        rows.retain(|row| row.attempts >= MIN_ATTEMPTS);
        rows.sort_by(|a, b| {
            let order = a.performance.total_cmp(&b.performance);
            if sort == "weak" {
                order
            } else {
                order.reverse()
            }
        });
    }
    rows.truncate(15);
    rows
}

pub struct PuzzleStats {
    account: String,
    accounts: Vec<String>,
    profile: Value,
    history: Value,
    dashboard: Value,
    storm: Value,
    loading: bool,
    error: Option<String>,
    reconnect: bool,
    sort: &'static str,
    themes: Vec<Theme>,
    chart: Entity<LineChart>,
    days_select: Entity<SelectState<Vec<Choice>>>,
    sort_select: Entity<SelectState<Vec<Choice>>>,
    account_select: Entity<SelectState<Vec<Choice>>>,
    epoch: u64,
    _subscriptions: Vec<Subscription>,
}

impl PuzzleStats {
    pub fn new(themes: Vec<Theme>, window: &mut Window, cx: &mut Context<Self>) -> PuzzleStats {
        let days = crate::prefs::get::<u32>(cx, DAYS, 30);
        let select =
            |items: Vec<Choice>, chosen: String, window: &mut Window, cx: &mut Context<Self>| {
                let state = cx.new(|cx| SelectState::new(items, None, window, cx));
                state.update(cx, |state, cx| {
                    state.set_selected_value(&SharedString::from(chosen), window, cx)
                });
                state
            };
        let days_select = select(
            PERIODS
                .iter()
                .map(|(value, label)| Choice::new(value.to_string(), *label))
                .collect(),
            days.to_string(),
            window,
            cx,
        );
        let sort_select = select(
            SORTS
                .iter()
                .map(|(value, label)| Choice::new(*value, *label))
                .collect(),
            "weak".into(),
            window,
            cx,
        );
        let account_select = select(Vec::new(), String::new(), window, cx);
        let subscriptions = vec![
            cx.subscribe(
                &days_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        crate::prefs::set(cx, DAYS, value.parse::<u32>().unwrap_or(30));
                        this.load(cx);
                    }
                },
            ),
            cx.subscribe(
                &sort_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.sort = SORTS
                            .iter()
                            .find(|(key, _)| *key == value.as_ref())
                            .map_or("weak", |(key, _)| *key);
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &account_select,
                |_, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        cx.emit(StatsEvent::Account(value.to_string()));
                    }
                },
            ),
        ];
        PuzzleStats {
            account: String::new(),
            accounts: Vec::new(),
            profile: Value::Null,
            history: Value::Null,
            dashboard: Value::Null,
            storm: Value::Null,
            loading: false,
            error: None,
            reconnect: false,
            sort: "weak",
            themes,
            chart: cx.new(|_| LineChart::new(px(220.), |rating| format!("{}", rating.round()))),
            days_select,
            sort_select,
            account_select,
            epoch: 0,
            _subscriptions: subscriptions,
        }
    }

    /// Shown for `account` among the connected `accounts`.
    pub fn shown(
        &mut self,
        account: String,
        accounts: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if accounts != self.accounts {
            let items: Vec<Choice> = accounts
                .iter()
                .map(|name| Choice::new(name.clone(), name.clone()))
                .collect();
            self.account_select
                .update(cx, |state, cx| state.set_items(items, window, cx));
            self.accounts = accounts;
        }
        let chosen = account.clone();
        self.account_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(chosen), window, cx)
        });
        self.account = account;
        self.load(cx);
    }

    /// `loadStats`: profile, rating history, dashboard and Storm results together.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        let epoch = self.epoch;
        let who = self.account.clone();
        if who.is_empty() {
            self.profile = Value::Null;
            self.history = Value::Null;
            self.dashboard = Value::Null;
            self.storm = Value::Null;
            self.loading = false;
            cx.notify();
            return;
        }
        self.loading = true;
        self.error = None;
        self.reconnect = false;
        cx.notify();
        let days = crate::prefs::get::<u32>(cx, DAYS, 30);
        let core = cx.global::<AppCore>().clone();
        let name = json!(who);
        let calls = (
            core.call("profile", vec![name.clone()]),
            core.call("ratingHistory", vec![name.clone()]),
            core.call("puzzleDashboard", vec![name.clone(), json!(days)]),
            core.call("stormDashboard", vec![name, json!(30)]),
        );
        cx.spawn(async move |this, cx| {
            let (profile, history, dashboard, storm) =
                (calls.0.await, calls.1.await, calls.2.await, calls.3.await);
            let _ = this.update(cx, |this, cx| {
                if epoch != this.epoch {
                    return;
                }
                this.loading = false;
                this.profile = profile.unwrap_or(Value::Null);
                this.history = history.unwrap_or(Value::Null);
                this.storm = storm.unwrap_or(Value::Null);
                match dashboard {
                    Ok(board)
                        if rules("isReconnect", std::slice::from_ref(&board))
                            == Ok(json!(true)) =>
                    {
                        this.dashboard = Value::Null;
                        this.reconnect = true;
                    }
                    Ok(board) => this.dashboard = board,
                    Err(cause) => {
                        this.dashboard = Value::Null;
                        this.error = Some(cause.message);
                    }
                }
                this.draw(cx);
            });
        })
        .detach();
    }

    /// The puzzle rating history on the chart.
    fn draw(&mut self, cx: &mut Context<Self>) {
        let points: Vec<(f64, f64)> = self
            .history
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| {
                entry["name"].as_str().is_some_and(|name| {
                    name.eq_ignore_ascii_case("puzzle") || name.eq_ignore_ascii_case("puzzles")
                })
            })
            .and_then(|entry| entry["points"].as_array())
            .map(|points| points.iter().filter_map(point_of).collect())
            .unwrap_or_default();
        let format = |millis: f64, pattern: &str| {
            chrono::DateTime::from_timestamp_millis(millis as i64)
                .map(|date| date.format(pattern).to_string())
                .unwrap_or_default()
        };
        let years =
            points.len() > 1 && points[points.len() - 1].0 - points[0].0 > 400. * 86_400_000.;
        let (low, high) = points
            .iter()
            .fold((f64::MAX, f64::MIN), |(low, high), (_, rating)| {
                (low.min(*rating), high.max(*rating))
            });
        let chart_points = points
            .iter()
            .map(|(date, rating)| ChartPoint {
                value: *rating,
                short: format(*date, if years { "%b %y" } else { "%b %-d" }).into(),
                long: format(*date, "%b %-d, %Y").into(),
            })
            .collect();
        let domain = if points.is_empty() {
            (0., 1.)
        } else {
            (low - 15., high + 15.)
        };
        let summary = format!(
            "Puzzle rating history from {} to {}",
            low.round(),
            high.round()
        );
        self.chart
            .update(cx, |chart, cx| chart.set(chart_points, domain, summary, cx));
        cx.notify();
    }

    fn point_count(&self) -> usize {
        self.history
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| {
                entry["name"].as_str().is_some_and(|name| {
                    name.eq_ignore_ascii_case("puzzle") || name.eq_ignore_ascii_case("puzzles")
                })
            })
            .and_then(|entry| entry["points"].as_array())
            .map_or(0, Vec::len)
    }
}

/// `[year, month0, day, rating]` as a UTC time and a rating.
fn point_of(raw: &Value) -> Option<(f64, f64)> {
    let parts = raw.as_array()?;
    let number = |index: usize| parts.get(index)?.as_f64().filter(|n| n.is_finite());
    let date = chrono::NaiveDate::from_ymd_opt(
        number(0)? as i32,
        number(1)? as u32 + 1,
        number(2)? as u32,
    )?;
    Some((
        date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis() as f64,
        number(3)?,
    ))
}

/// `.stat-card`.
fn stat(label: &str, value: String, sub: Option<AnyElement>, palette: &Palette) -> Div {
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
        .children(sub.map(|sub| {
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(sub)
        }))
}

/// Storm high scores, the best Storm and Streak, and recent Storm days.
pub fn storm_card(storm: &Value, profile: &Value, palette: &Palette) -> Div {
    let label = |text: &str| {
        div()
            .text_size(px(12.))
            .text_color(palette.text_muted)
            .child(text.to_string())
    };
    let value = |value: &Value| {
        div()
            .text_size(px(20.))
            .font_weight(FontWeight::BOLD)
            .child(
                value
                    .as_f64()
                    .map_or_else(|| "—".to_string(), |n| n.to_string()),
            )
    };
    let highs = h_flex().flex_wrap().gap(px(16.)).children(
        [
            ("Today", "day"),
            ("This week", "week"),
            ("This month", "month"),
            ("All time", "allTime"),
        ]
        .map(|(text, key)| {
            v_flex()
                .min_w(px(110.))
                .gap(px(2.))
                .child(label(text))
                .child(value(&storm["high"][key]))
        }),
    );
    let best = |text: &str, perf: &str| {
        v_flex()
            .min_w(px(110.))
            .gap(px(2.))
            .child(label(text))
            .child(value(&profile["perfs"][perf]["score"]))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(format!(
                        "{} runs",
                        widgets::format_count(
                            profile["perfs"][perf]["runs"].as_f64().unwrap_or_default()
                        )
                    )),
            )
    };
    let days: Vec<Value> = storm["days"]
        .as_array()
        .into_iter()
        .flatten()
        .take(8)
        .cloned()
        .collect();
    let table = if days.is_empty() {
        widgets::hint("No Storm runs on Lichess in the last 30 days.", palette).into_any_element()
    } else {
        let cell = |text: String, width: f32| div().w(px(width)).child(text);
        v_flex()
            .text_size(px(13.))
            .child(
                h_flex()
                    .pb(px(6.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(palette.text_muted)
                    .child(cell("DAY".into(), 110.))
                    .child(cell("RUNS".into(), 70.))
                    .child(cell("BEST SCORE".into(), 100.))
                    .child(cell("ACCURACY".into(), 90.))
                    .child(cell("BEST COMBO".into(), 100.)),
            )
            .children(days.iter().map(|day| {
                let number = |key: &str| day[key].as_f64().unwrap_or_default();
                let tried = number("moves") + number("errors");
                let accuracy = if tried > 0. {
                    (number("moves") / tried * 100.).round()
                } else {
                    0.
                };
                h_flex()
                    .py(px(4.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(cell(
                        day["_id"].as_str().unwrap_or_default().replace('/', "-"),
                        110.,
                    ))
                    .child(cell(number("runs").to_string(), 70.))
                    .child(cell(number("score").to_string(), 100.))
                    .child(cell(format!("{accuracy}%"), 90.))
                    .child(cell(number("combo").to_string(), 100.))
            }))
            .into_any_element()
    };
    widgets::card(palette)
        .gap(px(12.))
        .child(widgets::heading("Your results on Lichess", palette))
        .child(label("Storm high scores"))
        .child(highs)
        .child(
            h_flex()
                .flex_wrap()
                .gap(px(16.))
                .child(best("Storm best", "storm"))
                .child(best("Streak best", "streak")),
        )
        .child(table)
}

impl Render for PuzzleStats {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let page = |content: AnyElement| {
            div()
                .id("puzzle-stats")
                .size_full()
                .overflow_y_scroll()
                .child(
                    v_flex()
                        .gap(px(16.))
                        .px(crate::play_layout::side_padding(window))
                        .pt(px(20.))
                        .pb(crate::play_layout::PAGE_BOTTOM)
                        .child(content),
                )
        };
        if self.account.is_empty() {
            return page(
                widgets::card(&palette)
                    .py(px(30.))
                    .child(
                        widgets::empty_state(
                            icon("user-round-plus"),
                            "Connect a Lichess account",
                            None,
                            &palette,
                        )
                        .child(
                            h_flex().mt(px(12.)).child(
                                Button::new("stats-connect")
                                    .primary()
                                    .icon(icon("log-in"))
                                    .label("Connect Lichess")
                                    .on_click(
                                        cx.listener(|_, _, _, cx| cx.emit(StatsEvent::Connect)),
                                    ),
                            ),
                        ),
                    )
                    .into_any_element(),
            );
        }
        let days = crate::prefs::get::<u32>(cx, DAYS, 30);
        let perf = &self.profile["perfs"]["puzzle"];
        let global = &self.dashboard["global"];
        let first_try = global["nb"]
            .as_f64()
            .filter(|nb| *nb > 0.)
            .map(|nb| (global["firstWins"].as_f64().unwrap_or_default() / nb * 100.).round());
        let progress = perf["prog"].as_f64().filter(|prog| *prog != 0.);
        let controls = h_flex()
            .justify_end()
            .gap(px(8.))
            .when(self.accounts.len() > 1, |row| {
                row.child(
                    div()
                        .w(px(160.))
                        .child(Select::new(&self.account_select).small()),
                )
            })
            .child(
                div()
                    .w(px(140.))
                    .child(Select::new(&self.days_select).small()),
            )
            .child(
                Button::new("stats-refresh")
                    .outline()
                    .small()
                    .icon(icon("refresh-cw"))
                    .label("Refresh")
                    .loading(self.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
            );
        let stats = h_flex()
            .flex_wrap()
            .gap(px(12.))
            .child(stat(
                "Puzzle rating",
                perf["rating"].as_f64().map_or_else(
                    || "—".into(),
                    |rating| {
                        format!(
                            "{rating}{}",
                            if perf["prov"] == json!(true) { "?" } else { "" }
                        )
                    },
                ),
                Some(match progress {
                    Some(prog) => div()
                        .text_color(if prog > 0. { palette.win } else { palette.loss })
                        .child(format!(
                            "{} {} recently",
                            if prog > 0. { "▲" } else { "▼" },
                            prog.abs()
                        ))
                        .into_any_element(),
                    None => div().child(" ").into_any_element(),
                }),
                &palette,
            ))
            .child(stat(
                "Puzzles played",
                perf["games"]
                    .as_f64()
                    .map_or_else(|| "—".into(), widgets::format_count),
                None,
                &palette,
            ))
            .child(stat(
                "Solved first try",
                first_try.map_or_else(|| "—".into(), |share| format!("{share}%")),
                Some(div().child(format!("Last {days} days")).into_any_element()),
                &palette,
            ))
            .child(stat(
                "Performance",
                global["performance"]
                    .as_f64()
                    .map_or_else(|| "—".into(), |n| n.to_string()),
                Some(
                    div()
                        .child(match global["nb"].as_f64() {
                            Some(nb) => format!("{} puzzles", widgets::format_count(nb)),
                            None => format!("Last {days} days"),
                        })
                        .into_any_element(),
                ),
                &palette,
            ));
        let chart_card = widgets::card(&palette)
            .flex_1()
            .min_w(px(300.))
            .gap(px(10.))
            .child(widgets::heading("Puzzle rating history", &palette))
            .when(self.point_count() >= 2, |card| {
                card.child(self.chart.clone())
            })
            .when(self.point_count() < 2, |card| {
                card.child(widgets::empty_state(
                    icon("chart-line"),
                    "No rating history yet",
                    None,
                    &palette,
                ))
            });
        let rows = theme_rows(&self.dashboard, self.sort);
        let themes_body: AnyElement = if self.reconnect {
            widgets::empty_state(
                icon("key-round"),
                "Lichess needs your permission",
                Some("Reconnect this account to load puzzle statistics.".into()),
                &palette,
            )
            .child(
                h_flex().mt(px(10.)).child(
                    Button::new("stats-reconnect")
                        .primary()
                        .icon(icon("log-in"))
                        .label("Reconnect")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(StatsEvent::Reconnect))),
                ),
            )
            .into_any_element()
        } else if let Some(error) = &self.error {
            div()
                .text_size(px(13.))
                .text_color(palette.error)
                .child(error.clone())
                .into_any_element()
        } else if rows.is_empty() {
            widgets::empty_state(
                icon("puzzle"),
                "Nothing in this period",
                Some("Solve some puzzles on Lichess or in rated mode here, then refresh.".into()),
                &palette,
            )
            .into_any_element()
        } else {
            let cell = |width: f32| div().w(px(width)).text_right();
            v_flex()
                .text_size(px(13.))
                .child(
                    h_flex()
                        .pb(px(6.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.text_muted)
                        .child(div().flex_1().child("THEME"))
                        .child(cell(60.).child("PLAYED"))
                        .child(cell(76.).child("FIRST TRY"))
                        .child(cell(60.).child("PERF."))
                        .child(div().w(px(60.))),
                )
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    let key = row.key.clone();
                    h_flex()
                        .py(px(4.))
                        .border_t_1()
                        .border_color(palette.border)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(theme_name(&row.key, &self.themes)),
                        )
                        .child(cell(60.).child(row.attempts.to_string()))
                        .child(cell(76.).child(format!("{}%", row.first_try)))
                        .child(cell(60.).child(row.performance.to_string()))
                        .child(
                            div().w(px(60.)).flex().justify_end().child(
                                Button::new(("stats-train", index))
                                    .ghost()
                                    .xsmall()
                                    .label("Train")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(StatsEvent::Train(key.clone()))
                                    })),
                            ),
                        )
                }))
                .into_any_element()
        };
        let themes_card = widgets::card(&palette)
            .flex_1()
            .min_w(px(300.))
            .gap(px(10.))
            .child(
                h_flex()
                    .justify_between()
                    .gap(px(8.))
                    .child(v_flex().child(widgets::heading("Themes", &palette)).child(
                        widgets::hint(
                            format!("Where you are strongest and weakest, last {days} days."),
                            &palette,
                        ),
                    ))
                    .child(
                        div()
                            .w(px(150.))
                            .child(Select::new(&self.sort_select).small()),
                    ),
            )
            .child(themes_body);
        let stacked = window.viewport_size().width < px(900.);
        page(
            v_flex()
                .gap(px(16.))
                .child(controls)
                .child(stats)
                .child(
                    if stacked {
                        v_flex()
                    } else {
                        h_flex().items_start()
                    }
                    .gap(px(16.))
                    .child(chart_card)
                    .child(themes_card),
                )
                .into_any_element(),
        )
    }
}

impl PuzzleStats {
    /// Storm stats for the Rush tab: Connect without an account, then the results.
    pub fn storm_panel(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        if self.account.is_empty() {
            return widgets::card(palette)
                .gap(px(12.))
                .child(widgets::heading("Your results on Lichess", palette))
                .child(
                    widgets::empty_state(
                        icon("user-round-plus"),
                        "No Lichess account connected",
                        None,
                        palette,
                    )
                    .child(
                        h_flex().mt(px(10.)).child(
                            Button::new("storm-connect")
                                .primary()
                                .icon(icon("log-in"))
                                .label("Connect Lichess")
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(StatsEvent::Connect))),
                        ),
                    ),
                )
                .into_any_element();
        }
        if self.loading && !self.storm.is_object() {
            return widgets::card(palette)
                .gap(px(12.))
                .child(widgets::heading("Your results on Lichess", palette))
                .child(
                    h_flex()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .text_color(palette.text_muted)
                        .child(icon("loader-circle").size(px(14.)))
                        .child("Loading from Lichess…"),
                )
                .into_any_element();
        }
        storm_card(&self.storm, &self.profile, palette).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::theme_rows;
    use serde_json::json;

    #[test]
    fn themes_sort_by_strength_and_leave_out_the_rarely_tried() {
        let dashboard = json!({ "themes": {
            "fork": { "results": { "nb": 10, "firstWins": 5, "performance": 1500 } },
            "pin": { "results": { "nb": 4, "firstWins": 4, "performance": 1900 } },
            "skewer": { "results": { "nb": 2, "firstWins": 0, "performance": 1100 } },
        }});
        let weak: Vec<String> = theme_rows(&dashboard, "weak")
            .into_iter()
            .map(|row| row.key)
            .collect();
        assert_eq!(weak, vec!["fork", "pin"]);
        let strong: Vec<String> = theme_rows(&dashboard, "strong")
            .into_iter()
            .map(|row| row.key)
            .collect();
        assert_eq!(strong, vec!["pin", "fork"]);
        let played = theme_rows(&dashboard, "played");
        assert_eq!(played.len(), 3);
        assert_eq!(played[0].key, "fork");
        assert_eq!(played[0].first_try, 50);
    }
}
