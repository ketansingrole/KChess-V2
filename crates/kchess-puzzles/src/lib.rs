//! The Puzzles page: the Train tab with its three modes: Lichess practice, the downloaded database, and
//! rated puzzles. The session's transitions are Rust (`game/puzzle.rs`); this page fetches what
//! they ask for, plays the puzzle on the board and keeps the attempt log up to date.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_app::log::ResultExt as _;
use kchess_hack as _;

use kchess_nav::page::PageView;
// The app's services, the design system and the board, under the names this page's modules use.
use kchess_app::{accounts, core, loading, log, network, prefs, quit, settings, timer};
use kchess_board::{ai, board, game, puzzle_board, puzzle_db, rating_history, sound, widgets};
use kchess_contracts::calls;
use kchess_contracts::types::{AiKind, AiRequest};
use kchess_ui::{chart, dates, theme};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::searchable_list::{
    SearchableGroup, SearchableListDelegate as _, SearchableVec,
};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, Icon, IndexPath, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use kchess_contracts::types::{
    Puzzle, PuzzleAttempt, PuzzleAttemptOutcome, PuzzleProgress, PuzzleProgressQuery,
};
use kchess_domain::records::accounts::{ListedAccount, connected_account_index};
use serde_json::Value;
use shakmaty::Color;

use crate::core::AppCore;
use crate::game::puzzle::{Phase, PuzzleSession, PuzzleStatus, Selection, Step, local_day};
use crate::game::surface::TakesKeys as _;
use crate::puzzle_board::{Feedback, PuzzleBoard, PuzzleBoardEvent};
use crate::puzzle_db::PuzzleDbHandle;
use crate::theme::{Palette, radii, tint};
use kchess_domain::trainer::puzzle_session::{Answer, Detail, Failure, Reply, Request, Resume};
use kchess_nav::play_layout::{PANEL_MAX, PANEL_MIN, PlayLayout, side_padding};
use kchess_ui::icon;
mod daily;
mod history;
mod panel;
mod rush;
pub use rush::RushAction;
mod stats;
mod themes;
mod train;
mod train_view;

use panel::status_panel;
use themes::*;
use train::{Settle, as_contract, train_theme};

use crate::widgets::Pressable as _;
use crate::widgets::{Choice, Pill, Tone};
use kchess_nav::page::{PageTab, TabInfo};

/// A puzzle board's side, leaving room for the rows it shows around it (material, the move box),
/// and the height of the panel beside it, which ends level with those rows. Every tab with a
/// puzzle board sizes it here: a panel as tall as the board alone stopped short of the move box.
fn board_and_panel(
    layout: &PlayLayout,
    board: &Entity<PuzzleBoard>,
    window: &Window,
    cx: &App,
) -> (Pixels, Length) {
    let around = board.read(cx).around();
    let side = layout.board_size_around(px(0.), px(0.), around, window);
    (side, layout.fixed(side + around))
}

/// Puzzles' tabs, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Train,
    Daily,
    Rush,
    Stats,
    History,
}

impl PageTab for Tab {
    const ALL: &'static [Tab] = &[Tab::Train, Tab::Daily, Tab::Rush, Tab::Stats, Tab::History];
    const KEY: &'static str = "kchess:puzzle-tab";

    fn info(self) -> TabInfo {
        let (key, label, title, icon, keywords) = match self {
            Tab::Train => (
                "train",
                "Train",
                "Puzzle training",
                "puzzle",
                "tactics solve themes motifs mate patterns",
            ),
            Tab::Daily => ("daily", "Daily", "Daily puzzle", "calendar-days", "today"),
            Tab::Rush => (
                "rush",
                "Storm · Streak · Rush",
                "Puzzle Storm, Streak and Rush",
                "zap",
                "timed race survival",
            ),
            Tab::Stats => (
                "stats",
                "Lichess stats",
                "Puzzle stats",
                "chart-column",
                "rating progress",
            ),
            Tab::History => (
                "history",
                "History",
                "Puzzle history",
                "history",
                "solved attempts",
            ),
        };
        TabInfo {
            key,
            label,
            title,
            icon,
            keywords,
        }
    }
}

