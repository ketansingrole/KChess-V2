//! Insights: an account's synced games (Lichess or Chess.com) broken down by colour,
//! speed, opponent, length, clock, day, time of day, ending and opening, with time use from the
//! games' clocks, filtered by speed, period and rated or casual.

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

/// Results of the games where the player's clock fell under a tenth of its start, and of the
/// rest; none without clocked games.
fn by_clock(time: &Value) -> Vec<(String, (f64, f64, f64))> {
    if time.is_null() {
        return Vec::new();
    }
    [("Short of time", "trouble"), ("Time to spare", "spare")]
        .into_iter()
        .filter(|(_, key)| time[*key]["total"].as_f64().unwrap_or_default() > 0.)
        .map(|(label, key)| (label.to_string(), record(&time[key])))
        .collect()
}

/// Seconds as a short duration: `4.2 s`, `1 min 5 s`.
fn seconds(value: f64) -> String {
    if value < 60. {
        format!("{value:.1} s")
    } else {
        let whole = value.round() as i64;
        format!("{} min {} s", whole / 60, whole % 60)
    }
}

/// Blunders per hundred moves.
fn blunder_rate(entry: &Value) -> Option<f64> {
    let moves = entry["moves"].as_f64().filter(|moves| *moves > 0.)?;
    Some(100. * entry["blunders"].as_f64().unwrap_or_default() / moves)
}

