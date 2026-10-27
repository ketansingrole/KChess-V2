//! The watched game beside what can be watched (`TvViewer.vue`, `useTvPosition.ts`): the live
//! board with ticking clocks, the moves once Lichess's delayed export lines up, the head-to-head,
//! a channel's earlier games, and Analyse.

use std::collections::HashMap;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};
use shakmaty::Color;

use super::{WatchPage, label, live_badge};
use crate::assets::icon;
use crate::board::{BoardState, ChessBoard};
use crate::core::AppCore;
use crate::game::{rules, view};
use crate::play_layout::{LAYOUT_GAP, PANEL_MAX, PANEL_MIN};
use crate::settings::SettingsHandle;
use crate::theme::Palette;
use crate::widgets::{self, Side, Tone};

/// `ENDINGS`: how a game ended, as Lichess names its status.
fn ending(status: &str) -> &str {
    match status {
        "mate" => "Checkmate",
        "resign" => "Resignation",
        "outoftime" => "Time out",
        "timeout" => "Opponent left",
        "draw" => "Draw",
        "stalemate" => "Stalemate",
        "aborted" => "Aborted",
        "noStart" => "Did not start",
        "cheat" => "Cheat detected",
        "variantEnd" => "Variant win",
        other => other,
    }
}

/// `SPEEDS`: a speed's name.
fn speed_label(speed: &str) -> Option<&'static str> {
    Some(match speed {
        "ultraBullet" => "UltraBullet",
        "bullet" => "Bullet",
        "blitz" => "Blitz",
        "rapid" => "Rapid",
        "classical" => "Classical",
        "correspondence" => "Correspondence",
        _ => return None,
    })
}

fn speed_icon(speed: &str) -> Option<gpui_kit::component::Icon> {
    Some(match speed {
        "ultraBullet" => icon("rocket"),
        "bullet" => icon("zap"),
        "blitz" => icon("flame"),
        "rapid" => icon("rabbit"),
        "classical" => icon("turtle"),
        "correspondence" => icon("mail"),
        _ => return None,
    })
}

/// Lichess-style base time: 60 → "1", 30 → "½", 90 → "1.5".
fn base_minutes(seconds: f64) -> String {
    match seconds as i64 {
        15 => "¼".into(),
        30 => "½".into(),
        45 => "¾".into(),
        _ => {
            let minutes = (seconds / 60. * 10.).round() / 10.;
            if minutes.fract() == 0. {
                format!("{minutes:.0}")
            } else {
                minutes.to_string()
            }
        }
    }
}

/// "Black won · Checkmate", or "Live" while the game goes on (`frameResult`).
pub fn result_line(frame: &Value) -> String {
    if frame["finished"] != json!(true) {
        return "Live".into();
    }
    let how = frame["status"].as_str().map(ending).unwrap_or_default();
    match frame["winner"].as_str() {
        Some(winner) => format!(
            "{} won{}",
            if winner == "white" { "White" } else { "Black" },
            if how.is_empty() {
                String::new()
            } else {
                format!(" · {how}")
            }
        ),
        None if how.is_empty() => "Finished".into(),
        None => how.to_string(),
    }
}

/// "1+0 · Rated · Bullet", the line Lichess shows above a TV game (`frameInfo`).
pub fn info_line(frame: &Value, channels: &[Value]) -> String {
    let speed = frame["speed"].as_str().unwrap_or_default();
    let variant = if frame["source"] == json!("tv") {
        frame["channel"]
            .as_str()
            .filter(|channel| {
                !matches!(*channel, "best" | "bot" | "computer") && speed_label(channel).is_none()
            })
            .and_then(|channel| {
                channels
                    .iter()
                    .find(|c| c["key"] == json!(channel))
                    .and_then(|c| c["label"].as_str().map(str::to_string))
            })
    } else {
        frame["variant"]
            .as_str()
            .filter(|variant| *variant != "standard")
            .map(str::to_string)
    };
    let mut parts = Vec::new();
    if let (Some(initial), Some(increment)) = (
        frame["clock"]["initial"].as_f64(),
        frame["clock"]["increment"].as_f64(),
    ) {
        parts.push(format!("{}+{increment}", base_minutes(initial)));
    }
    if let Some(rated) = frame["rated"].as_bool() {
        parts.push(if rated { "Rated" } else { "Casual" }.into());
    }
    if !speed.is_empty() {
        parts.push(speed_label(speed).unwrap_or(speed).to_string());
    }
    parts.extend(variant);
    parts.join(" · ")
}