const MODE: &str = "kchess:puzzle-mode";
const ANGLE: &str = "kchess:puzzle-angle";
const DIFFICULTY: &str = "kchess:puzzle-difficulty";
const COLOR: &str = "kchess:puzzle-color";
const ACCOUNT: &str = "kchess:puzzle-account";
/// The opening an opening page chose puzzles of: its key is the angle while it is chosen.
const OPENING: &str = "kchess:puzzle-opening";

/// A pause on the final position before the next puzzle replaces it.
const AUTO_NEXT_DELAY: Duration = Duration::from_millis(800);

const MODES: [(&str, &str); 3] = [
    ("practice", "Practice"),
    ("offline", "Downloaded"),
    ("rated", "Rated"),
];
const DIFFICULTIES: [(&str, &str); 5] = [
    ("easiest", "Easiest"),
    ("easier", "Easier"),
    ("normal", "Normal"),
    ("harder", "Harder"),
    ("hardest", "Hardest"),
];
const COLORS: [(&str, &str); 3] = [("random", "Random"), ("white", "White"), ("black", "Black")];

/// What each mode means for the result, where the choice is made.
fn mode_hint(mode: &str) -> &'static str {
    match mode {
        "offline" => "Puzzles downloaded to this computer",
        "rated" => "Results affect your Lichess rating",
        _ => "Lichess puzzles, unrated",
    }
}

/// What the puzzle panel's buttons do, for Quick Pick to offer the same.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    Hint,
    Solution,
    Flip,
    /// A finished puzzle on the analysis board.
    Analyse,
    Next,
    /// A Storm, Streak or Rush run's own buttons.
    Rush(RushAction),
}

