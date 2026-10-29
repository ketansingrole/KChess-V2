//! Watch → Broadcasts: the tournaments Lichess
//! relays, searchable; a tournament's rounds and boards with their clocks; one board followed
//! move by move, and Analyse.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use shakmaty::Color;

use super::broadcast_engine::{BroadcastEngine, GRID_EVALUATIONS};
use super::{Pending, WatchPage, label};
use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::tree::format_eval;
use crate::game::{rules, view};
use crate::play_layout::{LAYOUT_GAP, PANEL_MAX, PANEL_MIN};
use crate::settings::SettingsHandle;
use crate::theme::{Palette, radii};
use crate::widgets::{self, Choice, KeyboardFocus as _, Side, Tone};

/// Typing pauses this long before a search is sent.
const SEARCH_DELAY: Duration = Duration::from_millis(350);

/// A board's clocks as the last update left them; the side to move's runs.
#[derive(Clone, Copy)]
struct Clocks {
    white: Option<f64>,
    black: Option<f64>,
    ticking: Option<Color>,
    at: Instant,
}

impl Clocks {
    /// Milliseconds left for `color`.
    fn remaining(&self, color: Color) -> Option<f64> {
        let base = if color == Color::White {
            self.white
        } else {
            self.black
        }? * 1000.;
        let elapsed = if self.ticking == Some(color) {
            self.at.elapsed().as_secs_f64() * 1000.
        } else {
            0.
        };
        Some((base - elapsed).max(0.))
    }

    fn pause(&mut self) {
        let Some(color) = self.ticking else {
            return;
        };
        let left = self.remaining(color).map(|ms| ms / 1000.);
        match color {
            Color::White => self.white = left,
            Color::Black => self.black = left,
        }
        self.ticking = None;
        self.at = Instant::now();
    }
}

/// "LIVE", "Finished", "in 20 min", or the day it starts (`BroadcastCard`'s timing).
fn timing(item: &Value, now: f64) -> String {
    if item["ongoing"] == json!(true) {
        return "LIVE".into();
    }
    if item["section"] == json!("past") {
        return "Finished".into();
    }
    let Some(starts) = item["startsAt"].as_f64() else {
        return "Upcoming".into();
    };
    let minutes = ((starts - now) / 60_000.).ceil();
    if minutes <= 0. {
        "Starting soon".into()
    } else if minutes < 60. {
        format!("in {minutes} min")
    } else if minutes < 24. * 60. {
        format!("in {} hours", (minutes / 60.).ceil())
    } else {
        use chrono::TimeZone as _;
        chrono::Local
            .timestamp_millis_opt(starts as i64)
            .single()
            .map(|date| date.format("%b %-d, %-I:%M %p").to_string())
            .unwrap_or_default()
    }
}

/// `resultText`.
fn result_text(game: &Value) -> String {
    if game["ongoing"] == json!(true) {
        return "Live".into();
    }
    match game["result"].as_str().unwrap_or_default() {
        "1-0" => "White won · 1–0".into(),
        "0-1" => "Black won · 0–1".into(),
        "1/2-1/2" => "Draw · ½–½".into(),
        other => other.to_string(),
    }
}

pub(super) fn setup_of(game: &Value) -> Value {
    json!({ "variant": "standard", "fen": game["startFen"] })
}

pub(super) fn moves_of(game: &Value) -> Vec<String> {
    serde_json::from_value(game["moves"].clone()).unwrap_or_default()
}