/// Time use: the clock against the opponent's, blunders short of time, and time by phase.
fn time_use(time: &Value, palette: &Palette) -> Option<AnyElement> {
    let games = time["games"].as_f64()?;
    let clock_at = time["clockAt"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            let percent = |key: &str| entry[key].as_f64().unwrap_or_default().round();
            figure(
                &format!("Clock at move {}", entry["move"]),
                format!("{}% left", percent("player")),
                Some(format!("Opponent {}%", percent("opponent"))),
                palette,
            )
        });
    let blunders = time.get("blunders").and_then(|blunders| {
        let short = blunder_rate(&blunders["short"]);
        let spare = blunder_rate(&blunders["spare"]);
        let (value, sub) = match (short, spare) {
            (Some(short), spare) => (
                format!("{short:.1}"),
                spare.map(|spare| format!("{spare:.1} with time to spare")),
            ),
            (None, Some(spare)) => ("–".into(), Some(format!("{spare:.1} with time to spare"))),
            (None, None) => return None,
        };
        Some(figure(
            "Blunders per 100 moves short of time",
            value,
            sub,
            palette,
        ))
    });
    let phases = time["phases"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            let name = match entry["phase"].as_str().unwrap_or_default() {
                "opening" => "Opening",
                "middlegame" => "Middlegame",
                _ => "Endgame",
            };
            let mut detail = format!(
                "{} a move",
                seconds(entry["seconds"].as_f64().unwrap_or_default())
            );
            if let Some(share) = entry["share"].as_f64() {
                detail.push_str(&format!(" · {}% of your time", share.round()));
            }
            h_flex()
                .justify_between()
                .gap(px(12.))
                .py(px(4.))
                .text_size(px(13.))
                .child(div().child(name))
                .child(
                    div()
                        .text_color(palette.text_muted)
                        .font_features(widgets::tabular())
                        .child(detail),
                )
        });
    Some(
        v_flex()
            .id("insights-time-use")
            .test_support()
            .gap(px(8.))
            .child(widgets::heading("Time use", palette))
            .child(widgets::hint(
                widgets::format_counted(games, "game with clocks", "games with clocks"),
                palette,
            ))
            .child(
                h_flex()
                    .flex_wrap()
                    .items_start()
                    .gap(px(32.))
                    .py(px(4.))
                    .children(clock_at)
                    .children(blunders),
            )
            .child(v_flex().max_w(px(480.)).children(phases))
            .into_any_element(),
    )
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
    /// Standard chess, or one of the variants the account has played.
    variant: Choices,
    /// The variant keys the select lists now; refilled when a report names others, on the next
    /// frame, where there is a window.
    variant_keys: Vec<String>,
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
        let variant = cx.new(|cx| {
            let mut state =
                SelectState::new(vec![Choice::new("standard", "Standard")], None, window, cx);
            state.set_selected_value(&SharedString::from("standard"), window, cx);
            state
        });
        let account_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let accounts = crate::accounts::handle(cx);
        let mut subscriptions: Vec<Subscription> = [&speed, &period, &rated, &variant]
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
            variant,
            variant_keys: vec!["standard".into()],
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

    /// The accounts with synced games: Lichess players, then Chess.com ones (`name@chess.com`).
    fn names(cx: &App) -> Vec<String> {
        let accounts = crate::accounts::handle(cx).read(cx);
        accounts
            .all()
            .iter()
            .filter_map(|account| account["username"].as_str().map(str::to_string))
            .chain(accounts.chesscom().iter().filter_map(|player| {
                player["username"]
                    .as_str()
                    .map(kchess_domain::chesscom::account_of)
            }))
            .collect()
    }

    /// The accounts offered; the active one first time, or the first.
    fn sync_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names = Self::names(cx);
        let items: Vec<Choice> = names
            .iter()
            .map(|name| match kchess_domain::chesscom::username_of(name) {
                Some(player) => Choice::new(name.clone(), format!("@{player} · Chess.com")),
                None => Choice::new(name.clone(), format!("@{name}")),
            })
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
    pub(crate) fn query(&self, cx: &App) -> Value {
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
        let variant = Self::chosen(&self.variant, cx);
        if variant != "standard" && variant != "all" {
            query["variant"] = json!(variant);
        }
        query
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        // A new epoch either way: a report still on its way for the previous account must not
        // land once there is none.
        self.epoch += 1;
        if self.account.is_empty() {
            self.report = None;
            self.loading = false;
            self.error = None;
            cx.notify();
            return;
        }
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

    /// Standard, the variants the report names and the one chosen, refilled into the select when
    /// they changed.
    fn sync_variants(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = Self::chosen(&self.variant, cx);
        let mut keys = vec!["standard".to_string()];
        for key in self
            .report
            .as_ref()
            .and_then(|report| report["variants"].as_array())
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .chain((chosen != "standard" && chosen != "all").then_some(chosen.clone()))
        {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        if keys == self.variant_keys {
            return;
        }
        let items: Vec<Choice> = keys
            .iter()
            .map(|key| Choice::new(key.clone(), crate::game::variant_label(key)))
            .collect();
        self.variant.update(cx, |select, cx| {
            select.set_items(items, window, cx);
            select.set_selected_value(&SharedString::from(chosen), window, cx);
        });
        self.variant_keys = keys;
    }

    /// A report has come in and none is loading.
    #[cfg(test)]
    pub(crate) fn loaded(&self) -> bool {
        self.report.is_some() && !self.loading
    }

    /// How many games the last report counted.
    #[cfg(test)]
    pub(crate) fn report_total(&self) -> Option<f64> {
        self.report.as_ref()?["total"].as_f64()
    }

    /// As if the last report had named `variants` as played.
    #[cfg(test)]
    pub(crate) fn show_report_variants(&mut self, variants: &[&str], cx: &mut Context<Self>) {
        if let Some(report) = &mut self.report {
            report["variants"] = json!(variants);
        }
        cx.notify();
    }

    /// As if the last report had had `time` as its time use.
    #[cfg(test)]
    pub(crate) fn show_time_use(&mut self, time: Value, cx: &mut Context<Self>) {
        if let Some(report) = &mut self.report {
            report["timeUse"] = time;
        }
        cx.notify();
    }

    /// Choose a variant as the select does.
    #[cfg(test)]
    pub(crate) fn choose_variant(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = SharedString::from(key.to_string());
        self.variant
            .update(cx, |select, cx| select.set_selected_value(&key, window, cx));
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
                .text_size(px(20.))
                .font_weight(FontWeight::MEDIUM)
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
        .gap(px(8.))
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
        self.sync_variants(window, cx);
        let variant_label = crate::game::variant_label(&Self::chosen(&self.variant, cx));
        let filters = h_flex()
            .gap(px(8.))
            .flex_wrap()
            .when(names.len() > 1, |row| {
                row.child(
                    div().w(px(160.)).child(
                        Select::new(&self.account_select)
                            .small()
                            .accessibility_label("Account"),
                    ),
                )
            })
            .child(
                fit(&self.speed, &SPEEDS, cx).child(
                    Select::new(&self.speed)
                        .small()
                        .accessibility_label("Speed"),
                ),
            )
            .child(
                fit(&self.period, &PERIODS, cx).child(
                    Select::new(&self.period)
                        .small()
                        .accessibility_label("Period"),
                ),
            )
            .child(
                fit(&self.rated, &RATED, cx).child(
                    Select::new(&self.rated)
                        .small()
                        .accessibility_label("Rated or casual"),
                ),
            )
            // Only for an account with variant games.
            .when(self.variant_keys.len() > 1, |row| {
                row.child(
                    div()
                        .w(px(variant_label.chars().count() as f32 * 7. + 40.))
                        .child(
                            Select::new(&self.variant)
                                .small()
                                .accessibility_label("Variant"),
                        ),
                )
            });
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
            // Filters wait for games to filter.
            .when(!names.is_empty(), |header| header.child(filters));
        let body: AnyElement = if names.is_empty() {
            widgets::card(&palette)
                .child(
                    widgets::empty_state(
                        icon("chart-pie"),
                        "No saved online games to explore",
                        None,
                        &palette,
                    )
                    .child(
                        h_flex()
                            .justify_center()
                            .gap(px(8.))
                            .mt(px(8.))
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
                                    .label("Analyse a PGN")
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
                        crate::widgets::try_again("insights-retry")
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    ),
                )
                .into_any_element()
        } else if self.loading && report.is_none() {
            widgets::loading("Loading insights…", &palette).into_any_element()
        } else if let Some(report) = report.filter(|_| total <= 0.) {
            let _ = report;
            widgets::card(&palette)
                .child(
                    widgets::empty_state(
                        icon("chart-pie"),
                        "No finished games match",
                        Some("Sync games from History, or widen the filters".into()),
                        &palette,
                    )
                    .child(
                        h_flex().justify_center().mt(px(8.)).child(
                            Button::new("insights-history")
                                .outline()
                                .label("Open history")
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
            ("By clock", by_clock(&report["timeUse"])),
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
        // Without clocked games there is nothing to break down by clock.
        let parts: Vec<(&'static str, Rows)> = parts
            .into_iter()
            .filter(|(title, rows)| *title != "By clock" || !rows.is_empty())
            .collect();
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
            .children(time_use(&report["timeUse"], palette))
            .child(
                v_flex()
                    .gap(px(8.))
                    .child(widgets::heading("Openings", palette))
                    .children(best.map(|(top, bottom)| {
                        widgets::hint(
                            format!("Best: {top} · weakest: {bottom} (at least 5 games)"),
                            palette,
                        )
                    }))
                    .when(openings.is_empty(), |list| {
                        list.child(widgets::hint(
                            "No opening has two synced games yet",
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
    use super::{
        InsightsPage, balance, best_and_weakest, blunder_rate, by_clock, by_time, seconds,
        speed_name,
    };
    use crate::e2e::Harness;

    /// Signing out while an account's report loaded kept the request current: the report then
    /// landed under no account, and the page kept loading until it did.
    #[gpui_kit::test]
    fn a_report_for_an_account_that_is_gone_does_not_land(cx: &mut gpui_kit::TestAppContext) {
        let mut app = Harness::start(cx);
        let (loading, stale) = app.with_window(|window, cx| {
            let page = cx.new(|cx| InsightsPage::new(window, cx));
            page.update(cx, |page, cx| {
                page.account = "Alice".into();
                page.load(cx);
                let asked = page.epoch;
                page.account.clear();
                page.load(cx);
                (page.loading, page.epoch == asked)
            })
        });
        assert!(!loading && !stale);
    }
    use serde_json::json;

    /// The columns put colour, speed and opponent rating first; length and day next;
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

    #[test]
    fn clock_figures_read_as_words() {
        assert!(by_clock(&serde_json::Value::Null).is_empty());
        let time = json!({
            "trouble": { "win": 1, "draw": 0, "loss": 2, "total": 3 },
            "spare": { "win": 0, "draw": 0, "loss": 0, "total": 0 },
        });
        assert_eq!(
            by_clock(&time),
            vec![("Short of time".to_string(), (1., 0., 2.))],
            "a side with no games is left out"
        );
        assert_eq!(seconds(4.25), "4.2 s");
        assert_eq!(seconds(65.), "1 min 5 s");
        assert_eq!(
            blunder_rate(&json!({ "moves": 50, "blunders": 2 })),
            Some(4.)
        );
        assert_eq!(blunder_rate(&json!({ "moves": 0, "blunders": 0 })), None);
    }
}