pub struct PuzzlesPage {
    session: PuzzleSession,
    /// The puzzle key the board shows (`:key="puzzleKey"`): a new key starts the board afresh.
    shown_key: u64,
    puzzle: Entity<PuzzleBoard>,
    layout: PlayLayout,
    theme_list: Vec<Theme>,
    theme_select: Entity<SelectState<ThemeChoices>>,
    difficulty_select: Entity<SelectState<Vec<SharedString>>>,
    /// The account rated puzzles go to, while more than one is connected.
    account_select: Entity<SelectState<Vec<Choice>>>,
    color_select: Entity<SelectState<Vec<SharedString>>>,
    timers: Vec<Task<()>>,
    /// The Daily and History tabs, made when first shown, and the tab shown last.
    daily: Option<Entity<daily::DailyPuzzle>>,
    history: Option<Entity<history::PuzzleHistory>>,
    stats: Option<Entity<stats::PuzzleStats>>,
    rush: Option<Entity<rush::PuzzleRush>>,
    shown_tab: String,
    progress: PuzzleProgress,
    started: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for PuzzlesPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn select_of(
    items: Vec<SharedString>,
    chosen: Option<usize>,
    window: &mut Window,
    cx: &mut Context<PuzzlesPage>,
) -> Entity<SelectState<Vec<SharedString>>> {
    cx.new(|cx| SelectState::new(items, chosen.map(IndexPath::new), window, cx))
}

impl PuzzlesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> PuzzlesPage {
        let theme_list = theme_items(cx);
        let angle = crate::prefs::get::<String>(cx, ANGLE, "mix".into());
        let difficulty = crate::prefs::get::<String>(cx, DIFFICULTY, "normal".into());
        let color = crate::prefs::get::<String>(cx, COLOR, "random".into());
        let theme_select = cx.new(|cx| {
            let choices = theme_choices(cx);
            let chosen = choices
                .position(&SharedString::from(angle.clone()))
                .or(Some(IndexPath::default()));
            SelectState::new(choices, chosen, window, cx).searchable(true)
        });
        let difficulty_select = select_of(
            DIFFICULTIES
                .iter()
                .map(|(_, label)| SharedString::from(*label))
                .collect(),
            DIFFICULTIES
                .iter()
                .position(|(key, _)| *key == difficulty)
                .or(Some(2)),
            window,
            cx,
        );
        let color_select = select_of(
            COLORS
                .iter()
                .map(|(_, label)| SharedString::from(*label))
                .collect(),
            COLORS.iter().position(|(key, _)| *key == color).or(Some(0)),
            window,
            cx,
        );
        let puzzle = cx.new(|cx| {
            let mut board = PuzzleBoard::new(true, window, cx);
            board.take_next();
            board
        });
        let puzzle_db = cx.global::<PuzzleDbHandle>().0.clone();
        let account_select = cx.new(|cx| SelectState::new(Vec::<Choice>::new(), None, window, cx));
        let subscriptions = vec![
            // The tab is a view preference: chosen here, from Quick Pick or from another page.
            cx.observe_global_in::<kchess_app::prefs::ViewPreferences>(
                window,
                |this, window, cx| this.show_tab(window, cx),
            ),
            cx.subscribe(
                &account_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(name)) = event {
                        crate::prefs::set(cx, ACCOUNT, name.to_string());
                        this.load_next(cx);
                    }
                },
            ),
            cx.subscribe(
                &puzzle,
                |this, _, event: &PuzzleBoardEvent, cx| match event {
                    PuzzleBoardEvent::Move {
                        index,
                        correct,
                        uci,
                    } => {
                        let step = this.session.played(*index, *correct, Some(uci.as_str()));
                        this.follow(step, Settle::Load, cx);
                    }
                    PuzzleBoardEvent::Outcome { win, detail } => this.report(*win, *detail, cx),
                    PuzzleBoardEvent::Done => this.done(cx),
                    PuzzleBoardEvent::Next => this.act(Action::Next, cx),
                },
            ),
            cx.subscribe(
                &theme_select,
                |this, _, event: &SelectEvent<ThemeChoices>, cx| {
                    if let SelectEvent::Confirm(Some(key)) = event {
                        crate::prefs::set(cx, ANGLE, key.to_string());
                        this.load_next(cx);
                    }
                },
            ),
            cx.subscribe(
                &difficulty_select,
                |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                    let row = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(2, |index| index.row);
                    crate::prefs::set(cx, DIFFICULTY, DIFFICULTIES[row.min(4)].0);
                    this.load_next(cx);
                },
            ),
            cx.subscribe(
                &color_select,
                |this, state, _: &SelectEvent<Vec<SharedString>>, cx| {
                    let row = state
                        .read(cx)
                        .selected_index(cx)
                        .map_or(0, |index| index.row);
                    crate::prefs::set(cx, COLOR, COLORS[row.min(2)].0);
                    this.load_next(cx);
                },
            ),
            // Another account (or none): the tabs show it afresh.
            // A reconnected login may now have the puzzle permission (`reconnect`).
            cx.observe_in(
                &crate::accounts::handle(cx),
                window,
                |this, accounts, window, cx| {
                    this.shown_tab.clear();
                    this.sync_accounts(window, cx);
                    this.show_tab(window, cx);
                    let waiting =
                        matches!(this.session.phase(), Phase::NoAccount | Phase::Reconnect);
                    if waiting && !accounts.read(cx).busy {
                        this.load_next(cx);
                    }
                    cx.notify();
                },
            ),
            // A database just downloaded has puzzles for a session waiting on it.
            cx.observe(&puzzle_db, |this, db, cx| {
                if db.read(cx).installed() && this.session.phase() == Phase::NoDatabase {
                    this.load_next(cx);
                }
                cx.notify();
            }),
        ];
        // A puzzle left unfinished at quit (after a wrong move, say) is logged as another puzzle
        // replacing it would log it.
        let weak = cx.entity().downgrade();
        crate::quit::on_quit(cx, move |cx| {
            let page = weak.upgrade()?;
            page.update(cx, |page, cx| {
                let record = page.session.invalidate(&local_day()).record?;
                let save = cx.global::<AppCore>().request(calls::RecordPuzzleAttempt {
                    attempt: as_contract(&record).ok()?,
                });
                Some(
                    Box::pin(async move { save.await.map(|_| ()).map_err(|cause| cause.message) })
                        as crate::quit::BoxedSave,
                )
            })
        });
        PuzzlesPage {
            session: PuzzleSession::new(),
            shown_key: 0,
            puzzle,
            layout: PlayLayout::default(),
            theme_list,
            theme_select,
            difficulty_select,
            account_select,
            color_select,
            timers: Vec::new(),
            daily: None,
            history: None,
            stats: None,
            rush: None,
            shown_tab: String::new(),
            progress: PuzzleProgress::default(),
            started: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The page was shown: read the accounts, the database and the log, and find a
    /// puzzle if there is none.
    pub fn shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_accounts(window, cx);
        self.show_tab(window, cx);
        // The puzzle of the day may be yesterday's, if the computer slept through midnight.
        if let Some(daily) = &self.daily {
            daily.update(cx, |daily, cx| daily.load(false, cx));
        }
        let core = cx.global::<AppCore>().clone();
        let puzzle_db = cx.global::<PuzzleDbHandle>().0.clone();
        puzzle_db.update(cx, |db, cx| db.refresh(cx));
        let progress = core.request(calls::PuzzleProgress {
            query: PuzzleProgressQuery {
                today: local_day(),
                limit: 10,
            },
        });
        let first = !self.started;
        self.started = true;
        cx.spawn(async move |this, cx| {
            let progress = progress.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(progress) = progress.warn_on_err() {
                    this.progress = progress;
                }
                if first || this.session.phase() == Phase::Idle {
                    this.load_next(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl PuzzlesPage {
    /// The History tab's view, made the first time it is wanted.
    fn history_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<history::PuzzleHistory> {
        if let Some(history) = &self.history {
            return history.clone();
        }
        let themes = self.theme_list.clone();
        let history = cx.new(|cx| history::PuzzleHistory::new(themes, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &history,
            window,
            |this, _, event: &history::HistoryEvent, window, cx| match event {
                history::HistoryEvent::Retry(puzzle) => this.retry(puzzle.clone(), cx),
                history::HistoryEvent::Analyse(puzzle, wrong) => kchess_board::analysis::open(
                    kchess_board::analysis::Open::Puzzle {
                        puzzle: puzzle.clone(),
                        wrong_moves: wrong.clone(),
                    },
                    window,
                    cx,
                ),
                history::HistoryEvent::Train => {
                    Tab::Train.store(cx);
                    cx.notify();
                }
                history::HistoryEvent::Reconnect => {
                    crate::accounts::handle(cx)
                        .update(cx, |accounts, cx| accounts.connect(window, cx));
                }
            },
        ));
        self.history = Some(history.clone());
        history
    }

    /// Puzzle Rush's view, made the first time it is wanted, over the Stats tab's.
    fn rush_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<rush::PuzzleRush> {
        if let Some(rush) = &self.rush {
            return rush.clone();
        }
        let stats = self.stats_view(window, cx);
        let rush = cx.new(|cx| rush::PuzzleRush::new(stats, window, cx));
        self.rush = Some(rush.clone());
        rush
    }

    /// What a tab does as it comes into view: History and Stats (and Puzzle Rush, which shows
    /// the stats' runs) read the account shown, and a run of Puzzle Rush ends when another tab
    /// replaces it. Run where the tab or the accounts change and as the page is shown, never
    /// while drawing (AGENTS.md, "Writing GPUI views").
    fn show_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab = Tab::stored(cx).info().key;
        if self.shown_tab == tab {
            return;
        }
        match tab {
            "history" => {
                let history = self.history_view(window, cx);
                let account = self.account(cx);
                history.update(cx, |history, cx| history.shown(account, cx));
            }
            "stats" | "rush" => {
                let stats = self.stats_view(window, cx);
                let (account, accounts) = (self.account(cx), self.accounts(cx));
                stats.update(cx, |stats, cx| stats.shown(account, accounts, window, cx));
            }
            _ => {}
        }
        if self.shown_tab == "rush"
            && let Some(rush) = &self.rush
        {
            rush.update(cx, |rush, cx| rush.leave(cx));
        }
        self.shown_tab = tab.to_string();
        cx.notify();
    }

    /// The Daily tab's view, made when first shown.
    fn daily_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<daily::DailyPuzzle> {
        if let Some(daily) = &self.daily {
            return daily.clone();
        }
        let themes = self.theme_list.clone();
        let daily = cx.new(|cx| daily::DailyPuzzle::new(themes, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &daily,
            window,
            |this, _, event: &daily::DailyEvent, window, cx| match event {
                daily::DailyEvent::UseDownloaded => this.use_downloaded(window, cx),
            },
        ));
        self.daily = Some(daily.clone());
        daily
    }

    /// The Train tab's board.
    #[cfg(any(test, feature = "test-support"))]
    pub fn board(&self) -> Entity<PuzzleBoard> {
        self.puzzle.clone()
    }

    /// The puzzle on the Train tab, told apart from the one before it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn puzzle_key(&self) -> u64 {
        self.session.puzzle_key()
    }

    /// The Train tab's board, while a finished puzzle is stepped back through: the position shown.
    #[cfg(any(test, feature = "test-support"))]
    pub fn viewing(&self, cx: &App) -> Option<usize> {
        self.puzzle.read(cx).viewing()
    }

    /// The puzzle board of the tab shown, while it shows a puzzle, and whether it is Train's.
    fn shown_board(&self, cx: &App) -> Option<(Entity<PuzzleBoard>, bool)> {
        match Tab::stored(cx).info().key {
            "train" => (self.puzzle.read(cx).puzzle().is_some()
                && self.session.phase() == Phase::Ready)
                .then(|| (self.puzzle.clone(), true)),
            "daily" => self
                .daily
                .as_ref()
                .and_then(|daily| daily.read(cx).shown_board())
                .map(|board| (board, false)),
            _ => None,
        }
    }

    /// A Storm, Streak or Rush run is going.
    pub fn run_going(&self, cx: &App) -> bool {
        self.rush
            .as_ref()
            .is_some_and(|rush| rush.read(cx).running())
    }

    /// The Storm · Streak · Rush view, while its tab is shown.
    fn shown_rush(&self, cx: &App) -> Option<Entity<rush::PuzzleRush>> {
        (Tab::stored(cx) == Tab::Rush)
            .then(|| self.rush.clone())
            .flatten()
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        if let Action::Rush(action) = action {
            if let Some(rush) = self.shown_rush(cx) {
                rush.update(cx, |rush, cx| rush.act(action, cx));
            }
            return;
        }
        let Some((board, _)) = self.shown_board(cx) else {
            return;
        };
        match action {
            Action::Hint => board.update(cx, |board, cx| board.hint(cx)),
            Action::Solution => board.update(cx, |board, cx| board.show_solution(cx)),
            Action::Flip => board.update(cx, |board, cx| board.flip(cx)),
            Action::Next => self.load_next(cx),
            Action::Analyse | Action::Rush(_) => {}
        }
    }

    /// The finished puzzle on the analysis board, as its button opens it; after this update,
    /// since opening another page leaves this one.
    pub(crate) fn analyse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&Action::Analyse) {
            return;
        }
        let Some((board, _)) = self.shown_board(cx) else {
            return;
        };
        let board = board.read(cx);
        let Some(puzzle) = board.puzzle().map(|shown| shown.puzzle.clone()) else {
            return;
        };
        let wrong = board.wrong_moves().to_vec();
        window.defer(cx, move |window, cx| {
            kchess_board::analysis::open(
                kchess_board::analysis::Open::Puzzle {
                    puzzle,
                    wrong_moves: wrong,
                },
                window,
                cx,
            )
        });
    }

    /// F turns the board, as its button does, while no text field has the keys; Return or Space
    /// goes on to the next puzzle once this one is over (its move field is shut by then), and
    /// plays a Storm, Streak or Rush run again once one is over.
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let again = Action::Rush(RushAction::Again);
        if matches!(event.keystroke.key.as_str(), "enter" | "space")
            && crate::board::page_has_keys(&self.focus, window, cx)
            && self.actions(cx).contains(&again)
        {
            self.act(again, cx);
            cx.stop_propagation();
            return;
        }
        if matches!(event.keystroke.key.as_str(), "enter" | "space")
            && self.actions(cx).contains(&Action::Next)
            && self.shown_board(cx).is_some_and(|(board, _)| {
                let board = board.read(cx);
                board.over()
                    && (crate::board::page_has_keys(&self.focus, window, cx)
                        || board.move_field_focused(window, cx))
            })
        {
            self.act(Action::Next, cx);
            cx.stop_propagation();
            return;
        }
        if !crate::board::page_has_keys(&self.focus, window, cx) {
            return;
        }
        // A finished puzzle's moves step back and forth, as a game's do.
        if let Some((board, _)) = self.shown_board(cx)
            && board.update(cx, |board, cx| board.step(&event.keystroke.key, cx))
        {
            cx.stop_propagation();
        }
    }

    /// Show a linked puzzle on the Daily tab's board.
    pub fn open_puzzle(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        Tab::Daily.store(cx);
        self.daily_view(window, cx)
            .update(cx, |daily, cx| daily.open(id, cx));
        cx.notify();
    }
}

