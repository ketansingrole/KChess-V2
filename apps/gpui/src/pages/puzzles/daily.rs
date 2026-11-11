//! Puzzles → Daily: Lichess's puzzle of the day on its
//! own board, with the status panel the Train tab uses. A later UTC day brings the next one.
//! A lichess.org/training link opens its puzzle here instead, until Today's puzzle.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::calls;
use serde_json::Value;

use super::{Theme, status_panel};
use crate::assets::icon;
use crate::core::AppCore;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::puzzle_board::PuzzleBoard;
use crate::theme::{Palette, radii};
use crate::widgets;

/// The page's way out when the daily puzzle cannot be fetched.
pub enum DailyEvent {
    UseDownloaded,
}

impl EventEmitter<DailyEvent> for DailyPuzzle {}

pub struct DailyPuzzle {
    puzzle: Option<Value>,
    error: Option<String>,
    loading: bool,
    /// The UTC day the puzzle was fetched on.
    day: String,
    /// The id of a linked puzzle shown instead of the daily one.
    linked: Option<String>,
    /// The latest fetch: an answer to an earlier one is dropped.
    request: u64,
    board: Entity<PuzzleBoard>,
    themes: Vec<Theme>,
    layout: PlayLayout,
    _subscriptions: Vec<Subscription>,
}

fn utc_day() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

impl DailyPuzzle {
    /// The board, while it shows a puzzle.
    pub(super) fn shown_board(&self) -> Option<Entity<PuzzleBoard>> {
        self.puzzle.as_ref().map(|_| self.board.clone())
    }

    pub fn new(themes: Vec<Theme>, window: &mut Window, cx: &mut Context<Self>) -> DailyPuzzle {
        let board = cx.new(|cx| PuzzleBoard::new(true, window, cx));
        DailyPuzzle {
            puzzle: None,
            error: None,
            loading: false,
            day: String::new(),
            linked: None,
            request: 0,
            _subscriptions: vec![cx.observe(&board, |_, _, cx| cx.notify())],
            board,
            themes,
            layout: PlayLayout::default(),
        }
    }

    /// `loadDaily`: fetch today's puzzle unless it is already shown (`force` fetches again).
    pub fn load(&mut self, force: bool, cx: &mut Context<Self>) {
        // A failed fetch stays on screen for Try again: the page draws this on every frame.
        if self.loading
            || self.linked.is_some()
            || (!force && self.error.is_some())
            || (self.puzzle.is_some() && !force && self.day == utc_day())
        {
            return;
        }
        self.loading = true;
        self.error = None;
        cx.notify();
        let day = utc_day();
        let daily = cx.global::<AppCore>().request(calls::PuzzleDaily {});
        self.fetch(daily, day, cx);
    }

    /// Show the puzzle a lichess.org/training link names.
    pub fn open(&mut self, id: String, cx: &mut Context<Self>) {
        self.linked = Some(id.clone());
        self.loading = true;
        self.error = None;
        self.puzzle = None;
        cx.notify();
        let puzzle = cx.global::<AppCore>().request(calls::PuzzleById {
            id: kchess_contracts::check::PuzzleId(id),
        });
        self.fetch(puzzle, String::new(), cx);
    }

    /// Back to the puzzle of the day.
    fn today(&mut self, cx: &mut Context<Self>) {
        self.linked = None;
        self.puzzle = None;
        // A linked puzzle still on its way is dropped when it arrives (`request`).
        self.loading = false;
        self.load(true, cx);
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        match self.linked.clone() {
            Some(id) => self.open(id, cx),
            None => self.load(true, cx),
        }
    }

    fn fetch(
        &mut self,
        request: impl std::future::Future<
            Output = Result<kchess_contracts::types::Puzzle, kchess_contracts::CoreError>,
        > + 'static,
        day: String,
        cx: &mut Context<Self>,
    ) {
        self.request += 1;
        let asked = self.request;
        cx.spawn(async move |this, cx| {
            let daily = request.await;
            let _ = this.update(cx, |this, cx| {
                // An answer for a puzzle no longer asked for.
                if this.request != asked {
                    return;
                }
                this.loading = false;
                match daily {
                    Ok(puzzle) => {
                        let puzzle = serde_json::json!(puzzle);
                        this.board
                            .update(cx, |board, cx| board.start(&puzzle, None, cx));
                        this.puzzle = Some(puzzle);
                        this.day = day;
                    }
                    Err(cause) => {
                        crate::log::write(
                            kchess_contracts::Level::Warn,
                            "puzzles",
                            &format!("loading a puzzle failed: {}", cause.message),
                        );
                        this.error = Some(cause.message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for DailyPuzzle {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.load(false, cx);
        let palette = *cx.global::<Palette>();
        let board_size = self.layout.board_size(window);
        let board_column = if self.puzzle.is_some() {
            div()
                .w(board_size)
                .flex_none()
                .child(self.board.clone())
                .into_any_element()
        } else {
            let content = match &self.error {
                Some(error) => widgets::empty_state(
                    icon("triangle-alert"),
                    if self.linked.is_some() {
                        "Couldn’t get the puzzle"
                    } else {
                        "Couldn’t get the daily puzzle"
                    },
                    Some(error.clone().into()),
                    &palette,
                )
                .child(
                    gpui_kit::component::h_flex()
                        .mt(px(12.))
                        .gap(px(8.))
                        .child(
                            Button::new("daily-downloaded")
                                .outline()
                                .label("Use downloaded puzzles")
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(DailyEvent::UseDownloaded)),
                                ),
                        )
                        .child(
                            crate::widgets::try_again("daily-retry")
                                .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                        ),
                )
                .into_any_element(),
                None => gpui_kit::component::h_flex()
                    .gap(px(8.))
                    .text_color(palette.text_muted)
                    .child(crate::widgets::spinner(px(16.)))
                    .child(if self.linked.is_some() {
                        "Fetching the puzzle…"
                    } else {
                        "Fetching today’s puzzle…"
                    })
                    .into_any_element(),
            };
            v_flex()
                .w(board_size)
                .h(board_size)
                .flex_none()
                .items_center()
                .justify_center()
                .p(px(24.))
                .border_1()
                .border_color(palette.border)
                .rounded(radii::XL)
                .bg(palette.elevated)
                .child(content)
                .into_any_element()
        };
        let panel = widgets::side_panel(&palette)
            .h(board_size)
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .gap(px(12.))
            .when(self.linked.is_some(), |panel| {
                panel.child(
                    Button::new("daily-today")
                        .ghost()
                        .small()
                        .icon(icon("calendar-days"))
                        .label("Today’s puzzle")
                        .on_click(cx.listener(|this, _, _, cx| this.today(cx))),
                )
            })
            .when_some(self.puzzle.as_ref(), |panel, _| {
                panel.child(status_panel(
                    &self.board,
                    &self.themes,
                    None,
                    false,
                    cx,
                    &palette,
                ))
            });
        self.layout
            .render("puzzle-daily", board_column, panel, window, cx)
    }
}
