//! The opening explorer and tablebases: the Lichess, Masters and Player
//! databases and the tablebase for a position, as the core looks them up (`positionLookup`), as a
//! list on the analysis board or as a grid of boards beside an opening's line. Its settings are
//! a view preference (`kchess:explorer`).

use futures::FutureExt as _;
use kchess_contracts::calls;
use kchess_contracts::check::{Fen, GameId};
use kchess_contracts::types::{
    ExplorerGame, ExplorerSpeed, LookupMove, LookupOptions, PositionLookup, PositionLookupKind,
};
use std::collections::HashMap;
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

/// The explorer's settings, as the profile keeps them. A value no version of the app writes
/// reads as the default, so an old or damaged preference still loads.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    #[serde(deserialize_with = "lenient")]
    kind: PositionLookupKind,
    #[serde(deserialize_with = "known_speeds")]
    speeds: Vec<ExplorerSpeed>,
    ratings: Vec<f64>,
    player: String,
    #[serde(
        serialize_with = "kchess_domain::color::by_name::serialize",
        deserialize_with = "lenient_color"
    )]
    color: Color,
    since: String,
    follow: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            kind: PositionLookupKind::Opening,
            speeds: vec![
                ExplorerSpeed::Blitz,
                ExplorerSpeed::Rapid,
                ExplorerSpeed::Classical,
            ],
            ratings: Vec::new(),
            player: String::new(),
            color: Color::White,
            since: String::new(),
            follow: false,
        }
    }
}

/// The stored database, or Lichess's games for a name this app does not know.
fn lenient<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<PositionLookupKind, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or(PositionLookupKind::Opening))
}

/// The stored speeds this app knows.
fn known_speeds<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ExplorerSpeed>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|speed| serde_json::from_value(speed.clone()).ok())
        .collect())
}

fn lenient_color<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_str()
        .and_then(kchess_domain::color::from_name)
        .unwrap_or(Color::White))
}

impl Prefs {
    /// The filters the chosen database takes; an unset one is left out.
    fn options(&self) -> LookupOptions {
        let since = (!self.since.is_empty()).then(|| self.since.clone());
        let kind = self.kind;
        let options = match kind {
            PositionLookupKind::Opening => LookupOptions {
                speeds: Some(self.speeds.clone()),
                ratings: Some(self.ratings.iter().map(|rating| *rating as u32).collect()),
                ..LookupOptions::default()
            },
            PositionLookupKind::Player => LookupOptions {
                player: Some(self.player.trim().to_string()),
                color: Some(self.color),
                speeds: Some(self.speeds.clone()),
                ..LookupOptions::default()
            },
            _ => LookupOptions::default(),
        };
        let since = match kind {
            PositionLookupKind::Masters => {
                since.map(|since| since.chars().take(4).collect::<String>())
            }
            PositionLookupKind::Tablebase => None,
            _ => since,
        };
        LookupOptions { since, ..options }
    }
}

fn toggled<T: PartialEq + Clone>(list: &[T], value: &T) -> Vec<T> {
    if list.contains(value) {
        list.iter()
            .filter(|entry| *entry != value)
            .cloned()
            .collect()
    } else {
        list.iter().cloned().chain([value.clone()]).collect()
    }
}

/// A tablebase result in words, from the side to move's view or (`opponent`) the other's.
fn category(value: &str, opponent: bool) -> String {
    let value = if opponent {
        value
            .split('-')
            .map(|part| match part {
                "win" => "loss",
                "loss" => "win",
                "blessed" => "cursed",
                "cursed" => "blessed",
                other => other,
            })
            .collect::<Vec<_>>()
            .join("-")
    } else {
        value.to_string()
    };
    value.replace('-', " ")
}

fn count(n: Option<i64>) -> u64 {
    n.map_or(0, |n| n.max(0) as u64)
}

fn games(entry: &LookupMove) -> u64 {
    count(entry.white) + count(entry.draws) + count(entry.black)
}