impl Render for PuzzlesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let tab = Tab::stored(cx).info().key;
        let tab_row = widgets::pills(
            "puzzle-tabs",
            Tab::pills(),
            Tab::stored(cx).index(),
            move |index, window, cx| {
                Tab::ALL[index].store(cx);
                window.refresh();
            },
            false,
            &palette,
        );
        // Train, as the tab row shows, for a tab this release does not have.
        let body = if tab == "daily" {
            self.daily_view(window, cx).into_any_element()
        } else if tab == "history" {
            self.history_view(window, cx).into_any_element()
        } else if tab == "stats" {
            self.stats_view(window, cx).into_any_element()
        } else if tab == "rush" {
            self.rush_view(window, cx).into_any_element()
        } else {
            self.render_train(window, cx, &palette)
        };
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .takes_keys(&self.focus)
            .child(
                div()
                    .flex_none()
                    .px(side_padding(window))
                    .pt(kchess_nav::play_layout::PAGE_TOP)
                    .child(tab_row),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

impl kchess_nav::page::PageAction for Action {
    fn info(&self) -> kchess_nav::page::ActionInfo {
        use kchess_nav::page::ActionInfo;
        let (id, label, glyph, keywords) = match self {
            Action::Hint => ("hint", "Hint", "lightbulb", "clue help"),
            Action::Solution => ("solution", "Solution", "eye", "show answer give up"),
            Action::Flip => return ActionInfo::flip("puzzle:flip"),
            Action::Analyse => (
                "analyse",
                "Analysis board",
                "microscope",
                "analyse study review",
            ),
            Action::Next => ("next", "Next puzzle", "arrow-right", "another skip"),
            Action::Rush(action) => match action {
                RushAction::Start => ("rush-start", "Start run", "play", "storm streak rush begin"),
                RushAction::Skip => ("rush-skip", "Skip puzzle", "skip-forward", "pass"),
                RushAction::End => ("rush-end", "End run", "flag", "stop finish quit"),
                RushAction::Again => ("rush-again", "Play again", "rotate-ccw", "restart retry"),
                RushAction::Back => ("rush-back", "Back to modes", "arrow-left", "menu"),
            },
        };
        ActionInfo::new(format!("puzzle:{id}"), label, glyph, keywords)
    }
}

impl PageView for PuzzlesPage {
    const INFO: kchess_nav::page::PageInfo = kchess_nav::page::PageInfo {
        page: kchess_nav::page::Page::Puzzles,
        label: "Puzzles",
        short: "Puzzles",
        icon: "puzzle",
        group: Some(kchess_nav::page::NavGroup::Train),
        keywords: "",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        PuzzlesPage::new(window, cx)
    }

    fn shown(this: &Entity<Self>, _again: bool, window: &mut Window, cx: &mut App) {
        this.update(cx, |puzzles, cx| puzzles.shown(window, cx));
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        if let Some(rush) = self.shown_rush(cx) {
            return rush
                .read(cx)
                .actions(cx)
                .into_iter()
                .map(Action::Rush)
                .collect();
        }
        let Some((board, train)) = self.shown_board(cx) else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        if board.read(cx).playing() {
            actions.extend([Action::Hint, Action::Solution]);
        }
        actions.push(Action::Flip);
        if board.read(cx).status().is_over() {
            actions.push(Action::Analyse);
        }
        if train {
            actions.push(Action::Next);
        }
        actions
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| match action {
            Action::Analyse => page.analyse(window, cx),
            action => page.act(action, cx),
        });
    }

    fn places(_: &App) -> Vec<kchess_nav::page::PageCommand> {
        kchess_nav::page::tab_places::<Self, Tab>("puzzles", |_, _, _, cx| cx.notify())
    }
}

