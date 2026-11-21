//! Puzzles → Daily: Lichess's puzzle of the day on its
//! own board, with the status panel the Train tab uses. A later UTC day brings the next one.
//! A lichess.org/training link opens its puzzle here instead, until Today's puzzle.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::types::Puzzle;

use super::{Theme, board_and_panel, status_panel};
use crate::core::AppCore;
use crate::puzzle_board::PuzzleBoard;
use crate::theme::{Palette, radii};
use crate::widgets;
use kchess_nav::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout};
use kchess_ui::icon;

/// The page's way out when the daily puzzle cannot be fetched.
pub enum DailyEvent {
    UseDownloaded,
}

impl EventEmitter<DailyEvent> for DailyPuzzle {}

pub struct DailyPuzzle {
    puzzle: Option<Puzzle>,
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
    /// Fetches the next day's puzzle once the day the shown one is from ends.
    rollover: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn utc_day() -> String {
    jiff::Timestamp::now().strftime("%Y-%m-%d").to_string()
}

/// How long until the UTC day ends, and Lichess has a new puzzle of the day.
fn until_tomorrow() -> std::time::Duration {
    const DAY: i64 = 24 * 60 * 60;
    let into = jiff::Timestamp::now().as_second().rem_euclid(DAY);
    std::time::Duration::from_secs((DAY - into).unsigned_abs())
}

impl DailyPuzzle {
    /// The board, while it shows a puzzle.
    pub(super) fn shown_board(&self) -> Option<Entity<PuzzleBoard>> {
        self.puzzle.as_ref().map(|_| self.board.clone())
    }

    pub fn new(themes: Vec<Theme>, window: &mut Window, cx: &mut Context<Self>) -> DailyPuzzle {
        let board = cx.new(|cx| PuzzleBoard::new(true, window, cx));
        let mut daily = DailyPuzzle {
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
            rollover: None,
        };
        daily.load(false, cx);
        daily
    }

    /// Fetch today's puzzle unless it is already shown (`force` fetches again): when the view is
    /// made, when the Puzzles page is shown again, and when the day ends.
    pub fn load(&mut self, force: bool, cx: &mut Context<Self>) {
        // A failed fetch stays on screen for Try again.
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

    /// Once the day the puzzle is from ends, the next day's.
    fn next_day(&mut self, cx: &mut Context<Self>) {
        let wait = until_tomorrow();
        self.rollover = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| this.load(false, cx)).ok();
        }));
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
                        this.board
                            .update(cx, |board, cx| board.start(puzzle.clone(), None, cx));
                        this.puzzle = Some(puzzle);
                        if !day.is_empty() {
                            this.day = day;
                            this.next_day(cx);
                        }
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
        let palette = *cx.global::<Palette>();
        let (board_size, panel_height) = board_and_panel(&self.layout, &self.board, window, cx);
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
            .h(panel_height)
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gpui_kit::AppContext as _;

    /// Drawing the puzzle of the day never fetches it: it is fetched when the view is made and
    /// again when the day ends. Drawing it once fetched it on every frame, and so only when
    /// something drew: a window left open overnight kept yesterday's.
    #[gpui_kit::test]
    fn the_next_days_puzzle_comes_when_the_day_ends_not_when_drawn(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let asked = Arc::new(AtomicUsize::new(0));
        let server = {
            let asked = Arc::clone(&asked);
            kchess_testkit::Server::start(move |request, out| {
                if request.path() != "/api/puzzle/daily" {
                    return kchess_testkit::respond(out, 404, "text/plain", b"");
                }
                asked.fetch_add(1, Ordering::SeqCst);
                let puzzle = serde_json::json!({
                    "game": { "id": "KcGame01" },
                    "puzzle": {
                        "id": "KcDay",
                        "fen": "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
                        "lastMove": "g8f6",
                        "solution": ["h5f7"],
                        "rating": 1100,
                        "themes": ["mateIn1", "short"],
                    },
                });
                kchess_testkit::respond(out, 200, "application/json", puzzle.to_string().as_bytes())
            })
            .expect("a stand-in Lichess")
        };
        let mut app =
            kchess_board::testing::Started::with_lichess(cx, Some(server.base().to_string()));
        let (window, daily) = app.with_window(|_, cx| {
            kchess_board::ai::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| super::DailyPuzzle::new(Vec::new(), window, cx))
            })
            .expect("a window")
        });
        app.wait_for("the puzzle of the day", |_, cx| {
            daily.read(cx).puzzle.is_some()
        });
        assert_eq!(asked.load(Ordering::SeqCst), 1);

        // The puzzle shown is yesterday's; drawing it asks for nothing.
        app.with_window(|_, cx| daily.update(cx, |daily, _| daily.day = "2000-01-01".into()));
        for _ in 0..3 {
            app.cx()
                .update_window(window, |_, window, _| window.refresh())
                .expect("the window");
            app.cx().run_until_parked();
        }
        assert_eq!(asked.load(Ordering::SeqCst), 1);

        // The day ends: the next day's is fetched.
        app.cx()
            .executor()
            .advance_clock(super::until_tomorrow() + std::time::Duration::from_secs(1));
        app.wait_for("the next day's puzzle", |_, _| {
            asked.load(Ordering::SeqCst) == 2
        });
        app.wait_for("the day to follow", |_, cx| {
            daily.read(cx).day == super::utc_day()
        });
    }
}
