//! Home: where to start, the position you left, the recent games played here, and connecting
//! Lichess.
//! It reads the unfinished computer game and the accounts again each time it is shown.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use shakmaty::Color;

use crate::assets::icon;
use crate::board::ChessBoard;
use crate::core::AppCore;
use crate::game::computer::{ComputerGame, Host};
use crate::game::{rules, view};
use crate::pages::Page;
use crate::play_layout::{PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::played::PlayedHandle;
use crate::theme::{Palette, radii};
use crate::widgets;
use crate::widgets::KeyboardFocus as _;

mod lichess;

/// Recent games on Home (`localGames`).
const RECENT: usize = 6;
/// A tile's narrowest width; two of them share a row when they fit, else one.
const TILE: Pixels = px(200.);
const TILE_HEIGHT: Pixels = px(64.);
/// The board's side, and the smaller one beside its details in a narrow window.
const BOARD: Pixels = px(300.);
const BOARD_SMALL: Pixels = px(200.);

pub struct HomePage {
    /// The computer game left unfinished, and how Home names it.
    continue_game: Option<(Unfinished, Value)>,
    /// The position on the board, and what it was drawn from.
    board: Option<(String, Entity<ChessBoard>)>,
    /// Your Lichess activity, once an account is connected, and the account it shows.
    activity: Option<(String, Entity<lichess::LichessActivity>)>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for HomePage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// An unfinished computer game as Home names it: the opponent, and where the game stands.
struct Unfinished {
    /// "Stockfish · Club Player · ~1600".
    opponent: String,
    /// "Move 3 · You play White · Your move".
    state: String,
}

/// `computerDescription`: the level, the move to play, your side and whose turn it is.
fn continue_line(session: &Value) -> Option<Unfinished> {
    let moves = session["moves"].as_array().map_or(0, Vec::len);
    if moves == 0 {
        return None;
    }
    let host = Host {
        now: 0.,
        allowed: true,
        ready: false,
    };
    if ComputerGame::new(Some(session), &host).ok()?.over(&host) {
        return None;
    }
    let black = session["color"] == "black";
    let side = if black { "Black" } else { "White" };
    // White moves on even plies.
    let yours = moves.is_multiple_of(2) != black;
    let level = rules("engineLevelLabel", &[session["level"].clone()])
        .ok()
        .and_then(|label| label.as_str().map(str::to_string))
        .unwrap_or_default();
    Some(Unfinished {
        opponent: format!("Stockfish · {level}"),
        state: format!(
            "Move {} · You play {side} · {}",
            moves / 2 + 1,
            if yours {
                "Your move"
            } else {
                "Stockfish to move"
            }
        ),
    })
}

/// `localResult`: "1–0", "½–½", or "Unfinished".
fn result_text(result: &str) -> String {
    match result {
        "1/2-1/2" => "½–½".into(),
        "*" => "Unfinished".into(),
        result => result.replace('-', "–"),
    }
}

/// "Oct 22": the day a game started, in local time.
fn short_date(millis: f64) -> String {
    use chrono::TimeZone as _;
    chrono::Local
        .timestamp_millis_opt(millis as i64)
        .single()
        .map(|date| date.format("%b %-d").to_string())
        .unwrap_or_default()
}

impl HomePage {
    pub fn new(cx: &mut Context<Self>) -> HomePage {
        let played = cx.global::<PlayedHandle>().0.clone();
        let mut page = HomePage {
            continue_game: None,
            board: None,
            activity: None,
            focus: cx.focus_handle(),
            _subscriptions: vec![
                cx.observe(&played, |_, _, cx| cx.notify()),
                cx.observe(&crate::accounts::handle(cx), |_, _, cx| cx.notify()),
            ],
        };
        page.refresh(cx);
        page
    }

    /// Read the unfinished computer game and the accounts again (the page was shown).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let core = cx.global::<AppCore>().clone();
        let library = core.call("library", Vec::new());
        crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
        if let Some((account, activity)) = &self.activity {
            let account = account.clone();
            activity.update(cx, |activity, cx| activity.load(&account, cx));
        }
        cx.spawn(async move |this, cx| {
            let library = library.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(library) = library {
                    let session = &library["sessions"]["computer"];
                    this.continue_game = continue_line(session).map(|line| (line, session.clone()));
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// The side to see a game from: yours, when you played Black.
fn orientation(game: &Value) -> Color {
    if game["color"] == "black" || game["black"] == "You" {
        Color::Black
    } else {
        Color::White
    }
}

/// `game`'s setup and moves, as the board shows them.
fn line(game: &Value) -> (Value, Vec<String>) {
    let moves = serde_json::from_value(game["moves"].clone()).unwrap_or_default();
    (game["setup"].clone(), moves)
}

/// `.home-tile`: a place to start, beside the board.
fn tile(
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    page: Page,
    palette: &Palette,
) -> impl IntoElement {
    h_flex()
        .id(id)
        .role(Role::Button)
        .keyboard_focus(palette.primary)
        .aria_label(label)
        .flex_1()
        .flex_basis(TILE)
        .min_w_0()
        .h(TILE_HEIGHT)
        .gap(px(12.))
        .px(px(16.))
        .rounded(radii::XL)
        .border_1()
        .bg(palette.elevated)
        .border_color(palette.border)
        .text_color(palette.highlighted)
        .cursor_pointer()
        .hover(|tile| {
            tile.bg(palette.accented)
                .border_color(palette.border_accented)
        })
        .on_click(move |_, window, cx| crate::shell::open(page, window, cx))
        .child(
            div()
                .flex_none()
                .size(px(36.))
                .rounded(radii::LG)
                .bg(palette.accented)
                .flex()
                .items_center()
                .justify_center()
                .child(glyph.size(px(18.)).text_color(palette.toned)),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .child(label),
        )
}

impl Render for HomePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let (connected, busy) = {
            let accounts = crate::accounts::handle(cx);
            let accounts = accounts.read(cx);
            (accounts.active(cx), accounts.busy)
        };
        let width = window.viewport_size().width;
        let stacked = width < px(900.);
        let tiles = [
            tile(
                "home-puzzles",
                icon("puzzle"),
                "Puzzles",
                Page::Puzzles,
                &palette,
            )
            .into_any_element(),
            tile(
                "home-analysis",
                icon("microscope"),
                "Analysis board",
                Page::Analysis,
                &palette,
            )
            .into_any_element(),
            tile(
                "home-local",
                icon("swords"),
                "Over the board",
                Page::Local,
                &palette,
            )
            .into_any_element(),
            tile(
                "home-online",
                icon("globe-2"),
                "Play on Lichess",
                Page::Online,
                &palette,
            )
            .into_any_element(),
        ];
        // Two rows of two.
        let mut tiles = tiles.into_iter();
        let tiles = v_flex()
            .gap(px(12.))
            .children((0..2).map(|_| h_flex().gap(px(12.)).children(tiles.by_ref().take(2))));

        let mut games: Vec<Value> = cx
            .global::<PlayedHandle>()
            .0
            .read(cx)
            .games()
            .iter()
            .filter(|game| game["source"] != "clock")
            .cloned()
            .collect();
        games.sort_by(|a, b| {
            let started = |game: &Value| game["startedAt"].as_f64().unwrap_or_default();
            started(b).total_cmp(&started(a))
        });
        // The game you left, else the last one you finished, else the starting position.
        let session = self.continue_game.as_ref().map(|(_, session)| session);
        let latest = games.first().cloned();
        let (setup, moves) = session
            .or(latest.as_ref())
            .map_or_else(Default::default, line);
        let side = session
            .or(latest.as_ref())
            .map_or(Color::White, orientation);
        let drawn = format!("{setup}{moves:?}{side:?}");
        if self.board.as_ref().is_none_or(|(shown, _)| *shown != drawn) {
            let mut state = view::board_state(&setup, &moves, moves.len(), side, None);
            state.preview = true;
            self.board = Some((drawn, cx.new(|_| ChessBoard::new(state))));
        }
        let board = self.board.as_ref().map(|(_, board)| board.clone());
        let names = |game: &Value| {
            format!(
                "{} vs {}",
                game["white"].as_str().unwrap_or_default(),
                game["black"].as_str().unwrap_or_default()
            )
        };
        // What the board shows: its title, the line under it, and the game it opens.
        let (title, detail, opens) = match (&self.continue_game, &latest) {
            (Some((game, _)), _) => (Some(game.opponent.clone()), Some(game.state.clone()), None),
            (None, Some(game)) => (
                Some(names(game)),
                Some(format!(
                    "{} · {}",
                    result_text(game["result"].as_str().unwrap_or("*")),
                    short_date(game["startedAt"].as_f64().unwrap_or_default())
                )),
                game["id"].as_str().map(str::to_string),
            ),
            (None, None) => (None, None, None),
        };
        let unfinished = self.continue_game.is_some();
        let side_length = if stacked { BOARD_SMALL } else { BOARD };
        let board_label = title.clone().unwrap_or_else(|| "Play the computer".into());
        let board_game = opens.clone();
        let board_view = div()
            .id("home-board")
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(board_label)
            .size(side_length)
            .flex_none()
            .cursor_pointer()
            .hover(|board| board.opacity(0.92))
            .on_click(move |_, window, cx| match &board_game {
                Some(id) => crate::shell::open_game(id, window, cx),
                None => crate::shell::open(Page::Computer, window, cx),
            })
            .children(board);
        let primary = Button::new(if unfinished {
            "home-continue"
        } else {
            "home-play"
        })
        .primary()
        .icon(icon(if unfinished { "play" } else { "monitor" }))
        .label(if unfinished {
            "Continue"
        } else {
            "Play the computer"
        })
        .on_click(|_, window, cx| crate::shell::open(Page::Computer, window, cx));
        let review = opens.map(|id| {
            Button::new("home-review")
                .outline()
                .label("Review")
                .on_click(move |_, window, cx| crate::shell::open_game(&id, window, cx))
        });
        let about = v_flex()
            .min_w_0()
            .gap(px(2.))
            .children(title.map(|title| {
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.highlighted)
                    .child(title)
            }))
            .children(detail.map(|detail| {
                div()
                    .text_size(px(13.))
                    .text_color(palette.text_muted)
                    .child(detail)
            }));
        let actions = h_flex()
            .gap(px(8.))
            .child(div().flex_1().child(primary.w_full()))
            .children(review);
        // The board you left (or last played), what it is, and what to do with it.
        let now = widgets::card(&palette).p(px(16.)).flex_none();
        let now = if stacked {
            now.child(
                h_flex().items_start().gap(px(16.)).child(board_view).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .self_stretch()
                        .gap(px(14.))
                        .child(about)
                        .child(div().flex_1())
                        .child(actions.flex_wrap()),
                ),
            )
        } else {
            now.w(side_length + px(34.))
                .gap(px(14.))
                .child(board_view)
                .child(about)
                .child(div().flex_1())
                .child(actions)
        };

        games.truncate(RECENT);
        let recent = widgets::card(&palette)
            .flex_1()
            .min_w_0()
            .gap(px(4.))
            .child(
                h_flex()
                    .justify_between()
                    .mb(px(4.))
                    .child(widgets::heading("Recent games", &palette))
                    .when(!games.is_empty(), |header| {
                        header.child(
                            // The arrow trails the label (`trailing-icon`).
                            Button::new("home-view-all")
                                .text()
                                .xsmall()
                                .accessibility_label("View all")
                                .child(
                                    h_flex()
                                        .gap(px(4.))
                                        .child("View all")
                                        .child(icon("arrow-right").size(px(14.))),
                                )
                                .on_click(|_, window, cx| {
                                    crate::shell::open(Page::History, window, cx)
                                }),
                        )
                    }),
            )
            .when(games.is_empty(), |card| {
                card.child(
                    widgets::empty_state(icon("history"), "No saved games yet", None, &palette)
                        .flex_1()
                        .justify_center()
                        .py(px(24.)),
                )
            })
            .child(
                v_flex()
                    .mx(px(-8.))
                    .children(games.into_iter().enumerate().map(|(index, game)| {
                        let id = game["id"].as_str().unwrap_or_default().to_string();
                        h_flex()
                            .id(("home-game", index))
                            .role(Role::Button)
                            .keyboard_focus(palette.primary)
                            .aria_label(names(&game))
                            .gap(px(12.))
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(radii::MD)
                            .text_size(px(13.))
                            .cursor_pointer()
                            .hover(|row| row.bg(palette.accented))
                            .on_click(move |_, window, cx| crate::shell::open_game(&id, window, cx))
                            .child(div().w(px(52.)).flex_none().child(
                                crate::pages::history::badge(
                                    &crate::pages::history::outcome(&game),
                                    Some(px(52.)),
                                    &palette,
                                ),
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(crate::pages::history::opponent(&game)),
                            )
                            .child(
                                div()
                                    .w(px(52.))
                                    .flex_none()
                                    .text_right()
                                    .text_color(palette.text_muted)
                                    .child(short_date(
                                        game["startedAt"].as_f64().unwrap_or_default(),
                                    )),
                            )
                    })),
            );
        // A connected account's activity follows; until then, one line to connect it.
        let activity = match connected.clone() {
            Some(account) => {
                let stale = self
                    .activity
                    .as_ref()
                    .is_none_or(|(shown, _)| *shown != account);
                if stale {
                    let view = self
                        .activity
                        .take()
                        .map(|(_, view)| view)
                        .unwrap_or_else(|| cx.new(|cx| lichess::LichessActivity::new(window, cx)));
                    view.update(cx, |view, cx| view.load(&account, cx));
                    self.activity = Some((account, view));
                }
                self.activity.as_ref().map(|(_, view)| view.clone())
            }
            None => {
                self.activity = None;
                None
            }
        };
        let connect = activity.is_none().then(|| {
            widgets::card(&palette).py(px(16.)).child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(36.))
                            .rounded(radii::LG)
                            .bg(palette.accented)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("chart-line").size(px(18.)).text_color(palette.toned)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(200.))
                            .child(widgets::heading("Lichess", &palette))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(palette.text_muted)
                                    .child("Ratings, games and puzzle progress"),
                            ),
                    )
                    .child(
                        Button::new("home-connect")
                            .outline()
                            .icon(icon("log-in"))
                            .label("Connect Lichess")
                            .loading(busy)
                            .disabled(busy)
                            .on_click(|_, window, cx| {
                                crate::accounts::handle(cx)
                                    .update(cx, |accounts, cx| accounts.connect(window, cx))
                            }),
                    ),
            )
        });
        let starts = v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(16.))
            .child(tiles)
            .child(recent);
        div()
            .id("home-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .px(side_padding(window))
                    .gap(px(32.))
                    .child(if stacked {
                        v_flex().gap(px(16.)).child(now).child(starts)
                    } else {
                        h_flex()
                            .items_stretch()
                            .gap(px(24.))
                            .child(now)
                            .child(starts)
                    })
                    .children(connect)
                    .children(activity),
            )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use shakmaty::Color;

    use super::{line, orientation, result_text};

    #[test]
    fn the_board_shows_the_game_from_your_side() {
        assert_eq!(orientation(&json!({ "color": "black" })), Color::Black);
        assert_eq!(
            orientation(&json!({ "white": "Expert", "black": "You" })),
            Color::Black
        );
        assert_eq!(
            orientation(&json!({ "white": "You", "black": "Sam" })),
            Color::White
        );
        let (_, moves) = line(&json!({ "moves": ["e2e4", "e7e5"] }));
        assert_eq!(moves, ["e2e4", "e7e5"]);
    }

    #[test]
    fn results_read_as_on_a_scoresheet() {
        assert_eq!(result_text("1-0"), "1–0");
        assert_eq!(result_text("1/2-1/2"), "½–½");
        assert_eq!(result_text("*"), "Unfinished");
    }
}
