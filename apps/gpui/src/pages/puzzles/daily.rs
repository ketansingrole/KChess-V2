//! Puzzles → Daily (`PuzzleDaily.vue`, the store's `loadDaily`): Lichess's puzzle of the day on its
//! own board, with the status panel the Train tab uses. A later UTC day brings the next one.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use super::{Theme, status_panel};
use crate::assets::icon;
use crate::core::AppCore;
use crate::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use crate::puzzle_board::PuzzleBoard;
use crate::theme::Palette;
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
    board: Entity<PuzzleBoard>,
    themes: Vec<Theme>,
    layout: PlayLayout,
    _subscriptions: Vec<Subscription>,
}

fn utc_day() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

impl DailyPuzzle {
    pub fn new(themes: Vec<Theme>, window: &mut Window, cx: &mut Context<Self>) -> DailyPuzzle {
        let board = cx.new(|cx| PuzzleBoard::new(true, window, cx));
        DailyPuzzle {
            puzzle: None,
            error: None,
            loading: false,
            day: String::new(),
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
            || (!force && self.error.is_some())
            || (self.puzzle.is_some() && !force && self.day == utc_day())
        {
            return;
        }
        self.loading = true;
        self.error = None;
        cx.notify();
        let day = utc_day();
        let daily = cx.global::<AppCore>().call("puzzleDaily", Vec::new());
        cx.spawn(async move |this, cx| {
            let daily = daily.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match daily {
                    Ok(puzzle) => {
                        this.board
                            .update(cx, |board, cx| board.start(&puzzle, None, cx));
                        this.puzzle = Some(puzzle);
                        this.day = day;
                    }
                    Err(cause) => {
                        crate::log::write(
                            kchess_core::Level::Warn,
                            "puzzles",
                            &format!("loading daily puzzle failed: {}", cause.message),
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
                    "Couldn’t get the daily puzzle",
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
                            Button::new("daily-retry")
                                .primary()
                                .icon(icon("rotate-cw"))
                                .label("Try again")
                                .on_click(cx.listener(|this, _, _, cx| this.load(true, cx))),
                        ),
                )
                .into_any_element(),
                None => gpui_kit::component::h_flex()
                    .gap(px(8.))
                    .text_color(palette.text_muted)
                    .child(icon("loader-circle").size(px(16.)))
                    .child("Fetching today’s puzzle…")
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
                .rounded(px(14.))
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
            .when_some(self.puzzle.as_ref(), |panel, _| {
                panel.child(status_panel(&self.board, &self.themes, None, cx, &palette))
            });
        self.layout
            .render("puzzle-daily", board_column, panel, window, cx)
    }
}