/// A Lichess account was signed out: forget the account puzzles were trained on and, with no
/// account left, the Lichess mode and tab too.
pub fn signed_out(accounts_left: bool, cx: &mut App) {
    crate::prefs::forget(cx, ACCOUNT);
    if !accounts_left {
        crate::prefs::forget(cx, MODE);
        crate::prefs::forget(cx, Tab::KEY);
    }
}

/// Open a Lichess puzzle by its id on Puzzles → Daily.
pub fn open_puzzle(id: String, window: &mut Window, cx: &mut App) {
    kchess_nav::shell::show::<PuzzlesPage>(window, cx, move |puzzles, window, cx| {
        puzzles.open_puzzle(id, window, cx)
    });
}

/// Puzzles from one opening, by the key Lichess names its puzzles by.
pub fn open_opening(name: &str, window: &mut Window, cx: &mut App) {
    let key = kchess_domain::training::openings::opening_key(name);
    PuzzlesPage::choose_opening(&key, name, cx);
    // A page made now starts with the opening; one made before is told.
    if let Some(puzzles) = kchess_nav::shell::view::<PuzzlesPage>(cx) {
        puzzles.update(cx, |puzzles, cx| puzzles.opening_chosen(window, cx));
    }
    kchess_nav::shell::open(kchess_nav::page::Page::Puzzles, window, cx);
}