/// How the games from a position ended, White, draws and Black; a share is
/// labelled once it is wider than `min_label` percent.
pub fn result_bar(entry: &LookupMove, min_label: u64, palette: &Palette) -> impl IntoElement {
    let counts = [count(entry.white), count(entry.draws), count(entry.black)];
    let total: u64 = counts.iter().sum();
    let looks = [
        (rgb(0xf5f5f5), rgb(0x222222)),
        (rgb(0x9e9e9e), rgb(0xffffff)),
        (rgb(0x333333), rgb(0xeeeeee)),
    ];
    let first = counts.iter().position(|&part| part > 0);
    let last = counts.iter().rposition(|&part| part > 0);
    let title = SharedString::from(format!(
        "White {} · Draw {} · Black {}",
        counts[0], counts[1], counts[2]
    ));
    h_flex()
        .id(SharedString::from(format!(
            "result-bar-{}-{}-{}",
            counts[0], counts[1], counts[2]
        )))
        .flex_1()
        .min_w(px(40.))
        .h(px(16.))
        .rounded(radii::XS)
        .overflow_hidden()
        .border_1()
        .border_color(palette.border)
        .text_size(px(11.))
        .line_height(px(14.))
        .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
        .children(counts.into_iter().zip(looks).enumerate().map(
            move |(index, (part, (bg, fg)))| {
                let pct = if total == 0 {
                    0
                } else {
                    (100. * part as f64 / total as f64).round() as u64
                };
                let ends = (Some(index) == first, Some(index) == last);
                widgets::bar_segment(div(), ends, radii::XS - px(1.))
                    .h_full()
                    .w(relative(pct as f32 / 100.))
                    .overflow_hidden()
                    .bg(bg)
                    .text_color(fg)
                    .flex()
                    .justify_center()
                    .when(pct > min_label, |part| part.child(format!("{pct}%")))
            },
        ))
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
    epoch: u64,
    show_filters: bool,
    accounts: Vec<String>,
    player: Entity<InputState>,
    since: Entity<InputState>,
    /// The variant whose games are looked up; None is standard chess.
    variant: Option<Variant>,
    lookup_task: Option<Task<()>>,
    follow_task: Option<Task<()>>,
    previews: HashMap<String, Entity<ChessBoard>>,
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
            epoch: 0,
            show_filters: false,
            accounts: Vec::new(),
            player,
            since,
            variant: None,
            lookup_task: None,
            follow_task: None,
            previews: HashMap::new(),
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
            let Ok(data) = data.await else { return };
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
        self.epoch += 1;
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
            self.epoch += 1;
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
        self.epoch += 1;
        let request = self.epoch;
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
                if request != this.epoch {
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
                            Notification::error(cause.message).title("Could not open the game"),
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
                    Notification::error(cause.message).title("Could not open the game"),
                    cx,
                ),
            });
        })
        .detach();
    }

    fn preview(&mut self, fen: &str, uci: &str, cx: &mut Context<Self>) -> Entity<ChessBoard> {
        let key = format!("{fen} {uci}");
        self.previews
            .entry(key)
            .or_insert_with(|| {
                let variant = self.variant.unwrap_or_default();
                let position = kchess_domain::rules::setup_start(variant, fen).unwrap_or_default();
                let last = uci
                    .get(0..2)
                    .and_then(|from| from.parse().ok())
                    .zip(uci.get(2..4).and_then(|to| to.parse().ok()));
                let mut state = BoardState::of(&position, Color::White, None, last);
                state.preview = true;
                cx.new(|_| ChessBoard::new(state))
            })
            .clone()
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

    fn render_filters(&self, prefs: &Prefs, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let toggle = |id: SharedString, label: String, on: bool| {
            Button::new(id)
                .xsmall()
                .label(label)
                .when(on, |button| button.primary())
                .when(!on, |button| button.ghost())
        };
        v_flex()
            .gap(px(8.))
            .text_size(px(12.))
            .when(prefs.kind != PositionLookupKind::Masters, |filters| {
                filters.child(h_flex().flex_wrap().gap(px(4.)).children(
                    ExplorerSpeed::ALL.iter().map(|&speed| {
                        toggle(
                            SharedString::from(format!("speed-{}", speed.as_str())),
                            speed_label(speed).into(),
                            prefs.speeds.contains(&speed),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_prefs(cx, |prefs| {
                                prefs.speeds = toggled(&prefs.speeds, &speed)
                            });
                            this.filters_changed(cx);
                        }))
                    }),
                ))
            })
            .when(prefs.kind == PositionLookupKind::Opening, |filters| {
                filters.child(
                    h_flex()
                        .flex_wrap()
                        .gap(px(4.))
                        .children(EXPLORER_RATINGS.map(|rating| {
                            toggle(
                                SharedString::from(format!("rating-{rating}")),
                                format!("{rating}"),
                                prefs.ratings.contains(&rating),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.update_prefs(cx, |prefs| {
                                        prefs.ratings = toggled(&prefs.ratings, &rating)
                                    });
                                    this.filters_changed(cx);
                                },
                            ))
                        }))
                        .child(
                            div()
                                .text_color(palette.text_muted)
                                .child("(none selected: all ratings)"),
                        ),
                )
            })
            .child(
                h_flex().gap(px(8.)).child("Since").child(
                    div()
                        .w(px(96.))
                        .child(Input::new(&self.since).xsmall().aria_label("Since")),
                ),
            )
    }

    fn render_player(&self, prefs: &Prefs, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let colors = [Color::White, Color::Black];
        let chosen = colors
            .iter()
            .position(|color| *color == prefs.color)
            .unwrap_or_default();
        h_flex()
            .flex_wrap()
            .gap(px(8.))
            .child(
                div()
                    .w(px(144.))
                    .child(Input::new(&self.player).xsmall().aria_label("Player")),
            )
            .children(
                self.accounts
                    .iter()
                    .filter(|name| **name != prefs.player)
                    .enumerate()
                    .map(|(index, name)| {
                        let name = name.clone();
                        Button::new(("explorer-account", index))
                            .xsmall()
                            .outline()
                            .label(name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_player(name.clone(), window, cx)
                            }))
                    }),
            )
            .child(widgets::pills(
                "explorer-color",
                vec![
                    Pill {
                        icon: None,
                        label: "as White".into(),
                        badge: None,
                        disabled: false,
                    },
                    Pill {
                        icon: None,
                        label: "as Black".into(),
                        badge: None,
                        disabled: false,
                    },
                ],
                chosen,
                {
                    let explorer = cx.entity().downgrade();
                    move |index, _, cx| {
                        let _ = explorer.update(cx, |this, cx| {
                            this.update_prefs(cx, |prefs| prefs.color = colors[index]);
                            this.filters_changed(cx);
                        });
                    }
                },
                true,
                palette,
            ))
    }

    fn render_moves(
        &mut self,
        result: &PositionLookup,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let moves = &result.saved.moves;
        let tablebase = result.saved.kind == PositionLookupKind::Tablebase;
        if self.layout == Layout::Grid {
            let fen = result.saved.fen.clone();
            let total = result
                .saved
                .total
                .map(|n| n.max(0) as u64)
                .filter(|total| *total > 0)
                .unwrap_or_else(|| moves.iter().map(games).sum());
            let width = self.area.size().width;
            let (columns, tile) = widgets::columns(width.max(TILE_MIN), TILE_MIN, TILE_GAP);
            let mut shown = std::collections::HashSet::new();
            let tiles: Vec<AnyElement> = moves
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let (uci, san) = (entry.uci.clone(), entry.san.clone());
                    let next =
                        kchess_domain::replay::replay_positions(&fen, std::slice::from_ref(&uci))
                            .get(1)
                            .map_or_else(|| fen.clone(), |position| position.fen.clone());
                    let share = if total == 0 {
                        0
                    } else {
                        (100. * games(entry) as f64 / total as f64).round() as u64
                    };
                    let opening = self.moves.as_ref().and_then(|moves| {
                        let line: Vec<String> =
                            moves.iter().cloned().chain([uci.clone()]).collect();
                        kchess_domain::training::openings::opening_at(
                            "standard",
                            STANDARD_FEN,
                            &line,
                            line.len() as f64,
                        )
                        .map(|opening| opening.name)
                    });
                    let board = self.preview(&next, &uci, cx);
                    shown.insert(format!("{next} {uci}"));
                    let label = match &opening {
                        Some(name) => format!("{san}, {name}"),
                        None => san.clone(),
                    };
                    v_flex()
                        .id(("explorer-tile", index))
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .w(tile)
                        .gap(px(4.))
                        .p(px(6.))
                        .border_1()
                        .border_color(palette.border)
                        .rounded(radii::MD)
                        .text_size(px(13.))
                        .cursor_pointer()
                        .pressable(palette)
                        .aria_label(label)
                        .on_click(
                            cx.listener(move |_, _, _, cx| {
                                cx.emit(ExplorerEvent::Play(uci.clone()))
                            }),
                        )
                        .child(div().w_full().h(tile - px(14.)).child(board))
                        .child(
                            h_flex()
                                .justify_between()
                                .items_baseline()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::MEDIUM).child(
                                    crate::notation::shown(&san, &crate::notation::figurine(cx)),
                                ))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(palette.text_muted)
                                        .child(format!("{share}%")),
                                ),
                        )
                        .children(opening.map(|name| {
                            div()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .truncate()
                                .child(name)
                        }))
                        .child(h_flex().child(result_bar(entry, 20, palette)))
                        .into_any_element()
                })
                .collect();
            // Boards of positions no longer shown are let go: each position stepped through
            // added a board per move, for as long as the page was open.
            self.previews.retain(|key, _| shown.contains(key));
            let mut rows = Vec::new();
            let mut tiles = tiles.into_iter().peekable();
            while tiles.peek().is_some() {
                rows.push(
                    h_flex()
                        .gap(TILE_GAP)
                        .items_start()
                        .children(tiles.by_ref().take(columns)),
                );
            }
            return v_flex()
                .w_full()
                .gap(TILE_GAP)
                .children(rows)
                .into_any_element();
        }
        let figurine = crate::notation::figurine(cx);
        v_flex()
            .gap(px(4.))
            .children(moves.iter().enumerate().map(|(index, entry)| {
                let (uci, san) = (entry.uci.clone(), entry.san.clone());
                h_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .id(("explorer-move", index))
                            .test_support()
                            .role(Role::Button)
                            .keyboard_focus(palette.primary)
                            .aria_label(format!("Play {san}"))
                            .w(px(56.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.primary)
                            .cursor_pointer()
                            .pressable_link()
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ExplorerEvent::Play(uci.clone()))
                            }))
                            .child(crate::notation::shown(&san, &figurine)),
                    )
                    .when(!tablebase, |row| {
                        row.child(
                            div()
                                .w(px(64.))
                                .text_right()
                                .text_size(px(12.))
                                .text_color(palette.text_muted)
                                .child(widgets::format_count(games(entry) as f64)),
                        )
                        .child(result_bar(entry, 12, palette))
                    })
                    .when(tablebase, |row| {
                        row.child(div().text_size(px(12.)).child(format!(
                            "{} for the current player",
                            category(entry.category.as_deref().unwrap_or_default(), true)
                        )))
                    })
            }))
            .into_any_element()
    }

    fn render_games(
        &self,
        result: &PositionLookup,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Option<Div> {
        let list = result
            .saved
            .games
            .as_ref()
            .filter(|games| !games.is_empty())?;
        let masters = result.saved.kind == PositionLookupKind::Masters;
        let title = if masters { "Top games" } else { "Recent games" };
        Some(
            v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text_muted)
                        .child(title),
                )
                .children(list.iter().enumerate().map(|(index, game)| {
                    let rating = |rating: Option<f64>| {
                        rating.map(|rating| format!("{rating}")).unwrap_or_default()
                    };
                    let score = match game.winner {
                        Some(Color::White) => "1-0",
                        Some(Color::Black) => "0-1",
                        None => "½-½",
                    };
                    let when = game
                        .year
                        .map(|year| year.to_string())
                        .or_else(|| game.month.clone())
                        .unwrap_or_default();
                    let muted = palette.text_muted;
                    let chosen = game.clone();
                    let guessed = game.clone();
                    let row = h_flex()
                        .id(("explorer-game", index))
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label("Open game")
                        .justify_between()
                        .gap(px(8.))
                        .px(px(4.))
                        .py(px(2.))
                        .rounded(radii::SM)
                        .text_size(px(12.))
                        .cursor_pointer()
                        .pressable(palette)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_game(chosen.clone(), window, cx)
                        }))
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap(px(4.))
                                .overflow_hidden()
                                .child(game.white.clone())
                                .child(div().text_color(muted).child(rating(game.white_rating)))
                                .child("–")
                                .child(game.black.clone())
                                .child(div().text_color(muted).child(rating(game.black_rating))),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(muted)
                                .child(format!("{score} {when}")),
                        );
                    // A master game can be guessed through, move by move, on Practice.
                    h_flex()
                        .gap(px(2.))
                        .child(row.flex_1().min_w_0())
                        .when(masters, |line| {
                            line.child(
                                Button::new(("explorer-guess", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(icon("target"))
                                    .tooltip("Guess the moves")
                                    .accessibility_label("Guess the moves")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.guess_game(guessed.clone(), window, cx)
                                    })),
                            )
                        })
                })),
        )
    }
}

