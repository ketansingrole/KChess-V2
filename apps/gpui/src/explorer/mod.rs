//! The opening explorer and tablebases: the Lichess, Masters and Player
//! databases, the game databases on this computer and the tablebase for a position, as the core
//! looks them up (`positionLookup`), as a list on the analysis board or as a grid of boards
//! beside an opening's line. Its settings are a view preference (`kchess:explorer`).

use crate::loading::Epoch;
use futures::FutureExt as _;
use futures::future::LocalBoxFuture;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, Fen, GameDbGameId, GameId};
use kchess_contracts::types::{
    ExplorerGame, ExplorerSpeed, LookupMove, LookupOptions, PositionLookup, PositionLookupKind,
};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::{CoreError, ErrorKind, Level};
use serde::{Deserialize, Serialize};
use shakmaty::Color;

use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::STANDARD_FEN;
use crate::game::played::GameOrigin;
use crate::game_db::{self, GameDbHandle};
use crate::pages::analysis::Opening;
use crate::play_layout::Measured;
use crate::theme::{Palette, radii};
use crate::widgets::Pressable as _;
use crate::widgets::{self, Choice, KeyboardFocus as _, Pill};
use kchess_board::live_game::Refused;
use kchess_domain::misc::constants::EXPLORER_RATINGS;
use kchess_domain::variant::Variant;

mod draw;
mod prefs;

use draw::*;
use prefs::*;

const PREFS: &str = "kchess:explorer";
/// Following moves waits for the board to settle.
const FOLLOW_DELAY: Duration = Duration::from_millis(350);
/// The grid's tiles.
const TILE_MIN: Pixels = px(120.);
const TILE_GAP: Pixels = px(10.);

/// The databases in the order the explorer offers them.
const KINDS: [PositionLookupKind; 5] = [
    PositionLookupKind::Opening,
    PositionLookupKind::Masters,
    PositionLookupKind::Player,
    PositionLookupKind::Database,
    PositionLookupKind::Tablebase,
];

/// The lowest rating of both players a game database's lookup can ask for; 0 is any.
const MIN_ELOS: [u32; 5] = [0, 2000, 2200, 2400, 2600];

fn kind_label(kind: PositionLookupKind) -> &'static str {
    match kind {
        PositionLookupKind::Opening => "Lichess",
        PositionLookupKind::Masters => "Masters",
        PositionLookupKind::Player => "Player",
        PositionLookupKind::Tablebase => "Tablebase",
        PositionLookupKind::Database => "Database",
    }
}

/// The databases that hold games of other variants: Masters games and the tablebase are standard
/// chess only.
fn holds_variants(kind: PositionLookupKind) -> bool {
    matches!(
        kind,
        PositionLookupKind::Opening | PositionLookupKind::Player
    )
}

#[cfg(test)]
use crate::widgets::speed_label;

/// How the explorer shows a position's continuations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    /// A list of moves on the analysis board, looked up when asked or while following moves.
    List,
    /// A grid of boards beside an opening's line, looked up whenever the position changes.
    /// It offers no tablebase and names each continuation's opening instead of the position's.
    Grid,
}

impl Layout {
    fn always_follows(self) -> bool {
        self == Layout::Grid
    }

    fn offers(self, kind: PositionLookupKind) -> bool {
        self == Layout::List || kind != PositionLookupKind::Tablebase
    }
}

pub enum ExplorerEvent {
    /// A move chosen from the results.
    Play(String),
}

impl EventEmitter<ExplorerEvent> for PositionExplorer {}

pub struct PositionExplorer {
    layout: Layout,
    fen: String,
    /// The moves from the starting position to `fen`: each continuation is named after its opening.
    moves: Option<Vec<String>>,
    result: Option<PositionLookup>,
    /// The last lookup's failure: with no account it offers Connect Lichess, otherwise Try again.
    failure: Option<CoreError>,
    /// The last lookup was refused while help was held back: made again once it is lent.
    refused: Refused,
    busy: bool,
    epoch: Epoch,
    show_filters: bool,
    accounts: Vec<String>,
    player: Entity<InputState>,
    since: Entity<InputState>,
    /// A game database's player, and which database.
    db_player: Entity<InputState>,
    databases: Entity<SelectState<Vec<Choice>>>,
    /// The databases the select lists, by id and name: an import's progress changes nothing.
    listed: Vec<(String, String)>,
    /// The game database the result shown was looked up in: its games open from there.
    result_database: Option<String>,
    /// The variant whose games are looked up; None is standard chess.
    variant: Option<Variant>,
    lookup_task: Option<Task<()>>,
    follow_task: Option<Task<()>>,
    previews: crate::board::Previews,
    area: Measured,
    _subscriptions: Vec<Subscription>,
}