#[cfg(test)]
mod tests {
    use super::panel::coach_allowed;
    use super::{PuzzleStatus, Tab, as_contract, theme_groups, theme_name, themes};
    use crate::widgets::{self, format_bytes};
    use kchess_contracts::types::LocalLadderQuery;
    use kchess_nav::page::PageTab as _;

    /// The session writes its numbers as floats (`1500.0`); read as JSON into the contract's
    /// integer fields they were refused, so a ladder never reached the core. The contract's own
    /// check reads them as the core reads what is sent to it.
    #[test]
    fn the_sessions_whole_floats_reach_the_core_as_integers() {
        let ladder: LocalLadderQuery =
            as_contract(&serde_json::json!({ "from": 600.0, "to": 1800.0, "count": 30.0 }))
                .expect("a ladder");
        assert_eq!((ladder.from, ladder.to, ladder.count), (600, 1800, 30));
        assert!(as_contract::<LocalLadderQuery>(&serde_json::json!({ "from": "x" })).is_err());
    }

    /// The theme list is the app's own file, read where it is shown.
    #[test]
    fn every_theme_group_decodes() {
        let groups = theme_groups();
        assert!(!groups.is_empty());
        assert!(groups.iter().all(|group| !group.themes.is_empty()));
    }

    #[test]
    fn a_rated_attempt_has_no_coach_until_it_is_over() {
        assert!(!coach_allowed(true, PuzzleStatus::Playing));
        for over in [
            PuzzleStatus::Solved,
            PuzzleStatus::Failed,
            PuzzleStatus::Revealed,
        ] {
            assert!(coach_allowed(true, over));
        }
        assert!(coach_allowed(false, PuzzleStatus::Playing));
    }

