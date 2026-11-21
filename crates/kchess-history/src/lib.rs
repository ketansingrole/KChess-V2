//! The History page: the games played against the
//! computer and over the board, filtered by result and searched by opponent or opening, twenty to a
//! page, each opened to go through
//! move by move, exported as PGN or deleted.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// The app's services, the design system and the board, under the names this page's modules use.
// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

use kchess_app::{accounts, core, loading, network, played, reviews, settings};
use kchess_board::{board, export, game, notation, widgets};
use kchess_ui::{dates, quick_pick, theme};

use crate::loading::Loading;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::Username;
use kchess_contracts::types::{HistoryKind, LichessGame, PlayedGame, Reply};
use kchess_domain::variant::Variant;
use kchess_nav::page::PageView;
use serde_json::json;
use shakmaty::Color;

use crate::played::{PlayedGames, PlayedHandle};
use kchess_nav::page::Page;
use kchess_nav::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use kchess_ui::icon;
use replay::GameReplay;

pub mod lichess;
mod list;
mod replay;
use crate::theme::Palette;
use crate::widgets::Pressable as _;
use crate::widgets::{KeyboardFocus as _, Look, Pill};
use kchess_board::analysis::Opening;
use list::*;

const PAGE_SIZE: usize = 20;
/// Below this page width the list keeps only the result and the players.
const NARROW: Pixels = px(620.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Computer,
    Board,
    Lichess,
    /// The online account's games bookmarked on Lichess.
    Bookmarks,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Filter {
    All,
    Win,
    Loss,
    White,
    Black,
    Draw,
    Unfinished,
}

/// The filter a game falls under; clock-only sessions show only under All.
fn filter_of(outcome: &str) -> Option<Filter> {
    Some(match outcome {
        "win" => Filter::Win,
        "loss" => Filter::Loss,
        "white" => Filter::White,
        "black" => Filter::Black,
        "draw" => Filter::Draw,
        "unfinished" | "ongoing" => Filter::Unfinished,
        _ => return None,
    })
}

/// Whether openings are named (Settings → Gameplay → _Show opening names_).
pub fn opening_names_shown(cx: &App) -> bool {
    crate::settings::frontend(cx).show_opening_name
}

/// A played game's opening, with its ECO code ("C20 King's Pawn Game"); none for a variant or a
/// line no opening names.
pub fn opening_name(game: &PlayedGame) -> Option<String> {
    if game.setup.variant != Variant::Standard {
        return None;
    }
    let opening = crate::game::opening_at(&game.setup, &game.moves, game.moves.len())?;
    Some(format!("{} {}", opening.eco, opening.name))
}

fn played_game(id: &str, cx: &App) -> Option<PlayedGame> {
    cx.global::<PlayedHandle>()
        .0
        .read(cx)
        .games()
        .iter()
        .find(|game| game.id == id)
        .cloned()
}

/// The label, look and winning side of an outcome.
fn badge_of(outcome: &str) -> (&'static str, Look, Option<Color>) {
    match outcome {
        "win" => ("Win", Look::Success, None),
        "loss" => ("Loss", Look::Error, None),
        "draw" => ("Draw", Look::NeutralSubtle, None),
        "white" => ("White won", Look::NeutralSubtle, Some(Color::White)),
        "black" => ("Black won", Look::NeutralSubtle, Some(Color::Black)),
        "unfinished" => ("Unfinished", Look::NeutralSoft, None),
        "ongoing" => ("In progress", Look::Info, None),
        _ => ("Clock", Look::NeutralSoft, None),
    }
}

/// The width of a column of result badges: "In progress", the longest, fits.
pub const BADGE_WIDTH: Pixels = px(96.);

/// A game's result as a badge, at least `width` wide: never narrower than its label.
pub fn badge(outcome: &str, width: Option<Pixels>, palette: &Palette) -> impl IntoElement {
    let (label, look, side) = badge_of(outcome);
    widgets::badge(label, look, palette)
        .when_some(width, |badge, width| badge.min_w(width))
        .justify_center()
        .children(side.map(widgets::side_dot))
}

/// Full moves, a lone white move counting as one.
pub fn move_count(plies: usize) -> String {
    let moves = plies.div_ceil(2);
    format!("{moves} {}", if moves == 1 { "move" } else { "moves" })
}

/// "300+3" as "5+3", each side of a split clock; "-" as "Untimed".
fn time_label(control: &str) -> String {
    if control == "-" {
        return "Untimed".into();
    }
    control
        .split(" / ")
        .map(|side| match side.split_once('+') {
            Some((seconds, increment)) => {
                match (seconds.parse::<f64>(), increment.parse::<f64>()) {
                    (Ok(seconds), Ok(increment)) => {
                        crate::widgets::time_control(seconds / 60., increment)
                    }
                    _ => side.to_string(),
                }
            }
            None => side.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Who the game was against: the engine for computer games, both names at the board.
pub fn opponent(game: &PlayedGame) -> String {
    match game.player_side() {
        Some(Color::White) => game.black.as_str().to_string(),
        Some(Color::Black) => game.white.as_str().to_string(),
        None => format!("{} vs {}", game.white.as_str(), game.black.as_str()),
    }
}

/// The line under the opponent: why the game ended, or the clock's summary.
fn details(game: &PlayedGame) -> String {
    if game.source == HistoryKind::Clock {
        return game
            .clock_summary
            .clone()
            .unwrap_or_else(|| game.reason.clone());
    }
    if game.result == "*" && !game.finished {
        String::new()
    } else {
        game.reason.as_str().to_string()
    }
}

/// The game as PGN, with the headers the app writes.
fn pgn(game: &PlayedGame) -> Option<String> {
    let clocks: Vec<&str> = game.time_control.as_str().split(" / ").collect();
    let split = clocks.len() > 1 && clocks[0] != clocks[1];
    let mut headers = json!({
        "Event": if game.source == HistoryKind::Computer {
            "KChess computer game"
        } else {
            "KChess over-the-board game"
        },
        "White": game.white.as_str(),
        "Black": game.black.as_str(),
        "Result": game.result.as_str(),
        "Date": date_header(game.started_at),
        "TimeControl": if split { "-" } else { clocks[0] },
        "Termination": if game.result.as_str() == "*" {
            "unterminated"
        } else {
            game.reason.as_str()
        },
    });
    if split {
        headers["WhiteTimeControl"] = json!(clocks[0]);
        headers["BlackTimeControl"] = json!(clocks[1]);
    }
    crate::game::setup_pgn(&game.setup, &game.moves, &headers)
}

/// PGN's `Date`: the UTC day the game started, "2026.10.22".
fn date_header(millis: f64) -> String {
    crate::dates::utc_format(millis, "%Y.%m.%d").unwrap_or_else(|| "????.??.??".into())
}

/// The game the last swipe back closed, to open again going forward while nothing changed since.
enum Redo {
    Played(Box<PlayedGame>),
    Lichess(Box<LichessGame>),
}

/// The page's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    AllGames,
    Export,
    Analyse,
    Review,
    Sync,
    ClearFilters,
}

/// Open the saved game on the analysis board.
fn analyse(game: &PlayedGame, window: &mut Window, cx: &mut App) {
    let Some(pgn) = pgn(game) else { return };
    kchess_board::analysis::open(
        kchess_board::analysis::Open::Pgn {
            pgn,
            opening: Opening::default(),
            unreadable: false,
        },
        window,
        cx,
    );
}

pub struct HistoryPage {
    source: Source,
    filter: Filter,
    /// The page of the list shown, from 0.
    page: usize,
    /// The game being gone through, and its board.
    open: Option<(PlayedGame, Entity<GameReplay>)>,
    message: Option<SharedString>,
    redo: Option<Redo>,
    /// Part of an opponent's or an opening's name the computer and board games are searched
    /// for; blank is none.
    search: String,
    search_input: Entity<InputState>,
    /// Each game's opening, by its id and length, worked out once.
    openings: std::cell::RefCell<std::collections::HashMap<(String, usize), Option<String>>>,
    /// Each played game's outcome by id, as of a revision of the played games: the list, its
    /// filters' counts and the pages ask for every game's outcome on every frame.
    outcomes: std::cell::RefCell<(u64, std::collections::HashMap<String, String>)>,
    /// The Lichess tab: synced games, their filters and the game open.
    lichess: lichess::LichessTab,
    /// The Bookmarks tab: the games, once read, or why they are not.
    bookmarks: Option<Vec<LichessGame>>,
    bookmarks_loading: Loading,
    bookmarks_reconnect: bool,
    area: Measured,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for HistoryPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl HistoryPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> HistoryPage {
        let played = cx.global::<PlayedHandle>().0.clone();
        let (lichess, mut subscriptions) = lichess::new_tab(window, cx);
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Opponent or opening"));
        subscriptions.push(
            cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let search = input.read(cx).value().trim().to_string();
                    if search != this.search {
                        this.search = search;
                        this.page = 0;
                        cx.notify();
                    }
                }
            }),
        );
        // Rows name their openings only while opening names are shown.
        let settings = cx.global::<crate::settings::SettingsHandle>().0.clone();
        subscriptions.push(cx.observe(&settings, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&played, |this, _, cx| {
            // A deleted game closes; the pages left shrink with the list.
            if let Some((game, _)) = &this.open
                && !this.all_games(cx).iter().any(|entry| entry.id == game.id)
            {
                this.open = None;
            }
            cx.notify();
        }));
        HistoryPage {
            source: Source::Computer,
            filter: Filter::All,
            page: 0,
            open: None,
            message: None,
            redo: None,
            search: String::new(),
            search_input,
            openings: Default::default(),
            outcomes: Default::default(),
            lichess,
            bookmarks: None,
            bookmarks_loading: Loading::default(),
            bookmarks_reconnect: false,
            area: Measured::default(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// A game's opening, as `C40 King's Knight Opening`, in standard chess; worked out once.
    /// A played game's outcome (`outcome`), kept until the played games change.
    fn outcome_of(&self, game: &PlayedGame, cx: &App) -> String {
        let revision = cx.global::<PlayedHandle>().0.read(cx).revision();
        let mut kept = self.outcomes.borrow_mut();
        if kept.0 != revision {
            *kept = (revision, std::collections::HashMap::new());
        }
        kept.1
            .entry(game.id.as_str().to_string())
            .or_insert_with(|| game.outcome().to_string())
            .clone()
    }

    fn opening(&self, game: &PlayedGame) -> Option<String> {
        let moves = game.moves.len();
        let key = (game.id.as_str().to_string(), moves);
        self.openings
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| opening_name(game))
            .clone()
    }

    /// Whether a game's opponent (or players) or opening has the searched text in it.
    fn found(&self, game: &PlayedGame) -> bool {
        if self.search.is_empty() {
            return true;
        }
        let wanted = self.search.to_lowercase();
        [
            Some(game.white.clone()),
            Some(game.black.clone()),
            self.opening(game),
        ]
        .into_iter()
        .flatten()
        .any(|name| name.to_lowercase().contains(&wanted))
    }

    /// The games of the source shown, newest first, that the search finds.
    fn games<'a>(&self, cx: &'a App) -> Vec<&'a PlayedGame> {
        self.all_games(cx)
            .into_iter()
            .filter(|game| self.found(game))
            .collect()
    }

    /// Every game of the source shown, newest first: the played games themselves, not copies
    /// (copying 500 games of history was half of drawing the page).
    fn all_games<'a>(&self, cx: &'a App) -> Vec<&'a PlayedGame> {
        let mut games: Vec<&PlayedGame> = cx
            .global::<PlayedHandle>()
            .0
            .read(cx)
            .games()
            .iter()
            .filter(|game| {
                (game.source == HistoryKind::Computer) == (self.source == Source::Computer)
            })
            .collect();
        games.sort_by(|a, b| b.started_at.total_cmp(&a.started_at));
        games
    }

    /// The games the list shows under its filter, newest first.
    fn listed<'a>(&self, cx: &'a App) -> Vec<&'a PlayedGame> {
        self.games(cx)
            .into_iter()
            .filter(|game| {
                self.filter == Filter::All
                    || filter_of(&self.outcome_of(game, cx)) == Some(self.filter)
            })
            .collect()
    }

    /// The game `step` rows from the open one in the list (-1 the one above, newer; 1 below),
    /// skipping clock sessions, which have no moves to go through.
    fn neighbour(&self, step: isize, cx: &App) -> Option<(usize, PlayedGame)> {
        let (open, _) = self.open.as_ref()?;
        let listed = self.listed(cx);
        let at = listed.iter().position(|game| game.id == open.id)?;
        let mut index = at as isize;
        loop {
            index += step;
            let game = listed.get(usize::try_from(index).ok()?)?;
            if game.source != HistoryKind::Clock {
                return Some((index as usize, (*game).clone()));
            }
        }
    }

    /// Open the game `step` rows away, the list's page following it.
    fn step_game(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((index, game)) = self.neighbour(step, cx) {
            self.page = index / PAGE_SIZE;
            self.open_game(game, window, cx);
        }
    }

    /// Show the Lichess tab's list of one account's games (Following's Games).
    pub fn show_lichess_account(
        &mut self,
        account: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_source(Source::Lichess, cx);
        self.set_lichess_account(account, window, cx);
    }

    /// Open the first game on the Lichess list's page, as its row does.
    #[cfg(any(test, feature = "test-support"))]
    pub fn open_first_lichess(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(game) = self.lichess.games.first().cloned() {
            self.open_lichess(game, window, cx);
        }
    }

    /// The first game on the Lichess list's page.
    #[cfg(any(test, feature = "test-support"))]
    pub fn first_lichess(&self) -> Option<LichessGame> {
        self.lichess.games.first().cloned()
    }

    /// The Lichess game open, and the ids its list's page shows.
    #[cfg(any(test, feature = "test-support"))]
    pub fn lichess_open(&self) -> (Option<String>, Vec<String>) {
        let id = |game: &LichessGame| game.id.clone();
        (
            self.lichess.open.as_ref().map(|(game, _)| id(game)),
            self.lichess.games.iter().map(id).collect(),
        )
    }

    /// The Lichess tab once loaded: its total and the openings of the games listed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn lichess_listed(&self) -> Option<(usize, Vec<String>)> {
        let tab = &self.lichess;
        (!tab.loading.busy()).then(|| {
            let openings = tab
                .games
                .iter()
                .map(|game| game.opening.clone().unwrap_or_default())
                .collect();
            (tab.total, openings)
        })
    }

    /// The Lichess tab searched for `text`, as typed in its field (the screenshots, which are
    /// taken on macOS).
    #[cfg(all(any(test, feature = "test-support"), target_os = "macos"))]
    pub fn search_lichess(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.choose_source(Source::Lichess, cx);
        self.lichess.search_input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
        self.lichess.search = text.to_string();
        self.lichess.page = 0;
        self.load_lichess(cx);
    }

    /// Show the Lichess tab's list (Home's View all).
    pub fn show_lichess(&mut self, cx: &mut Context<Self>) {
        self.choose_source(Source::Lichess, cx);
    }

    /// Open a synced Lichess game on the Lichess tab (Home's recent games).
    pub fn show_lichess_game(
        &mut self,
        game: LichessGame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_source(Source::Lichess, cx);
        self.open_lichess(game, window, cx);
    }

    fn choose_source(&mut self, source: Source, cx: &mut Context<Self>) {
        self.source = source;
        self.filter = Filter::All;
        self.page = 0;
        self.open = None;
        self.redo = None;
        if source == Source::Lichess {
            self.lichess.opening.invalidate();
            self.lichess.open = None;
            self.load_lichess(cx);
        }
        if source == Source::Bookmarks {
            self.load_bookmarks(cx);
        }
        cx.notify();
    }

    fn load_bookmarks(&mut self, cx: &mut Context<Self>) {
        self.bookmarks = None;
        self.bookmarks_reconnect = false;
        let account = crate::accounts::handle(cx).read(cx).active(cx);
        let Some(account) = account.filter(|_| crate::network::online(cx)) else {
            self.bookmarks_loading.stop();
            return;
        };
        let games = cx
            .global::<crate::core::AppCore>()
            .request(calls::BookmarkedGames {
                account: Username(account.clone()),
            });
        self.bookmarks_loading.load(
            cx,
            "history",
            |this: &mut Self| &mut this.bookmarks_loading,
            games,
            |this, answer, _| match answer {
                Ok(Reply::Done(games)) => this.bookmarks = Some(games),
                Ok(Reply::NeedsReconnect) => this.bookmarks_reconnect = true,
                Err(_) => {}
            },
        );
    }

    fn render_bookmarks(&self, cx: &App, palette: &Palette) -> AnyElement {
        let account = crate::accounts::handle(cx)
            .read(cx)
            .active(cx)
            .unwrap_or_default();
        let card = widgets::card(palette).gap(px(8.));
        let note =
            |text: String, color: Hsla| div().text_size(px(13.)).text_color(color).child(text);
        if !crate::network::online(cx) {
            return card
                .child(note(
                    "Offline · reconnect to see bookmarks".into(),
                    palette.text_muted,
                ))
                .into_any_element();
        }
        if self.bookmarks_reconnect {
            return card
                .child(note(format!("Connect @{account} again"), palette.warning))
                .child(
                    Button::new("bookmarks-reconnect")
                        .small()
                        .icon(icon("log-in"))
                        .label("Reconnect")
                        .on_click(|_, window, cx| {
                            crate::accounts::handle(cx)
                                .update(cx, |accounts, cx| accounts.connect(window, cx))
                        }),
                )
                .into_any_element();
        }
        if let Some(error) = self.bookmarks_loading.error() {
            return card
                .child(note(error.to_string(), palette.error))
                .into_any_element();
        }
        match &self.bookmarks {
            None => card
                .child(widgets::loading("Loading…", palette))
                .into_any_element(),
            Some(games) if games.is_empty() => card
                .items_center()
                .py(px(28.))
                .child(widgets::empty_state(
                    icon("bookmark"),
                    format!("No bookmarks on @{account}"),
                    None,
                    palette,
                ))
                .into_any_element(),
            Some(games) => card
                .children(games.iter().enumerate().map(|(index, game)| {
                    let chosen = game.clone();
                    lichess::game_row(
                        ("history-bookmark", index),
                        game,
                        None,
                        lichess::RowDetail::NoReview,
                        move |window, cx| {
                            kchess_board::analysis::open_lichess_game(chosen.clone(), window, cx)
                        },
                        palette,
                    )
                }))
                .into_any_element(),
        }
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.clear();
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn choose_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.page = 0;
        cx.notify();
    }

    fn open_game(&mut self, game: PlayedGame, window: &mut Window, cx: &mut Context<Self>) {
        let replay = cx.new(|cx| {
            GameReplay::new(
                game.setup.clone(),
                game.moves.clone(),
                Color::White,
                game.white.clone(),
                game.black.clone(),
                cx,
            )
        });
        window.focus(&replay.read(cx).focus_handle(cx), cx);
        self.open = Some((game, replay));
        self.message = None;
        self.redo = None;
        cx.notify();
    }

    /// Open a game by its id, on the tab it belongs to (Home's recent games). A clock session has
    /// nothing to go through.
    pub fn show_game(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let game = cx
            .global::<PlayedHandle>()
            .0
            .read(cx)
            .games()
            .iter()
            .find(|game| game.id == id)
            .cloned();
        let Some(game) = game else { return };
        self.source = if game.source == HistoryKind::Computer {
            Source::Computer
        } else {
            Source::Board
        };
        self.filter = Filter::All;
        self.page = 0;
        if game.source != HistoryKind::Clock {
            self.open_game(game, window, cx);
        }
        cx.notify();
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        match action {
            Action::AllGames if self.open.is_some() => self.close_game(window, cx),
            Action::AllGames => self.close_lichess(window, cx),
            Action::Export => {
                if let Some((game, _)) = self.open.clone() {
                    self.export(&game, cx);
                }
            }
            Action::Analyse => {
                if let Some((game, _)) = &self.open {
                    analyse(game, window, cx);
                }
            }
            Action::Review => {
                if let Some((game, replay)) = self.lichess.open.clone() {
                    let ply = replay.read(cx).ply();
                    lichess::open_review(game, ply, window, cx);
                }
            }
            Action::Sync => self.sync_lichess(window, cx),
            Action::ClearFilters => self.clear_lichess_filters(window, cx),
        }
    }

    fn close_game(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        self.redo = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Save the game as PGN where the player picks, or copy a clock session's summary.
    fn export(&mut self, game: &PlayedGame, cx: &mut Context<Self>) {
        if game.source == HistoryKind::Clock {
            let summary = format!(
                "{} / {}\n{}\n{}\nTime control: {}",
                game.white.as_str(),
                game.black.as_str(),
                game.reason.as_str(),
                game.clock_summary.as_deref().unwrap_or_default(),
                game.time_control.as_str(),
            );
            cx.write_to_clipboard(ClipboardItem::new_string(summary));
            self.message = Some("Clock summary copied".into());
            cx.notify();
            return;
        }
        let Some(pgn) = pgn(game) else {
            self.message = Some("Export failed".into());
            cx.notify();
            return;
        };
        let name = format!("kchess-{}.pgn", game.id.as_str());
        let saved = crate::export::save_text(&name, pgn, cx);
        cx.spawn(async move |this, cx| {
            let message: Option<SharedString> = match saved.await {
                Some(Ok(())) => Some("Saved the PGN".into()),
                Some(Err(cause)) => Some(format!("Export failed: {cause}").into()),
                None => None,
            };
            let _ = this.update(cx, |this, cx| {
                if message.is_some() {
                    this.message = message;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Save the games listed (filter and search applied) as one PGN file where the player picks.
    /// Clock sessions have no moves to save and are left out.
    fn export_shown(&mut self, cx: &mut Context<Self>) {
        let pgns: Vec<String> = self
            .listed(cx)
            .into_iter()
            .filter(|game| game.source != HistoryKind::Clock)
            .filter_map(pgn)
            .collect();
        if pgns.is_empty() {
            return;
        }
        let count = pgns.len();
        let name = format!(
            "kchess-{}-games-{}.pgn",
            if self.source == Source::Computer {
                "computer"
            } else {
                "board"
            },
            crate::dates::today("%Y-%m-%d")
        );
        let saved = crate::export::save_text(&name, pgns.join("\n\n"), cx);
        cx.spawn(async move |this, cx| {
            let message: Option<SharedString> = match saved.await {
                Some(Ok(())) => Some(
                    format!(
                        "Saved {count} {}",
                        if count == 1 { "game" } else { "games" }
                    )
                    .into(),
                ),
                Some(Err(cause)) => Some(format!("Export failed: {cause}").into()),
                None => None,
            };
            let _ = this.update(cx, |this, cx| {
                if message.is_some() {
                    this.message = message;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Delete a game at once; its notice's Undo puts it back where it was.
    fn delete(&mut self, game: PlayedGame, window: &mut Window, cx: &mut Context<Self>) {
        let played = cx.global::<PlayedHandle>().0.clone();
        let id = game.id.clone();
        let Some((index, removed)) =
            played.update(cx, |played: &mut PlayedGames, cx| played.remove(&id, cx))
        else {
            return;
        };
        window.push_notification(
            widgets::action_notification(
                Notification::new().title("Game deleted"),
                "undo-delete-game",
                "Undo",
                move |_, cx| {
                    let (played, removed) = (played.clone(), removed.clone());
                    played.update(cx, |played: &mut PlayedGames, cx| {
                        played.restore(index, removed, cx)
                    });
                },
            ),
            cx,
        );
    }

    pub fn step(
        &mut self,
        direction: kchess_nav::swipe::Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if direction < 0 {
            if let Some((game, _)) = self.open.clone() {
                self.close_game(window, cx);
                self.redo = Some(Redo::Played(Box::new(game)));
            } else if let Some((game, _)) = self.lichess.open.clone() {
                self.close_lichess(window, cx);
                self.redo = Some(Redo::Lichess(Box::new(game)));
            }
            return;
        }
        match self.redo.take() {
            Some(Redo::Played(game)) => self.open_game(*game, window, cx),
            Some(Redo::Lichess(game)) => self.open_lichess(*game, window, cx),
            None => {}
        }
    }

    fn narrow(&self) -> bool {
        let width = self.area.size().width;
        width > px(0.) && width < NARROW
    }
}

impl Render for HistoryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        // Bookmarks belong to a connected account; signing out leaves the synced games.
        if self.source == Source::Bookmarks
            && crate::accounts::handle(cx).read(cx).connected().is_empty()
        {
            self.choose_source(Source::Lichess, cx);
        }
        let error = cx.global::<PlayedHandle>().0.read(cx).error.clone();
        let header = v_flex()
            .px(side_padding(window))
            .pt(PAGE_TOP)
            .child(source_tabs(self.source, cx, &palette));
        // An open game has no text field: Escape anywhere in it goes back to the list.
        if let Some((game, replay)) = self.open.clone() {
            return div()
                .size_full()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if widgets::is_escape(event) {
                        this.close_game(window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(self.render_open(&game, &replay, window, cx, &palette))
                .into_any_element();
        }
        if self.source == Source::Lichess
            && let Some((game, replay)) = self.lichess.open.clone()
        {
            return div()
                .size_full()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if widgets::is_escape(event) {
                        this.close_lichess(window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(self.render_lichess_open(&game, &replay, window, cx, &palette))
                .into_any_element();
        }
        let body = match self.source {
            Source::Lichess => self.render_lichess(window, cx, &palette),
            Source::Bookmarks => self.render_bookmarks(cx, &palette),
            _ => self.render_list(cx, &palette),
        };
        div()
            .id("history-page")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_y_scroll()
            .child(self.area.probe(cx))
            .child(header)
            .child(
                v_flex()
                    .gap(px(16.))
                    .pt(px(20.))
                    .pb(PAGE_BOTTOM)
                    .px(side_padding(window))
                    .children(self.message.clone().map(|message| {
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(message)
                    }))
                    .children(error.map(|error| {
                        div()
                            .text_size(px(14.))
                            .text_color(palette.error)
                            .child(SharedString::from(error))
                    }))
                    .child(body),
            )
            .into_any_element()
    }
}

impl kchess_nav::page::PageAction for Action {
    fn info(&self) -> kchess_nav::page::ActionInfo {
        use kchess_nav::page::ActionInfo;
        match self {
            Action::AllGames => {
                ActionInfo::new("history:all", "All games", "arrow-left", "back list")
            }
            Action::Export => {
                ActionInfo::new("history:export", "Export PGN", "download", "save file")
            }
            Action::Analyse => ActionInfo::new(
                "history:analyse",
                "Open analysis",
                "chart-line",
                "analysis board",
            ),
            Action::Review => ActionInfo::new(
                "history:review",
                "Review",
                "chart-line",
                "analysis accuracy",
            ),
            Action::Sync => ActionInfo::new(
                "history:sync",
                "Sync games",
                "refresh-cw",
                "lichess download",
            ),
            Action::ClearFilters => {
                ActionInfo::new("history:clear", "Clear filters", "x", "reset all")
            }
        }
    }
}

impl PageView for HistoryPage {
    const INFO: kchess_nav::page::PageInfo = kchess_nav::page::PageInfo {
        page: Page::History,
        label: "Game history",
        short: "Game history",
        icon: "history",
        group: Some(kchess_nav::page::NavGroup::Review),
        keywords: "past games archive",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        HistoryPage::new(window, cx)
    }

    fn can_step(&self, direction: kchess_nav::swipe::Direction, cx: &App) -> bool {
        if direction < 0 {
            return self.open.is_some() || self.lichess.open.is_some();
        }
        match &self.redo {
            Some(Redo::Played(game)) => {
                self.open.is_none() && self.games(cx).iter().any(|entry| entry.id == game.id)
            }
            Some(Redo::Lichess(game)) => {
                self.source == Source::Lichess
                    && self.lichess.open.is_none()
                    && self.lichess.games.iter().any(|entry| entry.id == game.id)
            }
            None => false,
        }
    }

    fn step(
        &mut self,
        direction: kchess_nav::swipe::Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        HistoryPage::step(self, direction, window, cx);
    }

    fn search_fields(&self) -> Vec<Entity<InputState>> {
        vec![self.search_input.clone(), self.lichess.search_input.clone()]
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        if self.open.is_some() {
            return vec![Action::AllGames, Action::Export, Action::Analyse];
        }
        if self.source != Source::Lichess {
            return Vec::new();
        }
        if let Some((game, _)) = &self.lichess.open {
            let mut actions = vec![Action::AllGames];
            if crate::game::pgn_analysable(game.pgn.as_deref().unwrap_or_default()) {
                actions.push(Action::Review);
            }
            return actions;
        }
        let accounts = crate::accounts::handle(cx).read(cx);
        let mut actions = Vec::new();
        if !accounts.all().is_empty() && !accounts.busy {
            actions.push(Action::Sync);
        }
        if accounts.game_count() > 0 && self.lichess_filtered() {
            actions.push(Action::ClearFilters);
        }
        actions
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |page, cx| page.act(action, window, cx));
    }

    fn places(_: &App) -> Vec<kchess_nav::page::PageCommand> {
        vec![kchess_nav::page::PageCommand {
            info: kchess_nav::page::ActionInfo::new(
                "history:lichess",
                "Lichess games",
                "history",
                "synced online played",
            ),
            detail: Some(Self::INFO.label.into()),
            run: std::rc::Rc::new(open_lichess),
        }]
    }
}

/// Game history's Lichess tab showing one account's games.
pub fn open_account(account: &str, window: &mut Window, cx: &mut App) {
    let account = account.to_string();
    kchess_nav::shell::show::<HistoryPage>(window, cx, move |history, window, cx| {
        history.show_lichess_account(account, window, cx)
    });
}

/// Game history's Lichess tab.
pub fn open_lichess(window: &mut Window, cx: &mut App) {
    kchess_nav::shell::show::<HistoryPage>(window, cx, |history, _, cx| history.show_lichess(cx));
}

/// Open a synced Lichess game in Game history.
pub fn open_lichess_game(game: LichessGame, window: &mut Window, cx: &mut App) {
    kchess_nav::shell::show::<HistoryPage>(window, cx, move |history, window, cx| {
        history.show_lichess_game(game, window, cx)
    });
}

/// Open a played game in Game history (Home's recent games).
pub fn open_game(id: &str, window: &mut Window, cx: &mut App) {
    let id = id.to_string();
    kchess_nav::shell::show::<HistoryPage>(window, cx, move |history, window, cx| {
        history.show_game(&id, window, cx)
    });
}

#[cfg(test)]
mod tests {
    use super::{PlayedGame, move_count, time_label};
    use serde_json::json;

    /// Another page opens History at what it asked for: the Lichess tab, an account's games or
    /// one Lichess game there, or a game played here; each asks the shell to open History. The
    /// command palette offers the Lichess tab.
    #[gpui_kit::test]
    fn other_pages_open_history_at_what_they_asked(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        use kchess_nav::page::PageView as _;
        let played = played_game("computer", "1-0");
        let mut app = kchess_board::testing::Started::new(cx);
        let (page, shell) = app.with_window(|window, cx| {
            let library = kchess_contracts::types::Library {
                games: vec![played],
                ..Default::default()
            };
            crate::played::init(&library, cx);
            let page = cx.new(|cx| super::HistoryPage::new(window, cx));
            let shell = std::rc::Rc::new(kchess_nav::testing::OnlyPage::of(page.clone()));
            cx.set_global(kchess_nav::shell::Navigator(shell.clone()));
            (page, shell)
        });
        let source = |app: &mut kchess_board::testing::Started| {
            app.with_window(|_, cx| page.read(cx).source)
        };
        app.with_window(super::open_lichess);
        assert_eq!(source(&mut app), super::Source::Lichess);
        app.with_window(|window, cx| super::open_game("g", window, cx));
        assert_eq!(source(&mut app), super::Source::Computer);
        app.with_window(|window, cx| super::open_account("alice", window, cx));
        let account = app.with_window(|_, cx| page.read(cx).lichess.account.clone());
        assert_eq!(
            (source(&mut app), account.as_str()),
            (super::Source::Lichess, "alice")
        );
        let game = serde_json::from_value(json!({
            "id": "lichess1", "account": "alice", "opponent": "bob", "color": "white",
            "createdAt": 0, "lastMoveAt": 0, "rated": true, "speed": "blitz", "perf": "blitz",
            "status": "mate", "moves": "e4 e5",
        }))
        .expect("a Lichess game");
        // The game opens once its moves are read, in a task of the page's window; here it is
        // asked for.
        let before = app.with_window(|_, cx| page.read(cx).lichess.opening.current());
        app.with_window(|window, cx| super::open_lichess_game(game, window, cx));
        let asked = app.with_window(|_, cx| !page.read(cx).lichess.opening.is_current(before));
        assert_eq!((source(&mut app), asked), (super::Source::Lichess, true));
        assert_eq!(*shell.opened.borrow(), ["History"; 4]);
        let places = app.with_window(|_, cx| super::HistoryPage::places(cx).len());
        assert_eq!(places, 1);
    }

    fn played_game(source: &str, result: &str) -> PlayedGame {
        serde_json::from_value(json!({
            "id": "g", "source": source, "white": "You", "black": "Stockfish", "result": result,
            "finished": true, "reason": "-", "startedAt": 1, "updatedAt": 2,
            "timeControl": "-", "moves": ["e2e4", "e7e5"],
            "setup": { "variant": "standard", "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1" },
        }))
        .expect("a played game")
    }

    /// Each kind of saved game ends for its player, or by colour at the board.
    #[test]
    fn each_saved_game_ends_for_its_player_or_by_colour() {
        let moves: Vec<&str> = ["e2e4", "e7e5"].repeat(40);
        let game = |source: &str, white: &str, black: &str, result: &str, finished: bool| {
            serde_json::from_value::<PlayedGame>(json!({
                "id": "g", "source": source, "white": white, "black": black, "result": result,
                "finished": finished, "reason": "-", "startedAt": 1, "updatedAt": 2,
                "timeControl": "-", "moves": moves,
                "setup": { "variant": "standard", "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1" },
            }))
            .expect("a played game")
        };
        use shakmaty::Color::{Black, White};
        let games = [
            (game("clock", "A", "B", "*", true), "clock", None),
            (
                game("computer", "You", "Stockfish", "*", false),
                "ongoing",
                Some(White),
            ),
            (
                game("computer", "You", "Stockfish", "*", true),
                "unfinished",
                Some(White),
            ),
            (
                game("computer", "You", "Stockfish", "1/2-1/2", true),
                "draw",
                Some(White),
            ),
            (
                game("computer", "You", "Stockfish", "1-0", true),
                "win",
                Some(White),
            ),
            (
                game("computer", "You", "Stockfish", "0-1", true),
                "loss",
                Some(White),
            ),
            (
                game("computer", "Stockfish", "You", "1-0", true),
                "loss",
                Some(Black),
            ),
            (
                game("computer", "Stockfish", "You", "0-1", true),
                "win",
                Some(Black),
            ),
            (game("board", "Ann", "Bo", "1-0", true), "white", None),
            (game("board", "Ann", "Bo", "0-1", true), "black", None),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for (whole, expected, side) in &games {
            assert_eq!(whole.outcome(), *expected, "{whole:?}");
            assert_eq!(whole.player_side(), *side, "{whole:?}");
            seen.insert(whole.outcome());
        }
        // Every kind of outcome is among them.
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            [
                "black",
                "clock",
                "draw",
                "loss",
                "ongoing",
                "unfinished",
                "white",
                "win"
            ]
        );
    }

    #[test]
    fn time_controls_read_in_minutes() {
        assert_eq!(time_label("-"), "Untimed");
        assert_eq!(time_label("300+3"), "5+3");
        assert_eq!(time_label("180+2 / 600+2"), "3+2 / 10+2");
        assert_eq!(time_label("90+0"), "1.5+0");
        // A 30-second clock reads as Tournaments and the challenges write it.
        assert_eq!(time_label("30+0"), "½+0");
    }

    #[test]
    fn moves_count_full_moves() {
        assert_eq!(move_count(1), "1 move");
        assert_eq!(move_count(2), "1 move");
        assert_eq!(move_count(13), "7 moves");
    }
}
