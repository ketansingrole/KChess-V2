//! Finding a game to guess through without bringing one: a Lichess player's recent games, looked
//! up as on Players, or a famous game.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::Username;
use kchess_contracts::types::{LichessGame, PlayerSuggestion};
use serde_json::{Value, json};
use shakmaty::Color;

use super::classics::{CLASSICS, winner};
use crate::assets::icon;
use crate::core::AppCore;
use crate::pages::history::lichess::{RowDetail, game_row};
use crate::pages::players;
use crate::theme::Palette;
use crate::widgets;

/// A game picked to guess through, as the side to guess.
pub enum Found {
    Game { pgn: String, side: Color },
}

pub struct GameFinder {
    input: Entity<InputState>,
    /// Players whose names start with what is typed.
    matches: Vec<PlayerSuggestion>,
    search_epoch: usize,
    /// The player whose games are shown, and their games once Lichess sends them.
    player: Option<String>,
    games: Option<Vec<LichessGame>>,
    epoch: usize,
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
            matches: Vec::new(),
            search_epoch: 0,
            player: None,
            games: None,
            epoch: 0,
            opening: None,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    /// Players whose names start with what is typed, once typing settles.
    fn search_names(&mut self, cx: &mut Context<Self>) {
        self.search_epoch += 1;
        let epoch = self.search_epoch;
        let term = self.input.read(cx).value().trim().to_string();
        let shown = self
            .player
            .as_ref()
            .is_some_and(|player| player.eq_ignore_ascii_case(&term));
        if term.chars().count() < 3
            || !crate::game::is_username(&term)
            || shown
            || !crate::network::online(cx)
        {
            self.matches.clear();
            cx.notify();
            return;
        }
        let core = cx.global::<AppCore>().clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(players::SEARCH_WAIT).await;
            let current = this
                .read_with(cx, |this, _| this.search_epoch == epoch)
                .unwrap_or(false);
            if !current {
                return;
            }
            let found = core
                .request(calls::SearchPlayers {
                    term: Username(term),
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.search_epoch != epoch {
                    return;
                }
                if let Ok(found) = found {
                    this.matches = found;
                    this.matches.truncate(players::SUGGESTED);
                }
                cx.notify();
            });
        })
        .detach();
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
        self.epoch += 1;
        let epoch = self.epoch;
        self.search_epoch += 1;
        self.matches.clear();
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
                if this.epoch != epoch {
                    return;
                }
                match answer {
                    Ok(games) => {
                        // Named as Lichess spells it, among the players looked up lately.
                        if let Some(game) = games.first() {
                            this.player = Some(game.account.clone());
                        }
                        if let Some(player) = this.player.clone() {
                            players::remember(&player, cx);
                        }
                        this.games = Some(games);
                    }
                    Err(cause) => {
                        this.player = None;
                        this.error = Some(cause.message.into());
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
        let epoch = self.epoch;
        self.opening = Some(index);
        self.error = None;
        cx.notify();
        let export = cx
            .global::<AppCore>()
            .request(calls::ExportGame { id: json!(id) });
        cx.spawn(async move |this, cx| {
            let answer = export.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch || this.opening != Some(index) {
                    return;
                }
                this.opening = None;
                match answer {
                    Ok(Value::String(pgn)) => cx.emit(Found::Game { pgn, side }),
                    Ok(_) => this.error = Some("That game could not be read".into()),
                    Err(cause) => this.error = Some(cause.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn player_card(&self, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let online = crate::network::online(cx);
        let typed = !self.input.read(cx).value().trim().is_empty();
        let pick = |name: String| {
            cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| {
                this.pick(name.clone(), window, cx)
            })
        };
        let chips = |id: &'static str, names: Vec<(String, Option<String>, bool)>| {
            names
                .into_iter()
                .enumerate()
                .map(|(index, (name, title, online))| {
                    players::name_chip(
                        (id, index),
                        &name,
                        title.as_deref(),
                        online,
                        palette,
                        pick(name.clone()),
                    )
                    .into_any_element()
                })
                .collect::<Vec<_>>()
        };
        // What is typed matches these players; before that, the ones looked up lately.
        let suggestions = if self.matches.is_empty() {
            let shown = self.player.clone().unwrap_or_default().to_lowercase();
            let recent: Vec<_> =
                crate::prefs::get::<Vec<String>>(cx, players::RECENT_KEY, Vec::new())
                    .into_iter()
                    .filter(|name| name.to_lowercase() != shown)
                    .map(|name| (name, None, false))
                    .collect();
            (!recent.is_empty()).then(|| {
                players::chip_row(
                    "Recent",
                    chips("guess-recent-player", recent),
                    None,
                    palette,
                )
            })
        } else {
            let found = self
                .matches
                .iter()
                .map(|found| {
                    (
                        found.username.clone(),
                        found.title.clone(),
                        found.online == Some(true),
                    )
                })
                .collect();
            Some(players::chip_row(
                "Matches",
                chips("guess-player-match", found),
                None,
                palette,
            ))
        };
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
            players::name_chip(
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