pub struct Broadcasts {
    list: Option<Vec<Value>>,
    loading: bool,
    error: Option<SharedString>,
    query: Entity<InputState>,
    results: Vec<Value>,
    searching: bool,
    search_error: Option<SharedString>,
    search_epoch: u64,
    pub(super) tour: Option<Value>,
    round: String,
    round_select: Entity<SelectState<Vec<Choice>>>,
    /// The round's boards in the order they first came, by id.
    pub(super) games: Vec<Value>,
    clocks: HashMap<String, Clocks>,
    pub selected: String,
    /// The tournament and board the last swipe back left (no board: the tournament itself), to
    /// return to going forward while nothing else changed since.
    redo: Option<(String, String)>,
    ended: bool,
    ply: Option<usize>,
    pub(super) orientation: Color,
    board: Entity<ChessBoard>,
    /// Stockfish on the board followed and the grid's estimates.
    pub(super) engine: BroadcastEngine,
    previews: HashMap<String, Entity<ChessBoard>>,
    _search: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Broadcasts {
    pub fn new(window: &mut Window, cx: &mut Context<WatchPage>) -> Broadcasts {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search broadcasts"));
        let round_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let subscriptions = vec![
            cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.search(cx);
                }
            }),
            cx.subscribe_in(
                &round_select,
                window,
                |this, _, event: &SelectEvent<Vec<Choice>>, window, cx| {
                    if let SelectEvent::Confirm(Some(round)) = event
                        && *round != this.broadcasts.round.as_str()
                    {
                        this.open_round(round.to_string(), String::new(), window, cx);
                    }
                },
            ),
        ];
        Broadcasts {
            list: None,
            loading: false,
            error: None,
            query,
            results: Vec::new(),
            searching: false,
            search_error: None,
            search_epoch: 0,
            tour: None,
            round: String::new(),
            round_select,
            games: Vec::new(),
            clocks: HashMap::new(),
            selected: String::new(),
            redo: None,
            ended: false,
            ply: None,
            orientation: Color::White,
            board: cx.new(|_| {
                ChessBoard::new(BoardState::shown(
                    "8/8/8/8/8/8/8/8 w",
                    "standard",
                    Color::White,
                    None,
                ))
            }),
            previews: HashMap::new(),
            engine: BroadcastEngine::new(window, cx),
            _search: None,
            _subscriptions: subscriptions,
        }
    }

    /// Some clock is running.
    pub fn ticking(&self) -> bool {
        self.clocks.values().any(|clock| clock.ticking.is_some())
    }

    pub fn pause_clocks(&mut self) {
        for clock in self.clocks.values_mut() {
            clock.pause();
        }
    }

    /// The open tournament's round, to stream again after a tab switch or a return to the page.
    pub fn resumable(&self) -> Option<String> {
        (self.tour.is_some() && !self.round.is_empty()).then(|| self.round.clone())
    }

    pub(super) fn current(&self) -> Option<&Value> {
        self.games
            .iter()
            .find(|game| game["id"].as_str() == Some(self.selected.as_str()))
    }

    /// A round's update: new boards and moves; an error once it ends badly.
    pub fn update(&mut self, update: &Value) -> Option<String> {
        let ended = update["ended"] == json!(true);
        for game in update["games"].as_array().into_iter().flatten() {
            let id = game["id"].as_str().unwrap_or_default().to_string();
            let previous = self.games.iter().position(|g| g["id"] == game["id"]);
            let changed = previous.is_none_or(|index| {
                let old = &self.games[index];
                ["fen", "whiteClock", "blackClock", "ongoing"]
                    .iter()
                    .any(|key| old[*key] != game[*key])
            });
            // Repeated PGNs must not reset a clock that is already counting down.
            if changed {
                let moving = game["ongoing"] == json!(true)
                    && game["moves"]
                        .as_array()
                        .is_some_and(|moves| !moves.is_empty())
                    && !ended;
                let turn =
                    if game["fen"].as_str().and_then(|fen| fen.split(' ').nth(1)) == Some("b") {
                        Color::Black
                    } else {
                        Color::White
                    };
                self.clocks.insert(
                    id.clone(),
                    Clocks {
                        white: game["whiteClock"].as_f64(),
                        black: game["blackClock"].as_f64(),
                        ticking: moving.then_some(turn),
                        at: Instant::now(),
                    },
                );
                self.previews.remove(&id);
            }
            match previous {
                Some(index) => self.games[index] = game.clone(),
                None => self.games.push(game.clone()),
            }
        }
        if ended {
            self.pause_clocks();
            self.ended = true;
            return update["error"].as_str().map(str::to_string);
        }
        None
    }

    pub(super) fn shown_ply(&self) -> usize {
        let count = self.current().map_or(0, |game| moves_of(game).len());
        self.ply.unwrap_or(count).min(count)
    }

    pub(super) fn redraw(&mut self, cx: &mut App) {
        let Some(game) = self.current().cloned() else {
            return;
        };
        let moves = moves_of(&game);
        let ply = self.shown_ply();
        let mut state = if self.engine.variation_base.is_some() {
            let history = self.history();
            view::board_state(
                &setup_of(&game),
                &history,
                history.len(),
                self.orientation,
                None,
            )
        } else if ply < moves.len() {
            view::board_state(&setup_of(&game), &moves, ply, self.orientation, None)
        } else {
            BoardState::shown(
                game["fen"].as_str().unwrap_or_default(),
                "standard",
                self.orientation,
                game["lastMove"].as_str(),
            )
        };
        state.arrows = self.engine_arrows(cx);
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
    }

    /// ←/→ and ↑/↓ step through the open board's moves; F turns it.
    pub fn key(&mut self, key: &str, cx: &mut App) -> bool {
        let Some(game) = self.current() else {
            return false;
        };
        let count = moves_of(game).len();
        if key == "f" {
            self.orientation = !self.orientation;
        } else if let Some(ply) = view::navigate(key, self.shown_ply(), count) {
            self.return_to_game();
            self.ply = (ply < count).then_some(ply);
        } else {
            return false;
        }
        self.redraw(cx);
        true
    }

    fn select_game(&mut self, id: String, cx: &mut App) {
        self.redo = None;
        self.return_to_game();
        self.selected = id;
        self.ply = None;
        self.redraw(cx);
    }

    fn preview(&mut self, game: &Value, cx: &mut App) -> Entity<ChessBoard> {
        let id = game["id"].as_str().unwrap_or_default().to_string();
        self.previews
            .entry(id)
            .or_insert_with(|| {
                let mut state = BoardState::shown(
                    game["fen"].as_str().unwrap_or_default(),
                    "standard",
                    Color::White,
                    game["lastMove"].as_str(),
                );
                state.preview = true;
                cx.new(|_| ChessBoard::new(state))
            })
            .clone()
    }

    pub fn load(&mut self, cx: &mut Context<WatchPage>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        let call = cx.global::<AppCore>().call("broadcasts", Vec::new());
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                let broadcasts = &mut this.broadcasts;
                broadcasts.loading = false;
                match answer {
                    Ok(list) => {
                        broadcasts.list = Some(list.as_array().cloned().unwrap_or_default())
                    }
                    Err(cause) => broadcasts.error = Some(cause.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl WatchPage {
    /// A search, once typing pauses.
    fn search(&mut self, cx: &mut Context<Self>) {
        let broadcasts = &mut self.broadcasts;
        let query = broadcasts.query.read(cx).value().trim().to_string();
        broadcasts.search_epoch += 1;
        let epoch = broadcasts.search_epoch;
        broadcasts.results.clear();
        broadcasts.search_error = None;
        broadcasts.searching = !query.is_empty();
        cx.notify();
        if query.is_empty() {
            broadcasts._search = None;
            return;
        }
        let core = cx.global::<AppCore>().clone();
        broadcasts._search = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let answer = core.call("broadcasts", vec![json!(query)]).await;
            let _ = this.update(cx, |this, cx| {
                let broadcasts = &mut this.broadcasts;
                if broadcasts.search_epoch != epoch {
                    return;
                }
                broadcasts.searching = false;
                match answer {
                    Ok(found) => broadcasts.results = found.as_array().cloned().unwrap_or_default(),
                    Err(cause) => broadcasts.search_error = Some(cause.message.into()),
                }
                cx.notify();
            });
        }));
    }

    /// `openTour`: the tournament, at its default round (or the live one, or the last).
    fn open_tour(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.broadcasts.redo = None;
        self.stop(cx);
        let epoch = self.epoch;
        let call = cx
            .global::<AppCore>()
            .call("broadcastTour", vec![json!(id)]);
        cx.spawn_in(window, async move |this, cx| {
            let answer = call.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.epoch != epoch {
                    return;
                }
                match answer {
                    Ok(tour) => {
                        let rounds = tour["rounds"].as_array().cloned().unwrap_or_default();
                        let round = rounds
                            .iter()
                            .find(|r| r["id"] == tour["defaultRoundId"])
                            .or_else(|| rounds.iter().find(|r| r["ongoing"] == json!(true)))
                            .or(rounds.last())
                            .and_then(|r| r["id"].as_str().map(str::to_string));
                        let items: Vec<Choice> = rounds
                            .iter()
                            .map(|r| {
                                let name = r["name"].as_str().unwrap_or_default();
                                let state = if r["ongoing"] == json!(true) {
                                    " · live"
                                } else if r["finished"] == json!(true) {
                                    ""
                                } else {
                                    " · upcoming"
                                };
                                Choice::new(
                                    r["id"].as_str().unwrap_or_default().to_string(),
                                    format!("{name}{state}"),
                                )
                            })
                            .collect();
                        this.broadcasts
                            .round_select
                            .update(cx, |select, cx| select.set_items(items, window, cx));
                        this.broadcasts.tour = Some(tour);
                        if let Some(round) = round {
                            this.open_round(round, String::new(), window, cx);
                        }
                    }
                    Err(cause) => this.error = Some(cause.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `openRound`: a round's live feed; `game` stays selected once its board arrives.
    pub(super) fn open_round(
        &mut self,
        round: String,
        game: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.early.clear();
        self.session = None;
        self.round_session = None;
        self.phase = None;
        self.error = None;
        self.target = None;
        self.live.clear();
        let broadcasts = &mut self.broadcasts;
        broadcasts.round = round.clone();
        broadcasts.games.clear();
        broadcasts.clocks.clear();
        broadcasts.previews.clear();
        broadcasts.selected = game;
        broadcasts.ply = None;
        broadcasts.ended = false;
        let shown = SharedString::from(round.clone());
        broadcasts.round_select.update(cx, |select, cx| {
            select.set_selected_value(&shown, window, cx)
        });
        self.pending = Some(Pending::Round);
        cx.notify();
        let call = cx
            .global::<AppCore>()
            .call("watchBroadcast", vec![json!(round)]);
        cx.spawn(async move |this, cx| {
            let answer = call.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                match answer {
                    Ok(session) => {
                        this.round_session = session.as_f64();
                        this.replay_early(cx);
                        this.broadcasts.redraw(cx);
                    }
                    Err(cause) => {
                        this.pending = None;
                        this.early.clear();
                        this.error = Some(cause.message.into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn leave_tour(&mut self, cx: &mut Context<Self>) {
        let broadcasts = &mut self.broadcasts;
        broadcasts.redo = None;
        broadcasts.tour = None;
        broadcasts.round.clear();
        // No board stays selected: its keys belong to the tour just left.
        broadcasts.selected.clear();
        broadcasts.games.clear();
        broadcasts.clocks.clear();
        broadcasts.ended = false;
        self.stop(cx);
    }

    /// A swipe steps out of a broadcast's board, then its tournament, before leaving the page
    /// (`useBroadcastNav`); forward returns to what it left while that is still there.
    pub fn can_step(&self, direction: crate::swipe::Direction) -> bool {
        if self.tab != super::Tab::Broadcasts {
            return false;
        }
        let broadcasts = &self.broadcasts;
        if direction < 0 {
            return !broadcasts.selected.is_empty() || broadcasts.tour.is_some();
        }
        match &broadcasts.redo {
            Some((tour, game)) if !game.is_empty() => {
                broadcasts.tour.as_ref().and_then(|t| t["id"].as_str()) == Some(tour.as_str())
                    && broadcasts.selected.is_empty()
                    && broadcasts
                        .games
                        .iter()
                        .any(|g| g["id"].as_str() == Some(game))
            }
            Some(_) => broadcasts.tour.is_none(),
            None => false,
        }
    }

    pub fn step(
        &mut self,
        direction: crate::swipe::Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tour_id = |broadcasts: &Broadcasts| {
            broadcasts
                .tour
                .as_ref()
                .and_then(|tour| tour["id"].as_str())
                .unwrap_or_default()
                .to_string()
        };
        if direction < 0 {
            let tour = tour_id(&self.broadcasts);
            if !self.broadcasts.selected.is_empty() {
                let game = std::mem::take(&mut self.broadcasts.selected);
                self.broadcasts.redo = Some((tour, game));
            } else if self.broadcasts.tour.is_some() {
                self.leave_tour(cx);
                self.broadcasts.redo = Some((tour, String::new()));
            }
        } else if let Some((tour, game)) = self.broadcasts.redo.take() {
            if game.is_empty() {
                self.open_tour(tour, window, cx);
            } else {
                self.broadcasts.select_game(game, cx);
            }
        }
        cx.notify();
    }

    /// Back, the tournament's name and its round, beside the tabs while a tournament is open.
    pub(super) fn broadcasts_toolbar(
        &self,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Option<AnyElement> {
        let tour = self.broadcasts.tour.as_ref()?;
        let rounds = tour["rounds"].as_array().map_or(0, Vec::len);
        Some(
            h_flex()
                .gap(px(8.))
                .child(
                    Button::new("broadcast-back")
                        .ghost()
                        .icon(icon("arrow-left"))
                        .accessibility_label("All broadcasts")
                        .on_click(cx.listener(|this, _, _, cx| this.leave_tour(cx))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(15.))
                        .font_weight(FontWeight(650.))
                        .text_color(palette.highlighted)
                        .child(tour["name"].as_str().unwrap_or_default().to_string()),
                )
                .when(self.broadcasts.current().is_none(), |bar| {
                    let allowed = cx
                        .global::<crate::online::OnlineHandle>()
                        .0
                        .read(cx)
                        .assistance_allowed();
                    bar.child(
                        h_flex()
                            .gap(px(6.))
                            .text_size(px(13.))
                            .child(
                                Switch::new("broadcast-grid-evaluations")
                                    .small()
                                    .checked(crate::prefs::get(cx, GRID_EVALUATIONS, true))
                                    .disabled(!self.broadcasts.engine_ready() || !allowed)
                                    .accessibility_label("Evaluation bars")
                                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                                        crate::prefs::set(cx, GRID_EVALUATIONS, *on);
                                        this.engine_changed(cx);
                                    })),
                            )
                            .child("Evaluation bars"),
                    )
                })
                .child(
                    div().w(px(190.)).child(
                        Select::new(&self.broadcasts.round_select)
                            .small()
                            .placeholder(if rounds > 0 {
                                "Choose a round"
                            } else {
                                "No rounds yet"
                            }),
                    ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn broadcasts_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        if self.broadcasts.tour.is_none() {
            return self.directory(cx, palette);
        }
        if self.broadcasts.current().is_some() {
            return self.broadcast_game(window, cx, palette);
        }
        let games = self.broadcasts.games.clone();
        let grid_evaluations = crate::prefs::get(cx, GRID_EVALUATIONS, true)
            && cx
                .global::<crate::online::OnlineHandle>()
                .0
                .read(cx)
                .assistance_allowed();
        let cells: Vec<AnyElement> = games
            .iter()
            .enumerate()
            .map(|(index, game)| {
                let board = self.broadcasts.preview(game, cx);
                let id = game["id"].as_str().unwrap_or_default().to_string();
                let ongoing = game["ongoing"] == json!(true);
                let result = game["result"].as_str().unwrap_or_default().to_string();
                // The grid's estimates, while they are wanted and allowed.
                let estimate = grid_evaluations.then(|| self.broadcasts.grid_evaluation(game));
                let name = |side: &str| {
                    div()
                        .text_size(px(12.))
                        .truncate()
                        .child(label(&game[side]))
                };
                v_flex()
                    .id(("broadcast-cell", index))
                    .role(Role::Button)
                    .keyboard_focus(palette.primary)
                    .aria_label(format!(
                        "{} – {}",
                        label(&game["white"]),
                        label(&game["black"])
                    ))
                    .gap(px(4.))
                    .p(px(8.))
                    .w(px(190.))
                    .rounded(radii::LG)
                    .border_1()
                    .border_color(palette.border)
                    .bg(palette.elevated)
                    .cursor_pointer()
                    .hover(|cell| cell.border_color(palette.border_accented))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.broadcasts.select_game(id.clone(), cx);
                        this.engine_changed(cx);
                    }))
                    .child(name("black"))
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(174.))
                            .gap(px(6.))
                            .when_some(estimate.clone(), |row, (line, _)| {
                                row.child(
                                    crate::engine_view::eval_bar(
                                        line.as_ref(),
                                        Color::White,
                                        (!ongoing).then_some(result.as_str()),
                                    )
                                    .w(px(8.)),
                                )
                            })
                            .child(div().flex_1().h_full().child(board)),
                    )
                    .child(name("white"))
                    .child(
                        h_flex()
                            .justify_between()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(if ongoing {
                                "Live".to_string()
                            } else {
                                result.clone()
                            })
                            .when_some(
                                estimate.filter(|_| ongoing),
                                |status, (line, unavailable)| {
                                    let (text, tip) = match (&line, unavailable) {
                                        (_, true) => {
                                            ("—".to_string(), "Evaluation unavailable".to_string())
                                        }
                                        (Some(line), _) => (
                                            format_eval(Some(line)),
                                            format!(
                                                "Stockfish · depth {}",
                                                line["depth"]
                                                    .as_f64()
                                                    .or(self
                                                        .broadcasts
                                                        .engine
                                                        .engine
                                                        .update
                                                        .as_ref()
                                                        .and_then(|u| u["depth"].as_f64()))
                                                    .unwrap_or_default()
                                            ),
                                        ),
                                        (None, _) => (
                                            format_eval(None),
                                            "Waiting for evaluation".to_string(),
                                        ),
                                    };
                                    status.child(
                                        div()
                                            .id(("broadcast-cell-eval", index))
                                            .tooltip(move |window, cx| {
                                                gpui_kit::component::tooltip::Tooltip::new(
                                                    tip.clone(),
                                                )
                                                .build(window, cx)
                                            })
                                            .child(text),
                                    )
                                },
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        v_flex()
            .gap(px(12.))
            .when(self.broadcasts.ended && games.is_empty(), |body| {
                body.child(widgets::hint("No games in this round yet.", palette))
            })
            .child(h_flex().flex_wrap().gap(px(14.)).children(cells))
            .into_any_element()
    }

    /// The tournaments: live now, coming up and recently finished, or the search's results.
    fn directory(&mut self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let broadcasts = &self.broadcasts;
        let searching = !broadcasts.query.read(cx).value().trim().is_empty();
        let sections: Vec<(&str, Vec<Value>)> = if searching {
            vec![("Search results", broadcasts.results.clone())]
        } else {
            let list = broadcasts.list.clone().unwrap_or_default();
            [
                ("active", "Live now"),
                ("upcoming", "Coming up"),
                ("past", "Recently finished"),
            ]
            .into_iter()
            .map(|(key, title)| {
                (
                    title,
                    list.iter()
                        .filter(|item| item["section"] == json!(key))
                        .cloned()
                        .collect(),
                )
            })
            .filter(|(_, items): &(&str, Vec<Value>)| !items.is_empty())
            .collect()
        };
        let disabled = Self::playing_own_game(cx);
        let now = chrono::Utc::now().timestamp_millis() as f64;
        let status = if broadcasts.searching {
            Some("Searching…")
        } else if !searching && broadcasts.loading {
            Some("Loading broadcasts…")
        } else if searching && broadcasts.results.is_empty() && broadcasts.search_error.is_none() {
            Some("No broadcasts found.")
        } else {
            None
        };
        let mut index = 0usize;
        v_flex()
            .gap(px(20.))
            .child(
                h_flex()
                    .justify_between()
                    .flex_wrap()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Tournament broadcasts"),
                    )
                    .child(
                        div().w(px(300.)).child(
                            Input::new(&broadcasts.query)
                                .prefix(icon("search").size(px(14.)))
                                .aria_label("Search broadcasts"),
                        ),
                    ),
            )
            .children(broadcasts.search_error.clone().map(|error| {
                div()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(error)
            }))
            .children(status.map(|status| widgets::hint(status, palette)))
            .when(!searching && broadcasts.error.is_some(), |body| {
                body.child(
                    h_flex()
                        .gap(px(12.))
                        .text_size(px(13.))
                        .child(
                            div()
                                .text_color(palette.error)
                                .child(broadcasts.error.clone().unwrap_or_default()),
                        )
                        .child(
                            Button::new("broadcasts-retry")
                                .small()
                                .outline()
                                .label("Retry broadcasts")
                                .on_click(cx.listener(|this, _, _, cx| this.broadcasts.load(cx))),
                        ),
                )
            })
            .children(sections.into_iter().map(|(title, items)| {
                v_flex()
                    .gap(px(12.))
                    .child(widgets::heading(title, palette))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(px(14.))
                            .children(items.into_iter().map(|item| {
                                index += 1;
                                let tour = item["tourId"].as_str().unwrap_or_default().to_string();
                                let live = item["ongoing"] == json!(true);
                                // The cover, once it has arrived; the placeholder until then.
                                let cover = item["image"].as_str().and_then(|url| {
                                    crate::remote_image::handle(cx)
                                        .update(cx, |images, cx| images.get(url, cx))
                                });
                                v_flex()
                                    .id(("broadcast-card", index))
                                    .role(Role::Button)
                                    .keyboard_focus(palette.primary)
                                    .aria_label(
                                        item["tourName"].as_str().unwrap_or_default().to_string(),
                                    )
                                    .w(px(260.))
                                    .rounded(radii::XL)
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(if live { palette.error } else { palette.border })
                                    .bg(palette.elevated)
                                    .when(disabled, |card| card.opacity(0.6))
                                    .when(!disabled, |card| {
                                        card.cursor_pointer()
                                            .hover(|card| {
                                                card.border_color(palette.border_accented)
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_tour(tour.clone(), window, cx)
                                            }))
                                    })
                                    .child(
                                        div()
                                            .h(px(130.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .overflow_hidden()
                                            .bg(palette.accented)
                                            .map(|area| match cover {
                                                Some(cover) => area.child(
                                                    img(cover)
                                                        .size_full()
                                                        .object_fit(ObjectFit::Cover),
                                                ),
                                                None => area.child(
                                                    icon("radio")
                                                        .size(px(28.))
                                                        .text_color(palette.text_muted),
                                                ),
                                            }),
                                    )
                                    .child(
                                        v_flex()
                                            .gap(px(4.))
                                            .p(px(12.))
                                            .child(
                                                h_flex()
                                                    .justify_between()
                                                    .gap(px(8.))
                                                    .text_size(px(12.))
                                                    .text_color(palette.text_muted)
                                                    .child(
                                                        div().truncate().child(
                                                            item["roundName"]
                                                                .as_str()
                                                                .unwrap_or_default()
                                                                .to_string(),
                                                        ),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_none()
                                                            .when(live, |time| {
                                                                time.text_color(palette.error)
                                                                    .font_weight(FontWeight::BOLD)
                                                            })
                                                            .child(timing(&item, now)),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(14.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child(
                                                        item["tourName"]
                                                            .as_str()
                                                            .unwrap_or_default()
                                                            .to_string(),
                                                    ),
                                            )
                                            .children(item["players"].as_str().map(|players| {
                                                div()
                                                    .text_size(px(12.))
                                                    .text_color(palette.text_muted)
                                                    .line_clamp(2)
                                                    .child(players.to_string())
                                            })),
                                    )
                            })),
                    )
            }))
            .into_any_element()
    }

    /// One board followed: players and clocks, the moves, All boards and Analyse.
    fn broadcast_game(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let broadcasts = &self.broadcasts;
        let Some(game) = broadcasts.current().cloned() else {
            return div().into_any_element();
        };
        let set = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .piece_set
            .clone();
        let orientation = broadcasts.orientation;
        let moves = moves_of(&game);
        let ply = broadcasts.shown_ply();
        let setup = setup_of(&game);
        let fen = if ply < moves.len() {
            view::position_at(&setup, &moves, ply)
                .as_ref()
                .map(kchess_domain::rules::make_fen)
                .unwrap_or_default()
        } else {
            game["fen"].as_str().unwrap_or_default().to_string()
        };
        let clocks = broadcasts
            .clocks
            .get(game["id"].as_str().unwrap_or_default())
            .copied();
        let line = |color: Color| {
            let player = &game[if color == Color::White {
                "white"
            } else {
                "black"
            }];
            let running = clocks.is_some_and(|clocks| clocks.ticking == Some(color));
            let clock = clocks
                .and_then(|clocks| clocks.remaining(color))
                .and_then(|ms| {
                    rules("formatClock", &[json!(ms)])
                        .ok()
                        .and_then(|text| text.as_str().map(str::to_string))
                });
            widgets::player_line(
                h_flex()
                    .gap(px(10.))
                    .child(widgets::avatar(
                        icon("user"),
                        if color == Color::White {
                            Side::White
                        } else {
                            Side::Black
                        },
                        running,
                        palette,
                    ))
                    .child(label(player))
                    .children(widgets::material(
                        &fen,
                        color == Color::White,
                        &set,
                        palette,
                    )),
                None,
                clock.map(|text| widgets::clock(text.into(), running, palette).into_any_element()),
            )
        };
        // The bar's room is kept beside the board whether or not it shows, as Electron's padding.
        let bar_room = crate::engine_view::EVAL_BAR + px(8.);
        let board_size = self.layout.board_size_in(bar_room, super::HEADER, window);
        let eval_bar = self.broadcast_eval_bar(cx);
        let board_column = v_flex()
            .w(board_size + bar_room)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(div().pl(bar_room).child(line(!orientation)))
            .child(
                h_flex()
                    .h(board_size)
                    .gap(px(8.))
                    .map(|row| match eval_bar {
                        Some(bar) => row.child(bar),
                        None => row.pl(bar_room),
                    })
                    .child(
                        div()
                            .size(board_size)
                            .child(crate::board::cached(&broadcasts.board)),
                    ),
            )
            .child(div().pl(bar_room).child(line(orientation)));
        let sans = view::sans(&setup, &moves);
        let page = cx.entity().downgrade();
        let select = move |ply: usize, _: &mut Window, cx: &mut App| {
            let _ = page.update(cx, |page, cx| {
                let count = page
                    .broadcasts
                    .current()
                    .map_or(0, |game| moves_of(game).len());
                page.broadcasts.return_to_game();
                page.broadcasts.ply = (ply < count).then_some(ply);
                page.engine_changed(cx);
            });
        };
        let flip_page = cx.entity().downgrade();
        let flip: widgets::ClickHandler = Box::new(move |_, _, cx| {
            let _ = flip_page.update(cx, |page, cx| {
                page.broadcasts.orientation = !page.broadcasts.orientation;
                page.broadcasts.redraw(cx);
                cx.notify();
            });
        });
        let ongoing = game["ongoing"] == json!(true);
        let banner =
            h_flex()
                .gap(px(8.))
                .child(div().flex_1().min_w_0().child(widgets::StatusBanner::new(
                    Tone::Plain,
                    if ongoing { icon("radio") } else { icon("flag") },
                    result_text(&game).into(),
                )));
        let pgn = game["pgn"].as_str().unwrap_or_default().to_string();
        let origin = json!({ "white": game["white"]["name"], "black": game["black"]["name"] });
        let panel = widgets::side_panel(palette)
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .child(self.broadcast_engine_panel(cx, palette))
            .child(banner)
            .child(div().flex_1().min_h(px(120.)).child(widgets::moves_list(
                "broadcast-moves",
                &sans,
                ply,
                view::black_first(&setup),
                "No moves yet.".into(),
                select.clone(),
                palette,
            )))
            .child(widgets::move_controls(
                ply,
                moves.len(),
                select,
                ("Flip board (F)".into(), flip),
            ))
            .child(
                h_flex()
                    .gap(px(8.))
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(palette.border)
                    .child(
                        Button::new("broadcast-all")
                            .outline()
                            .icon(icon("layout-grid"))
                            .label("All boards")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.broadcasts.selected.clear();
                                this.broadcasts.redo = None;
                                this.broadcasts.return_to_game();
                                this.engine_changed(cx);
                            })),
                    )
                    .child(
                        Button::new("broadcast-analyse")
                            .outline()
                            .icon(icon("microscope"))
                            .label("Analyse")
                            .on_click(move |_, window, cx| {
                                let (pgn, origin) = (pgn.clone(), origin.clone());
                                crate::shell::replace_analysis(
                                    window,
                                    cx,
                                    move |analysis, _, cx| {
                                        analysis.open_game(
                                            &pgn,
                                            ply,
                                            Some(orientation),
                                            Some(origin.clone()),
                                            false,
                                            cx,
                                        )
                                    },
                                );
                            }),
                    ),
            );
        if self.layout.is_stacked() {
            return v_flex()
                .items_center()
                .gap(LAYOUT_GAP)
                .child(board_column)
                .child(div().w(board_size).max_w_full().child(panel))
                .into_any_element();
        }
        h_flex()
            .items_start()
            .justify_center()
            .gap(LAYOUT_GAP)
            .child(board_column)
            .child(panel)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serde_json::json;
    use shakmaty::Color;

    use super::{Clocks, result_text, timing};

    #[test]
    fn cards_say_when_a_broadcast_is() {
        let now = 1_000_000_000_000.;
        assert_eq!(timing(&json!({ "ongoing": true }), now), "LIVE");
        assert_eq!(timing(&json!({ "section": "past" }), now), "Finished");
        assert_eq!(
            timing(
                &json!({ "section": "upcoming", "startsAt": now + 20. * 60_000. }),
                now
            ),
            "in 20 min"
        );
        assert_eq!(
            timing(
                &json!({ "section": "upcoming", "startsAt": now + 150. * 60_000. }),
                now
            ),
            "in 3 hours"
        );
        assert_eq!(
            timing(&json!({ "section": "upcoming", "startsAt": now - 1. }), now),
            "Starting soon"
        );
    }

    #[test]
    fn results_read_in_words() {
        assert_eq!(result_text(&json!({ "ongoing": true })), "Live");
        assert_eq!(result_text(&json!({ "result": "0-1" })), "Black won · 0–1");
        assert_eq!(result_text(&json!({ "result": "1/2-1/2" })), "Draw · ½–½");
    }

    #[test]
    fn only_the_side_to_move_loses_time_and_a_pause_keeps_it() {
        let mut clocks = Clocks {
            white: Some(60.),
            black: Some(30.),
            ticking: Some(Color::Black),
            at: Instant::now() - Duration::from_secs(10),
        };
        assert_eq!(clocks.remaining(Color::White), Some(60_000.));
        let black = clocks.remaining(Color::Black).unwrap();
        assert!((19_000. ..=20_100.).contains(&black));
        clocks.pause();
        assert_eq!(clocks.ticking, None);
        assert!((19. ..=20.1).contains(&clocks.black.unwrap()));
    }
}
