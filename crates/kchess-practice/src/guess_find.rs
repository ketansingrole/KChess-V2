//! Finding a game to guess through without bringing one: a Lichess player's recent games, looked
//! up as on Players, or a famous game.

use crate::loading::Epoch;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::{GameId, Username};
use kchess_contracts::types::{GameSite, LichessGame};
use shakmaty::Color;

use super::classics::{CLASSICS, winner};
use crate::core::AppCore;
use crate::player_search::{self, NameGroup, NameMatches};
use crate::theme::Palette;
use crate::widgets;
use kchess_history::lichess::{RowDetail, game_row};
use kchess_ui::icon;

/// A game picked to guess through, as the side to guess.
pub enum Found {
    Game { pgn: String, side: Color },
}

pub struct GameFinder {
    input: Entity<InputState>,
    /// Players whose names start with what is typed.
    names: NameMatches,
    /// The player whose games are shown, and their games once Lichess sends them.
    player: Option<String>,
    games: Option<Vec<LichessGame>>,
    epoch: Epoch,
    /// The row of the game being fetched.
    opening: Option<usize>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Found> for GameFinder {}

impl GameFinder {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> GameFinder {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Lichess username"));
        let subscriptions = vec![cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => this.search_names(cx),
                InputEvent::PressEnter { .. } => this.show_games(None, cx),
                _ => {}
            },
        )];
        GameFinder {
            input,
            names: NameMatches::default(),
            player: None,
            games: None,
            epoch: Epoch::default(),
            opening: None,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    /// Players whose names start with what is typed, once typing settles.
    fn search_names(&mut self, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).value().to_string();
        let shown = self.player.clone();
        self.names.search(
            &typed,
            shown.as_deref(),
            |finder: &mut Self| &mut finder.names,
            cx,
        );
    }

