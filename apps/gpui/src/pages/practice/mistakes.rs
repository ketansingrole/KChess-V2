//! Your mistakes (`MistakeTrainer.vue`): positions from your reviewed games where you went wrong,
//! solved again on a schedule the core keeps (a right answer doubles the wait, a wrong one brings
//! it back in ten minutes), and the games they come from.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};
use shakmaty::Color;

use crate::assets::icon;
use crate::core::AppCore;
use crate::pages::Page;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, side_padding};
use crate::puzzle_board::{PuzzleBoard, PuzzleBoardEvent};
use crate::theme::{Palette, judgment_color};
use crate::widgets;

/// Answered right this many times in a row, a position is next due in four days or more: learned
/// rather than still learning.
const LEARNED_STREAK: f64 = 3.;
const GAMES_SHOWN: usize = 8;
/// What counts as due is looked at again every minute.
const CLOCK: Duration = Duration::from_secs(60);

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0., |since| since.as_millis() as f64)
}

fn due_at(item: &Value) -> f64 {
    item["dueAt"].as_f64().unwrap_or(0.)
}

fn id_of(item: &Value) -> &str {
    item["id"].as_str().unwrap_or_default()
}

/// A drill as a puzzle for the board.
fn puzzle(item: &Value) -> Value {
    json!({
        "id": item["id"],
        "fen": item["fen"],
        "solution": item["solution"],
        "rating": 0,
        "themes": [],
    })
}

/// A game's title: your opponent when your side is known, else the players.
fn title(game: &Value) -> String {
    let (white, black) = (
        game["white"].as_str().unwrap_or_default(),
        game["black"].as_str().unwrap_or_default(),
    );
    match game["player"].as_str() {
        Some(player) => {
            let opponent = if player == "white" { black } else { white };
            if opponent.is_empty() {
                format!("You as {player}")
            } else {
                format!("vs {opponent}")
            }
        }
        None if !white.is_empty() && !black.is_empty() => format!("{white} – {black}"),
        None => "Analysis board game".into(),
    }
}

/// `judgmentCounts`: "2 blunders", worst first, leaving out the kinds that did not happen.
fn judgment_counts(items: &[&Value]) -> Vec<(&'static str, String)> {
    [
        ("blunder", "blunders"),
        ("mistake", "mistakes"),
        ("inaccuracy", "inaccuracies"),
    ]
    .into_iter()
    .filter_map(|(judgment, plural)| {
        let count = items
            .iter()
            .filter(|item| item["judgment"] == json!(judgment))
            .count();
        (count > 0).then(|| {
            (
                judgment,
                format!("{count} {}", if count == 1 { judgment } else { plural }),
            )
        })
    })
    .collect()
}