impl Render for PositionExplorer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let prefs = self.shown_prefs(cx);
        let kinds: Vec<PositionLookupKind> = KINDS
            .into_iter()
            .filter(|kind| self.layout.offers(*kind))
            .filter(|kind| !self.variants_only() || holds_variants(*kind))
            .collect();
        let chosen = kinds
            .iter()
            .position(|kind| *kind == prefs.kind)
            .unwrap_or_default();
        let needs_player = self.needs_player(cx);
        let choose_kind = {
            let explorer = cx.entity().downgrade();
            let kinds = kinds.clone();
            move |index: usize, window: &mut Window, cx: &mut App| {
                let kind = kinds[index];
                let _ = explorer.update(cx, |this, cx| {
                    this.lookup(Some(kind), true, Some(window), cx)
                });
            }
        };
        let head = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .child(widgets::pills(
                "explorer-kind",
                kinds
                    .iter()
                    .map(|kind| Pill {
                        icon: None,
                        label: kind_label(*kind).into(),
                        badge: None,
                        disabled: false,
                    })
                    .collect(),
                chosen,
                choose_kind,
                true,
                &palette,
            ))
            .when(
                !matches!(
                    prefs.kind,
                    PositionLookupKind::Tablebase | PositionLookupKind::Masters
                ),
                |head| {
                    head.child(
                        Button::new("explorer-filters")
                            .xsmall()
                            .size(px(24.))
                            .ghost()
                            .icon(icon("sliders-horizontal"))
                            .selected(self.show_filters)
                            .tooltip("Explorer filters")
                            .accessibility_label("Explorer filters")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_filters = !this.show_filters;
                                cx.notify();
                            })),
                    )
                },
            )
            .child(
                div()
                    .id("explorer-about")
                    .text_color(palette.text_muted)
                    .tooltip(|window, cx| {
                        Tooltip::new(
                            "Lookups send this position to Lichess. The Lichess, Masters and \
                             Player databases use a connected account; saved results stay \
                             available offline. Statistics describe games, not move quality.",
                        )
                        .build(window, cx)
                    })
                    .child(icon("info").size(px(14.))),
            );
        let status = if self.busy {
            Some(if prefs.kind == PositionLookupKind::Player {
                "Indexing the player’s games — this can take a while the first time…"
            } else {
                "Loading position lookup…"
            })
        } else {
            None
        };
        let result = self.result.clone();
        let body = result.map(|result| {
            let total = result.saved.total.filter(|total| *total > 0);
            let fetched = chrono::DateTime::from_timestamp_millis(result.saved.fetched_at)
                .map(|date| date.with_timezone(&chrono::Local).format("%b %-d, %Y").to_string())
                .unwrap_or_default();
            let empty = result.saved.moves.is_empty();
            v_flex()
                .gap(px(8.))
                .text_size(px(13.))
                .child(
                    h_flex()
                        .gap(px(4.))
                        .child(self.heading(&result, &prefs))
                        .children(total.map(|total| {
                            div()
                                .text_color(palette.text_muted)
                                .child(format!("· {} games", widgets::format_count(total as f64)))
                        })),
                )
                .when(result.saved.kind == PositionLookupKind::Tablebase, |body| {
                    let dtz = result
                        .saved
                        .dtz
                        .flatten()
                        .map_or_else(|| "unknown".to_string(), |dtz| dtz.to_string());
                    body.child(
                        div()
                            .id("tablebase-dtz")
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .underline()
                            .tooltip(|window, cx| {
                                Tooltip::new(
                                    "Plies to a pawn move or capture; not distance to mate. \
                                     Cursed wins and blessed losses account for the fifty-move rule.",
                                )
                                .build(window, cx)
                            })
                            .child(format!("DTZ {dtz}")),
                    )
                })
                .when(result.cached, |body| {
                    body.child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!(
                                "{} · {fetched}",
                                result.message.as_deref().unwrap_or("Saved lookup")
                            )),
                    )
                })
                .when(empty, |body| body.child("No moves found for this position"))
                .child(self.render_moves(&result, cx, &palette))
                .children(self.render_games(&result, cx, &palette))
        });
        v_flex()
            .relative()
            .w_full()
            .gap(px(8.))
            .child(self.area.probe(cx))
            .child(head)
            .when(prefs.kind == PositionLookupKind::Player, |explorer| {
                explorer.child(self.render_player(&prefs, cx, &palette))
            })
            .when(
                self.show_filters && prefs.kind != PositionLookupKind::Tablebase,
                |explorer| explorer.child(self.render_filters(&prefs, cx, &palette)),
            )
            .when(self.layout == Layout::List, |explorer| {
                explorer.child(
                    h_flex()
                        .flex_wrap()
                        .gap(px(12.))
                        .text_size(px(13.))
                        .child(
                            Button::new("explorer-lookup")
                                .xsmall()
                                .min_h(px(24.))
                                .outline()
                                .icon(icon("search"))
                                .label("Look up this position")
                                .loading(self.busy)
                                .disabled(needs_player)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.lookup(None, true, Some(window), cx)
                                })),
                        )
                        .child(widgets::labelled_switch(
                            "explorer-follow",
                            "Follow moves",
                            prefs.follow,
                            false,
                            {
                                let explorer = cx.entity().downgrade();
                                move |on, _, cx| {
                                    let on = *on;
                                    let _ = explorer.update(cx, |this, cx| {
                                        this.update_prefs(cx, |prefs| prefs.follow = on);
                                        if on {
                                            this.lookup(None, false, None, cx);
                                        }
                                    });
                                }
                            },
                        )),
                )
            })
            .when(needs_player, |explorer| {
                explorer.child(widgets::hint(
                    "Enter a Lichess username to see the moves they played here",
                    &palette,
                ))
            })
            .children(status.map(|status| widgets::loading(status, &palette)))
            .when(!self.error.is_empty(), |explorer| {
                explorer.child(
                    div()
                        .text_size(px(13.))
                        .text_color(palette.error)
                        .child(self.error.clone()),
                )
            })
            .children(body)
    }
}