    #[test]
    fn sizes_and_counts_read_plainly() {
        assert_eq!(format_bytes(0.), "0 B");
        assert_eq!(format_bytes(1536.), "1.5 KB");
        assert_eq!(format_bytes(300. * 1024. * 1024.), "300 MB");
        assert_eq!(widgets::format_count(100_000.), "100,000");
        assert_eq!(widgets::format_count(999.), "999");
    }

    #[test]
    fn theme_names_come_from_the_shared_list() {
        let list = themes();
        assert_eq!(list[0].name, "Healthy mix");
        assert_eq!(theme_name("mateIn2", &list), "Mate in 2");
        assert_eq!(theme_name("someNewTheme", &list), "Some New Theme");
    }

    /// Another page opens Puzzles at what it asked for: a linked puzzle on Daily, an opening's
    /// puzzles or a theme on Train.
    #[gpui_kit::test]
    fn other_pages_open_puzzles_at_what_they_asked(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let mut app = kchess_board::testing::Started::new(cx);
        let shell = app.with_window(|window, cx| {
            crate::puzzle_db::init(cx);
            let page = cx.new(|cx| super::PuzzlesPage::new(window, cx));
            let shell = std::rc::Rc::new(kchess_nav::testing::OnlyPage::of(page));
            cx.set_global(kchess_nav::shell::Navigator(shell.clone()));
            shell
        });
        let tab_and_angle = |app: &mut kchess_board::testing::Started| {
            app.with_window(|_, cx| {
                (
                    Tab::stored(cx),
                    crate::prefs::get::<String>(cx, super::ANGLE, String::new()),
                )
            })
        };
        app.with_window(|window, cx| {
            Tab::Rush.store(cx);
            super::open_puzzle("abcde".into(), window, cx);
        });
        assert_eq!(tab_and_angle(&mut app).0, Tab::Daily);
        app.with_window(|window, cx| {
            Tab::Rush.store(cx);
            super::open_opening("Sicilian Defense", window, cx);
        });
        let opening = kchess_domain::training::openings::opening_key("Sicilian Defense");
        assert_eq!(tab_and_angle(&mut app), (Tab::Train, opening));
        app.with_window(|window, cx| {
            Tab::Rush.store(cx);
            super::train_theme("fork".into(), window, cx);
        });
        app.cx().run_until_parked();
        assert_eq!(tab_and_angle(&mut app), (Tab::Train, "fork".to_string()));
        assert_eq!(*shell.opened.borrow(), ["Puzzles", "Puzzles"]);
    }