impl PositionExplorer {
    pub fn new(layout: Layout, window: &mut Window, cx: &mut Context<Self>) -> PositionExplorer {
        let prefs = Self::prefs(cx);
        let player = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Lichess username")
                .default_value(prefs.player.clone())
        });
        let since = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("YYYY-MM")
                .default_value(prefs.since.clone())
        });
        let db_player = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Player")
                .default_value(prefs.db_player.clone())
        });
        let databases = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let subscriptions = vec![
            // Connecting an account answers a lookup that needed one.
            cx.observe(&crate::accounts::handle(cx), |this, accounts, cx| {
                let signed_out = this
                    .failure
                    .as_ref()
                    .is_some_and(|failure| failure.kind == Some(ErrorKind::Login));
                if signed_out && !accounts.read(cx).connected().is_empty() {
                    this.lookup(None, false, None, cx);
                }
            }),
            // Help lent again answers a lookup refused while Lichess was asked for a game in
            // progress: just connected, the lookup went before the check had answered.
            Refused::watch(cx, |this| &mut this.refused, Self::look_again),
            cx.subscribe_in(
                &player,
                window,
                |this, state, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let name = state.read(cx).value().to_string();
                        this.update_prefs(cx, |prefs| prefs.player = name);
                    }
                    InputEvent::PressEnter { .. } => this.lookup(None, true, Some(window), cx),
                    _ => {}
                },
            ),
            cx.subscribe(&since, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let since = state.read(cx).value().trim().to_string();
                    this.update_prefs(cx, |prefs| prefs.since = since);
                    this.filters_changed(cx);
                }
            }),
            cx.subscribe_in(
                &db_player,
                window,
                |this, state, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let name = state.read(cx).value().to_string();
                        this.update_prefs(cx, |prefs| prefs.db_player = name);
                        this.filters_changed(cx);
                    }
                    InputEvent::PressEnter { .. } => this.lookup(None, true, Some(window), cx),
                    _ => {}
                },
            ),
            cx.subscribe(
                &databases,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(id)) = event {
                        let id = id.to_string();
                        this.update_prefs(cx, |prefs| prefs.database = id);
                        this.result = None;
                        this.lookup(None, false, None, cx);
                    }
                },
            ),
            // An import adds a database, and another window may delete one.
            cx.observe_in(
                &cx.global::<GameDbHandle>().0.clone(),
                window,
                |this, _, window, cx| this.sync_databases(window, cx),
            ),
        ];
        // A database this layout cannot show falls back to Lichess's.
        if !layout.offers(prefs.kind) {
            Self::save_prefs(
                cx,
                Prefs {
                    kind: PositionLookupKind::Opening,
                    ..prefs
                },
            );
        }
        let mut explorer = PositionExplorer {
            layout,
            fen: String::new(),
            moves: None,
            result: None,
            failure: None,
            refused: Refused::default(),
            busy: false,
            epoch: Epoch::default(),
            show_filters: false,
            accounts: Vec::new(),
            player,
            since,
            db_player,
            databases,
            listed: Vec::new(),
            result_database: None,
            variant: None,
            lookup_task: None,
            follow_task: None,
            previews: Default::default(),
            area: Measured::default(),
            _subscriptions: subscriptions,
        };
        explorer.load_accounts(window, cx);
        explorer.sync_databases(window, cx);
        explorer
    }

    /// The game databases offered: the chosen one while it is there, else the first. One that
    /// arrives while the Database tab waits for it is looked up at once.
    fn sync_databases(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let listed: Vec<(String, String)> = cx
            .global::<GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .iter()
            .map(|database| (database.id.clone(), database.name.clone()))
            .collect();
        if listed == self.listed && !self.listed.is_empty() {
            return;
        }
        let waiting = self.needs_database(cx);
        let chosen = Self::prefs(cx).database;
        let chosen = if listed.iter().any(|(id, _)| *id == chosen) {
            chosen
        } else {
            listed.first().map(|(id, _)| id.clone()).unwrap_or_default()
        };
        let items = listed
            .iter()
            .map(|(id, name)| Choice::new(id.clone(), name.clone()))
            .collect();
        self.listed = listed;
        self.databases
            .update(cx, |select, cx| select.set_items(items, window, cx));
        let value = SharedString::from(chosen.clone());
        self.databases.update(cx, |select, cx| {
            select.set_selected_value(&value, window, cx)
        });
        if Self::prefs(cx).database != chosen {
            self.update_prefs(cx, |prefs| prefs.database = chosen);
        }
        if waiting && !self.needs_database(cx) {
            self.lookup(None, false, None, cx);
        }
        cx.notify();
    }

    /// The Database tab with no database to look up in: it offers an import instead.
    fn needs_database(&self, cx: &App) -> bool {
        let prefs = self.shown_prefs(cx);
        prefs.kind == PositionLookupKind::Database
            && !cx
                .global::<GameDbHandle>()
                .0
                .read(cx)
                .databases()
                .iter()
                .any(|database| database.id == prefs.database)
    }

    /// The name of the game database `id`.
    fn database_name(id: &str, cx: &App) -> Option<String> {
        cx.global::<GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .iter()
            .find(|database| database.id == id)
            .map(|database| database.name.clone())
    }

    fn prefs(cx: &App) -> Prefs {
        crate::prefs::get(cx, PREFS, Prefs::default())
    }

    /// Whether only the databases that hold variants can answer: Chess960 is standard chess
    /// from another start, so the masters' games and the tablebase still do.
    fn variants_only(&self) -> bool {
        self.variant
            .is_some_and(|variant| variant != Variant::Chess960)
    }

    /// The settings as this position uses them: a variant is looked up in Lichess's database
    /// when the chosen one holds standard chess only, and the variant goes with the filters.
    fn shown_prefs(&self, cx: &App) -> Prefs {
        let mut prefs = Self::prefs(cx);
        if self.variants_only() && !holds_variants(prefs.kind) {
            prefs.kind = PositionLookupKind::Opening;
        }
        prefs
    }

    fn options(&self, prefs: &Prefs) -> LookupOptions {
        let mut options = prefs.options();
        if let Some(variant) = self.variant
            && holds_variants(prefs.kind)
        {
            options.variant = Some(variant);
        }
        options
    }

    fn save_prefs(cx: &mut App, prefs: Prefs) {
        crate::prefs::set(cx, PREFS, prefs);
    }

    fn update_prefs(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Prefs)) {
        let mut prefs = Self::prefs(cx);
        change(&mut prefs);
        Self::save_prefs(cx, prefs);
        cx.notify();
    }

    /// The connected accounts, offered as players; the first is the player when none is set.
    fn load_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let data = cx.global::<AppCore>().request(calls::LoadData {});
        cx.spawn_in(window, async move |this, cx| {
            let Some(data) = data.await.warn_on_err() else {
                return;
            };
            let accounts: Vec<String> = data
                .accounts
                .into_iter()
                .filter(|account| account.connected)
                .map(|account| account.username)
                .collect();
            let _ = this.update_in(cx, |this, window, cx| {
                if Self::prefs(cx).player.is_empty()
                    && let Some(first) = accounts.first()
                {
                    this.set_player(first.clone(), window, cx);
                }
                this.accounts = accounts;
                cx.notify();
            });
        })
        .detach();
    }

    fn set_player(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.player
            .update(cx, |input, cx| input.set_value(name.clone(), window, cx));
        self.update_prefs(cx, |prefs| prefs.player = name);
    }

    fn following(&self, cx: &App) -> bool {
        self.layout.always_follows() || Self::prefs(cx).follow
    }

    fn needs_player(&self, cx: &App) -> bool {
        let prefs = self.shown_prefs(cx);
        prefs.kind == PositionLookupKind::Player && !crate::game::is_username(prefs.player.trim())
    }

    /// Show a position (`opened`: the explorer just came into view). Following moves looks up
    /// another position once the board settles, and the one shown when the explorer opens.
    pub fn show(
        &mut self,
        variant: Variant,
        fen: &str,
        moves: Option<Vec<String>>,
        opened: bool,
        cx: &mut Context<Self>,
    ) {
        let variant = Some(variant).filter(|variant| *variant != Variant::Standard);
        if self.fen == fen && self.moves == moves && self.variant == variant {
            if opened
                && self.following(cx)
                && self.result.is_none()
                && self.failure.is_none()
                && !self.busy
            {
                self.lookup(None, false, None, cx);
            }
            return;
        }
        self.fen = fen.to_string();
        self.moves = moves;
        self.variant = variant;
        self.epoch.invalidate();
        self.result = None;
        self.failure = None;
        self.refused.asking();
        self.busy = false;
        self.lookup_task = None;
        if self.following(cx) {
            self.follow_lookup(cx);
        }
        cx.notify();
    }

    fn follow_lookup(&mut self, cx: &mut Context<Self>) {
        self.follow_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FOLLOW_DELAY).await;
            let _ = this.update(cx, |this, cx| this.lookup(None, false, None, cx));
        }));
    }

    /// The grid follows its position: another filter looks it up again. So does a game
    /// database's answer on the list, which comes from this computer.
    fn filters_changed(&mut self, cx: &mut Context<Self>) {
        let local = self.shown_prefs(cx).kind == PositionLookupKind::Database
            && (self.result.is_some() || self.following(cx));
        if self.layout.always_follows() || local {
            self.follow_lookup(cx);
        }
    }

    /// The chosen database looked up again, as a refused lookup is once help is lent.
    fn look_again(&mut self, cx: &mut Context<Self>) {
        self.lookup(None, false, None, cx);
    }

    /// Look the position up in `kind` (the chosen database when None); `asked` by the user, a
    /// missing player name takes the focus.
    fn lookup(
        &mut self,
        kind: Option<PositionLookupKind>,
        asked: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if let Some(kind) = kind
            && kind != self.shown_prefs(cx).kind
        {
            self.update_prefs(cx, |prefs| prefs.kind = kind);
            self.result = None;
            self.failure = None;
            self.refused.asking();
        }
        if self.needs_player(cx) || self.needs_database(cx) {
            // A lookup still on its way for another database must not land here.
            self.epoch.invalidate();
            self.lookup_task = None;
            self.busy = false;
            self.result = None;
            self.failure = None;
            self.refused.asking();
            if asked
                && self.needs_player(cx)
                && let Some(window) = window
            {
                self.player.update(cx, |input, cx| input.focus(window, cx));
            }
            cx.notify();
            return;
        }
        let request = self.epoch.start();
        let prefs = self.shown_prefs(cx);
        let database = (prefs.kind == PositionLookupKind::Database).then(|| prefs.database.clone());
        // Replaced by a newer lookup (`lookup_task`), it is dropped, which stops it in the core:
        // a game database's lookup may replay a million games.
        let call = cx.global::<AppCore>().lookup(calls::PositionLookup {
            kind: prefs.kind,
            fen: Fen(self.fen.clone()),
            options: Some(self.options(&prefs)),
        });
        self.busy = true;
        self.failure = None;
        self.refused.asking();
        cx.notify();
        self.lookup_task = Some(cx.spawn(async move |this, cx| {
            let outcome = call.await;
            let _ = this.update(cx, |this, cx| {
                if !this.epoch.is_current(request) {
                    return;
                }
                this.busy = false;
                this.refused
                    .answered(&outcome, |this| &mut this.refused, Self::look_again, cx);
                match outcome {
                    Ok(data) => {
                        this.result = Some(data);
                        this.result_database = database;
                    }
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "position-explorer",
                            &format!("Lookup failed: {}", cause.message),
                        );
                        this.failure = Some(if cause.message.is_empty() {
                            CoreError {
                                message: "Lookup failed.".into(),
                                ..cause
                            }
                        } else {
                            cause
                        });
                    }
                }
                cx.notify();
            });
        }));
    }

    /// The kind of the result shown.
    fn result_kind(&self) -> Option<PositionLookupKind> {
        self.result.as_ref().map(|result| result.saved.kind)
    }

    /// A listed game's PGN: from the masters' games, the game database it was found in, or
    /// Lichess.
    fn game_pgn(&self, id: &str, cx: &App) -> LocalBoxFuture<'static, Result<String, CoreError>> {
        let core = cx.global::<AppCore>();
        match self.result_kind() {
            Some(PositionLookupKind::Masters) => core
                .request(calls::MastersGame {
                    id: GameId(id.to_string()),
                })
                .boxed_local(),
            Some(PositionLookupKind::Database) => {
                let database = self.result_database.clone().unwrap_or_default();
                match id.parse::<u64>() {
                    Ok(game) => core
                        .request(calls::GameDbGame {
                            database: DatabaseId(database),
                            game: GameDbGameId(game),
                        })
                        .boxed_local(),
                    Err(_) => {
                        futures::future::ready(Err(CoreError::new("That game is not listed.")))
                            .boxed_local()
                    }
                }
            }
            _ => core
                .request(calls::ExportGame {
                    id: GameId(id.to_string()),
                })
                .boxed_local(),
        }
    }

    /// Open a listed game on the analysis board, asking first when that would replace unsaved
    /// analysis.
    fn open_game(&mut self, game: ExplorerGame, window: &mut Window, cx: &mut Context<Self>) {
        // Only a Lichess game is known by its id there.
        let lichess = !matches!(
            self.result_kind(),
            Some(PositionLookupKind::Masters | PositionLookupKind::Database)
        );
        let id = game.id.clone();
        let call = self.game_pgn(&id, cx);
        cx.spawn_in(window, async move |_, cx| {
            let outcome = call.await;
            let _ = cx.update(|window, cx| {
                let pgn = match outcome {
                    Ok(pgn) => pgn,
                    Err(cause) => {
                        window.push_notification(
                            Notification::error(kchess_app::loading::failure("explorer", &cause))
                                .title("Could not open the game"),
                            cx,
                        );
                        return;
                    }
                };
                let origin = GameOrigin {
                    game_id: lichess.then_some(id),
                    white: Some(game.white.clone()),
                    black: Some(game.black.clone()),
                    ..GameOrigin::default()
                };
                crate::pages::analysis::replace(window, cx, move |analysis, window, cx| {
                    let opening = Opening {
                        origin: Some(origin.clone()),
                        ..Opening::default()
                    };
                    let opened = analysis.open_pgn(&pgn, opening, cx);
                    if !opened {
                        window.push_notification(
                            Notification::error("That game could not be read")
                                .title("Could not open the game"),
                            cx,
                        );
                    }
                    opened
                });
            });
        })
        .detach();
    }

    /// Guess a listed game's moves on Practice, as the side that won (White after a draw).
    fn guess_game(&mut self, game: ExplorerGame, window: &mut Window, cx: &mut Context<Self>) {
        let call = self.game_pgn(&game.id, cx);
        let side = if game.winner == Some(Color::Black) {
            Color::Black
        } else {
            Color::White
        };
        cx.spawn_in(window, async move |_, cx| {
            let outcome = call.await;
            let _ = cx.update(|window, cx| match outcome {
                Ok(pgn) => crate::pages::practice::guess_moves(&pgn, Some(side), window, cx),
                Err(cause) => window.push_notification(
                    Notification::error(kchess_app::loading::failure("explorer", &cause))
                        .title("Could not open the game"),
                    cx,
                ),
            });
        })
        .detach();
    }

    fn preview(&mut self, fen: &str, uci: &str, cx: &mut Context<Self>) -> Entity<ChessBoard> {
        let variant = self.variant.unwrap_or_default();
        self.previews.board(
            format!("{fen} {uci}"),
            || BoardState::preview(fen, variant, Color::White, Some(uci)),
            cx,
        )
    }

    fn heading(&self, result: &PositionLookup, prefs: &Prefs, cx: &App) -> String {
        let saved = &result.saved;
        if let Some(opening) = &saved.opening
            && self.layout == Layout::List
        {
            return opening.clone();
        }
        match saved.kind {
            PositionLookupKind::Tablebase => format!(
                "Tablebase · {} for the side to move",
                category(saved.category.as_deref().unwrap_or_default(), false)
            ),
            PositionLookupKind::Masters => "Master games (over the board)".into(),
            PositionLookupKind::Player => {
                format!(
                    "Games of {} as {}",
                    prefs.player,
                    kchess_domain::color::name(prefs.color)
                )
            }
            PositionLookupKind::Opening => "Lichess games".into(),
            PositionLookupKind::Database => {
                let name = self
                    .result_database
                    .as_deref()
                    .and_then(|id| Self::database_name(id, cx))
                    .unwrap_or_else(|| "Game database".into());
                match prefs.db_player.trim() {
                    "" => name,
                    player => format!("{name} · {player}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
