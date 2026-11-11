//! Home: where to start, the position you left, the recent games played here, and connecting
//! Lichess.
//! It reads the unfinished computer game and the accounts again each time it is shown.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_domain::variant::Setup;
use serde_json::Value;
use shakmaty::{Color, Position as _};

use crate::assets::icon;
use crate::board::ChessBoard;
use crate::core::AppCore;
use crate::game::view;
use crate::pages::Page;
use crate::play_layout::{Measured, PAGE_BOTTOM, PAGE_TOP, side_padding};
use crate::played::PlayedHandle;
use crate::theme::{Palette, radii};
use crate::widgets;
use crate::widgets::KeyboardFocus as _;
use crate::widgets::Pressable as _;
use kchess_domain::games::Host;
use kchess_domain::games::computer::ComputerGame;

mod lichess;

/// A game's variant by name, when it is not standard chess.
fn variant_of(game: &Value) -> Option<String> {
    game["setup"]["variant"]
        .as_str()
        .filter(|variant| *variant != "standard")
        .map(|variant| crate::game::variant_label(variant).to_string())
}

/// Recent games on Home.
const RECENT: usize = 6;
/// A tile's narrowest width; two of them share a row when they fit, else one.
const TILE: Pixels = px(200.);
const TILE_HEIGHT: Pixels = px(56.);
/// The board's side, and the smaller one beside its details in a narrow window.
const BOARD: Pixels = px(300.);
const BOARD_SMALL: Pixels = px(200.);
/// The widest Home grows; past it, the page keeps to the middle.
const CONTENT_MAX: Pixels = px(1280.);
/// Gaps between the cards in a group, and between the board's card and the rest.
const GAP: Pixels = px(12.);
const COLUMN_GAP: Pixels = px(24.);

pub struct HomePage {
    /// The computer game left unfinished, and how Home names it.
    continue_game: Option<(Unfinished, Value)>,
    /// The position on the board, and what it was drawn from.
    board: Option<(String, Entity<ChessBoard>)>,
    /// The last game's opening, by the game and its length, worked out once.
    last_opening: Option<((String, usize), Option<String>)>,
    /// Your Lichess activity, once an account is connected (else a Chess.com player's), and the
    /// account it shows.
    activity: Option<(String, Entity<lichess::LichessActivity>)>,
    /// The newest played games (`RECENT`), as of a revision of the played games: Home drew them
    /// by sorting every played game on every frame.
    recent: std::cell::RefCell<(Option<u64>, Vec<Value>)>,
    /// The page's content, as last laid out: it, not the window, decides what goes beside what.
    area: Measured,
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
    /// "Stockfish · Club Player", and its strength, "~1600".
    opponent: (String, Option<String>),
    /// "Move 3", "You play White", "Your move".
    state: Vec<String>,
}

/// The level, the move to play, your side and whose turn it is, as the game holds them.
fn continue_line(session: &Value) -> Option<Unfinished> {
    let host = Host {
        now: 0.,
        allowed: true,
        ready: false,
    };
    let game = ComputerGame::resume(Some(session), &host).ok()?;
    let moves = game.moves();
    if moves.is_empty() || game.over() {
        return None;
    }
    let side = if game.color() == Color::Black {
        "Black"
    } else {
        "White"
    };
    // Whose move it is and the move number, from the position reached: a game started from a
    // position (Black to move, or at move 30) does not count from the usual start. White moves
    // on even plies when the position cannot be replayed.
    let (to_move, number) = view::position_at(game.setup(), moves, moves.len()).map_or_else(
        || {
            let even = moves.len().is_multiple_of(2);
            (Color::from_white(even), moves.len() / 2 + 1)
        },
        |reached| (reached.turn(), reached.fullmoves().get() as usize),
    );
    let level = crate::game::engine_level(game.level());
    let name = level.map_or("", |level| level.label.as_str());
    let strength = level.map(|level| match level.elo {
        Some(elo) => format!("~{}", kchess_domain::js::number_to_string(elo)),
        None => "Full strength".to_string(),
    });
    // A variant is named first, as the game page names it.
    let variant = Some(game.setup().variant)
        .filter(|variant| *variant != kchess_domain::variant::Variant::Standard)
        .map(|variant| variant.label().to_string());
    Some(Unfinished {
        opponent: (format!("Stockfish · {name}"), strength),
        state: variant
            .into_iter()
            .chain([
                format!("Move {number}"),
                format!("You play {side}"),
                if to_move == game.color() {
                    "Your move"
                } else {
                    "Stockfish to move"
                }
                .to_string(),
            ])
            .collect(),
    })
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

/// Home's buttons, as Quick Pick offers them; its Sync games is the app's own _Sync Lichess
/// games_ there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    /// The last game, open in Game history.
    Review,
}

