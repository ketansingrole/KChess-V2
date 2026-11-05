//! Game history: the games played against the
//! computer and over the board, filtered by result and searched by opponent or opening, twenty to a
//! page, each opened to go through
//! move by move, exported as PGN or deleted.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use shakmaty::Color;

use crate::assets::icon;
use crate::game::rules;
use crate::pages::Page;
use crate::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::played::{PlayedGames, PlayedHandle};
use crate::replay::GameReplay;

pub(crate) mod lichess;
use crate::theme::Palette;
use crate::widgets::{self, KeyboardFocus as _, Look, Pill};

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

/// The filter a game falls under (`filterOf`); clock-only sessions show only under All.
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

/// `archivedOutcome`, shared with the Electron app.
pub(crate) fn outcome(game: &Value) -> String {
    rules("archivedOutcome", std::slice::from_ref(game))
        .ok()
        .and_then(|outcome| outcome.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn player_side(game: &Value) -> Option<Color> {
    match rules("archivedPlayerSide", std::slice::from_ref(game))
        .ok()?
        .as_str()?
    {
        "white" => Some(Color::White),
        "black" => Some(Color::Black),
        _ => None,
    }
}

fn text<'a>(game: &'a Value, key: &str) -> &'a str {
    game[key].as_str().unwrap_or_default()
}

/// `ARCHIVED_BADGES`: the label, look and winning side of an outcome.
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

/// A game's result as a badge.
pub(crate) fn badge(outcome: &str, width: Option<Pixels>, palette: &Palette) -> impl IntoElement {
    let (label, look, side) = badge_of(outcome);
    widgets::badge(label, look, palette)
        .when_some(width, |badge, width| badge.w(width))
        .justify_center()
        .children(side.map(widgets::side_dot))
}

/// `moveCount`: full moves, a lone white move counting as one.
pub(crate) fn move_count(plies: usize) -> String {
    let moves = plies.div_ceil(2);
    format!("{moves} {}", if moves == 1 { "move" } else { "moves" })
}

/// `timeLabel`: "300+3" as "5+3", each side of a split clock; "-" as "Untimed".
fn time_label(control: &str) -> String {
    if control == "-" {
        return "Untimed".into();
    }
    control
        .split(" / ")
        .map(|side| match side.split_once('+') {
            Some((seconds, increment)) => match seconds.parse::<f64>() {
                Ok(seconds) => format!("{}+{increment}", seconds / 60.),
                Err(_) => side.to_string(),
            },
            None => side.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Who the game was against: the engine for computer games, both names at the board.
pub(crate) fn opponent(game: &Value) -> String {
    match player_side(game) {
        Some(Color::White) => text(game, "black").to_string(),
        Some(Color::Black) => text(game, "white").to_string(),
        None => format!("{} vs {}", text(game, "white"), text(game, "black")),
    }
}

/// The line under the opponent: why the game ended, or the clock's summary.
fn details(game: &Value) -> String {
    if text(game, "source") == "clock" {
        return game["clockSummary"]
            .as_str()
            .unwrap_or(text(game, "reason"))
            .to_string();
    }
    if text(game, "result") == "*" && game["finished"] != json!(true) {
        String::new()
    } else {
        text(game, "reason").to_string()
    }
}

/// The game as PGN, with the headers the app writes.
fn pgn(game: &Value) -> Option<String> {
    let clocks: Vec<&str> = text(game, "timeControl").split(" / ").collect();
    let split = clocks.len() > 1 && clocks[0] != clocks[1];
    let mut headers = json!({
        "Event": if text(game, "source") == "computer" {
            "KChess computer game"
        } else {
            "KChess over-the-board game"
        },
        "White": text(game, "white"),
        "Black": text(game, "black"),
        "Result": text(game, "result"),
        "Date": date_header(game["startedAt"].as_f64().unwrap_or_default()),
        "TimeControl": if split { "-" } else { clocks[0] },
        "Termination": if text(game, "result") == "*" {
            "unterminated"
        } else {
            text(game, "reason")
        },
    });
    if split {
        headers["WhiteTimeControl"] = json!(clocks[0]);
        headers["BlackTimeControl"] = json!(clocks[1]);
    }
    rules(
        "setupPgn",
        &[game["setup"].clone(), game["moves"].clone(), headers],
    )
    .ok()
    .and_then(|pgn| pgn.as_str().map(str::to_string))
}

/// PGN's `Date`: the UTC day the game started, "2026.10.22".
fn date_header(millis: f64) -> String {
    chrono::DateTime::from_timestamp_millis(millis as i64)
        .map(|date| date.format("%Y.%m.%d").to_string())
        .unwrap_or_else(|| "????.??.??".into())
}

/// The game the last swipe back closed, to open again going forward while nothing changed since.
enum Redo {
    Played(Value),
    Lichess(Value),
}

/// The page's actions, as its buttons offer them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    AllGames,
    Export,
    Analyse,
    Review,
    Sync,
    ClearFilters,
}

/// Whether the analysis board plays the saved game.
fn analysable(game: &Value) -> bool {
    crate::game::analysable(game["setup"]["variant"].as_str().unwrap_or("standard"))
}

/// Open the saved game on the analysis board.
fn analyse(game: &Value, window: &mut Window, cx: &mut App) {
    let Some(pgn) = pgn(game) else { return };
    crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
        analysis.open_game(&pgn, 0, None, None, false, cx)
    });
}