/// A name Lichess has a profile for.
fn real_player(name: &str) -> bool {
    name != "?" && name != "Anonymous" && !name.starts_with("Stockfish level")
}

/// "2½": a score as Lichess writes it.
fn points(value: f64) -> String {
    let whole = value.floor();
    if value.fract() != 0. {
        if whole == 0. {
            "½".into()
        } else {
            format!("{whole}½")
        }
    } else {
        format!("{whole}")
    }
}

/// One side's points in a finished past game: 1, 0 or ½.
fn past_score(entry: &Value, side: &str) -> &'static str {
    match (entry["winner"].as_str(), entry["status"].as_str()) {
        (Some(winner), _) if winner == side => "1",
        (Some(_), _) => "0",
        (None, None | Some("created" | "started" | "aborted" | "noStart")) => "",
        (None, Some(_)) => "½",
    }
}

fn past_result(entry: &Value) -> &'static str {
    match (entry["winner"].as_str(), entry["status"].as_str()) {
        (Some("white"), _) => "1–0",
        (Some(_), _) => "0–1",
        (None, None | Some("created" | "started")) => "Result pending",
        (None, Some("aborted" | "noStart")) => "Aborted",
        (None, Some(_)) => "½–½",
    }
}

fn color_of(name: &str) -> Color {
    if name == "black" {
        Color::Black
    } else {
        Color::White
    }
}

pub struct Live {
    pub frame: Option<Value>,
    frame_at: Instant,
    /// The ply looked at, or `None` to follow the game.
    ply: Option<usize>,
    flipped: bool,
    board: Entity<ChessBoard>,
    /// The head-to-head of the pairing shown (`white|black`).
    head_to_head: Option<(String, Value)>,
    pairing: String,
    past: HashMap<String, Entity<ChessBoard>>,
}