#[cfg(test)]
mod tests {
    use super::{Layout, PositionExplorer, Prefs, category, speed_label, toggled};
    use crate::e2e::Harness;
    use gpui_kit::AppContext as _;
    use kchess_contracts::types::{ExplorerSpeed, PositionLookupKind};
    use shakmaty::Color;

    /// Choosing the player database with no name yet left a lookup for the other database
    /// current: its answer then showed under the player tab, and the spinner stayed on until it.
    #[gpui_kit::test]
    fn a_lookup_for_another_database_does_not_land_on_the_player_tab(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let mut app = Harness::start(cx);
        let (busy, stale) = app.with_window(|window, cx| {
            let explorer = cx.new(|cx| PositionExplorer::new(Layout::List, window, cx));
            explorer.update(cx, |explorer, cx| {
                explorer.lookup(Some(PositionLookupKind::Masters), false, None, cx);
                let asked = explorer.epoch;
                explorer.lookup(Some(PositionLookupKind::Player), false, None, cx);
                (explorer.busy, explorer.epoch == asked)
            })
        });
        assert!(!busy && !stale);
    }
    use crate::game::is_username;
    use serde_json::json;

    #[test]
    fn options_leave_out_what_is_not_set() {
        let mut prefs = Prefs::default();
        assert_eq!(
            json!(prefs.options()),
            json!({ "speeds": ["blitz", "rapid", "classical"], "ratings": [] })
        );
        prefs.kind = PositionLookupKind::Masters;
        prefs.since = "2019-05".into();
        assert_eq!(json!(prefs.options()), json!({ "since": "2019" }));
        prefs.kind = PositionLookupKind::Tablebase;
        assert_eq!(json!(prefs.options()), json!({}));
    }

