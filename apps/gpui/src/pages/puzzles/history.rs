//! Puzzles → History: every attempt, the ones kept
//! on this computer (with partial credit) and the ones Lichess recorded (only the result), in one
//! list, each to try again or study on the analysis board; with today's and all-time totals.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{Theme, theme_name};
use crate::assets::icon;
use crate::core::AppCore;
use crate::game::rules;
use crate::theme::{Palette, radii};
use crate::widgets::{self, Choice, Look};

const FILTER: &str = "kchess:puzzle-history-filter";
/// `HISTORY_FILTERS`.
const FILTERS: [(&str, &str); 5] = [
    ("all", "All"),
    ("practice", "Practice"),
    ("offline", "Downloaded"),
    ("rated", "Rated"),
    ("lichess", "Lichess"),
];
/// Lichess's record of an attempt also kept here is the same attempt within this window.
const SAME_ATTEMPT_MS: f64 = 15. * 60_000.;

/// What the page does for a row.
pub enum HistoryEvent {
    Retry(Value),
    Analyse(Value, Vec<Value>),
    Train,
    Reconnect,
}

impl EventEmitter<HistoryEvent> for PuzzleHistory {}

/// One row: an attempt kept here, or one Lichess recorded (`PuzzleHistoryRow`).
struct Row {
    at: f64,
    /// `ATTEMPT_BADGES`' label and look.
    badge: (&'static str, Look),
    moves: Option<String>,
    puzzle: Value,
    wrong: Vec<Value>,
    /// `HISTORY_MODES`: the mode shown, and the one the filter matches.
    mode: &'static str,
    played_in: &'static str,
}

fn badge_of(outcome: &str) -> (&'static str, Look) {
    match outcome {
        "solved" => ("Solved", Look::Success),
        "hinted" => ("With hint", Look::Success),
        "partial" => ("Partial", Look::Info),
        "missed" => ("Missed", Look::Error),
        _ => ("Revealed", Look::NeutralSubtle),
    }
}

fn mode_of(mode: &str) -> &'static str {
    match mode {
        "offline" => "offline",
        "rated" => "rated",
        "retry" => "retry",
        "lichess" => "lichess",
        _ => "practice",
    }
}

/// `HISTORY_MODES`' label, icon and tip.
fn mode_look(mode: &str) -> (&'static str, Icon, &'static str) {
    match mode {
        "offline" => (
            "Downloaded",
            icon("hard-drive"),
            "From the puzzle database on this computer",
        ),
        "rated" => (
            "Rated",
            icon("cloud-upload"),
            "Counted in your Lichess puzzle rating",
        ),
        "retry" => ("Retry", icon("rotate-ccw"), "Tried again: not counted"),
        "lichess" => (
            "Lichess",
            icon("cloud-download"),
            "Recorded by Lichess: only the result is known",
        ),
        _ => ("Practice", icon("dumbbell"), "Lichess puzzle, unrated"),
    }
}