impl Live {
    pub fn new(cx: &mut App) -> Live {
        Live {
            frame: None,
            frame_at: Instant::now(),
            ply: None,
            flipped: false,
            board: cx.new(|_| {
                ChessBoard::new(BoardState::shown(
                    "8/8/8/8/8/8/8/8 w",
                    "standard",
                    Color::White,
                    None,
                ))
            }),
            head_to_head: None,
            pairing: String::new(),
            past: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.frame = None;
        self.ply = None;
    }

    pub fn set_frame(&mut self, frame: Value, cx: &mut Context<WatchPage>) {
        let game = frame["gameId"].clone();
        if self.frame.as_ref().map(|old| &old["gameId"]) != Some(&game) {
            self.ply = None;
            self.flipped = false;
        }
        let pairing = format!(
            "{}|{}",
            frame["white"]["name"].as_str().unwrap_or_default(),
            frame["black"]["name"].as_str().unwrap_or_default()
        );
        self.frame = Some(frame);
        self.frame_at = Instant::now();
        if pairing != self.pairing {
            self.pairing = pairing.clone();
            self.head_to_head = None;
            self.load_head_to_head(pairing, cx);
        }
        self.redraw(cx);
    }

    fn load_head_to_head(&mut self, pairing: String, cx: &mut Context<WatchPage>) {
        let (white, black) = pairing.split_once('|').unwrap_or_default();
        if !real_player(white) || !real_player(black) {
            return;
        }
        let call = cx
            .global::<AppCore>()
            .call("crosstable", vec![json!(white), json!(black)]);
        cx.spawn(async move |this, cx| {
            // Optional context: the game is watchable without it.
            if let Ok(table) = call.await
                && table["nbGames"].as_f64().is_some_and(|games| games > 0.)
            {
                let _ = this.update(cx, |page, cx| {
                    if page.live.pairing == pairing {
                        page.live.head_to_head = Some((pairing, table));
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// The feed stopped: the running clock keeps what it had left.
    pub fn freeze_clock(&mut self) {
        let elapsed = self.frame_at.elapsed().as_secs_f64();
        if let Some(frame) = &mut self.frame
            && frame["finished"] != json!(true)
            && frame["lastMove"].is_string()
        {
            let key = if turn_of(frame) == Color::Black {
                "blackClock"
            } else {
                "whiteClock"
            };
            if let Some(left) = frame[key].as_f64() {
                frame[key] = json!((left - elapsed).max(0.));
            }
        }
        self.frame_at = Instant::now();
    }

    /// The game's start and moves, once Lichess's delayed export lines up with the feed.
    fn setup(&self) -> Option<Value> {
        let frame = self.frame.as_ref()?;
        let fen = frame["startFen"].as_str()?;
        frame["moves"].is_array().then(
            || json!({ "variant": frame["variant"].as_str().unwrap_or("standard"), "fen": fen }),
        )
    }

    fn moves(&self) -> Vec<String> {
        if self.setup().is_none() {
            return Vec::new();
        }
        self.frame
            .as_ref()
            .and_then(|frame| serde_json::from_value(frame["moves"].clone()).ok())
            .unwrap_or_default()
    }

    fn shown_ply(&self) -> usize {
        let count = self.moves().len();
        self.ply.unwrap_or(count).min(count)
    }

    fn browsing(&self) -> bool {
        self.setup().is_some() && self.ply.is_some() && self.shown_ply() < self.moves().len()
    }

    fn orientation(&self) -> Color {
        let side = self.frame.as_ref().map_or(Color::White, |frame| {
            color_of(frame["orientation"].as_str().unwrap_or("white"))
        });
        if self.flipped { !side } else { side }
    }

    fn select(&mut self, ply: usize) {
        let count = self.moves().len();
        self.ply = (ply < count).then_some(ply);
    }

    pub fn redraw(&mut self, cx: &mut App) {
        let Some(frame) = &self.frame else {
            return;
        };
        let orientation = self.orientation();
        let state = match self.setup() {
            Some(setup) if self.browsing() => {
                view::board_state(&setup, &self.moves(), self.shown_ply(), orientation, None)
            }
            _ => BoardState::shown(
                frame["fen"].as_str().unwrap_or_default(),
                frame["variant"].as_str().unwrap_or("standard"),
                orientation,
                frame["lastMove"].as_str().filter(|uci| !uci.contains('@')),
            ),
        };
        self.board
            .update(cx, |board, cx| board.set_state(state, cx));
    }

    /// ←/→ and ↑/↓ step through the moves once they are known; F turns the board.
    pub fn key(&mut self, key: &str, cx: &mut App) -> bool {
        if self.frame.is_none() {
            return false;
        }
        if key == "f" {
            self.flipped = !self.flipped;
            self.redraw(cx);
            return true;
        }
        if self.setup().is_none() {
            return false;
        }
        let Some(ply) = view::navigate(key, self.shown_ply(), self.moves().len()) else {
            return false;
        };
        self.select(ply);
        self.redraw(cx);
        true
    }

    /// One side's clock, the side to move's running while connected (`clockOf`).
    fn clock(&self, color: Color, connected: bool) -> Option<(String, bool)> {
        let frame = self.frame.as_ref()?;
        let base = frame[if color == Color::White {
            "whiteClock"
        } else {
            "blackClock"
        }]
        .as_f64()?;
        let running = connected
            && frame["finished"] != json!(true)
            && turn_of(frame) == color
            && frame["lastMove"].is_string();
        let elapsed = if running {
            self.frame_at.elapsed().as_secs_f64()
        } else {
            0.
        };
        let millis = (base - elapsed).max(0.) * 1000.;
        let text = rules("formatClock", &[json!(millis)])
            .ok()
            .and_then(|text| text.as_str().map(str::to_string))?;
        Some((text, running))
    }

    /// "All time 3–2 · 5 games" and the current match.
    fn score_lines(&self) -> Vec<(&'static str, String, f64)> {
        let (Some((_, table)), Some(frame)) = (&self.head_to_head, &self.frame) else {
            return Vec::new();
        };
        let white = frame["white"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        let black = frame["black"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        let score = |users: &Value| {
            format!(
                "{}–{}",
                points(users[&white].as_f64().unwrap_or_default()),
                points(users[&black].as_f64().unwrap_or_default())
            )
        };
        let mut lines = vec![(
            "All time",
            score(&table["users"]),
            table["nbGames"].as_f64().unwrap_or_default(),
        )];
        if let Some(games) = table["matchup"]["nbGames"]
            .as_f64()
            .filter(|games| *games > 0.)
        {
            lines.push(("Current match", score(&table["matchup"]["users"]), games));
        }
        lines
    }

    /// A channel's earlier games (`Previously on this channel`), each opened on the analysis
    /// board.
    pub fn past_games(&mut self, cx: &mut App, palette: &Palette) -> Option<AnyElement> {
        let previous = self.frame.as_ref()?["previous"].as_array()?.clone();
        if previous.is_empty() {
            return None;
        }
        let cells: Vec<AnyElement> = previous
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let id = entry["gameId"].as_str().unwrap_or_default().to_string();
                let orientation = color_of(entry["orientation"].as_str().unwrap_or("white"));
                let board = self
                    .past
                    .entry(id.clone())
                    .or_insert_with(|| {
                        let mut state = BoardState::shown(
                            entry["fen"].as_str().unwrap_or_default(),
                            "standard",
                            orientation,
                            entry["lastMove"].as_str().filter(|uci| !uci.contains('@')),
                        );
                        state.preview = true;
                        cx.new(|_| ChessBoard::new(state))
                    })
                    .clone();
                let (top, bottom) = if orientation == Color::White {
                    ("black", "white")
                } else {
                    ("white", "black")
                };
                let line = |side: &'static str, slot: usize| {
                    h_flex()
                        .justify_between()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .child(
                            crate::pages::players::player_link(
                                ("watch-past-player", index * 2 + slot),
                                entry[side]["name"].as_str().unwrap_or("?").to_string(),
                            )
                            .child(""),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .child(past_score(entry, side)),
                        )
                };
                let open = entry.clone();
                v_flex()
                    .gap(px(4.))
                    .p(px(8.))
                    .w(px(190.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(palette.border)
                    .bg(palette.elevated)
                    .child(line(top, 0))
                    .child(
                        div()
                            .id(("watch-past-board", index))
                            .cursor_pointer()
                            .on_click(move |_, window, cx| {
                                crate::pages::players::open_exported(
                                    open["gameId"].as_str().unwrap_or_default(),
                                    open["white"]["name"].clone(),
                                    open["black"]["name"].clone(),
                                    color_of(open["orientation"].as_str().unwrap_or("white")),
                                    window,
                                    cx,
                                )
                            })
                            .child(div().w_full().h(px(174.)).child(board)),
                    )
                    .child(line(bottom, 1))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(palette.text_muted)
                            .child(past_result(entry)),
                    )
                    .into_any_element()
            })
            .collect();
        Some(
            v_flex()
                .gap(px(12.))
                .child(widgets::heading("Previously on this channel", palette))
                .child(h_flex().flex_wrap().gap(px(14.)).children(cells))
                .into_any_element(),
        )
    }
}

/// The side to move in a frame's FEN.
fn turn_of(frame: &Value) -> Color {
    if frame["fen"].as_str().and_then(|fen| fen.split(' ').nth(1)) == Some("b") {
        Color::Black
    } else {
        Color::White
    }
}

impl WatchPage {
    /// The list, the board and its panel; or the list and an empty card.
    pub(super) fn live_layout(
        &mut self,
        list: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let Some(frame) = self.live.frame.clone() else {
            return h_flex()
                .items_start()
                .gap(LAYOUT_GAP)
                .child(list)
                .child(widgets::card(palette).flex_1().child(widgets::empty_state(
                    icon("tv"),
                    if self.target.is_some() {
                        "Connecting…"
                    } else {
                        "Pick something to watch"
                    },
                    None,
                    palette,
                )))
                .into_any_element();
        };
        let set = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .piece_set
            .clone();
        let connected = self.phase.as_deref() == Some("connected");
        let orientation = self.live.orientation();
        let fen = frame["fen"].as_str().unwrap_or_default().to_string();
        let crazyhouse = frame["variant"] == json!("crazyhouse");
        let turn = turn_of(&frame);
        let finished = frame["finished"] == json!(true);
        let line = |color: Color, index: usize| {
            let player = &frame[if color == Color::White {
                "white"
            } else {
                "black"
            }];
            let name = player["name"].as_str().unwrap_or("?").to_string();
            let shown = label(player);
            let identity = h_flex()
                .gap(px(10.))
                .child(widgets::avatar(
                    icon("user"),
                    if color == Color::White {
                        Side::White
                    } else {
                        Side::Black
                    },
                    turn == color && !finished,
                    palette,
                ))
                .child(if real_player(&name) {
                    let link = name.clone();
                    div()
                        .id(("watch-player", index))
                        .cursor_pointer()
                        .hover(|name| name.underline())
                        .on_click(move |_, window, cx| crate::shell::open_player(&link, window, cx))
                        .child(shown)
                        .into_any_element()
                } else {
                    div().child(shown).into_any_element()
                })
                .children(if crazyhouse {
                    None
                } else {
                    widgets::material(&fen, color == Color::White, &set, palette)
                });
            let clock = self.live.clock(color, connected).map(|(text, running)| {
                widgets::clock(text.into(), running, palette).into_any_element()
            });
            widgets::player_line(identity, None, clock)
        };
        let board_size = self
            .layout
            .board_size_in(px(240.) + LAYOUT_GAP, super::HEADER, window);
        let board_column = v_flex()
            .w(board_size)
            .flex_none()
            .gap(widgets::STACK_GAP)
            .child(line(!orientation, 0))
            .child(crate::board::cached(&self.live.board))
            .child(line(orientation, 1));

        let browsing = self.live.browsing();
        let info = info_line(&frame, &self.channels);
        let banner_icon = if finished {
            icon("flag")
        } else {
            frame["speed"]
                .as_str()
                .and_then(speed_icon)
                .unwrap_or_else(|| {
                    super::channel_icon(frame["channel"].as_str().unwrap_or_default())
                })
        };
        let (title, detail) = if finished {
            (result_line(&frame), (!info.is_empty()).then_some(info))
        } else if info.is_empty() {
            ("Live game".into(), None)
        } else {
            (info, None)
        };
        let action: Option<AnyElement> = if browsing {
            Some(
                Button::new("watch-back-live")
                    .xsmall()
                    .ghost()
                    .icon(icon("radio"))
                    .label("Back to live")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.ply = None;
                        this.live.redraw(cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            )
        } else if !finished {
            Some(live_badge(palette).into_any_element())
        } else {
            None
        };
        let banner = h_flex()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().child(widgets::status_banner(
                Tone::Plain,
                banner_icon,
                title.into(),
                detail.map(Into::into),
                None,
                palette,
            )))
            .children(action);
        let scores = self.live.score_lines();
        let head_to_head = (!scores.is_empty()).then(|| {
            let white = frame["white"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let black = frame["black"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            h_flex()
                .flex_wrap()
                .gap(px(16.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(10.))
                .border_1()
                .border_color(palette.border)
                .bg(palette.elevated)
                .text_size(px(13.))
                .child(
                    h_flex()
                        .gap(px(4.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(crate::pages::players::player_link("watch-h2h-white", white))
                        .child("–")
                        .child(crate::pages::players::player_link("watch-h2h-black", black)),
                )
                .children(scores.into_iter().map(|(label, score, games)| {
                    h_flex()
                        .gap(px(6.))
                        .child(div().text_color(palette.text_muted).child(label))
                        .child(div().font_weight(FontWeight::BOLD).child(score))
                        .child(div().text_color(palette.text_muted).child(format!(
                            "{games} game{}",
                            if games == 1. { "" } else { "s" }
                        )))
                }))
        });
        let setup = self.live.setup();
        let moves = self.live.moves();
        let sans = setup
            .as_ref()
            .map(|setup| view::sans(setup, &moves))
            .unwrap_or_default();
        let empty = if frame["source"] != json!("tv") {
            "Lichess does not send this game’s moves."
        } else if crazyhouse {
            "No move list for Crazyhouse."
        } else {
            "Loading moves…"
        };
        let page = cx.entity().downgrade();
        let select = move |ply: usize, _: &mut Window, cx: &mut App| {
            let _ = page.update(cx, |page, cx| {
                page.live.select(ply);
                page.live.redraw(cx);
                cx.notify();
            });
        };
        let flip_page = cx.entity().downgrade();
        let flip: widgets::ClickHandler = Box::new(move |_, _, cx| {
            let _ = flip_page.update(cx, |page, cx| {
                page.live.flipped = !page.live.flipped;
                page.live.redraw(cx);
                cx.notify();
            });
        });
        let panel = widgets::side_panel(palette)
            .h(self.layout.panel_height(board_size))
            .flex_1()
            .min_w(PANEL_MIN)
            .max_w(PANEL_MAX)
            .child(banner)
            .children(head_to_head)
            .child(div().flex_1().min_h(px(120.)).child(widgets::moves_list(
                "watch-moves",
                &sans,
                self.live.shown_ply(),
                setup.as_ref().is_some_and(view::black_first),
                empty.into(),
                select.clone(),
                palette,
            )))
            .child(widgets::move_controls(
                self.live.shown_ply(),
                moves.len(),
                select,
                ("Flip board".into(), flip),
            ))
            .when(frame["source"] == json!("game"), |panel| {
                panel.child(widgets::hint(
                    "Moves shown with Lichess’s anti-cheat delay.",
                    palette,
                ))
            })
            .child(
                widgets::panel_actions(
                    false,
                    [
                        Button::new("watch-analyse")
                            .outline()
                            .w_full()
                            .icon(icon("microscope"))
                            .label("Analyse")
                            .on_click(cx.listener(|this, _, window, cx| this.analyse(window, cx)))
                            .into_any_element()
                            .into(),
                        Button::new("watch-stop")
                            .outline()
                            .w_full()
                            .icon(icon("square"))
                            .label("Stop")
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                            .into_any_element()
                            .into(),
                    ],
                )
                .pt(px(14.))
                .border_t_1()
                .border_color(palette.border),
            );
        if self.layout.is_stacked() {
            let column = |child: AnyElement| div().w(board_size).max_w_full().child(child);
            return v_flex()
                .items_center()
                .gap(LAYOUT_GAP)
                .child(board_column)
                .child(column(panel.into_any_element()))
                .child(column(list.into_any_element()))
                .into_any_element();
        }
        h_flex()
            .items_start()
            .justify_center()
            .gap(LAYOUT_GAP)
            .child(list.h(self.layout.panel_height_px(board_size)))
            .child(board_column)
            .child(panel)
            .into_any_element()
    }

    /// `analyseTv`: the game so far on the analysis board, at the move looked at.
    fn analyse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(frame) = self.live.frame.clone() else {
            return;
        };
        let orientation = self.live.orientation();
        let origin = json!({
            "white": frame["white"]["name"],
            "black": frame["black"]["name"],
            "gameId": frame["gameId"],
        });
        if let Some(setup) = self.live.setup() {
            let pgn = rules(
                "setupPgn",
                &[
                    setup,
                    json!(self.live.moves()),
                    json!({ "White": frame["white"]["name"], "Black": frame["black"]["name"] }),
                ],
            );
            if let Ok(Value::String(pgn)) = pgn {
                let ply = self.live.shown_ply();
                crate::shell::replace_analysis(window, cx, move |analysis, _, cx| {
                    analysis.open_game(
                        &pgn,
                        ply,
                        Some(orientation),
                        Some(origin.clone()),
                        false,
                        cx,
                    )
                });
                return;
            }
        }
        crate::pages::players::open_exported(
            frame["gameId"].as_str().unwrap_or_default(),
            frame["white"]["name"].clone(),
            frame["black"]["name"].clone(),
            orientation,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{base_minutes, info_line, past_result, past_score, points, result_line};
    use serde_json::json;

    #[test]
    fn a_finished_game_says_who_won_and_how() {
        assert_eq!(
            result_line(&json!({ "finished": true, "winner": "black", "status": "mate" })),
            "Black won · Checkmate"
        );
        assert_eq!(
            result_line(&json!({ "finished": true, "status": "stalemate" })),
            "Stalemate"
        );
        assert_eq!(result_line(&json!({ "finished": false })), "Live");
    }

    #[test]
    fn the_info_line_reads_like_lichess() {
        let frame = json!({
            "source": "tv", "channel": "atomic", "speed": "blitz", "rated": true,
            "clock": { "initial": 180, "increment": 2 },
        });
        let channels = vec![json!({ "key": "atomic", "label": "Atomic" })];
        assert_eq!(info_line(&frame, &channels), "3+2 · Rated · Blitz · Atomic");
        assert_eq!(base_minutes(30.), "½");
        assert_eq!(base_minutes(90.), "1.5");
    }

    #[test]
    fn scores_use_halves() {
        assert_eq!(points(2.5), "2½");
        assert_eq!(points(0.5), "½");
        assert_eq!(points(3.), "3");
    }

    #[test]
    fn past_games_show_each_sides_points() {
        let won = json!({ "winner": "white", "status": "mate" });
        assert_eq!(
            (past_score(&won, "white"), past_score(&won, "black")),
            ("1", "0")
        );
        assert_eq!(past_result(&won), "1–0");
        let drawn = json!({ "status": "draw" });
        assert_eq!(past_score(&drawn, "white"), "½");
        assert_eq!(
            past_result(&json!({ "status": "started" })),
            "Result pending"
        );
    }
}