    /// A player's recent games: `name`, or the one typed.
    fn show_games(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        let name = name.unwrap_or_else(|| self.input.read(cx).value().trim().to_string());
        if !crate::game::is_username(&name) {
            self.error = Some(if name.is_empty() {
                "Enter a Lichess username".into()
            } else {
                "Lichess usernames are 2–30 letters, digits, - or _".into()
            });
            cx.notify();
            return;
        }
        let epoch = self.epoch.start();
        self.names.clear();
        self.player = Some(name.clone());
        self.games = None;
        self.opening = None;
        self.error = None;
        cx.notify();
        let games = cx.global::<AppCore>().request(calls::RecentGames {
            username: Username(name.clone()),
            rated: None,
        });
        cx.spawn(async move |this, cx| {
            let answer = games.await;
            let _ = this.update(cx, |this, cx| {
                if !this.epoch.is_current(epoch) {
                    return;
                }
                match answer {
                    Ok(games) => {
                        // Named as Lichess spells it, among the players looked up lately.
                        if let Some(game) = games.first() {
                            this.player = Some(game.account.clone());
                        }
                        if let Some(player) = this.player.clone() {
                            player_search::remember(GameSite::Lichess, &player, cx);
                        }
                        this.games = Some(games);
                    }
                    Err(cause) => {
                        this.player = None;
                        this.error = Some(kchess_app::loading::failure("guess", &cause).into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Show a picked player's games, their name in the field.
    fn pick(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value(name.clone(), window, cx));
        self.show_games(Some(name), cx);
    }

    /// Fetch one of the player's games and guess it as them.
    fn open_game(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(game) = self.games.as_ref().and_then(|games| games.get(index)) else {
            return;
        };
        let side = game.color;
        let id = game.id.clone();
        let epoch = self.epoch.current();
        self.opening = Some(index);
        self.error = None;
        cx.notify();
        let export = cx
            .global::<AppCore>()
            .request(calls::ExportGame { id: GameId(id) });
        cx.spawn(async move |this, cx| {
            let answer = export.await;
            let _ = this.update(cx, |this, cx| {
                if !this.epoch.is_current(epoch) || this.opening != Some(index) {
                    return;
                }
                this.opening = None;
                match answer {
                    Ok(pgn) => cx.emit(Found::Game { pgn, side }),
                    Err(cause) => {
                        this.error = Some(kchess_app::loading::failure("guess", &cause).into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn player_card(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let online = crate::network::online(cx);
        let typed = !self.input.read(cx).value().trim().is_empty();
        // What is typed matches these players; before that, the ones looked up lately.
        let finder = cx.entity().downgrade();
        let suggestions = player_search::name_rows(
            "guess-player-match",
            &self.names,
            vec![NameGroup::new(
                "Recent",
                "guess-recent-player",
                player_search::recent(GameSite::Lichess, cx),
            )],
            self.player.as_deref(),
            palette,
            |name| {
                let finder = finder.clone();
                Box::new(move |_, window, cx| {
                    let _ = finder.update(cx, |finder, cx| finder.pick(name.clone(), window, cx));
                })
            },
        );
        let finder = cx.entity().downgrade();
        let games: Option<Div> = self.player.as_ref().map(|player| {
            let list = v_flex().gap(px(2.));
            match &self.games {
                None => list.child(widgets::loading("Loading…", palette)),
                Some(games) if games.is_empty() => {
                    list.child(widgets::hint(format!("No games by {player} yet"), palette))
                }
                Some(games) => list.children(games.iter().enumerate().map(|(index, game)| {
                    let finder = finder.clone();
                    let row = game_row(
                        ("guess-player-game", index),
                        game,
                        None,
                        RowDetail::Brief,
                        move |_, cx| {
                            let _ = finder.update(cx, |this, cx| this.open_game(index, cx));
                        },
                        palette,
                    );
                    if self.opening == Some(index) {
                        h_flex()
                            .child(div().flex_1().min_w_0().child(row))
                            .child(widgets::loading("", palette))
                            .into_any_element()
                    } else {
                        row
                    }
                })),
            }
        });
        widgets::card(palette)
            .gap(px(12.))
            .child(widgets::heading("A player's games", palette))
            .child(
                h_flex()
                    .w_full()
                    .gap(px(8.))
                    .child(
                        div().flex_1().min_w(px(192.)).max_w(px(480.)).child(
                            Input::new(&self.input)
                                .prefix(icon("user-search").size(px(16.)))
                                .cleanable(true)
                                .aria_label("Lichess username"),
                        ),
                    )
                    .child(
                        Button::new("guess-player-games")
                            .primary()
                            .icon(icon("search"))
                            .label("Show games")
                            .loading(self.player.is_some() && self.games.is_none())
                            .disabled(!online || !typed)
                            .on_click(cx.listener(|this, _, _, cx| this.show_games(None, cx))),
                    ),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .id("guess-find-error")
                    .test_support()
                    .aria_label(error.clone())
                    .relative()
                    .text_size(px(13.))
                    .text_color(palette.error)
                    .child(widgets::live_region(
                        "guess-find-error-spoken",
                        error.clone(),
                    ))
                    .child(error)
            }))
            .children(suggestions)
            .children(games)
    }

    fn classics_card(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let chips = CLASSICS.iter().enumerate().map(|(index, classic)| {
            player_search::name_chip(
                ("guess-classic", index),
                classic.name,
                None,
                false,
                palette,
                cx.listener(move |_, _, _, cx| {
                    let pgn = CLASSICS[index].pgn;
                    cx.emit(Found::Game {
                        pgn: pgn.to_string(),
                        side: winner(pgn),
                    });
                }),
            )
            .into_any_element()
        });
        widgets::card(palette)
            .gap(px(12.))
            .child(widgets::heading("Famous games", palette))
            .child(h_flex().flex_wrap().gap(px(6.)).children(chips))
    }
}

impl Render for GameFinder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        v_flex()
            .gap(px(16.))
            .child(self.player_card(cx, &palette))
            .child(self.classics_card(cx, &palette))
    }
}

#[cfg(test)]
mod tests {
    use super::GameFinder;
    use gpui_kit::AppContext as _;
    use kchess_contracts::types::LichessGame;

    fn lichess_game() -> LichessGame {
        serde_json::from_value(serde_json::json!({
            "id": "g", "account": "alice", "createdAt": 0, "lastMoveAt": 0, "rated": true,
            "speed": "blitz", "perf": "blitz", "status": "mate", "color": "white",
            "opponent": "bob", "moves": "",
        }))
        .expect("a Lichess game")
    }

    fn started(
        cx: &mut gpui_kit::TestAppContext,
    ) -> (
        kchess_board::testing::Started,
        kchess_testkit::Server,
        gpui_kit::Entity<GameFinder>,
    ) {
        // Lichess, refusing everything.
        let server = kchess_testkit::Server::start(|_, out| {
            kchess_testkit::respond(out, 500, "text/plain", b"")
        })
        .expect("a stand-in Lichess");
        let mut app =
            kchess_board::testing::Started::with_lichess(cx, Some(server.base().to_string()));
        let finder = app.with_window(|window, cx| cx.new(|cx| GameFinder::new(window, cx)));
        (app, server, finder)
    }

    /// A player's games that Lichess will not send are logged and shown as the failure, and the
    /// player is forgotten.
    #[gpui_kit::test]
    fn refused_games_of_a_player_are_logged_and_shown(cx: &mut gpui_kit::TestAppContext) {
        let (mut app, _server, finder) = started(cx);
        let before = kchess_app::tests::logged_in("guess");
        app.with_window(|_, cx| {
            finder.update(cx, |finder, cx| {
                finder.show_games(Some("alice".into()), cx);
                assert_eq!(finder.player.as_deref(), Some("alice"), "asked for");
            })
        });
        app.wait_for("the failure logged", |_, _| {
            kchess_app::tests::logged_in("guess") > before
        });
        app.wait_for("the failure shown", |_, cx| finder.read(cx).error.is_some());
        let player = app.with_window(|_, cx| finder.read(cx).player.clone());
        assert_eq!(player, None);
    }

    /// A game Lichess will not export is logged and shown as the failure, and the row stops
    /// loading.
    #[gpui_kit::test]
    fn a_refused_game_is_logged_and_shown(cx: &mut gpui_kit::TestAppContext) {
        let (mut app, _server, finder) = started(cx);
        let before = kchess_app::tests::logged_in("guess");
        app.with_window(|_, cx| {
            finder.update(cx, |finder, cx| {
                finder.games = Some(vec![lichess_game()]);
                finder.open_game(0, cx);
                assert_eq!(finder.opening, Some(0), "asked for");
            })
        });
        app.wait_for("the failure logged", |_, _| {
            kchess_app::tests::logged_in("guess") > before
        });
        app.wait_for("the failure shown", |_, cx| finder.read(cx).error.is_some());
        let opening = app.with_window(|_, cx| finder.read(cx).opening);
        assert_eq!(opening, None);
    }
}