/// `puzzleHistoryRows`: local attempts and Lichess activity, newest first, each attempt once.
fn rows_of(local: &[Value], lichess: &[Value]) -> Vec<Row> {
    let mut rows: Vec<Row> = local
        .iter()
        .map(|attempt| {
            let count = |key: &str| attempt[key].as_u64().unwrap_or_default();
            let wrong = count("wrong");
            let moves = format!(
                "{}/{} moves{}",
                count("found"),
                count("total"),
                if wrong > 0 {
                    format!(" · {wrong} wrong")
                } else {
                    String::new()
                }
            );
            let mode = mode_of(attempt["mode"].as_str().unwrap_or_default());
            Row {
                at: attempt["at"].as_f64().unwrap_or_default(),
                badge: badge_of(attempt["outcome"].as_str().unwrap_or_default()),
                moves: Some(moves),
                puzzle: attempt["puzzle"].clone(),
                wrong: attempt["wrongMoves"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
                mode: if attempt["retry"] == json!(true) {
                    "retry"
                } else {
                    mode
                },
                played_in: mode,
            }
        })
        .collect();
    for entry in lichess {
        let date = entry["date"].as_f64().unwrap_or_default();
        let kept = local.iter().any(|attempt| {
            attempt["puzzle"]["id"] == entry["puzzle"]["id"]
                && (attempt["at"].as_f64().unwrap_or_default() - date).abs() < SAME_ATTEMPT_MS
        });
        if kept {
            continue;
        }
        rows.push(Row {
            at: date,
            badge: if entry["win"] == json!(true) {
                badge_of("solved")
            } else {
                badge_of("missed")
            },
            moves: None,
            puzzle: entry["puzzle"].clone(),
            wrong: Vec::new(),
            mode: "lichess",
            played_in: "lichess",
        });
    }
    rows.sort_by(|a, b| b.at.total_cmp(&a.at));
    rows
}

/// `playedText`: the time today, "Yesterday" and the time, else the date.
fn played_text(at: f64, now: chrono::DateTime<chrono::Local>) -> String {
    use chrono::TimeZone as _;
    let Some(played) = chrono::Local.timestamp_millis_opt(at as i64).single() else {
        return String::new();
    };
    let today = now.date_naive();
    let time = played.format("%-I:%M %p").to_string();
    if played.date_naive() == today {
        time
    } else if today.pred_opt() == Some(played.date_naive()) {
        format!("Yesterday {time}")
    } else if played.format("%Y").to_string() == now.format("%Y").to_string() {
        played.format("%b %-d").to_string()
    } else {
        played.format("%b %-d, %Y").to_string()
    }
}

pub struct PuzzleHistory {
    /// `puzzleProgress`: today's and all-time totals and the attempts kept here.
    progress: Value,
    /// `puzzleActivity`: what Lichess recorded for the account.
    activity: Vec<Value>,
    account: String,
    loading: bool,
    error: Option<String>,
    reconnect: bool,
    loaded: String,
    themes: Vec<Theme>,
    filter: Entity<SelectState<Vec<Choice>>>,
    epoch: u64,
    _subscriptions: Vec<Subscription>,
}

impl PuzzleHistory {
    pub fn new(themes: Vec<Theme>, window: &mut Window, cx: &mut Context<Self>) -> PuzzleHistory {
        let chosen = crate::prefs::get::<String>(cx, FILTER, "all".into());
        let filter = cx.new(|cx| {
            SelectState::new(
                FILTERS
                    .iter()
                    .map(|(value, label)| Choice::new(*value, *label))
                    .collect(),
                None,
                window,
                cx,
            )
        });
        filter.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(chosen), window, cx)
        });
        let subscriptions =
            vec![
                cx.subscribe(&filter, |_, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        crate::prefs::set(cx, FILTER, value.to_string());
                        cx.notify();
                    }
                }),
            ];
        PuzzleHistory {
            progress: Value::Null,
            activity: Vec::new(),
            account: String::new(),
            loading: false,
            error: None,
            reconnect: false,
            loaded: String::new(),
            themes,
            filter,
            epoch: 0,
            _subscriptions: subscriptions,
        }
    }

    /// The tab was shown for `account` (the rated account; empty without one).
    pub fn shown(&mut self, account: String, cx: &mut Context<Self>) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let progress = cx.global::<AppCore>().call(
            "puzzleProgress",
            vec![json!({ "today": today, "limit": 200 })],
        );
        cx.spawn(async move |this, cx| {
            if let Ok(progress) = progress.await {
                let _ = this.update(cx, |this, cx| {
                    this.progress = progress;
                    cx.notify();
                });
            }
        })
        .detach();
        if account != self.account {
            self.account = account;
            self.activity.clear();
            self.loaded.clear();
        }
        self.load_activity(false, cx);
    }

    /// `loadActivity`: what Lichess recorded (`force` asks again).
    fn load_activity(&mut self, force: bool, cx: &mut Context<Self>) {
        let who = self.account.clone();
        if who.is_empty() || (!force && self.loaded == who) {
            return;
        }
        self.epoch += 1;
        let epoch = self.epoch;
        self.loading = true;
        self.error = None;
        self.reconnect = false;
        cx.notify();
        let activity = cx
            .global::<AppCore>()
            .call("puzzleActivity", vec![json!(who), json!(100)]);
        cx.spawn(async move |this, cx| {
            let result = activity.await;
            let _ = this.update(cx, |this, cx| {
                if epoch != this.epoch {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(result)
                        if rules("isReconnect", std::slice::from_ref(&result))
                            == Ok(json!(true)) =>
                    {
                        this.reconnect = true;
                    }
                    Ok(result) => {
                        this.activity = result.as_array().cloned().unwrap_or_default();
                        this.loaded = who;
                    }
                    Err(cause) => this.error = Some(cause.message),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn summary(&self, palette: &Palette) -> Option<Div> {
        let all = &self.progress["all"];
        let attempts = all["attempts"].as_f64().filter(|attempts| *attempts > 0.)?;
        let today = &self.progress["today"];
        let number = |value: &Value, key: &str| value[key].as_f64().unwrap_or_default();
        let found = number(all, "movesFound");
        let tried = number(all, "movesTried");
        let accuracy = (tried > 0.).then(|| (found / tried * 100.).round());
        let solved_share =
            ((number(all, "solved") + number(all, "hinted")) / attempts * 100.).round();
        let partial = number(all, "partial");
        let card = |label: &str, value: String, sub: String| {
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
        };
        Some(
            h_flex()
                .flex_wrap()
                .gap(px(12.))
                .child(card(
                    "Today",
                    widgets::format_count(number(today, "attempts")),
                    format!(
                        "{} solved · {}/{} moves",
                        number(today, "solved") + number(today, "hinted"),
                        number(today, "movesFound"),
                        number(today, "movesTried")
                    ),
                ))
                .child(card(
                    "All time",
                    widgets::format_count(attempts),
                    if partial > 0. {
                        format!(
                            "{solved_share}% solved · {} partial",
                            widgets::format_count(partial)
                        )
                    } else {
                        format!("{solved_share}% solved")
                    },
                ))
                .child(card(
                    "Move accuracy",
                    accuracy.map_or_else(|| "—".into(), |accuracy| format!("{accuracy}%")),
                    format!(
                        "{} of {} moves right",
                        widgets::format_count(found),
                        widgets::format_count(tried)
                    ),
                ))
                .child(card(
                    "Days in a row",
                    format!("{}", number(&self.progress, "dayStreak")),
                    format!("Best {}", number(&self.progress, "bestDayStreak")),
                )),
        )
    }
}

impl Render for PuzzleHistory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let filter = crate::prefs::get::<String>(cx, FILTER, "all".into());
        let local = self.progress["recent"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let all_rows = rows_of(&local, &self.activity);
        let any = !all_rows.is_empty();
        let rows: Vec<Row> = all_rows
            .into_iter()
            .filter(|row| filter == "all" || row.played_in == filter)
            .collect();
        let has_account = !self.account.is_empty();
        let header = h_flex()
            .justify_between()
            .mb(px(10.))
            .child(
                div()
                    .w(px(160.))
                    .when(any, |slot| slot.child(Select::new(&self.filter).small())),
            )
            .when(has_account, |row| {
                row.child(
                    Button::new("puzzle-history-refresh")
                        .outline()
                        .small()
                        .icon(icon("refresh-cw"))
                        .label("Refresh")
                        .tooltip("Load the latest puzzle history from Lichess")
                        .loading(self.loading)
                        .on_click(cx.listener(|this, _, _, cx| this.load_activity(true, cx))),
                )
            });
        let now = chrono::Local::now();
        let body: AnyElement = if self.loading && rows.is_empty() {
            h_flex()
                .gap(px(8.))
                .text_size(px(13.))
                .text_color(palette.text_muted)
                .child(icon("loader-circle").size(px(14.)))
                .child("Loading from Lichess…")
                .into_any_element()
        } else if any && rows.is_empty() {
            let label = mode_look(if filter == "all" { "practice" } else { &filter }).0;
            div()
                .text_size(px(13.))
                .text_color(palette.text_muted)
                .child(format!("No {label} puzzles yet."))
                .into_any_element()
        } else if rows.is_empty() {
            widgets::empty_state(icon("puzzle"), "No puzzles yet", None, &palette)
                .child(
                    h_flex().mt(px(10.)).child(
                        Button::new("puzzle-history-train")
                            .primary()
                            .icon(icon("puzzle"))
                            .label("Train")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(HistoryEvent::Train))),
                    ),
                )
                .into_any_element()
        } else {
            let muted = palette.text_muted;
            let head = h_flex()
                .px(px(10.))
                .pb(px(8.))
                .gap(px(12.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(div().w(px(96.)).child("RESULT"))
                .child(div().w(px(120.)).child("MODE"))
                .child(div().flex_1().child("THEMES"))
                .child(div().w(px(130.)).child("MOVES"))
                .child(div().w(px(56.)).child("RATING"))
                .child(div().w(px(110.)).child("PLAYED"))
                .child(div().w(px(120.)));
            let list = rows.into_iter().enumerate().map(|(index, row)| {
                let (label, glyph, tip) = mode_look(row.mode);
                let themes: Vec<String> = row.puzzle["themes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|theme| theme_name(theme, &self.themes))
                    .collect();
                let (retry, analysed, wrong) =
                    (row.puzzle.clone(), row.puzzle.clone(), row.wrong.clone());
                h_flex()
                    .id(("puzzle-attempt", index))
                    .px(px(10.))
                    .py(px(8.))
                    .gap(px(12.))
                    .rounded(radii::MD)
                    .text_size(px(13.))
                    .hover(|row| row.bg(palette.accented))
                    .child(
                        div().w(px(96.)).child(
                            widgets::badge(row.badge.0, row.badge.1, &palette)
                                .w(px(80.))
                                .justify_center(),
                        ),
                    )
                    .child(
                        h_flex()
                            .id(("puzzle-mode", index))
                            .w(px(120.))
                            .gap(px(6.))
                            .text_color(muted)
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                            })
                            .child(glyph.size(px(14.)))
                            .child(label),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(themes.join(", ")))
                    .child(
                        div()
                            .w(px(130.))
                            .text_color(if row.moves.is_some() {
                                muted
                            } else {
                                palette.border_accented
                            })
                            .child(row.moves.unwrap_or_else(|| "—".into())),
                    )
                    .child(div().w(px(56.)).child(format!(
                        "{}",
                        row.puzzle["rating"].as_f64().unwrap_or_default()
                    )))
                    .child(
                        div()
                            .w(px(110.))
                            .text_color(muted)
                            .child(played_text(row.at, now)),
                    )
                    .child(
                        h_flex()
                            .w(px(120.))
                            .gap(px(4.))
                            .child(
                                Button::new(("puzzle-retry", index))
                                    .outline()
                                    .xsmall()
                                    .label("Try again")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(HistoryEvent::Retry(retry.clone()))
                                    })),
                            )
                            .child(
                                Button::new(("puzzle-analyse", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(icon("microscope"))
                                    .tooltip("Analysis board")
                                    .accessibility_label("Open on the analysis board")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(HistoryEvent::Analyse(
                                            analysed.clone(),
                                            wrong.clone(),
                                        ))
                                    })),
                            ),
                    )
            });
            v_flex().child(head).children(list).into_any_element()
        };
        let notes = if has_account && self.reconnect {
            Some(
                h_flex()
                    .justify_between()
                    .gap(px(12.))
                    .mb(px(8.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child("Lichess needs your permission to show its puzzle history."),
                    )
                    .child(
                        Button::new("puzzle-history-reconnect")
                            .outline()
                            .xsmall()
                            .icon(icon("log-in"))
                            .label("Reconnect")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(HistoryEvent::Reconnect))),
                    )
                    .into_any_element(),
            )
        } else {
            self.error.clone().filter(|_| has_account).map(|error| {
                div()
                    .mb(px(8.))
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error)
                    .into_any_element()
            })
        };
        div()
            .id("puzzle-history")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap(px(16.))
                    .px(crate::play_layout::side_padding(window))
                    .pt(px(20.))
                    .pb(crate::play_layout::PAGE_BOTTOM)
                    .children(self.summary(&palette))
                    .child(
                        widgets::card(&palette)
                            .child(header)
                            .children(notes)
                            .child(body),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::rows_of;
    use serde_json::json;

    #[test]
    fn lichess_records_of_attempts_kept_here_are_left_out() {
        let local = vec![json!({
            "id": 1, "at": 1_000_000.0, "outcome": "partial", "mode": "rated", "retry": false,
            "found": 1, "total": 3, "wrong": 2, "puzzle": { "id": "abc" }
        })];
        let lichess = vec![
            json!({ "date": 1_000_000.0 + 60_000.0, "win": false, "puzzle": { "id": "abc" } }),
            json!({ "date": 2_000_000.0, "win": true, "puzzle": { "id": "def" } }),
        ];
        let rows = rows_of(&local, &lichess);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].mode, "lichess");
        assert_eq!(rows[1].moves.as_deref(), Some("1/3 moves · 2 wrong"));
        assert_eq!(rows[1].badge.0, "Partial");
    }
}