/// Your newest `limit` saved games, newest first; the chess clock's are not games to show.
/// Only those are copied: Home reads this on every render, and up to 500 games are saved.
fn played_games(cx: &App, limit: usize) -> Vec<Value> {
    let mut games: Vec<&Value> = cx
        .global::<PlayedHandle>()
        .0
        .read(cx)
        .games()
        .iter()
        .filter(|game| game["source"] != "clock")
        .collect();
    games.sort_by(|a, b| {
        let started = |game: &Value| game["startedAt"].as_f64().unwrap_or_default();
        started(b).total_cmp(&started(a))
    });
    games.into_iter().take(limit).cloned().collect()
}

impl HomePage {
    pub fn new(cx: &mut Context<Self>) -> HomePage {
        let played = cx.global::<PlayedHandle>().0.clone();
        // The last game's opening shows only while opening names are.
        let settings = cx.global::<crate::settings::SettingsHandle>().0.clone();
        let mut page = HomePage {
            continue_game: None,
            board: None,
            last_opening: None,
            activity: None,
            recent: Default::default(),
            area: Measured::default(),
            focus: cx.focus_handle(),
            _subscriptions: vec![
                cx.observe(&played, |_, _, cx| cx.notify()),
                cx.observe(&crate::accounts::handle(cx), |_, _, cx| cx.notify()),
                cx.observe(&settings, |_, _, cx| cx.notify()),
            ],
        };
        page.refresh(cx);
        page
    }

    /// The newest played games, worked out again only once the played games change.
    fn with_recent<R>(&self, cx: &App, read: impl FnOnce(&[Value]) -> R) -> R {
        let revision = cx.global::<PlayedHandle>().0.read(cx).revision();
        let mut kept = self.recent.borrow_mut();
        if kept.0 != Some(revision) {
            *kept = (Some(revision), played_games(cx, RECENT));
        }
        read(&kept.1)
    }

    /// The game _Review_ opens: the last one, unless a computer game is left unfinished.
    fn reviewable(&self, cx: &App) -> Option<String> {
        if self.continue_game.is_some() {
            return None;
        }
        self.with_recent(cx, |games| {
            games.first()?["id"].as_str().map(str::to_string)
        })
    }

    /// What Home's buttons can do now.
    pub(crate) fn actions(&self, cx: &App) -> Vec<Action> {
        if self.reviewable(cx).is_some() {
            vec![Action::Review]
        } else {
            Vec::new()
        }
    }