pub struct Mistakes {
    items: Vec<Value>,
    games: Vec<Value>,
    error: Option<String>,
    has_accounts: bool,
    /// The drill on the board, or none on the home view.
    current: Option<String>,
    done: bool,
    show_all: bool,
    now: f64,
    puzzle: Entity<PuzzleBoard>,
    layout: PlayLayout,
    _clock: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Mistakes {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Mistakes {
        let puzzle = cx.new(|cx| PuzzleBoard::new(false, window, cx));
        let subscriptions =
            vec![cx.subscribe(
                &puzzle,
                |this, _, event: &PuzzleBoardEvent, cx| match event {
                    PuzzleBoardEvent::Outcome { win, .. } => {
                        if let Some(id) = this.current.clone() {
                            this.answered(&id, *win, cx);
                        }
                    }
                    PuzzleBoardEvent::Done => {
                        this.done = true;
                        cx.notify();
                    }
                    PuzzleBoardEvent::Move { .. } => {}
                },
            )];
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK).await;
                if this
                    .update(cx, |this, cx| {
                        this.now = now();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut trainer = Mistakes {
            items: Vec::new(),
            games: Vec::new(),
            error: None,
            has_accounts: false,
            current: None,
            done: false,
            show_all: false,
            now: now(),
            puzzle,
            layout: PlayLayout::default(),
            _clock: clock,
            _subscriptions: subscriptions,
        };
        trainer.shown(cx);
        trainer
    }

    /// The tab was opened (`onMounted`): the overview, with the drills and games as they are now.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        self.current = None;
        self.done = false;
        self.show_all = false;
        self.now = now();
        let core = cx.global::<AppCore>().clone();
        let (library, games, data) = (
            core.call("library", Vec::new()),
            core.call("mistakeGames", Vec::new()),
            core.call("loadData", Vec::new()),
        );
        cx.spawn(async move |this, cx| {
            let (library, games, data) = (library.await, games.await, data.await);
            let _ = this.update(cx, |this, cx| {
                match library {
                    Ok(library) => {
                        this.items = library["mistakes"].as_array().cloned().unwrap_or_default();
                    }
                    Err(cause) => this.error = Some(cause.message),
                }
                match games {
                    Ok(games) => this.games = games.as_array().cloned().unwrap_or_default(),
                    Err(cause) => crate::log::write(
                        Level::Warn,
                        "mistake-trainer",
                        &format!("Loading the games failed: {}", cause.message),
                    ),
                }
                if let Ok(data) = data {
                    this.has_accounts = data["accounts"]
                        .as_array()
                        .is_some_and(|accounts| !accounts.is_empty());
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn due(&self) -> Vec<&Value> {
        self.items
            .iter()
            .filter(|item| due_at(item) <= self.now)
            .collect()
    }

    /// The next position: the one due longest, or with `all` the next one at all (Review early).
    fn next(&mut self, all: bool, cx: &mut Context<Self>) {
        let mut candidates: Vec<&Value> = if all {
            self.items.iter().collect()
        } else {
            self.due()
        };
        candidates.sort_by(|a, b| due_at(a).total_cmp(&due_at(b)));
        let current = self.current.as_deref();
        let item = candidates
            .iter()
            .find(|item| self.done || Some(id_of(item)) != current)
            .map(|item| (*item).clone());
        // The position on the board is the only one left: keep it rather than blank the board.
        if item.is_none() && candidates.iter().any(|item| Some(id_of(item)) == current) {
            return;
        }
        self.done = false;
        self.current = item.as_ref().map(|item| id_of(item).to_string());
        if let Some(item) = item {
            self.puzzle
                .update(cx, |board, cx| board.start(&puzzle(&item), None, cx));
        }
        cx.notify();
    }

    /// Schedule the drill again: sooner when it went wrong.
    fn answered(&mut self, id: &str, clean: bool, cx: &mut Context<Self>) {
        let answer = cx
            .global::<AppCore>()
            .call("answerMistake", vec![json!(id), json!(clean)]);
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                match answer {
                    Ok(items) => {
                        this.items = items.as_array().cloned().unwrap_or_default();
                        this.error = None;
                    }
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "mistakes",
                            &format!("saving drills failed: {}", cause.message),
                        );
                        this.error = Some(cause.message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_practice(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let board_size = self.layout.board_size(window);
        let due = self.due().len();
        let hinted = self.puzzle.read(cx).hinted();
        let panel = widgets::side_panel(palette)
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .child(
                v_flex()
                    .child(widgets::heading("Practice your mistakes", palette))
                    .child(widgets::hint(
                        format!("{due} due · {} saved", self.items.len()),
                        palette,
                    )),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(14.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .when(self.done, |panel| {
                panel.child(
                    div()
                        .text_size(px(14.))
                        .child("Position completed. Your next revisit has been scheduled."),
                )
            })
            .when(!self.done, |panel| {
                panel.child(
                    h_flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            Button::new("mistake-hint")
                                .when(hinted, |button| button.primary())
                                .when(!hinted, |button| button.outline())
                                .label("Hint: piece to move")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.puzzle.update(cx, |board, cx| board.hint(cx))
                                })),
                        )
                        .child(
                            Button::new("mistake-solution")
                                .ghost()
                                .label("Show solution")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.puzzle.update(cx, |board, cx| board.show_solution(cx))
                                })),
                        ),
                )
            })
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        Button::new("mistake-next")
                            .primary()
                            .label("Next due position")
                            .disabled(due == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.next(false, cx))),
                    )
                    .child(
                        Button::new("mistake-early")
                            .outline()
                            .label("Review early")
                            .on_click(cx.listener(|this, _, _, cx| this.next(true, cx))),
                    ),
            );
        let board_column = div().w(board_size).flex_none().child(self.puzzle.clone());
        div()
            .size_full()
            .child(
                self.layout
                    .render("mistakes", board_column, panel, window, cx),
            )
            .into_any_element()
    }

    fn render_games(&self, cx: &mut Context<Self>, palette: &Palette) -> Option<impl IntoElement> {
        if self.games.is_empty() {
            return None;
        }
        let shown = if self.show_all {
            self.games.len()
        } else {
            GAMES_SHOWN
        };
        let rows = self
            .games
            .iter()
            .take(shown)
            .enumerate()
            .map(|(index, game)| {
                let key = game["key"].as_str().unwrap_or_default();
                let items: Vec<&Value> = self
                    .items
                    .iter()
                    .filter(|item| id_of(item).starts_with(&format!("{key}:")))
                    .collect();
                let due = items.iter().filter(|item| due_at(item) <= self.now).count();
                let positions = items.len();
                let sub = format!(
                    "{} · {positions} {}{}",
                    crate::game::game_date(game["playedAt"].as_f64().unwrap_or_default()),
                    if positions == 1 {
                        "position"
                    } else {
                        "positions"
                    },
                    if due > 0 {
                        format!(" · {due} due")
                    } else {
                        String::new()
                    }
                );
                let side = match game["player"].as_str() {
                    Some("white") => Some(Color::White),
                    Some("black") => Some(Color::Black),
                    _ => None,
                };
                h_flex()
                    .gap(px(10.))
                    .py(px(8.))
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(palette.border)
                    })
                    // Your side; the slot stays when it is not known, so the titles line up.
                    .child(match side {
                        Some(side) => widgets::side_dot(side),
                        None => div().size(px(9.)).flex_none(),
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::BOLD)
                                    .child(title(game)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(palette.text_muted)
                                    .child(sub),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .justify_end()
                            .gap_x(px(10.))
                            .gap_y(px(2.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .children(judgment_counts(&items).into_iter().map(
                                |(judgment, text)| {
                                    div()
                                        .text_color(judgment_color(judgment, palette.dark))
                                        .child(text)
                                },
                            )),
                    )
            });
        Some(
            widgets::card(palette)
                .child(
                    h_flex()
                        .justify_between()
                        .gap(px(8.))
                        .child(widgets::heading("Games", palette))
                        .when(self.has_accounts, |head| {
                            head.child(
                                Button::new("mistakes-insights")
                                    .link()
                                    .xsmall()
                                    .icon(icon("chart-pie"))
                                    .label("Lichess insights")
                                    .on_click(|_, window, cx| {
                                        crate::shell::open(Page::Insights, window, cx)
                                    }),
                            )
                        }),
                )
                .child(v_flex().mt(px(8.)).children(rows))
                .when(self.games.len() > GAMES_SHOWN && !self.show_all, |card| {
                    card.child(
                        h_flex().child(
                            Button::new("mistakes-all-games")
                                .link()
                                .xsmall()
                                .label(format!("Show all {} games", self.games.len()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_all = true;
                                    cx.notify();
                                })),
                        ),
                    )
                }),
        )
    }

    fn render_home(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let due = self.due().len();
        let learned = self
            .items
            .iter()
            .filter(|item| item["streak"].as_f64().unwrap_or(0.) >= LEARNED_STREAK)
            .count();
        let stat = |label: &'static str, value: usize| {
            v_flex()
                .flex_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_muted)
                        .child(label),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .text_size(px(26.))
                        .font_weight(FontWeight(750.))
                        .child(value.to_string()),
                )
        };
        let overview = widgets::card(palette)
            .child(widgets::heading("Practice your mistakes", palette))
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(14.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .when(self.items.is_empty(), |card| {
                card.child(
                    widgets::empty_state(
                        icon("book-open"),
                        "No mistakes yet",
                        Some(
                            "When you review one of your games, its mistakes are added here."
                                .into(),
                        ),
                        palette,
                    )
                    .py(px(24.))
                    .child(
                        Button::new("review-a-game")
                            .primary()
                            .icon(icon("history"))
                            .label("Review a game")
                            .on_click(|_, window, cx| {
                                crate::shell::open(Page::History, window, cx)
                            }),
                    ),
                )
            })
            .when(!self.items.is_empty(), |card| {
                card.child(
                    h_flex()
                        .mt(px(12.))
                        .max_w(px(420.))
                        .gap(px(12.))
                        .child(stat("Due now", due))
                        .child(stat("Learning", self.items.len() - learned))
                        .child(stat("Learned", learned)),
                )
                .when(due == 0, |card| {
                    card.child(
                        div()
                            .mt(px(12.))
                            .child(widgets::hint("You're caught up.", palette)),
                    )
                })
                .child(
                    h_flex()
                        .mt(px(12.))
                        .gap(px(8.))
                        .child(
                            Button::new("practice-due")
                                .primary()
                                .label("Practice due positions")
                                .disabled(due == 0)
                                .on_click(cx.listener(|this, _, _, cx| this.next(false, cx))),
                        )
                        .child(
                            Button::new("practice-early")
                                .outline()
                                .label("Review early")
                                .on_click(cx.listener(|this, _, _, cx| this.next(true, cx))),
                        ),
                )
            });
        div()
            .id("mistakes-home")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap(px(14.))
                    .px(side_padding(window))
                    .pt(crate::play_layout::PAGE_TOP)
                    .pb(crate::play_layout::PAGE_BOTTOM)
                    .child(overview)
                    .children(self.render_games(cx, palette)),
            )
            .into_any_element()
    }
}

impl Render for Mistakes {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        if self.current.is_some() {
            self.render_practice(window, cx, &palette)
        } else {
            self.render_home(window, cx, &palette)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{judgment_counts, title};
    use serde_json::json;

    #[test]
    fn a_game_is_named_by_your_opponent() {
        let game = json!({ "white": "You", "black": "Stockfish", "player": "white" });
        assert_eq!(title(&game), "vs Stockfish");
        assert_eq!(title(&json!({ "player": "black" })), "You as black");
        assert_eq!(title(&json!({ "white": "A", "black": "B" })), "A – B");
        assert_eq!(title(&json!({})), "Analysis board game");
    }

    #[test]
    fn counts_read_worst_first() {
        let items = [
            json!({ "judgment": "mistake" }),
            json!({ "judgment": "blunder" }),
            json!({ "judgment": "blunder" }),
        ];
        let items: Vec<_> = items.iter().collect();
        let counts: Vec<String> = judgment_counts(&items)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(counts, ["2 blunders", "1 mistake"]);
    }
}
