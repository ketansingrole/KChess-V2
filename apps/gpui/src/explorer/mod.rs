//! The opening explorer and tablebases: the Lichess, Masters and Player
//! databases and the tablebase for a position, as the core looks them up (`positionLookup`), as a
//! list on the analysis board or as a grid of boards beside an opening's line. Its settings are
//! a view preference (`kchess:explorer`).

use crate::loading::Epoch;
use futures::FutureExt as _;
use kchess_app::log::ResultExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::{Fen, GameId};
use kchess_contracts::types::{
    ExplorerGame, ExplorerSpeed, LookupMove, LookupOptions, PositionLookup, PositionLookupKind,
};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::Level;
use serde::{Deserialize, Serialize};
use shakmaty::Color;

use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::STANDARD_FEN;
use crate::game::played::GameOrigin;
use crate::pages::analysis::Opening;
use crate::play_layout::Measured;
use crate::theme::{Palette, radii};
use crate::widgets::Pressable as _;
use crate::widgets::{self, KeyboardFocus as _, Pill};
use kchess_domain::misc::constants::EXPLORER_RATINGS;
use kchess_domain::perf::Speed;
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
const KINDS: [PositionLookupKind; 4] = [
    PositionLookupKind::Opening,
    PositionLookupKind::Masters,
    PositionLookupKind::Player,
    PositionLookupKind::Tablebase,
];

fn kind_label(kind: PositionLookupKind) -> &'static str {
    match kind {
        PositionLookupKind::Opening => "Lichess",
        PositionLookupKind::Masters => "Masters",
        PositionLookupKind::Player => "Player",
        PositionLookupKind::Tablebase => "Tablebase",
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

/// A speed's name on its filter button: correspondence is shortened to fit the row.
fn speed_label(speed: ExplorerSpeed) -> &'static str {
    match Speed::from_key(speed.as_str()) {
        Some(Speed::Correspondence) => "Corr",
        Some(speed) => speed.label(),
        None => speed.as_str(),
    }
}

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
    error: String,
    busy: bool,
    epoch: Epoch,
    show_filters: bool,
    accounts: Vec<String>,
    player: Entity<InputState>,
    since: Entity<InputState>,
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
        let subscriptions = vec![
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
            error: String::new(),
            busy: false,
            epoch: Epoch::default(),
            show_filters: false,
            accounts: Vec::new(),
            player,
            since,
            variant: None,
            lookup_task: None,
            follow_task: None,
            previews: Default::default(),
            area: Measured::default(),
            _subscriptions: subscriptions,
        };
        explorer.load_accounts(window, cx);
        explorer
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
                && self.error.is_empty()
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
        self.error.clear();
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

    /// The grid follows its position: another filter looks it up again.
    fn filters_changed(&mut self, cx: &mut Context<Self>) {
        if self.layout.always_follows() {
            self.follow_lookup(cx);
        }
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
            self.error.clear();
        }
        if self.needs_player(cx) {
            // A lookup still on its way for another database must not land here.
            self.epoch.invalidate();
            self.lookup_task = None;
            self.busy = false;
            self.result = None;
            self.error.clear();
            if asked && let Some(window) = window {
                self.player.update(cx, |input, cx| input.focus(window, cx));
            }
            cx.notify();
            return;
        }
        let request = self.epoch.start();
        let prefs = self.shown_prefs(cx);
        let call = cx.global::<AppCore>().request(calls::PositionLookup {
            kind: prefs.kind,
            fen: Fen(self.fen.clone()),
            options: Some(self.options(&prefs)),
        });
        self.busy = true;
        self.error.clear();
        cx.notify();
        self.lookup_task = Some(cx.spawn(async move |this, cx| {
            let outcome = call.await;
            let _ = this.update(cx, |this, cx| {
                if !this.epoch.is_current(request) {
                    return;
                }
                this.busy = false;
                match outcome {
                    Ok(data) => this.result = Some(data),
                    Err(cause) => {
                        crate::log::write(
                            Level::Warn,
                            "position-explorer",
                            &format!("Lookup failed: {}", cause.message),
                        );
                        this.error = if cause.message.is_empty() {
                            "Lookup failed. Retry.".into()
                        } else {
                            cause.message
                        };
                    }
                }
                cx.notify();
            });
        }));
    }

    /// Open a listed game on the analysis board, asking first when that would replace unsaved
    /// analysis.
    fn open_game(&mut self, game: ExplorerGame, window: &mut Window, cx: &mut Context<Self>) {
        let masters = self
            .result
            .as_ref()
            .is_some_and(|result| result.saved.kind == PositionLookupKind::Masters);
        let id = game.id.clone();
        let core = cx.global::<AppCore>();
        let call = if masters {
            core.request(calls::MastersGame {
                id: GameId(id.clone()),
            })
            .boxed_local()
        } else {
            core.request(calls::ExportGame {
                id: GameId(id.clone()),
            })
            .boxed_local()
        };
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
                    game_id: (!masters).then_some(id),
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

    /// Guess a master game's moves on Practice, as the side that won (White after a draw).
    fn guess_game(&mut self, game: ExplorerGame, window: &mut Window, cx: &mut Context<Self>) {
        let id = game.id.clone();
        let call = cx
            .global::<AppCore>()
            .request(calls::MastersGame { id: GameId(id) });
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

    fn heading(&self, result: &PositionLookup, prefs: &Prefs) -> String {
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
        }
    }
}

#[cfg(test)]
mod tests;
