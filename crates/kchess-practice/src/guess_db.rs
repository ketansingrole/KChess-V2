//! A game from your databases to guess through: the games of the database picked, or of a player
//! in it, listed as the Database page lists them. Shown only when there is a database.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, GameDbGameId};
use kchess_contracts::types::{GameDbQuery, GameDbRow};
use shakmaty::Color;

use super::classics::winner;
use super::guess_find::Found;
use crate::core::AppCore;
use crate::game_db::GameDbHandle;
use crate::game_db_list;
use crate::loading::{Epoch, Loading};
use crate::theme::Palette;
use crate::widgets::{self, Choice};
use kchess_ui::icon;

/// How many games are listed: the latest, or a player's latest.
const LISTED: u32 = 20;

/// The list's height: six rows, the rest scrolled to.
const LIST_HEIGHT: Pixels = px(216.);

/// Under this width a row keeps White, Black and the result, as on the Database page.
const NARROW: Pixels = px(720.);

pub struct DatabaseFinder {
    select: Entity<SelectState<Vec<Choice>>>,
    /// The databases in the picker, as (id, name).
    listed: Vec<(String, String)>,
    database: Option<String>,
    player: Entity<InputState>,
    games: Vec<GameDbRow>,
    loading: Loading,
    /// The row of the game being fetched, and its request.
    opening: Option<usize>,
    open_epoch: Epoch,
    /// Why the game picked could not be fetched.
    open_error: Option<SharedString>,
    /// The list's scroll, kept to it rather than shared with the page's.
    scroll: widgets::NestedScroll,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Found> for DatabaseFinder {}

impl DatabaseFinder {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> DatabaseFinder {
        let select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let player = cx.new(|cx| InputState::new(window, cx).placeholder("Player"));
        let databases = cx.global::<GameDbHandle>().0.clone();
        let subscriptions = vec![
            cx.subscribe(&select, |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.database = Some(value.to_string());
                    this.load(cx);
                }
            }),
            // Typing keeps the last answer's games until the next one comes, as on the
            // Database page.
            cx.subscribe(&player, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.load(cx);
                }
            }),
            cx.observe_in(&databases, window, |this, _, window, cx| {
                this.sync_databases(window, cx)
            }),
        ];
        let mut finder = DatabaseFinder {
            select,
            listed: Vec::new(),
            database: None,
            player,
            games: Vec::new(),
            loading: Loading::default(),
            opening: None,
            open_epoch: Epoch::default(),
            open_error: None,
            scroll: widgets::NestedScroll::new(),
            _subscriptions: subscriptions,
        };
        finder.sync_databases(window, cx);
        finder
    }

    /// The picker follows the databases listed, keeping the one picked while it is there.
    fn sync_databases(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let listed: Vec<(String, String)> = cx
            .global::<GameDbHandle>()
            .0
            .read(cx)
            .databases()
            .iter()
            .map(|info| (info.id.clone(), info.name.clone()))
            .collect();
        if listed == self.listed {
            return;
        }
        self.listed = listed.clone();
        let chosen = self
            .database
            .clone()
            .filter(|id| listed.iter().any(|(listed, _)| listed == id))
            .or_else(|| listed.first().map(|(id, _)| id.clone()));
        let items: Vec<Choice> = listed
            .iter()
            .map(|(id, name)| Choice::new(id.clone(), name.clone()))
            .collect();
        self.select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            if let Some(id) = &chosen {
                state.set_selected_value(&SharedString::from(id.clone()), window, cx);
            }
        });
        self.database = chosen;
        self.load(cx);
    }

    fn typed(&self, cx: &App) -> Option<String> {
        let typed = self.player.read(cx).value().trim().to_string();
        (!typed.is_empty()).then_some(typed)
    }

    /// The latest games of the database picked, or of the player typed.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.open_epoch.invalidate();
        self.opening = None;
        self.open_error = None;
        let Some(database) = self.database.clone() else {
            self.loading.stop();
            self.games.clear();
            cx.notify();
            return;
        };
        let query = GameDbQuery {
            database,
            player: self.typed(cx),
            limit: LISTED,
            ..GameDbQuery::default()
        };
        let request = cx.global::<AppCore>().request(calls::GameDbGames { query });
        self.loading.load(
            cx,
            "guess",
            |this| &mut this.loading,
            request,
            |this, answer, _| match answer {
                Ok(page) => this.games = page.games,
                Err(_) => this.games.clear(),
            },
        );
        cx.notify();
    }

    /// Fetch game `index`'s moves and guess it as the player typed, else as its winner.
    fn open_game(&mut self, index: usize, cx: &mut Context<Self>) {
        let (Some(database), Some(game)) = (self.database.clone(), self.games.get(index)) else {
            return;
        };
        let side = self.typed(cx).and_then(|typed| side_of(game, &typed));
        let request = cx.global::<AppCore>().request(calls::GameDbGame {
            database: DatabaseId(database),
            game: GameDbGameId(game.id),
        });
        let epoch = self.open_epoch.start();
        self.opening = Some(index);
        self.open_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let answer = request.await;
            this.update(cx, |this, cx| {
                if !this.open_epoch.is_current(epoch) {
                    return;
                }
                this.opening = None;
                match answer {
                    Ok(pgn) => {
                        let side = side.unwrap_or_else(|| winner(&pgn));
                        cx.emit(Found::Game { pgn, side });
                    }
                    Err(cause) => {
                        this.open_error = Some(kchess_app::loading::failure("guess", &cause).into())
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// The side the player typed had in `game`: the one whose name holds it, when only one does.
fn side_of(game: &GameDbRow, typed: &str) -> Option<Color> {
    let typed = typed.to_lowercase();
    let white = game.white.to_lowercase().contains(&typed);
    let black = game.black.to_lowercase().contains(&typed);
    match (white, black) {
        (true, false) => Some(Color::White),
        (false, true) => Some(Color::Black),
        _ => None,
    }
}

impl Render for DatabaseFinder {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.listed.is_empty() {
            return div().into_any_element();
        }
        let palette = *cx.global::<Palette>();
        let narrow = window.viewport_size().width < NARROW;
        let filters = h_flex()
            .gap(px(8.))
            .child(
                div().w(px(220.)).flex_none().child(
                    Select::new(&self.select)
                        .small()
                        .accessibility_label("Database"),
                ),
            )
            .child(
                div().flex_1().min_w_0().max_w(px(320.)).child(
                    Input::new(&self.player)
                        .small()
                        .prefix(icon("search").size(px(14.)))
                        .cleanable(true)
                        .aria_label("Player"),
                ),
            );
        let error = self.loading.error().or(self.open_error.as_ref()).cloned();
        // What the list holds right now: its games, or the line said in their place.
        let body: AnyElement = if let Some(error) = error {
            div()
                .id("guess-db-error")
                .test_support()
                .aria_label(error.clone())
                .relative()
                .px(px(12.))
                .text_size(px(13.))
                .text_color(palette.error)
                .child(widgets::live_region("guess-db-error-spoken", error.clone()))
                .child(error)
                .into_any_element()
        } else if self.loading.busy() && self.games.is_empty() {
            div()
                .px(px(12.))
                .child(widgets::loading("Loading…", &palette))
                .into_any_element()
        } else if self.games.is_empty() {
            div()
                .px(px(12.))
                .child(widgets::hint(
                    match self.typed(cx) {
                        Some(player) => format!("No games by {player}"),
                        None => "No games yet".to_string(),
                    },
                    &palette,
                ))
                .into_any_element()
        } else {
            let finder = cx.entity().downgrade();
            v_flex()
                .children(self.games.iter().enumerate().map(|(index, game)| {
                    let finder = finder.clone();
                    game_db_list::row(
                        ("guess-db-game", index),
                        game,
                        narrow,
                        (self.opening == Some(index))
                            .then(|| widgets::loading("", &palette).into_any_element()),
                        move |_, _, cx| {
                            finder.update(cx, |this, cx| this.open_game(index, cx)).ok();
                        },
                        &palette,
                    )
                }))
                .into_any_element()
        };
        widgets::card(&palette)
            .id("guess-databases")
            .test_support()
            .gap(px(12.))
            .child(widgets::heading("From your databases", &palette))
            .child(filters)
            .child(game_db_list::head(narrow, &palette))
            // One height whatever it holds, so the games arriving (or a search's answer) move
            // nothing under the card.
            .child(
                widgets::nested_scroll(div().id("guess-db-games").h(LIST_HEIGHT), &self.scroll)
                    .test_support()
                    .child(body),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::side_of;
    use kchess_contracts::types::GameDbRow;
    use shakmaty::Color;

    fn row(white: &str, black: &str) -> GameDbRow {
        serde_json::from_value(serde_json::json!({
            "id": 1, "white": white, "black": black, "result": "1-0", "plies": 40,
        }))
        .expect("a database row")
    }

    /// A game found by a player's name is guessed as them, whichever side they had; a name both
    /// players' hold, or neither's, leaves it to the result.
    #[test]
    fn a_players_game_is_guessed_as_them() {
        assert_eq!(
            side_of(&row("Carlsen, Magnus", "Nepomniachtchi, Ian"), "carlsen"),
            Some(Color::White)
        );
        assert_eq!(
            side_of(&row("Nepomniachtchi, Ian", "Carlsen, Magnus"), "Magnus"),
            Some(Color::Black)
        );
        assert_eq!(
            side_of(&row("Polgar, Judit", "Polgar, Susan"), "polgar"),
            None
        );
        assert_eq!(side_of(&row("Anand, V", "Kramnik, V"), "topalov"), None);
    }
}
