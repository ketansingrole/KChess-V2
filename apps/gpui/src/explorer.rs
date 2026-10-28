//! The opening explorer and tablebases: the Lichess, Masters and Player
//! databases and the tablebase for a position, as the core looks them up (`positionLookup`), as a
//! list on the analysis board or as a grid of boards beside an opening's line. Its settings are
//! the view preference both apps keep (`kchess:explorer`).

use std::collections::HashMap;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shakmaty::Color;

use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::{STANDARD_FEN, rules};
use crate::play_layout::Measured;
use crate::theme::Palette;
use crate::widgets::{self, KeyboardFocus as _, Pill};

const PREFS: &str = "kchess:explorer";
/// Following moves waits for the board to settle.
const FOLLOW_DELAY: Duration = Duration::from_millis(350);
/// The grid's tiles (`.explorer-grid`).
const TILE_MIN: Pixels = px(120.);
const TILE_GAP: Pixels = px(10.);

const KINDS: [(&str, &str); 4] = [
    ("opening", "Lichess"),
    ("masters", "Masters"),
    ("player", "Player"),
    ("tablebase", "Tablebase"),
];
/// `EXPLORER_SPEEDS` and their labels.
const SPEEDS: [(&str, &str); 6] = [
    ("ultraBullet", "UltraBullet"),
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("correspondence", "Corr."),
];
/// `EXPLORER_RATINGS`.
const RATINGS: [f64; 9] = [400., 1000., 1200., 1400., 1600., 1800., 2000., 2200., 2500.];

/// The explorer's settings, as both apps keep them.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    kind: String,
    speeds: Vec<String>,
    ratings: Vec<f64>,
    player: String,
    color: String,
    since: String,
    follow: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            kind: "opening".into(),
            speeds: vec!["blitz".into(), "rapid".into(), "classical".into()],
            ratings: Vec::new(),
            player: String::new(),
            color: "white".into(),
            since: String::new(),
            follow: false,
        }
    }
}

impl Prefs {
    /// `options()`: the filters the chosen database takes; an unset one is left out.
    fn options(&self) -> Value {
        let since = (!self.since.is_empty()).then(|| self.since.clone());
        let mut options = match self.kind.as_str() {
            "opening" => json!({ "speeds": self.speeds, "ratings": self.ratings }),
            "player" => json!({
                "player": self.player.trim(),
                "color": self.color,
                "speeds": self.speeds,
            }),
            _ => json!({}),
        };
        let since = match self.kind.as_str() {
            "masters" => since.map(|since| since.chars().take(4).collect::<String>()),
            "tablebase" => None,
            _ => since,
        };
        if let Some(since) = since {
            options["since"] = json!(since);
        }
        options
    }
}