    /// Run an action as its button does, if it is still allowed.
    pub(crate) fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions(cx).contains(&action) {
            return;
        }
        match action {
            Action::Review => {
                if let Some(id) = self.reviewable(cx) {
                    crate::pages::history::open_game(&id, window, cx);
                }
            }
        }
    }

    /// The activity card shown, if any.
    #[cfg(test)]
    pub(crate) fn activity(&self) -> Option<Entity<lichess::LichessActivity>> {
        self.activity.as_ref().map(|(_, view)| view.clone())
    }

    /// Read the unfinished computer game (that session alone: the whole library replays every
    /// played game) and the accounts again (the page was shown).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let session = cx.global::<AppCore>().request(calls::Session {
            kind: kchess_contracts::types::SessionKind::Computer,
        });
        crate::accounts::handle(cx).update(cx, |accounts, cx| accounts.refresh(cx));
        if let Some((account, activity)) = &self.activity {
            let account = account.clone();
            activity.update(cx, |activity, cx| activity.load(&account, cx));
        }
        cx.spawn(async move |this, cx| {
            let session = session.await;
            let _ = this.update(cx, |this, cx| {
                match session {
                    Ok(session) => {
                        this.continue_game = session.and_then(|session| {
                            continue_line(&session).map(|line| (line, session.clone()))
                        });
                    }
                    // The card stays as it was; a failed read is not the game being over.
                    Err(cause) => crate::log::write(
                        kchess_contracts::Level::Warn,
                        "library",
                        &format!("Reading the unfinished game failed: {}", cause.message),
                    ),
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
fn line(game: &Value) -> (Setup, Vec<String>) {
    let moves = serde_json::from_value(game["moves"].clone()).unwrap_or_default();
    (setup_of(game), moves)
}

/// A game's setup; one that cannot be read shows no position.
fn setup_of(game: &Value) -> Setup {
    crate::game::setup_of(&game["setup"]).unwrap_or_else(|| Setup::standard_at(""))
}

/// Whether the board goes over the rest for a page `width` wide, and how many tiles share a
/// row: the board's card beside two tiles abreast, else the board over tiles four, two or one
/// abreast, as many as fit whole.
fn arrangement(width: Pixels) -> (bool, usize) {
    let card = BOARD + px(34.);
    let stacked = width < card + COLUMN_GAP + TILE * 2. + GAP;
    let tiles_width = if stacked {
        width
    } else {
        width - card - COLUMN_GAP
    };
    let per_row = if stacked && tiles_width >= TILE * 4. + GAP * 3. {
        4
    } else if tiles_width >= TILE * 2. + GAP {
        2
    } else {
        1
    };
    (stacked, per_row)
}

/// A place to start, beside the board.
fn tile(
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    page: Page,
    palette: &Palette,
) -> impl IntoElement {
    h_flex()
        .id(id)
        .test_support()
        .role(Role::Button)
        .keyboard_focus(palette.primary)
        .aria_label(label)
        .flex_1()
        .flex_basis(TILE)
        .min_w_0()
        .h(TILE_HEIGHT)
        .gap(px(12.))
        .px(px(12.))
        .rounded(radii::XL)
        .border_1()
        .bg(palette.elevated)
        .border_color(palette.border)
        .text_color(palette.highlighted)
        .cursor_pointer()
        .pressable_tile(palette)
        .on_click(move |_, window, cx| crate::shell::open(page, window, cx))
        .child(
            div()
                .flex_none()
                .size(px(32.))
                .rounded(radii::LG)
                .bg(palette.accented)
                .flex()
                .items_center()
                .justify_center()
                .child(glyph.size(px(16.)).text_color(palette.toned)),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(14.))
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
            // Without a Lichess account, the first Chess.com player's games are the activity.
            let account = accounts.active(cx).or_else(|| {
                accounts
                    .chesscom()
                    .first()
                    .map(|player| kchess_domain::chesscom::account_of(&player.username))
            });
            (account, accounts.busy)
        };
        let padding = side_padding(window);
        // Before the first layout, the window less the sidebar is the guess.
        let measured = self.area.size().width;
        let width = if measured > px(0.) {
            measured - padding * 2.
        } else {
            window.viewport_size().width - px(260.)
        };
        let (stacked, per_row) = arrangement(width);
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
        let mut tiles = tiles.into_iter().peekable();
        let mut rows = Vec::new();
        while tiles.peek().is_some() {
            rows.push(h_flex().gap(GAP).children(tiles.by_ref().take(per_row)));
        }
        let tiles = v_flex().gap(GAP).children(rows);

        let mut games = self.with_recent(cx, <[Value]>::to_vec);
        // The game you left, else the last one you finished, else the starting position.
        let session = self.continue_game.as_ref().map(|(_, session)| session);
        let latest = games.first().cloned();
        let (setup, moves) = session
            .or(latest.as_ref())
            .map_or_else(|| (Setup::standard_at(""), Vec::new()), line);
        let side = session
            .or(latest.as_ref())
            .map_or(Color::White, orientation);
        let drawn = format!("{setup:?}{moves:?}{side:?}");
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
            (Some((game, _)), _) => (
                Some(game.opponent.clone()),
                // Beside the smaller board, a line each.
                Some(if stacked {
                    v_flex().children(game.state.clone()).into_any_element()
                } else {
                    widgets::dotted(game.state.clone()).into_any_element()
                }),
                None,
            ),
            // The last game, as its row in Recent games says it: who won, and against whom; and
            // the opening it was.
            (None, Some(game)) => {
                let key = (
                    game["id"].as_str().unwrap_or_default().to_string(),
                    game["moves"].as_array().map_or(0, Vec::len),
                );
                if self
                    .last_opening
                    .as_ref()
                    .is_none_or(|(seen, _)| *seen != key)
                {
                    let name = crate::pages::history::opening_name(game);
                    self.last_opening = Some((key, name));
                }
                // A variant game is named by its variant, as its row is; a standard one by its
                // opening, while opening names are shown.
                let opening = variant_of(game).or_else(|| {
                    self.last_opening
                        .as_ref()
                        .and_then(|(_, name)| name.clone())
                        .filter(|_| crate::pages::history::opening_names_shown(cx))
                });
                (
                    Some((crate::pages::history::opponent(game), None)),
                    Some(
                        v_flex()
                            .gap(px(6.))
                            .child(
                                h_flex()
                                    .gap(px(8.))
                                    .child(crate::pages::history::badge(
                                        &crate::pages::history::outcome(game),
                                        None,
                                        &palette,
                                    ))
                                    .child(short_date(
                                        game["startedAt"].as_f64().unwrap_or_default(),
                                    )),
                            )
                            .children(opening.map(|opening| {
                                div()
                                    .id("home-opening")
                                    .test_support()
                                    .truncate()
                                    .aria_label(opening.clone())
                                    .child(opening)
                            }))
                            .into_any_element(),
                    ),
                    game["id"].as_str().map(str::to_string),
                )
            }
            (None, None) => (None, None, None),
        };
        let unfinished = self.continue_game.is_some();
        let side_length = if stacked { BOARD_SMALL } else { BOARD };
        let board_label = title.as_ref().map_or_else(
            || "Play the computer".into(),
            |(name, strength)| match strength {
                Some(strength) => format!("{name} · {strength}"),
                None => name.clone(),
            },
        );
        let board_game = opens.clone();
        let board_view = div()
            .id("home-board")
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(board_label)
            .size(side_length)
            .flex_none()
            .cursor_pointer()
            .pressable_fade(0.92)
            .on_click(move |_, window, cx| match &board_game {
                Some(id) => crate::pages::history::open_game(id, window, cx),
                None => crate::shell::open(Page::Computer, window, cx),
            })
            .children(board);
        let primary = Button::new(if unfinished {
            "home-continue"
        } else {
            "home-play"
        })
        .primary()
        .icon(icon(if unfinished { "play" } else { "bot" }))
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
                .on_click(move |_, window, cx| crate::pages::history::open_game(&id, window, cx))
        });
        let has_about = title.is_some();
        let about = v_flex()
            .min_w_0()
            .gap(px(2.))
            // The strength follows the name, muted, and drops to the next line whole.
            .children(title.map(|(name, strength)| {
                h_flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap_x(px(6.))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.highlighted)
                            .child(name),
                    )
                    .children(strength.map(|strength| {
                        div()
                            .whitespace_nowrap()
                            .text_color(palette.text_muted)
                            .child(strength)
                    }))
            }))
            .children(detail.map(|detail| {
                div()
                    .mt(px(2.))
                    .text_size(px(13.))
                    .text_color(palette.text_muted)
                    .child(detail)
            }));
        // Side by side under the board; stacked beside the smaller one.
        let actions = if stacked {
            v_flex()
                // A button's width, not the card's, in a wide window.
                .w_full()
                .max_w(px(320.))
                .gap(px(8.))
                .child(primary.w_full())
                .children(review.map(|review| review.w_full()))
        } else {
            h_flex()
                .gap(px(8.))
                .child(div().flex_1().child(primary.w_full()))
                .children(review)
        };
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
                        // With nothing to name, the action stands level with the board's middle.
                        .when(!has_about, |side| side.justify_center())
                        .when(has_about, |side| side.child(about).child(div().flex_1()))
                        .child(actions),
                ),
            )
        } else {
            now.w(side_length + px(34.))
                .gap(px(14.))
                .child(board_view)
                .when(has_about, |card| card.child(about))
                .child(div().flex_1())
                .child(actions)
        };

        games.truncate(RECENT);
        // Before the first game there is nothing to list: the board's card already starts one.
        let has_games = !games.is_empty();
        let recent = widgets::card(&palette)
            .flex_1()
            .min_w_0()
            .gap(px(4.))
            .child(
                h_flex()
                    .justify_between()
                    .mb(px(4.))
                    .child(widgets::heading("Recent games", &palette))
                    .child(
                        // The arrow trails the label.
                        widgets::text_button("home-view-all")
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
                    ),
            )
            .child(
                v_flex()
                    .mx(px(-12.))
                    .children(games.into_iter().enumerate().map(|(index, game)| {
                        let id = game["id"].as_str().unwrap_or_default().to_string();
                        h_flex()
                            .id(("home-game", index))
                            .test_support()
                            .role(Role::Button)
                            .keyboard_focus(palette.primary)
                            .aria_label(match variant_of(&game) {
                                Some(variant) => format!("{}, {variant}", names(&game)),
                                None => names(&game),
                            })
                            .gap(px(12.))
                            .px(px(12.))
                            .py(px(8.))
                            .rounded(radii::MD)
                            .text_size(px(13.))
                            .cursor_pointer()
                            .pressable(&palette)
                            .on_click(move |_, window, cx| {
                                crate::pages::history::open_game(&id, window, cx)
                            })
                            .child(
                                div()
                                    .id(("home-game-outcome", index))
                                    .test_support()
                                    .flex_none()
                                    .child(crate::pages::history::badge(
                                        &crate::pages::history::outcome(&game),
                                        Some(crate::pages::history::BADGE_WIDTH),
                                        &palette,
                                    )),
                            )
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
                                    .flex_none()
                                    .text_right()
                                    .text_color(palette.text_muted)
                                    .child({
                                        let count = crate::pages::history::move_count(
                                            game["moves"].as_array().map_or(0, Vec::len),
                                        );
                                        match variant_of(&game) {
                                            Some(variant) => format!("{variant} · {count}"),
                                            None => count,
                                        }
                                    }),
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
        // A connected account's (or Chess.com player's) activity follows; until then, one line to
        // connect it.
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
                                    .child("Your ratings and games"),
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
            .when(has_games, |starts| starts.child(recent));
        // The connect card is one more card in the grid; a connected account's section, with its
        // own heading, stands further off.
        let section_gap = match (connect.is_some(), stacked) {
            (true, true) => px(16.),
            (true, false) => px(24.),
            (false, _) => px(32.),
        };
        div()
            .id("home-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .relative()
                    .w_full()
                    .max_w(CONTENT_MAX + padding * 2.)
                    .mx_auto()
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .px(padding)
                    .gap(section_gap)
                    .child(self.area.probe(cx))
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

impl crate::pages::PageAction for Action {
    fn info(&self) -> crate::pages::ActionInfo {
        match self {
            Action::Review => crate::pages::ActionInfo::new(
                "home:review",
                "Review last game",
                "microscope",
                "analyse recent",
            ),
        }
    }
}

impl crate::pages::PageView for HomePage {
    const INFO: crate::pages::PageInfo = crate::pages::PageInfo {
        page: Page::Dashboard,
        label: "Home",
        short: "Home",
        icon: "house",
        group: Some(crate::pages::NavGroup::Home),
        keywords: "start overview",
    };

    type Action = Action;

    fn create(_: &mut Window, cx: &mut Context<Self>) -> Self {
        HomePage::new(cx)
    }

    /// Home shows the unfinished game and accounts as they are now.
    fn shown(this: &Entity<Self>, _again: bool, _window: &mut Window, cx: &mut App) {
        this.update(cx, |home, cx| home.refresh(cx));
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        HomePage::actions(self, cx)
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        this.update(cx, |home, cx| home.act(action, window, cx));
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use shakmaty::Color;

    use gpui_kit::px;

    use super::{TILE, arrangement, line, orientation};

    /// A 1000px window with the sidebar open leaves Home about 680px: the tiles once shared
    /// what was left beside the board and were cut short ("Analysis bo…").
    #[test]
    fn tiles_are_never_narrower_than_a_tile() {
        assert_eq!(arrangement(px(683.)), (true, 2));
        assert_eq!(arrangement(px(1280.)), (false, 2));
        assert_eq!(arrangement(px(380.)), (true, 1));
        assert_eq!(arrangement(px(769.)), (true, 2));
        for width in (300..1400).step_by(7) {
            let (stacked, per_row) = arrangement(px(width as f32));
            let beside = if stacked { 0. } else { 334. + 24. };
            let room = width as f32 - beside - 12. * (per_row as f32 - 1.);
            assert!(
                per_row == 1 || room / per_row as f32 >= TILE.as_f32(),
                "{width}px fits {per_row} tiles"
            );
        }
    }

    /// The unfinished game's card counts from where it started: a Crazyhouse game from the
    /// analysis board with Black to move at move 4, and the usual start.
    #[test]
    fn an_unfinished_game_says_whose_move_from_its_own_start() {
        let card = |session: serde_json::Value| {
            super::continue_line(&session)
                .expect("an unfinished game")
                .state
        };
        let from_position = json!({
            "version": 2,
            "setup": {
                "variant": "crazyhouse",
                "fen": "rnb1kbnr/ppp1pppp/8/3q4/8/4P3/PPPP1PPP/RNBQKBNR/P b KQkq - 0 3",
            },
            "moves": ["d5e5"],
            "level": "club",
            "color": "black",
        });
        assert_eq!(
            card(from_position),
            [
                "Crazyhouse",
                "Move 4",
                "You play Black",
                "Stockfish to move"
            ]
        );
        let usual = json!({
            "moves": ["e2e4", "e7e5", "g1f3"],
            "level": "club",
            "color": "black",
        });
        assert_eq!(card(usual), ["Move 2", "You play Black", "Your move"]);
    }

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
}
