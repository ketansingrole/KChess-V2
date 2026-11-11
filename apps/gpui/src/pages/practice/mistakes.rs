//! Your mistakes: positions from your reviewed games where you went wrong,
//! solved again on a schedule the core keeps (a right answer doubles the wait, a wrong one brings
//! it back in ten minutes), and the games they come from.

use kchess_contracts::calls;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_contracts::check::LibraryId;
use kchess_contracts::types::{Drill, MistakeGame};
use serde_json::{Value, json};

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

/// A drill as a puzzle for the board.
fn puzzle(item: &Drill) -> Value {
    json!({
        "id": item.id,
        "fen": item.fen,
        "solution": item.solution,
        "variant": item.variant,
        "rating": 0,
        "themes": [],
    })
}

/// A game's title: your opponent when your side is known, else the players.
fn title(game: &MistakeGame) -> String {
    let (white, black) = (
        game.white.as_deref().unwrap_or_default(),
        game.black.as_deref().unwrap_or_default(),
    );
    match game.player.map(|player| player.fold_wb("white", "black")) {
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

/// How many of a game's drills have each judgment.
fn judgment_counts(items: &[&Drill]) -> Vec<(&'static str, String)> {
    let count = |judgment: &str| {
        items
            .iter()
            .filter(|item| item.judgment == judgment)
            .count()
    };
    crate::reviews::judgment_counts(&json!({
        "blunder": count("blunder"),
        "mistake": count("mistake"),
        "inaccuracy": count("inaccuracy"),
    }))
}

/// The mistakes trainer's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MistakeAction {
    Flip,
    Hint,
    Solution,
    /// The next due position, from a drill.
    Next,
    /// The due positions, from the overview.
    Due,
    Early,
}

pub struct Mistakes {
    items: Vec<Drill>,
    /// Until the saved drills arrive, "no mistakes yet" would not be true.
    loading: bool,
    /// Each load of the overview; an answer to an earlier one is dropped.
    load: u64,
    games: Vec<MistakeGame>,
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
        let clock = crate::timer::every(CLOCK, cx, |this, cx| {
            this.now = kchess_games::now_ms();
            cx.notify();
            true
        });
        let mut trainer = Mistakes {
            items: Vec::new(),
            loading: false,
            load: 0,
            games: Vec::new(),
            error: None,
            has_accounts: false,
            current: None,
            done: false,
            show_all: false,
            now: kchess_games::now_ms(),
            puzzle,
            layout: PlayLayout::default(),
            _clock: clock,
            _subscriptions: subscriptions,
        };
        trainer.shown(cx);
        trainer
    }

    /// The tab was opened: the overview, with the drills and games as they are now.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        self.current = None;
        self.done = false;
        self.show_all = false;
        self.now = kchess_games::now_ms();
        self.loading = true;
        self.load += 1;
        let load = self.load;
        let core = cx.global::<AppCore>().clone();
        let (library, games, data) = (
            core.request(calls::Library {}),
            core.request(calls::MistakeGames {}),
            core.request(calls::LoadData {}),
        );
        cx.spawn(async move |this, cx| {
            let (library, games, data) = (library.await, games.await, data.await);
            let _ = this.update(cx, |this, cx| {
                if load != this.load {
                    return;
                }
                this.loading = false;
                match library {
                    // A failure to load is the last load's: one that worked clears it.
                    Ok(library) => {
                        this.items = library.mistakes;
                        this.error = None;
                    }
                    Err(cause) => this.error = Some(cause.message),
                }
                match games {
                    Ok(games) => this.games = games,
                    Err(cause) => crate::log::write(
                        Level::Warn,
                        "mistake-trainer",
                        &format!("Loading the games failed: {}", cause.message),
                    ),
                }
                if let Ok(data) = data {
                    this.has_accounts = !data.accounts.is_empty();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn due(&self) -> Vec<&Drill> {
        self.items
            .iter()
            .filter(|item| item.due_at_ms() <= self.now)
            .collect()
    }

    /// The board the drills are played on.
    #[cfg(test)]
    pub(crate) fn board(&self) -> &Entity<PuzzleBoard> {
        &self.puzzle
    }

    #[cfg(test)]
    pub(crate) fn seed(&mut self, items: Vec<Value>, cx: &mut Context<Self>) {
        // A drill as the core lists it, with what a test leaves out filled in.
        self.items = items
            .into_iter()
            .map(|item| {
                let mut drill =
                    json!({ "judgment": "mistake", "dueAt": 0, "streak": 0, "attempts": 0 });
                for (key, value) in item.as_object().into_iter().flatten() {
                    drill[key] = value.clone();
                }
                serde_json::from_value(drill).expect("a drill")
            })
            .collect();
        cx.notify();
    }

    /// What the trainer's buttons can do now: the drill's, or the overview's.
    pub(crate) fn actions(&self) -> Vec<MistakeAction> {
        let due = !self.due().is_empty();
        let mut actions = Vec::new();
        if self.current.is_some() {
            if !self.done {
                actions.extend([MistakeAction::Hint, MistakeAction::Solution]);
            }
            actions.push(MistakeAction::Flip);
            if due {
                actions.push(MistakeAction::Next);
            }
        } else if self.items.is_empty() {
            return actions;
        } else if due {
            actions.push(MistakeAction::Due);
        }
        actions.push(MistakeAction::Early);
        actions
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: MistakeAction, cx: &mut Context<Self>) {
        if !self.actions().contains(&action) {
            return;
        }
        match action {
            MistakeAction::Flip => self.puzzle.update(cx, |board, cx| board.flip(cx)),
            MistakeAction::Hint => self.puzzle.update(cx, |board, cx| board.hint(cx)),
            MistakeAction::Solution => self.puzzle.update(cx, |board, cx| board.show_solution(cx)),
            MistakeAction::Next | MistakeAction::Due => self.next(false, cx),
            MistakeAction::Early => self.next(true, cx),
        }
    }

    /// The next position: the one due longest, or with `all` the next one at all (Review early).
    fn next(&mut self, all: bool, cx: &mut Context<Self>) {
        let mut candidates: Vec<&Drill> = if all {
            self.items.iter().collect()
        } else {
            self.due()
        };
        candidates.sort_by(|a, b| a.due_at_ms().total_cmp(&b.due_at_ms()));
        let current = self.current.as_deref();
        let item = candidates
            .iter()
            .find(|item| self.done || Some(item.id.as_str()) != current)
            .map(|item| (*item).clone());
        // The position on the board is the only one left: keep it rather than blank the board.
        if item.is_none()
            && candidates
                .iter()
                .any(|item| Some(item.id.as_str()) == current)
        {
            return;
        }
        self.done = false;
        self.current = item.as_ref().map(|item| item.id.clone());
        if let Some(item) = item {
            self.puzzle
                .update(cx, |board, cx| board.start(&puzzle(&item), None, cx));
        }
        cx.notify();
    }

    /// Schedule the drill again: sooner when it went wrong.
    fn answered(&mut self, id: &str, clean: bool, cx: &mut Context<Self>) {
        let answer = cx.global::<AppCore>().request(calls::AnswerMistake {
            id: LibraryId(id.to_string()),
            solved: clean,
        });
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                match answer {
                    Ok(items) => {
                        this.items = items;
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

    /// A key for the drill on the board: once it is over Return or Space goes on to the next due
    /// position (as on Puzzles), and the arrows, Home and End step through its moves.
    /// Whether the key was taken.
    pub fn key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if self.current.is_none() {
            return false;
        }
        if matches!(key, "enter" | "space")
            && self.done
            && self.actions().contains(&MistakeAction::Next)
        {
            self.next(false, cx);
            return true;
        }
        self.puzzle.update(cx, |board, cx| board.step(key, cx))
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
                let key = game.key.as_str();
                let items: Vec<&Drill> = self
                    .items
                    .iter()
                    .filter(|item| item.id.starts_with(&format!("{key}:")))
                    .collect();
                let due = items
                    .iter()
                    .filter(|item| item.due_at_ms() <= self.now)
                    .count();
                let positions = items.len();
                let sub = format!(
                    "{} · {positions} {}{}",
                    crate::game::game_date(game.played_at),
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
                let side = game.player;
                h_flex()
                    .gap(px(8.))
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
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::MEDIUM)
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
                            .gap_x(px(8.))
                            .gap_y(px(2.))
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .children(judgment_counts(&items).into_iter().map(
                                |(judgment, text)| {
                                    div()
                                        .text_color(judgment_color(judgment, palette.dark))
                                        .child(text)
                                },
                            )),
                    )
                    .child({
                        let (practised, all) = (game.clone(), self.items.clone());
                        Button::new(SharedString::from(format!("practise-game-{index}")))
                            .outline()
                            .small()
                            .label("Practice")
                            .accessibility_label(format!(
                                "Practice {}, {}",
                                title(game),
                                crate::game::game_date(game.played_at)
                            ))
                            .on_click(move |_, window, cx| {
                                crate::pages::analysis::practise_game(
                                    practised.clone(),
                                    all.clone(),
                                    window,
                                    cx,
                                )
                            })
                    })
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
                                    .label("Insights")
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
            .filter(|item| item.streak() >= LEARNED_STREAK)
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
                        .text_size(px(28.))
                        .font_weight(FontWeight::MEDIUM)
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
            .when(self.items.is_empty() && !self.loading, |card| {
                card.child(
                    widgets::empty_state(
                        icon("book-open"),
                        "No mistakes yet",
                        Some(
                            "When you review one of your games, its mistakes are added here".into(),
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
                            .child(widgets::hint("You're caught up", palette)),
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
    use super::{MistakeGame, judgment_counts, title};
    use serde_json::{Value, json};

    /// A failed load's error stayed on the overview after a later load worked.
    #[gpui_kit::test]
    fn a_load_that_works_clears_an_earlier_failure(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = crate::e2e::Harness::start(cx);
        let page = app.with_window(|window, cx| {
            let page = cx.new(|cx| super::Mistakes::new(window, cx));
            page.update(cx, |page, cx| {
                page.error = Some("The library could not be read".into());
                page.shown(cx);
            });
            page
        });
        app.wait_for("the overview to load", |_, cx| !page.read(cx).loading);
        let error = app.with_window(|_, cx| page.read(cx).error.clone());
        assert_eq!(error, None);
    }

    #[test]
    fn a_game_is_named_by_your_opponent() {
        let game = |value: Value| -> MistakeGame {
            let mut game = json!({ "key": "k", "fen": "", "moves": [], "playedAt": 0 });
            game.as_object_mut()
                .expect("an object")
                .extend(value.as_object().cloned().unwrap_or_default());
            serde_json::from_value(game).expect("a game")
        };
        let played = game(json!({ "white": "You", "black": "Stockfish", "player": "white" }));
        assert_eq!(title(&played), "vs Stockfish");
        assert_eq!(title(&game(json!({ "player": "black" }))), "You as black");
        assert_eq!(title(&game(json!({ "white": "A", "black": "B" }))), "A – B");
        assert_eq!(title(&game(json!({}))), "Analysis board game");
    }

    #[test]
    fn counts_read_worst_first() {
        let drill = |judgment: &str| -> kchess_contracts::types::Drill {
            serde_json::from_value(json!({
                "id": "k:1", "fen": "", "solution": [], "judgment": judgment,
                "dueAt": 0, "streak": 0, "attempts": 0,
            }))
            .expect("a drill")
        };
        let items = [drill("mistake"), drill("blunder"), drill("blunder")];
        let items: Vec<_> = items.iter().collect();
        let counts: Vec<String> = judgment_counts(&items)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        assert_eq!(counts, ["2 blunders", "1 mistake"]);
    }
}
