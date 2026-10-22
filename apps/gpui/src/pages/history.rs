//! Game history (`pages/history.vue`, `LocalGameHistory.vue`): the games played against the
//! computer and over the board, filtered by result, twenty to a page, each opened to go through
//! move by move, exported as PGN or deleted.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
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
use crate::theme::{Palette, tint};
use crate::widgets::{self, Pill};

const PAGE_SIZE: usize = 20;
/// Below this page width the list keeps only the result and the players.
const NARROW: Pixels = px(620.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Computer,
    Board,
    Lichess,
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
fn outcome(game: &Value) -> String {
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

/// How a badge is drawn (`UBadge` color and variant).
#[derive(Clone, Copy)]
enum Look {
    Success,
    Error,
    NeutralSubtle,
    NeutralSoft,
    Info,
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

/// `.side-dot`: a piece colour, the same light or dark disc in either theme.
fn side_dot(color: Color) -> Div {
    div()
        .size(px(9.))
        .flex_none()
        .rounded_full()
        .border_1()
        .border_color(rgb(0x6b6b6b))
        .bg(rgb(if color == Color::White {
            0xf4f4f2
        } else {
            0x1f1f1f
        }))
}

/// `ResultBadge.vue`.
fn badge(outcome: &str, width: Option<Pixels>, palette: &Palette) -> impl IntoElement {
    let (label, look, side) = badge_of(outcome);
    let (bg, fg, ring) = match look {
        Look::Success => (tint(palette.success, 10.), palette.success, None),
        Look::Error => (tint(palette.error, 10.), palette.error, None),
        Look::Info => (tint(palette.info, 10.), palette.info, None),
        Look::NeutralSoft => (palette.elevated, palette.toned, None),
        Look::NeutralSubtle => (
            palette.elevated,
            palette.toned,
            Some(palette.border_accented),
        ),
    };
    h_flex()
        .when_some(width, |badge, width| badge.w(width))
        .justify_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(6.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(12.))
        .line_height(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .when_some(ring, |badge, ring| badge.border_1().border_color(ring))
        .children(side.map(side_dot))
        .child(label)
}

/// `formatGameDate`: "Oct 22, 2026" in local time.
fn date(millis: f64) -> String {
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_millis_opt(millis as i64)
        .single()
        .map(|date| date.format("%b %-d, %Y").to_string())
        .unwrap_or_default()
}

/// `moveCount`: full moves, a lone white move counting as one.
fn move_count(plies: usize) -> String {
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
fn opponent(game: &Value) -> String {
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

/// The game as PGN, with the headers Electron writes (`LocalGameHistory.vue`, `pgn`).
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

pub struct HistoryPage {
    source: Source,
    filter: Filter,
    /// The page of the list shown, from 0.
    page: usize,
    /// The game being gone through, and its board.
    open: Option<(Value, Entity<GameReplay>)>,
    message: Option<SharedString>,
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
    pub fn new(cx: &mut Context<Self>) -> HistoryPage {
        let played = cx.global::<PlayedHandle>().0.clone();
        HistoryPage {
            source: Source::Computer,
            filter: Filter::All,
            page: 0,
            open: None,
            message: None,
            area: Measured::default(),
            focus: cx.focus_handle(),
            _subscriptions: vec![cx.observe(&played, |this, _, cx| {
                // A deleted game closes; the pages left shrink with the list.
                if let Some((game, _)) = &this.open
                    && !this.games(cx).iter().any(|entry| entry["id"] == game["id"])
                {
                    this.open = None;
                }
                cx.notify();
            })],
        }
    }

    /// The games of the source shown, newest first.
    fn games(&self, cx: &App) -> Vec<Value> {
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

    fn choose_source(&mut self, source: Source, cx: &mut Context<Self>) {
        self.source = source;
        self.filter = Filter::All;
        self.page = 0;
        self.open = None;
        cx.notify();
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

    fn close_game(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
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
        let folder = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = format!("kchess-{}.pgn", text(game, "id"));
        let chosen = cx.prompt_for_new_path(&folder, Some(&name));
        cx.spawn(async move |this, cx| {
            let message: Option<SharedString> = match chosen.await {
                Ok(Ok(Some(path))) => Some(match std::fs::write(&path, pgn) {
                    Ok(()) => "Saved the PGN".into(),
                    Err(cause) => format!("Export failed: {cause}").into(),
                }),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(cause)) => Some(format!("Export failed: {cause}").into()),
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

    fn ask_delete(&mut self, game: Value, window: &mut Window, cx: &mut Context<Self>) {
        let id = text(&game, "id").to_string();
        window.open_alert_dialog(cx, move |dialog: AlertDialog, _, _| {
            let id = id.clone();
            dialog
                .title("Delete saved game?")
                .description(
                    "This removes the game from this device. Export a copy first if you want to keep it.",
                )
                .ok_text("Delete game")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let played = cx.global::<PlayedHandle>().0.clone();
                    played.update(cx, |played: &mut PlayedGames, cx| played.remove(&id, cx));
                    true
                })
        });
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
    let sources = [Source::Computer, Source::Board, Source::Lichess];
    let page = cx.entity().downgrade();
    widgets::pill_tabs(
        "history-source",
        vec![
            (None, "Computer".into()),
            (None, "Over the board".into()),
            (None, "Lichess".into()),
        ],
        sources.iter().position(|item| *item == source).unwrap_or(0),
        move |index, _, cx| {
            let _ = page.update(cx, |page, cx| page.choose_source(sources[index], cx));
        },
        palette,
    )
}

/// An empty list's message and the action that fills it (`UEmpty`).
fn empty(
    glyph: &'static str,
    title: &'static str,
    action: Button,
    palette: &Palette,
) -> impl IntoElement {
    widgets::card(palette)
        .items_center()
        .gap(px(12.))
        .py(px(28.))
        .child(
            match glyph {
                "search-x" => icon("search-x"),
                _ => icon("history"),
            }
            .size(px(24.))
            .text_color(palette.text_muted),
        )
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.highlighted)
                .child(title),
        )
        .child(action)
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
        if games.is_empty() {
            let (label, page) = if self.source == Source::Computer {
                ("Play the computer", Page::Computer)
            } else {
                ("Play over the board", Page::Local)
            };
            return empty(
                "history",
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
            row.gap(px(12.))
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
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(palette.dimmed)
                .child(label.to_uppercase())
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
                let details = details(&game);
                let variant = game["setup"]["variant"].as_str().unwrap_or("standard");
                let mut kind = time_label(text(&game, "timeControl"));
                if variant != "standard" {
                    let mut letters = variant.chars();
                    if let Some(first) = letters.next() {
                        kind.push_str(&format!(" · {}{}", first.to_uppercase(), letters.as_str()));
                    }
                }
                let players = v_flex()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .min_w_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .children(side.map(side_dot))
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
                        date(game["startedAt"].as_f64().unwrap_or_default())
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
                                            page.ask_delete(deleted, window, cx)
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
                        .pl(px(12.))
                        .pr(px(8.))
                        .py(px(11.))
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
                            .child(date(game["startedAt"].as_f64().unwrap_or_default()))
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
                        .gap(px(10.))
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
                                .child("No games match this filter"),
                        )
                        .child(
                            Button::new("show-all")
                                .outline()
                                .label("Show all games")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.choose_filter(Filter::All, cx)
                                })),
                        ),
                )
            })
            .when(pages > 1, |card| card.child(self.pager(current, pages, cx)));
        v_flex()
            .gap(px(16.))
            .child(h_flex().child(filter_tabs))
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
        let error = cx.global::<PlayedHandle>().0.read(cx).error.clone();
        let header = v_flex()
            .px(side_padding(window))
            .pt(PAGE_TOP)
            .child(source_tabs(self.source, cx, &palette));
        if let Some((game, replay)) = self.open.clone() {
            return self.render_open(&game, &replay, window, cx, &palette);
        }
        let body = match self.source {
            Source::Lichess => widgets::card(&palette)
                .items_center()
                .py(px(40.))
                .text_color(palette.text_muted)
                .child("Use the Electron app for Lichess games for now.")
                .into_any_element(),
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