    /// A tab is told it is shown where it is chosen, never while the page draws: History and
    /// Stats read the account as they come into view, the page draws the chosen tab's view, and
    /// leaving Puzzle Rush ends a run under way.
    #[gpui_kit::test]
    fn a_tab_is_shown_as_it_is_chosen_and_leaving_rush_ends_its_run(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::AppContext as _;
        use gpui_kit::test::TestWindowExt as _;
        // Lichess, for the account signed in: it answers nothing.
        let server = kchess_testkit::Server::start(|_, out| {
            kchess_testkit::respond(out, 404, "text/plain", b"")
        })
        .expect("a stand-in Lichess");
        let mut app =
            kchess_board::testing::Started::with_lichess(cx, Some(server.base().to_string()));
        app.with_account("alice", true);
        let (window, page) = app.with_window(|_, cx| {
            crate::puzzle_db::init(cx);
            Tab::Train.store(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| super::PuzzlesPage::new(window, cx))
            })
            .expect("a window")
        });
        let choose = |tab: Tab, app: &mut kchess_board::testing::Started| {
            app.with_window(|_, cx| tab.store(cx));
            app.cx().run_until_parked();
            app.cx()
                .update_window(window, |_, window, cx| window.render_frame(cx))
                .expect("the window");
        };
        let shown = |app: &mut kchess_board::testing::Started| {
            app.with_window(|_, cx| {
                let page = page.read(cx);
                (
                    page.shown_tab.clone(),
                    page.history.is_some(),
                    page.stats.is_some(),
                )
            })
        };
        let draws = |id: &'static str, app: &mut kchess_board::testing::Started| {
            app.cx()
                .update_window(window, |_, window, _| window.try_find(id).is_some())
                .expect("the window")
        };
        app.with_window(|window, cx| page.update(cx, |page, cx| page.shown(window, cx)));
        assert_eq!(shown(&mut app), ("train".into(), false, false));

        choose(Tab::History, &mut app);
        assert_eq!(shown(&mut app), ("history".into(), true, false));
        assert!(draws("puzzle-history", &mut app));
        let history = app.with_window(|_, cx| page.read(cx).history.clone().expect("History"));
        assert_eq!(
            app.with_window(|_, cx| history.read(cx).account.clone()),
            "alice"
        );
        assert!(app.with_window(|_, cx| page.read(cx).daily.is_none()));
        choose(Tab::Stats, &mut app);
        assert_eq!(shown(&mut app), ("stats".into(), true, true));
        assert!(draws("puzzle-stats", &mut app));
        let stats = app.with_window(|_, cx| page.read(cx).stats.clone().expect("Stats"));
        assert_eq!(
            app.with_window(|_, cx| stats.read(cx).account.clone()),
            "alice"
        );
        choose(Tab::Daily, &mut app);
        assert!(app.with_window(|_, cx| page.read(cx).daily.is_some()));

        choose(Tab::Rush, &mut app);
        assert_eq!(shown(&mut app).0, "rush");
        let rush = app.with_window(|_, cx| page.read(cx).rush.clone().expect("Rush is drawn"));
        app.with_window(|_, cx| {
            rush.update(cx, |rush, _| rush.stage = super::rush::Stage::Running)
        });
        choose(Tab::Train, &mut app);
        assert!(app.with_window(|_, cx| rush.read(cx).stage != super::rush::Stage::Running));
    }

    /// The command palette offers each of the page's tabs.
    #[gpui_kit::test]
    fn each_tab_is_a_place_to_go(cx: &mut gpui_kit::TestAppContext) {
        use kchess_nav::page::PageView as _;
        let places = cx.read(|cx| super::PuzzlesPage::places(cx).len());
        assert_eq!(places, Tab::ALL.len());
    }
}