    /// The preference an earlier version stored reads as it did when its values are all known,
    /// and saves back the same; a value this app does not know reads as the default.
    #[test]
    fn stored_preferences_still_load() {
        let stored = json!({
            "kind": "player",
            "speeds": ["bullet", "correspondence"],
            "ratings": [1600.0, 2000.0],
            "player": "DrNykterstein",
            "color": "black",
            "since": "2020-01",
            "follow": true,
        });
        let prefs: Prefs = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(prefs.kind, PositionLookupKind::Player);
        assert_eq!(
            prefs.speeds,
            [ExplorerSpeed::Bullet, ExplorerSpeed::Correspondence]
        );
        assert_eq!(prefs.color, Color::Black);
        assert_eq!(json!(prefs), stored);

        let odd: Prefs = serde_json::from_value(json!({
            "kind": "lichess",
            "speeds": ["blitz", "hyperBullet"],
            "color": "green",
        }))
        .unwrap();
        assert_eq!(odd.kind, PositionLookupKind::Opening);
        assert_eq!(odd.speeds, [ExplorerSpeed::Blitz]);
        assert_eq!(odd.color, Color::White);
        let empty: Prefs = serde_json::from_value(json!({ "speeds": "fast" })).unwrap();
        assert_eq!(empty.speeds, Vec::<ExplorerSpeed>::new());
        assert_eq!(speed_label(ExplorerSpeed::Correspondence), "Corr");
        assert_eq!(speed_label(ExplorerSpeed::UltraBullet), "UltraBullet");
    }

    #[test]
    fn a_tablebase_result_reads_from_either_side() {
        assert_eq!(category("cursed-win", false), "cursed win");
        assert_eq!(category("win", true), "loss");
        assert_eq!(category("cursed-win", true), "blessed loss");
        assert_eq!(category("blessed-loss", true), "cursed win");
        assert_eq!(category("draw", true), "draw");
    }

    #[test]
    fn filters_toggle_and_names_are_checked() {
        assert_eq!(toggled(&[1, 2], &2), vec![1]);
        assert_eq!(toggled(&[1], &2), vec![1, 2]);
        assert!(is_username("DrNykterstein"));
        assert!(!is_username("a"));
        assert!(!is_username("no spaces"));
    }
}