pub struct HistoryPage {
    source: Source,
    filter: Filter,
    /// The page of the list shown, from 0.
    page: usize,
    /// The game being gone through, and its board.
    open: Option<(Value, Entity<GameReplay>)>,
    message: Option<SharedString>,
    redo: Option<Redo>,
    /// Part of an opponent's or an opening's name the computer and board games are searched
    /// for; blank is none.
    search: String,
    search_input: Entity<InputState>,
    /// Each game's opening (`openingAt`), by its id and length, worked out once.
    openings: std::cell::RefCell<std::collections::HashMap<(String, usize), Option<String>>>,
    /// The Lichess tab: synced games, their filters and the game open.
    lichess: lichess::LichessTab,
    /// The Bookmarks tab: the games, once read, or why they are not.
    bookmarks: Option<Vec<Value>>,
    bookmarks_error: Option<SharedString>,
    bookmarks_reconnect: bool,
    bookmarks_epoch: u64,
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
        subscriptions.push(cx.observe(&played, |this, _, cx| {
            // A deleted game closes; the pages left shrink with the list.
            if let Some((game, _)) = &this.open
                && !this
                    .all_games(cx)
                    .iter()
                    .any(|entry| entry["id"] == game["id"])
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
            lichess,
            bookmarks: None,
            bookmarks_error: None,
            bookmarks_reconnect: false,
            bookmarks_epoch: 0,
            area: Measured::default(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// A game's opening, as `C40 King's Knight Opening`, in standard chess.
    fn opening(&self, game: &Value) -> Option<String> {
        if game["setup"]["variant"].as_str().unwrap_or("standard") != "standard" {
            return None;
        }
        let moves = game["moves"].as_array().map_or(0, Vec::len);
        let key = (text(game, "id").to_string(), moves);
        self.openings
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let opening =
                    rules("openingAt", &[game["setup"].clone(), game["moves"].clone()]).ok()?;
                Some(format!(
                    "{} {}",
                    opening["eco"].as_str()?,
                    opening["name"].as_str()?
                ))
            })
            .clone()
    }

    /// Whether a game's opponent (or players) or opening has the searched text in it.
    fn found(&self, game: &Value) -> bool {
        if self.search.is_empty() {
            return true;
        }
        let wanted = self.search.to_lowercase();
        [
            Some(text(game, "white").to_string()),
            Some(text(game, "black").to_string()),
            self.opening(game),
        ]
        .into_iter()
        .flatten()
        .any(|name| name.to_lowercase().contains(&wanted))
    }

    /// The games of the source shown, newest first, that the search finds.
    fn games(&self, cx: &App) -> Vec<Value> {
        self.all_games(cx)
            .into_iter()
            .filter(|game| self.found(game))
            .collect()
    }

    /// Every game of the source shown, newest first.
    fn all_games(&self, cx: &App) -> Vec<Value> {
        let mut games: Vec<Value> = cx
            .global::<PlayedHandle>()
            .0
            .read(cx)
            .games()
            .iter()
            .filter(|game| {
                (text(game, "source") == "computer") == (self.source == Source::Computer)
            })
            .cloned()
            .collect();
        games.sort_by(|a, b| {
            let started = |game: &Value| game["startedAt"].as_f64().unwrap_or_default();
            started(b).total_cmp(&started(a))
        });
        games
    }

    /// The games the list shows under its filter, newest first.
    fn listed(&self, cx: &App) -> Vec<Value> {
        self.games(cx)
            .into_iter()
            .filter(|game| {
                self.filter == Filter::All || filter_of(&outcome(game)) == Some(self.filter)
            })
            .collect()
    }

    /// The game `step` rows from the open one in the list (-1 the one above, newer; 1 below),
    /// skipping clock sessions, which have no moves to go through.
    fn neighbour(&self, step: isize, cx: &App) -> Option<(usize, Value)> {
        let (open, _) = self.open.as_ref()?;
        let listed = self.listed(cx);
        let at = listed.iter().position(|game| game["id"] == open["id"])?;
        let mut index = at as isize;
        loop {
            index += step;
            let game = listed.get(usize::try_from(index).ok()?)?;
            if text(game, "source") != "clock" {
                return Some((index as usize, game.clone()));
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
    #[cfg(test)]
    pub(crate) fn open_first_lichess(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(game) = self.lichess.games.first().cloned() {
            self.open_lichess(game, window, cx);
        }
    }

    /// The Lichess game open, and the ids its list's page shows.
    #[cfg(test)]
    pub(crate) fn lichess_open(&self) -> (Option<String>, Vec<String>) {
        let id = |game: &Value| game["id"].as_str().unwrap_or_default().to_string();
        (
            self.lichess.open.as_ref().map(|(game, _)| id(game)),
            self.lichess.games.iter().map(id).collect(),
        )
    }

    /// The Lichess tab once loaded: its total and the openings of the games listed.
    #[cfg(test)]
    pub(crate) fn lichess_listed(&self) -> Option<(usize, Vec<String>)> {
        let tab = &self.lichess;
        (!tab.loading).then(|| {
            let openings = tab
                .games
                .iter()
                .map(|game| game["opening"].as_str().unwrap_or_default().to_string())
                .collect();
            (tab.total, openings)
        })
    }

    /// The Lichess tab searched for `text`, as typed in its field (screenshots).
    #[cfg(test)]
    pub(crate) fn search_lichess(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub fn show_lichess_game(&mut self, game: Value, window: &mut Window, cx: &mut Context<Self>) {
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
            self.lichess.open = None;
            self.load_lichess(cx);
        }
        if source == Source::Bookmarks {
            self.load_bookmarks(cx);
        }
        cx.notify();
    }

    fn load_bookmarks(&mut self, cx: &mut Context<Self>) {
        self.bookmarks_epoch += 1;
        let epoch = self.bookmarks_epoch;
        self.bookmarks = None;
        self.bookmarks_error = None;
        self.bookmarks_reconnect = false;
        let account = crate::accounts::handle(cx).read(cx).active(cx);
        let Some(account) = account.filter(|_| crate::network::online(cx)) else {
            return;
        };
        let games = cx
            .global::<crate::core::AppCore>()
            .call("bookmarkedGames", vec![json!(account)]);
        cx.spawn(async move |this, cx| {
            let answer = games.await;
            let _ = this.update(cx, |this, cx| {
                if this.bookmarks_epoch != epoch {
                    return;
                }
                match answer {
                    Ok(Value::Array(games)) => this.bookmarks = Some(games),
                    Ok(_) => this.bookmarks_reconnect = true,
                    Err(cause) => this.bookmarks_error = Some(cause.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
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
                        .icon(icon("link"))
                        .label("Reconnect")
                        .on_click(|_, window, cx| {
                            crate::accounts::handle(cx)
                                .update(cx, |accounts, cx| accounts.connect(window, cx))
                        }),
                )
                .into_any_element();
        }
        if let Some(error) = &self.bookmarks_error {
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
                            crate::pages::players::PlayersPage::open_game(
                                chosen.clone(),
                                window,
                                cx,
                            )
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

    fn open_game(&mut self, game: Value, window: &mut Window, cx: &mut Context<Self>) {
        let moves: Vec<String> = serde_json::from_value(game["moves"].clone()).unwrap_or_default();
        let replay = cx.new(|cx| {
            GameReplay::new(
                game["setup"].clone(),
                moves,
                Color::White,
                text(&game, "white").to_string(),
                text(&game, "black").to_string(),
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
            .find(|game| game["id"] == id)
            .cloned();
        let Some(game) = game else { return };
        self.source = if text(&game, "source") == "computer" {
            Source::Computer
        } else {
            Source::Board
        };
        self.filter = Filter::All;
        self.page = 0;
        if text(&game, "source") != "clock" {
            self.open_game(game, window, cx);
        }
        cx.notify();
    }

    /// What the page's buttons can do now: the open game's, or the synced games'.
    pub(crate) fn actions(&self, cx: &App) -> Vec<Action> {
        if let Some((game, _)) = &self.open {
            let mut actions = vec![Action::AllGames, Action::Export];
            if analysable(game) {
                actions.push(Action::Analyse);
            }
            return actions;
        }
        if self.source != Source::Lichess {
            return Vec::new();
        }
        if let Some((game, _)) = &self.lichess.open {
            let mut actions = vec![Action::AllGames];
            if crate::game::pgn_analysable(game["pgn"].as_str().unwrap_or_default()) {
                actions.push(Action::Review);
            }
            return actions;
        }
        let accounts = crate::accounts::handle(cx).read(cx);
        let mut actions = Vec::new();
        if !accounts.all().is_empty() && !accounts.busy {
            actions.push(Action::Sync);
        }
        if accounts.data["gameCount"].as_f64().unwrap_or_default() > 0. && self.lichess_filtered() {
            actions.push(Action::ClearFilters);
        }
        actions
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
    fn export(&mut self, game: &Value, cx: &mut Context<Self>) {
        if text(game, "source") == "clock" {
            let summary = format!(
                "{} / {}\n{}\n{}\nTime control: {}",
                text(game, "white"),
                text(game, "black"),
                text(game, "reason"),
                game["clockSummary"].as_str().unwrap_or_default(),
                text(game, "timeControl"),
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
        let name = format!("kchess-{}.pgn", text(game, "id"));
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
            .shown_games(cx)
            .iter()
            .filter(|game| text(game, "source") != "clock")
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
            chrono::Local::now().format("%Y-%m-%d")
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

    /// The games the list shows across its pages: searched, then filtered by result.
    fn shown_games(&self, cx: &App) -> Vec<Value> {
        self.games(cx)
            .into_iter()
            .filter(|game| {
                self.filter == Filter::All || filter_of(&outcome(game)) == Some(self.filter)
            })
            .collect()
    }

    /// Delete a game at once; its notice's Undo puts it back where it was.
    fn delete(&mut self, game: Value, window: &mut Window, cx: &mut Context<Self>) {
        let played = cx.global::<PlayedHandle>().0.clone();
        let id = text(&game, "id").to_string();
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

    /// A swipe steps out of the open game before leaving the page, and forward opens the game it
    /// closed again while that game is still listed and nothing else was opened.
    pub fn can_step(&self, direction: crate::swipe::Direction, cx: &App) -> bool {
        if direction < 0 {
            return self.open.is_some() || self.lichess.open.is_some();
        }
        match &self.redo {
            Some(Redo::Played(game)) => {
                self.open.is_none() && self.games(cx).iter().any(|entry| entry["id"] == game["id"])
            }
            Some(Redo::Lichess(game)) => {
                self.source == Source::Lichess
                    && self.lichess.open.is_none()
                    && self
                        .lichess
                        .games
                        .iter()
                        .any(|entry| entry["id"] == game["id"])
            }
            None => false,
        }
    }

    pub fn step(
        &mut self,
        direction: crate::swipe::Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if direction < 0 {
            if let Some((game, _)) = self.open.clone() {
                self.close_game(window, cx);
                self.redo = Some(Redo::Played(game));
            } else if let Some((game, _)) = self.lichess.open.clone() {
                self.close_lichess(window, cx);
                self.redo = Some(Redo::Lichess(game));
            }
            return;
        }
        match self.redo.take() {
            Some(Redo::Played(game)) => self.open_game(game, window, cx),
            Some(Redo::Lichess(game)) => self.open_lichess(game, window, cx),
            None => {}
        }
    }

    fn narrow(&self) -> bool {
        let width = self.area.size().width;
        width > px(0.) && width < NARROW
    }
}

/// The page's top row: where the games come from.
fn source_tabs(
    source: Source,
    cx: &mut Context<HistoryPage>,
    palette: &Palette,
) -> impl IntoElement {
    let connected = !crate::accounts::handle(cx).read(cx).connected().is_empty();
    let mut sources = vec![Source::Computer, Source::Board, Source::Lichess];
    let mut labels: Vec<(Option<Icon>, SharedString)> = vec![
        (None, "Computer".into()),
        (None, "Over the board".into()),
        (None, "Lichess".into()),
    ];
    if connected {
        sources.push(Source::Bookmarks);
        labels.push((None, "Bookmarks".into()));
    }
    let page = cx.entity().downgrade();
    widgets::pill_tabs(
        "history-source",
        labels,
        sources.iter().position(|item| *item == source).unwrap_or(0),
        move |index, _, cx| {
            let _ = page.update(cx, |page, cx| page.choose_source(sources[index], cx));
        },
        palette,
    )
}

/// An empty list's message and the action that fills it (`UEmpty`).
fn empty(glyph: Icon, title: &'static str, action: Button, palette: &Palette) -> impl IntoElement {
    widgets::card(palette)
        .items_center()
        .py(px(28.))
        .child(widgets::empty_state(glyph, title, None, palette).child(action))
}

impl HistoryPage {
    fn render_open(
        &self,
        game: &Value,
        replay: &Entity<GameReplay>,
        window: &Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let exported = game.clone();
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .px(side_padding(window))
                    .pt(PAGE_TOP)
                    .child(
                        Button::new("all-games")
                            .ghost()
                            .icon(icon("arrow-left"))
                            .label("All games")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_game(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("previous-game")
                            .ghost()
                            .icon(icon("chevron-left"))
                            .tooltip("Previous game")
                            .accessibility_label("Previous game")
                            .disabled(self.neighbour(-1, cx).is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.step_game(-1, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("next-game")
                            .ghost()
                            .icon(icon("chevron-right"))
                            .tooltip("Next game")
                            .accessibility_label("Next game")
                            .disabled(self.neighbour(1, cx).is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.step_game(1, window, cx)),
                            ),
                    )
                    .child(badge(&outcome(game), None, palette))
                    .child(format!(
                        "{} vs {}",
                        text(game, "white"),
                        text(game, "black")
                    ))
                    .child(
                        Button::new("export-open")
                            .outline()
                            .label("Export PGN")
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.export(&exported, cx)),
                            ),
                    )
                    .child({
                        let opened = game.clone();
                        Button::new("open-analysis")
                            .primary()
                            .icon(icon("chart-line"))
                            .label("Open analysis")
                            .disabled(!analysable(game))
                            .on_click(move |_, window, cx| analyse(&opened, window, cx))
                    })
                    .children(self.message.clone().map(|message| {
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(message)
                    })),
            )
            .child(div().flex_1().min_h_0().child(replay.clone()))
            .into_any_element()
    }

    fn render_list(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let games = self.games(cx);
        if games.is_empty() && self.all_games(cx).is_empty() {
            let (label, page) = if self.source == Source::Computer {
                ("Play the computer", Page::Computer)
            } else {
                ("Play over the board", Page::Local)
            };
            return empty(
                icon("history"),
                "No saved games yet",
                Button::new("history-play")
                    .primary()
                    .label(label)
                    .on_click(move |_, window, cx| crate::shell::open(page, window, cx)),
                palette,
            )
            .into_any_element();
        }
        let outcomes: Vec<String> = games.iter().map(outcome).collect();
        let count = |filter: Filter| match filter {
            Filter::All => games.len(),
            filter => outcomes
                .iter()
                .filter(|outcome| filter_of(outcome) == Some(filter))
                .count(),
        };
        let filters: Vec<(Filter, &str)> = if self.source == Source::Computer {
            vec![
                (Filter::All, "All"),
                (Filter::Win, "Wins"),
                (Filter::Loss, "Losses"),
                (Filter::Draw, "Draws"),
                (Filter::Unfinished, "Unfinished"),
            ]
        } else {
            vec![
                (Filter::All, "All"),
                (Filter::White, "White won"),
                (Filter::Black, "Black won"),
                (Filter::Draw, "Draws"),
                (Filter::Unfinished, "Unfinished"),
            ]
        };
        let page = cx.entity().downgrade();
        let choices: Vec<Filter> = filters.iter().map(|(filter, _)| *filter).collect();
        let filter_tabs = widgets::pills(
            "history-filter",
            filters
                .iter()
                .map(|(filter, label)| {
                    let count = count(*filter);
                    Pill {
                        icon: None,
                        label: (*label).into(),
                        badge: Some(count.to_string().into()),
                        // An empty filter cannot lead anywhere; the selected one stays reachable.
                        disabled: count == 0 && self.filter != *filter,
                    }
                })
                .collect(),
            choices
                .iter()
                .position(|filter| *filter == self.filter)
                .unwrap_or(0),
            move |index, _, cx| {
                let _ = page.update(cx, |page, cx| page.choose_filter(choices[index], cx));
            },
            true,
            palette,
        );
        let shown: Vec<(&Value, &String)> = games
            .iter()
            .zip(&outcomes)
            .filter(|(_, outcome)| {
                self.filter == Filter::All || filter_of(outcome) == Some(self.filter)
            })
            .collect();
        let pages = shown.len().div_ceil(PAGE_SIZE).max(1);
        let current = self.page.min(pages - 1);
        let narrow = self.narrow();
        let columns = |row: Stateful<Div>, cells: [AnyElement; 5]| {
            let [result, players, game, date, actions] = cells;
            row.test_support()
                .gap(px(12.))
                .child(div().w(px(96.)).flex_none().child(result))
                .child(div().flex_1().min_w_0().child(players))
                .when(!narrow, |row| {
                    row.child(div().w(px(112.)).flex_none().child(game))
                        .child(div().w(px(104.)).flex_none().child(date))
                })
                .child(div().w(px(32.)).flex_none().child(actions))
        };
        let head = |label: &'static str| {
            div()
                .text_size(px(12.))
                .text_color(palette.text_muted)
                .child(label)
                .into_any_element()
        };
        let header = columns(
            h_flex()
                .id("history-head")
                .pl(px(12.))
                .pr(px(8.))
                .pb(px(8.))
                .border_b_1()
                .border_color(palette.border),
            [
                head("Result"),
                head(if self.source == Source::Computer {
                    "Opponent"
                } else {
                    "Players"
                }),
                head("Game"),
                head("Date"),
                div().into_any_element(),
            ],
        );
        let rows = shown
            .iter()
            .skip(current * PAGE_SIZE)
            .take(PAGE_SIZE)
            .enumerate()
            .map(|(index, (game, outcome))| {
                let game = (*game).clone();
                let id = text(&game, "id").to_string();
                let clock = text(&game, "source") == "clock";
                let side = player_side(&game);
                // Why it ended, and the opening it reached, as a Lichess game's line has.
                let details = [Some(details(&game)), self.opening(&game)]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                let variant = game["setup"]["variant"].as_str().unwrap_or("standard");
                let mut kind = time_label(text(&game, "timeControl"));
                if variant != "standard" {
                    kind.push_str(&format!(" · {}", crate::game::variant_label(variant)));
                }
                let players = v_flex()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .min_w_0()
                            .font_weight(FontWeight::MEDIUM)
                            .children(side.map(widgets::side_dot))
                            .child(div().min_w_0().truncate().child(opponent(&game))),
                    )
                    .when(!details.is_empty(), |cell| {
                        cell.child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(details),
                        )
                    });
                let game_cell = v_flex()
                    .child(if clock {
                        "Clock only".to_string()
                    } else {
                        move_count(game["moves"].as_array().map_or(0, Vec::len))
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(kind),
                    );
                let exported = game.clone();
                let deleted = game.clone();
                let finished = game["finished"] == json!(true);
                let page = cx.entity().downgrade();
                let actions = Button::new(SharedString::from(format!("actions-{id}")))
                    .ghost()
                    .small()
                    .icon(icon("ellipsis"))
                    .accessibility_label(format!(
                        "Actions for the game from {}",
                        crate::game::game_date(game["startedAt"].as_f64().unwrap_or_default())
                    ))
                    // Aligned to the button's end, as Electron's menu is, so it stays in the window.
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                        let (exported, deleted) = (exported.clone(), deleted.clone());
                        let (export_page, delete_page) = (page.clone(), page.clone());
                        menu.item(
                            PopupMenuItem::new(if clock { "Copy summary" } else { "Export PGN" })
                                .icon(if clock {
                                    icon("copy")
                                } else {
                                    icon("download")
                                })
                                .on_click(move |_, _, cx| {
                                    let _ = export_page
                                        .update(cx, |page, cx| page.export(&exported, cx));
                                }),
                        )
                        .when(finished, |menu| {
                            menu.separator().item(
                                PopupMenuItem::new("Delete").icon(icon("trash-2")).on_click(
                                    move |_, window, cx| {
                                        let deleted = deleted.clone();
                                        let _ = delete_page.update(cx, |page, cx| {
                                            page.delete(deleted, window, cx)
                                        });
                                    },
                                ),
                            )
                        })
                    });
                let opened = game.clone();
                columns(
                    h_flex()
                        .id(("history-row", index))
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label(format!(
                            "{} vs {}",
                            text(&game, "white"),
                            text(&game, "black")
                        ))
                        .pl(px(12.))
                        .pr(px(8.))
                        .py(px(12.))
                        .when(index > 0, |row| {
                            row.border_t_1().border_color(palette.border)
                        })
                        .hover(|row| row.bg(palette.accented))
                        .when(!clock, |row| {
                            row.cursor_pointer().on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.open_game(opened.clone(), window, cx)
                                },
                            ))
                        }),
                    [
                        badge(outcome, Some(px(96.)), palette).into_any_element(),
                        players.into_any_element(),
                        game_cell.into_any_element(),
                        div()
                            .text_color(palette.text_muted)
                            .child(crate::game::game_date(
                                game["startedAt"].as_f64().unwrap_or_default(),
                            ))
                            .into_any_element(),
                        actions.into_any_element(),
                    ],
                )
                .into_any_element()
            })
            .collect::<Vec<_>>();
        let list = widgets::card(palette)
            .text_size(px(14.))
            .child(header)
            .children(rows)
            .when(shown.is_empty(), |card| {
                card.child(
                    v_flex()
                        .items_center()
                        .gap(px(8.))
                        .py(px(20.))
                        .child(
                            icon("search-x")
                                .size(px(20.))
                                .text_color(palette.text_muted),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette.highlighted)
                                .child("No games match"),
                        )
                        .child(
                            Button::new("show-all")
                                .outline()
                                .label("Show all games")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.clear_search(window, cx);
                                    this.choose_filter(Filter::All, cx)
                                })),
                        ),
                )
            })
            .when(pages > 1, |card| card.child(self.pager(current, pages, cx)));
        v_flex()
            .gap(px(16.))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(12.))
                    .child(filter_tabs)
                    .child(
                        div().w(px(200.)).child(
                            Input::new(&self.search_input)
                                .small()
                                .cleanable(true)
                                .prefix(icon("search").size(px(14.)))
                                .aria_label("Search by opponent or opening"),
                        ),
                    )
                    .child(
                        Button::new("history-export-all")
                            .outline()
                            .small()
                            .icon(icon("download"))
                            .label("Export")
                            .tooltip("Save the games listed as one PGN file")
                            .disabled(shown.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.export_shown(cx))),
                    ),
            )
            .child(list)
            .into_any_element()
    }

    /// `UPagination`: the previous page, every page by number, and the next.
    fn pager(&self, current: usize, pages: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let go = |label: SharedString,
                  target: usize,
                  disabled: bool,
                  chosen: bool,
                  cx: &mut Context<Self>| {
            Button::new(SharedString::from(format!("history-page-{label}")))
                .small()
                .when(chosen, |button| button.primary())
                .when(!chosen, |button| button.outline())
                .label(label)
                .disabled(disabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page = target;
                    cx.notify();
                }))
        };
        h_flex()
            .pt(px(16.))
            .gap(px(4.))
            .child(go(
                "‹".into(),
                current.saturating_sub(1),
                current == 0,
                false,
                cx,
            ))
            .children(
                (0..pages)
                    .map(|page| {
                        go(
                            (page + 1).to_string().into(),
                            page,
                            false,
                            page == current,
                            cx,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .child(go(
                "›".into(),
                (current + 1).min(pages - 1),
                current + 1 == pages,
                false,
                cx,
            ))
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

#[cfg(test)]
mod tests {
    use super::{move_count, time_label};

    #[test]
    fn time_controls_read_in_minutes() {
        assert_eq!(time_label("-"), "Untimed");
        assert_eq!(time_label("300+3"), "5+3");
        assert_eq!(time_label("180+2 / 600+2"), "3+2 / 10+2");
        assert_eq!(time_label("90+0"), "1.5+0");
    }

    #[test]
    fn moves_count_full_moves() {
        assert_eq!(move_count(1), "1 move");
        assert_eq!(move_count(2), "1 move");
        assert_eq!(move_count(13), "7 moves");
    }
}