/// Lichess usernames: what the player database accepts.
fn is_username(name: &str) -> bool {
    (2..=30).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
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

fn count(value: &Value) -> u64 {
    value.as_f64().map_or(0, |n| n.max(0.) as u64)
}

fn games(entry: &Value) -> u64 {
    count(&entry["white"]) + count(&entry["draws"]) + count(&entry["black"])
}

/// How the games from a position ended, White, draws and Black; a share is
/// labelled once it is wider than `min_label` percent.
pub fn result_bar(entry: &Value, min_label: u64, palette: &Palette) -> impl IntoElement {
    let counts = [
        count(&entry["white"]),
        count(&entry["draws"]),
        count(&entry["black"]),
    ];
    let total: u64 = counts.iter().sum();
    let looks = [
        (rgb(0xf5f5f5), rgb(0x222222)),
        (rgb(0x9e9e9e), rgb(0xffffff)),
        (rgb(0x333333), rgb(0xeeeeee)),
    ];
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
        .rounded(px(4.))
        .overflow_hidden()
        .border_1()
        .border_color(palette.border)
        .text_size(px(10.))
        .line_height(px(14.))
        .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
        .children(counts.into_iter().zip(looks).map(move |(part, (bg, fg))| {
            let pct = if total == 0 {
                0
            } else {
                (100. * part as f64 / total as f64).round() as u64
            };
            div()
                .h_full()
                .w(relative(pct as f32 / 100.))
                .overflow_hidden()
                .bg(bg)
                .text_color(fg)
                .flex()
                .justify_center()
                .when(pct > min_label, |part| part.child(format!("{pct}%")))
        }))
}

pub enum ExplorerEvent {
    /// A move chosen from the results.
    Play(String),
}

impl EventEmitter<ExplorerEvent> for PositionExplorer {}

pub struct PositionExplorer {
    /// Continuations as a grid of boards, looked up whenever the position changes.
    grid: bool,
    fen: String,
    /// The moves from the starting position to `fen`: each continuation is named after its opening.
    moves: Option<Vec<String>>,
    result: Option<Value>,
    error: String,
    busy: bool,
    epoch: u64,
    show_filters: bool,
    accounts: Vec<String>,
    player: Entity<InputState>,
    since: Entity<InputState>,
    lookup_task: Option<Task<()>>,
    follow_task: Option<Task<()>>,
    previews: HashMap<String, Entity<ChessBoard>>,
    area: Measured,
    _subscriptions: Vec<Subscription>,
}

impl PositionExplorer {
    pub fn new(grid: bool, window: &mut Window, cx: &mut Context<Self>) -> PositionExplorer {
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
        // A database the grid cannot show falls back to Lichess's.
        if grid && prefs.kind == "tablebase" {
            Self::save_prefs(
                cx,
                Prefs {
                    kind: "opening".into(),
                    ..prefs
                },
            );
        }
        let mut explorer = PositionExplorer {
            grid,
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
        let data = cx.global::<AppCore>().call("loadData", Vec::new());
        cx.spawn_in(window, async move |this, cx| {
            let Ok(data) = data.await else { return };
            let accounts: Vec<String> = data["accounts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|account| account["connected"] == json!(true))
                .filter_map(|account| account["username"].as_str().map(str::to_string))
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
        self.grid || Self::prefs(cx).follow
    }

    fn needs_player(cx: &App) -> bool {
        let prefs = Self::prefs(cx);
        prefs.kind == "player" && !is_username(prefs.player.trim())
    }

    /// Show a position (`opened`: the explorer just came into view). Following moves looks up
    /// another position once the board settles, and the one shown when the explorer opens.
    pub fn show(
        &mut self,
        fen: &str,
        moves: Option<Vec<String>>,
        opened: bool,
        cx: &mut Context<Self>,
    ) {
        if self.fen == fen && self.moves == moves {
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
        if self.grid {
            self.follow_lookup(cx);
        }
    }

    /// Look the position up in `kind` (the chosen database when None); `asked` by the user, a
    /// missing player name takes the focus.
    fn lookup(
        &mut self,
        kind: Option<&str>,
        asked: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if let Some(kind) = kind
            && kind != Self::prefs(cx).kind
        {
            self.update_prefs(cx, |prefs| prefs.kind = kind.to_string());
            self.result = None;
            self.error.clear();
        }
        if Self::needs_player(cx) {
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
        let prefs = Self::prefs(cx);
        let call = cx.global::<AppCore>().call(
            "positionLookup",
            vec![json!(prefs.kind), json!(self.fen), prefs.options()],
        );
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
    fn open_game(&mut self, game: Value, window: &mut Window, cx: &mut Context<Self>) {
        let masters = self
            .result
            .as_ref()
            .is_some_and(|result| result["kind"] == json!("masters"));
        let id = game["id"].as_str().unwrap_or_default().to_string();
        let call = cx.global::<AppCore>().call(
            if masters { "mastersGame" } else { "exportGame" },
            vec![json!(id)],
        );
        cx.spawn_in(window, async move |_, cx| {
            let outcome = call.await;
            let _ = cx.update(|window, cx| {
                let pgn = match outcome {
                    Ok(pgn) => pgn.as_str().unwrap_or_default().to_string(),
                    Err(cause) => {
                        window.push_notification(
                            Notification::error(cause.message).title("Could not open the game"),
                            cx,
                        );
                        return;
                    }
                };
                let origin = json!({
                    "white": game["white"],
                    "black": game["black"],
                    "gameId": (!masters).then_some(id),
                });
                crate::shell::replace_analysis(window, cx, move |analysis, window, cx| {
                    let opened = analysis.open_game(&pgn, 0, None, Some(origin.clone()), false, cx);
                    if !opened {
                        window.push_notification(
                            Notification::error("That game could not be read.")
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

    fn preview(&mut self, fen: &str, uci: &str, cx: &mut Context<Self>) -> Entity<ChessBoard> {
        let key = format!("{fen} {uci}");
        self.previews
            .entry(key)
            .or_insert_with(|| {
                let position =
                    kchess_domain::rules::setup_start("standard", fen).unwrap_or_default();
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

    fn heading(&self, result: &Value, prefs: &Prefs) -> String {
        if let Some(opening) = result["opening"].as_str()
            && !self.grid
        {
            return opening.to_string();
        }
        match result["kind"].as_str().unwrap_or_default() {
            "tablebase" => format!(
                "Tablebase · {} for the side to move",
                category(result["category"].as_str().unwrap_or_default(), false)
            ),
            "masters" => "Master games (over the board)".into(),
            "player" => format!("Games of {} as {}", prefs.player, prefs.color),
            _ => "Lichess games".into(),
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
            .when(prefs.kind != "masters", |filters| {
                filters.child(h_flex().flex_wrap().gap(px(4.)).children(SPEEDS.map(
                    |(speed, label)| {
                        toggle(
                            SharedString::from(format!("speed-{speed}")),
                            label.into(),
                            prefs.speeds.iter().any(|chosen| chosen == speed),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_prefs(cx, |prefs| {
                                prefs.speeds = toggled(&prefs.speeds, &speed.to_string())
                            });
                            this.filters_changed(cx);
                        }))
                    },
                )))
            })
            .when(prefs.kind == "opening", |filters| {
                filters.child(
                    h_flex()
                        .flex_wrap()
                        .gap(px(4.))
                        .children(RATINGS.map(|rating| {
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
        let colors = ["white", "black"];
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
                            this.update_prefs(cx, |prefs| prefs.color = colors[index].into());
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
        result: &Value,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let moves = result["moves"].as_array().cloned().unwrap_or_default();
        let tablebase = result["kind"] == json!("tablebase");
        if self.grid {
            let fen = result["fen"].as_str().unwrap_or(&self.fen).to_string();
            let total = result["total"]
                .as_f64()
                .map(|n| n as u64)
                .filter(|total| *total > 0)
                .unwrap_or_else(|| moves.iter().map(games).sum());
            let width = self.area.size().width;
            let (columns, tile) = widgets::columns(width.max(TILE_MIN), TILE_MIN, TILE_GAP);
            let tiles: Vec<AnyElement> = moves
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let uci = entry["uci"].as_str().unwrap_or_default().to_string();
                    let san = entry["san"].as_str().unwrap_or_default().to_string();
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
                        rules(
                            "openingAt",
                            &[
                                json!({ "variant": "standard", "fen": STANDARD_FEN }),
                                json!(line),
                                json!(line.len()),
                            ],
                        )
                        .ok()?["name"]
                            .as_str()
                            .map(str::to_string)
                    });
                    let board = self.preview(&next, &uci, cx);
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
                        .rounded(px(8.))
                        .text_size(px(13.))
                        .cursor_pointer()
                        .hover(|tile| tile.bg(palette.accented))
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
                                .child(div().font_weight(FontWeight::BOLD).child(san))
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
        v_flex()
            .gap(px(4.))
            .children(moves.iter().enumerate().map(|(index, entry)| {
                let uci = entry["uci"].as_str().unwrap_or_default().to_string();
                let san = entry["san"].as_str().unwrap_or_default().to_string();
                h_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .id(("explorer-move", index))
                            .role(Role::Button)
                            .keyboard_focus(palette.primary)
                            .aria_label(format!("Play {san}"))
                            .w(px(56.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette.primary)
                            .cursor_pointer()
                            .hover(|chip| chip.underline())
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ExplorerEvent::Play(uci.clone()))
                            }))
                            .child(san),
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
                            category(entry["category"].as_str().unwrap_or_default(), true)
                        )))
                    })
            }))
            .into_any_element()
    }

    fn render_games(
        &self,
        result: &Value,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> Option<Div> {
        let list = result["games"]
            .as_array()
            .filter(|games| !games.is_empty())?;
        let title = if result["kind"] == json!("masters") {
            "Top games"
        } else {
            "Recent games"
        };
        Some(
            v_flex()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.text_muted)
                        .child(title),
                )
                .children(list.iter().enumerate().map(|(index, game)| {
                    let rating = |key: &str| {
                        game[key]
                            .as_f64()
                            .map(|rating| format!("{rating}"))
                            .unwrap_or_default()
                    };
                    let score = match game["winner"].as_str() {
                        Some("white") => "1-0",
                        Some("black") => "0-1",
                        _ => "½-½",
                    };
                    let when = game["year"]
                        .as_i64()
                        .map(|year| year.to_string())
                        .or_else(|| game["month"].as_str().map(str::to_string))
                        .unwrap_or_default();
                    let muted = palette.text_muted;
                    let chosen = game.clone();
                    h_flex()
                        .id(("explorer-game", index))
                        .role(Role::Button)
                        .keyboard_focus(palette.primary)
                        .aria_label("Open game")
                        .justify_between()
                        .gap(px(8.))
                        .px(px(4.))
                        .py(px(2.))
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(|row| row.bg(palette.accented))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_game(chosen.clone(), window, cx)
                        }))
                        .child(
                            h_flex()
                                .min_w_0()
                                .gap(px(4.))
                                .overflow_hidden()
                                .child(game["white"].as_str().unwrap_or("?").to_string())
                                .child(div().text_color(muted).child(rating("whiteRating")))
                                .child("–")
                                .child(game["black"].as_str().unwrap_or("?").to_string())
                                .child(div().text_color(muted).child(rating("blackRating"))),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(muted)
                                .child(format!("{score} {when}")),
                        )
                })),
        )
    }
}

impl Render for PositionExplorer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let prefs = Self::prefs(cx);
        let kinds: Vec<(&str, &str)> = KINDS
            .into_iter()
            .filter(|(kind, _)| !self.grid || *kind != "tablebase")
            .collect();
        let chosen = kinds
            .iter()
            .position(|(kind, _)| *kind == prefs.kind)
            .unwrap_or_default();
        let needs_player = Self::needs_player(cx);
        let choose_kind = {
            let explorer = cx.entity().downgrade();
            let kinds: Vec<&'static str> = kinds.iter().map(|(kind, _)| *kind).collect();
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
                    .map(|(_, label)| Pill {
                        icon: None,
                        label: (*label).into(),
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
                prefs.kind != "tablebase" && prefs.kind != "masters",
                |head| {
                    head.child(
                        Button::new("explorer-filters")
                            .xsmall()
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
            Some(if prefs.kind == "player" {
                "Indexing the player’s games — this can take a while the first time…"
            } else {
                "Loading position lookup…"
            })
        } else {
            None
        };
        let result = self.result.clone();
        let body = result.map(|result| {
            let total = result["total"].as_f64().filter(|total| *total > 0.);
            let fetched = result["fetchedAt"]
                .as_f64()
                .and_then(|millis| chrono::DateTime::from_timestamp_millis(millis as i64))
                .map(|date| date.with_timezone(&chrono::Local).format("%b %-d, %Y").to_string())
                .unwrap_or_default();
            let empty = result["moves"].as_array().is_none_or(Vec::is_empty);
            v_flex()
                .gap(px(6.))
                .text_size(px(13.))
                .child(
                    h_flex()
                        .gap(px(4.))
                        .child(self.heading(&result, &prefs))
                        .children(total.map(|total| {
                            div()
                                .text_color(palette.text_muted)
                                .child(format!("· {} games", widgets::format_count(total)))
                        })),
                )
                .when(result["kind"] == json!("tablebase"), |body| {
                    let dtz = result["dtz"]
                        .as_i64()
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
                .when(result["cached"] == json!(true), |body| {
                    body.child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(format!(
                                "{} · {fetched}",
                                result["message"].as_str().unwrap_or("Saved lookup")
                            )),
                    )
                })
                .when(empty, |body| body.child("No moves found for this position."))
                .child(self.render_moves(&result, cx, &palette))
                .children(self.render_games(&result, cx, &palette))
        });
        v_flex()
            .relative()
            .w_full()
            .gap(px(8.))
            .child(self.area.probe(cx))
            .child(head)
            .when(prefs.kind == "player", |explorer| {
                explorer.child(self.render_player(&prefs, cx, &palette))
            })
            .when(self.show_filters && prefs.kind != "tablebase", |explorer| {
                explorer.child(self.render_filters(&prefs, cx, &palette))
            })
            .when(!self.grid, |explorer| {
                explorer.child(
                    h_flex()
                        .gap(px(12.))
                        .child(
                            Button::new("explorer-lookup")
                                .xsmall()
                                .outline()
                                .icon(icon("search"))
                                .label("Look up this position")
                                .loading(self.busy)
                                .disabled(needs_player)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.lookup(None, true, Some(window), cx)
                                })),
                        )
                        .child(
                            Switch::new("explorer-follow")
                                .xsmall()
                                .checked(prefs.follow)
                                .label("Follow moves")
                                .on_click(cx.listener(|this, on: &bool, _, cx| {
                                    let on = *on;
                                    this.update_prefs(cx, |prefs| prefs.follow = on);
                                    if on {
                                        this.lookup(None, false, None, cx);
                                    }
                                })),
                        ),
                )
            })
            .when(needs_player, |explorer| {
                explorer.child(widgets::hint(
                    "Enter a Lichess username to see the moves they played here.",
                    &palette,
                ))
            })
            .children(status.map(|status| div().text_size(px(13.)).child(status)))
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
    use super::{Prefs, category, is_username, toggled};
    use serde_json::json;

    #[test]
    fn options_leave_out_what_is_not_set() {
        let mut prefs = Prefs::default();
        assert_eq!(
            prefs.options(),
            json!({ "speeds": ["blitz", "rapid", "classical"], "ratings": [] })
        );
        prefs.kind = "masters".into();
        prefs.since = "2019-05".into();
        assert_eq!(prefs.options(), json!({ "since": "2019" }));
        prefs.kind = "tablebase".into();
        assert_eq!(prefs.options(), json!({}));
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
