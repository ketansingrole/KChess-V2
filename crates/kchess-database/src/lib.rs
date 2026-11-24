//! The Database page: the games of a game database on this computer, searched by player, side,
//! event, years, rating, result and position, a page at a time, newest, oldest or best rated
//! first; each opened on the analysis board. PGN files are imported here (picked or dropped),
//! and a database deleted.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

use kchess_app::{core, loading, prefs};
use kchess_board::{game_db, widgets};
use kchess_ui::theme;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_board::analysis::{Open, Opening};
use kchess_board::game::played::GameOrigin;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, GameDbGameId};
use kchess_contracts::types::{
    DatabaseInfo, GameDbQuery, GameDbResult, GameDbRow, GameDbSide, GameDbSort,
};
use kchess_nav::page::{ActionInfo, Page, PageAction, PageView};
use kchess_nav::play_layout::{PAGE_BOTTOM, PAGE_TOP, side_padding};
use kchess_ui::icon;
use serde::{Deserialize, Serialize};

use crate::core::AppCore;
use crate::game_db::GameDbHandle;
use crate::loading::Loading;
use crate::theme::{Palette, radii};
use crate::widgets::{Choice, KeyboardFocus as _, Pill, Pressable as _};

/// What the page keeps between runs: the database shown, the order and the page size.
const PREFS: &str = "kchess:database";
const PAGE_SIZES: [usize; 4] = [20, 50, 100, 200];
const SIDES: [(&str, &str); 3] = [
    ("either", "Either side"),
    ("white", "As White"),
    ("black", "As Black"),
];
const RATINGS: [(&str, &str); 6] = [
    ("0", "Any rating"),
    ("2000", "Both 2000+"),
    ("2200", "Both 2200+"),
    ("2400", "Both 2400+"),
    ("2600", "Both 2600+"),
    ("2700", "Both 2700+"),
];
const SORTS: [(&str, &str); 3] = [
    ("newest", "Newest first"),
    ("oldest", "Oldest first"),
    ("rating", "Best rated first"),
];
const RESULTS: [&str; 4] = ["All", "White won", "Draws", "Black won"];
/// Below this width the table keeps the players and the result.
const NARROW: Pixels = px(720.);

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
struct Prefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_size: Option<usize>,
}

fn stored(cx: &App) -> Prefs {
    prefs::get(cx, PREFS, Prefs::default())
}

fn store(cx: &mut App, change: impl FnOnce(&mut Prefs)) {
    let mut kept = stored(cx);
    change(&mut kept);
    prefs::keep(cx, PREFS, kept);
}

pub struct DatabasePage {
    focus: FocusHandle,
    /// The database shown; the first listed when the one kept is gone.
    database: Option<String>,
    /// The databases the picker lists (id, name), to fill it only when they change.
    listed: Vec<(String, String)>,
    database_select: Entity<SelectState<Vec<Choice>>>,
    player_input: Entity<InputState>,
    side_select: Entity<SelectState<Vec<Choice>>>,
    event_input: Entity<InputState>,
    from_input: Entity<InputState>,
    to_input: Entity<InputState>,
    rating_select: Entity<SelectState<Vec<Choice>>>,
    sort_select: Entity<SelectState<Vec<Choice>>>,
    size_select: Entity<SelectState<Vec<Choice>>>,
    side: GameDbSide,
    min_elo: Option<u32>,
    sort: GameDbSort,
    result: Option<GameDbResult>,
    /// Only the games that reached this position (a FEN), from the analysis board.
    position: Option<String>,
    page: usize,
    page_size: usize,
    games: Vec<GameDbRow>,
    total: usize,
    /// `gameDbGames`'s load: answers to an earlier page or filter are dropped.
    loading: Loading,
    _subscriptions: Vec<Subscription>,
}

fn choices(items: &[(&str, &str)]) -> Vec<Choice> {
    items
        .iter()
        .map(|(value, label)| Choice::new(*value, *label))
        .collect()
}

fn sort_of(key: &str) -> GameDbSort {
    match key {
        "oldest" => GameDbSort::Oldest,
        "rating" => GameDbSort::Rating,
        _ => GameDbSort::Newest,
    }
}

fn sort_key(sort: GameDbSort) -> &'static str {
    match sort {
        GameDbSort::Oldest => "oldest",
        GameDbSort::Rating => "rating",
        GameDbSort::Newest => "newest",
    }
}

/// A year typed in a year field; anything else is no year.
fn year(text: &str) -> Option<i32> {
    let text = text.trim();
    (text.len() == 4)
        .then(|| text.parse::<i32>().ok())
        .flatten()
}

impl DatabasePage {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let kept = stored(cx);
        let sort = sort_of(kept.sort.as_deref().unwrap_or("newest"));
        let page_size = kept
            .page_size
            .filter(|size| PAGE_SIZES.contains(size))
            .unwrap_or(PAGE_SIZES[0]);
        let database_select = cx.new(|cx| SelectState::new(Vec::new(), None, window, cx));
        let side_select = cx.new(|cx| SelectState::new(choices(&SIDES), None, window, cx));
        let rating_select = cx.new(|cx| SelectState::new(choices(&RATINGS), None, window, cx));
        let sort_select = cx.new(|cx| SelectState::new(choices(&SORTS), None, window, cx));
        let sizes: Vec<Choice> = PAGE_SIZES
            .iter()
            .map(|size| Choice::new(size.to_string(), format!("{size} per page")))
            .collect();
        let size_select = cx.new(|cx| SelectState::new(sizes, None, window, cx));
        for (select, value) in [
            (&side_select, "either".to_string()),
            (&rating_select, "0".to_string()),
            (&sort_select, sort_key(sort).to_string()),
            (&size_select, page_size.to_string()),
        ] {
            select.update(cx, |state, cx| {
                state.set_selected_value(&SharedString::from(value), window, cx)
            });
        }
        let field = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let player_input = field("Player", window, cx);
        let event_input = field("Event", window, cx);
        let from_input = field("From year", window, cx);
        let to_input = field("To year", window, cx);
        let mut subscriptions = vec![
            cx.subscribe(
                &database_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.choose_database(value.to_string(), cx);
                    }
                },
            ),
            cx.subscribe(
                &side_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.side = match value.as_ref() {
                            "white" => GameDbSide::White,
                            "black" => GameDbSide::Black,
                            _ => GameDbSide::Either,
                        };
                        this.reload(cx);
                    }
                },
            ),
            cx.subscribe(
                &rating_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.min_elo = value.parse::<u32>().ok().filter(|elo| *elo > 0);
                        this.reload(cx);
                    }
                },
            ),
            cx.subscribe(
                &sort_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.sort = sort_of(value);
                        let key = sort_key(this.sort);
                        store(cx, |kept| kept.sort = Some(key.to_string()));
                        this.reload(cx);
                    }
                },
            ),
            cx.subscribe(
                &size_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event
                        && let Ok(size) = value.parse::<usize>()
                    {
                        this.page_size = size;
                        store(cx, |kept| kept.page_size = Some(size));
                        this.reload(cx);
                    }
                },
            ),
        ];
        for input in [&player_input, &event_input, &from_input, &to_input] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.reload(cx);
                }
            }));
        }
        let databases = cx.global::<GameDbHandle>().0.clone();
        subscriptions.push(cx.observe_in(&databases, window, |this, _, window, cx| {
            this.sync_databases(window, cx)
        }));
        let mut page = DatabasePage {
            focus: cx.focus_handle(),
            database: kept.database,
            listed: Vec::new(),
            database_select,
            player_input,
            side_select,
            event_input,
            from_input,
            to_input,
            rating_select,
            sort_select,
            size_select,
            side: GameDbSide::Either,
            min_elo: None,
            sort,
            result: None,
            position: None,
            page: 0,
            page_size,
            games: Vec::new(),
            total: 0,
            loading: Loading::default(),
            _subscriptions: subscriptions,
        };
        page.sync_databases(window, cx);
        page
    }

    fn databases<'a>(&self, cx: &'a App) -> &'a [DatabaseInfo] {
        cx.global::<GameDbHandle>().0.read(cx).databases()
    }

    fn shown_database<'a>(&self, cx: &'a App) -> Option<&'a DatabaseInfo> {
        let id = self.database.as_deref()?;
        self.databases(cx).iter().find(|info| info.id == id)
    }

    /// The picker follows the databases listed: one imported, renamed or deleted elsewhere
    /// joins, changes or leaves it, and a database gone is replaced by the first one left.
    fn sync_databases(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let listed: Vec<(String, String)> = self
            .databases(cx)
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
        self.database_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            if let Some(id) = &chosen {
                state.set_selected_value(&SharedString::from(id.clone()), window, cx);
            }
        });
        // A database's count changed (an import added to it): its pages are read again too.
        self.database = chosen;
        self.reload(cx);
    }

    fn choose_database(&mut self, id: String, cx: &mut Context<Self>) {
        if self.database.as_deref() == Some(id.as_str()) {
            return;
        }
        let kept = id.clone();
        store(cx, |prefs| prefs.database = Some(kept));
        self.database = Some(id);
        self.reload(cx);
    }

    fn text(input: &Entity<InputState>, cx: &App) -> Option<String> {
        let value = input.read(cx).value().trim().to_string();
        (!value.is_empty()).then_some(value)
    }

    /// The search the filters make of database `database`, for the page shown.
    fn query(&self, database: String, cx: &App) -> GameDbQuery {
        let player = Self::text(&self.player_input, cx);
        GameDbQuery {
            database,
            side: player.as_ref().map(|_| self.side),
            player,
            event: Self::text(&self.event_input, cx),
            year_from: Self::text(&self.from_input, cx).and_then(|text| year(&text)),
            year_to: Self::text(&self.to_input, cx).and_then(|text| year(&text)),
            min_elo: self.min_elo,
            result: self.result,
            position: self.position.clone(),
            sort: Some(self.sort),
            offset: u32::try_from(self.page * self.page_size).unwrap_or(u32::MAX),
            limit: u32::try_from(self.page_size).unwrap_or(20),
            ..GameDbQuery::default()
        }
    }

    fn filtered(&self, cx: &App) -> bool {
        [
            &self.player_input,
            &self.event_input,
            &self.from_input,
            &self.to_input,
        ]
        .iter()
        .any(|input| Self::text(input, cx).is_some())
            || self.min_elo.is_some()
            || self.result.is_some()
            || self.position.is_some()
    }

    /// The first page of what the filters now match.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.page = 0;
        self.load(cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(database) = self.database.clone() else {
            self.loading.stop();
            self.games.clear();
            self.total = 0;
            cx.notify();
            return;
        };
        let query = self.query(database, cx);
        let request = cx.global::<AppCore>().request(calls::GameDbGames { query });
        self.loading.load(
            cx,
            "database",
            |this| &mut this.loading,
            request,
            |this, answer, cx| match answer {
                Ok(page) => {
                    this.total = usize::try_from(page.total).unwrap_or(usize::MAX);
                    this.games = page.games;
                    let pages = this.total.div_ceil(this.page_size).max(1);
                    if this.page >= pages {
                        this.page = pages - 1;
                        this.load(cx);
                    }
                }
                Err(_) => {
                    this.games.clear();
                    this.total = 0;
                }
            },
        );
    }

    fn choose_result(&mut self, index: usize, cx: &mut Context<Self>) {
        self.result = match index {
            1 => Some(GameDbResult::White),
            2 => Some(GameDbResult::Draw),
            3 => Some(GameDbResult::Black),
            _ => None,
        };
        self.reload(cx);
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            &self.player_input,
            &self.event_input,
            &self.from_input,
            &self.to_input,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        for (select, value) in [(&self.side_select, "either"), (&self.rating_select, "0")] {
            select.update(cx, |state, cx| {
                state.set_selected_value(&SharedString::new_static(value), window, cx)
            });
        }
        self.side = GameDbSide::Either;
        self.min_elo = None;
        self.result = None;
        self.position = None;
        self.reload(cx);
    }

    /// The games that reached `fen`, every other filter cleared.
    pub fn show_position(&mut self, fen: String, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_filters(window, cx);
        self.position = Some(fen);
        self.reload(cx);
    }

    fn open_game(&mut self, row: &GameDbRow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(database) = self.database.clone() else {
            return;
        };
        let request = cx.global::<AppCore>().request(calls::GameDbGame {
            database: DatabaseId(database),
            game: GameDbGameId(row.id),
        });
        let origin = GameOrigin {
            white: Some(row.white.clone()),
            black: Some(row.black.clone()),
            ..GameOrigin::default()
        };
        cx.spawn_in(window, async move |_, cx| {
            let pgn = request.await;
            cx.update(|window, cx| match pgn {
                Ok(pgn) => kchess_board::analysis::open(
                    Open::Pgn {
                        pgn,
                        opening: Opening {
                            origin: Some(origin),
                            ..Opening::default()
                        },
                        unreadable: false,
                    },
                    window,
                    cx,
                ),
                Err(cause) => window.push_notification(
                    Notification::error(loading::failure("database", &cause))
                        .title("Could not open the game"),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
    }

    /// Asks, then deletes the database shown.
    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.shown_database(cx).cloned() else {
            return;
        };
        let games = widgets::format_counted(info.games as f64, "game", "games");
        widgets::open_alert_dialog(window, cx, move |dialog: AlertDialog, _, _| {
            let id = info.id.clone();
            dialog
                .title(format!("Delete {}?", info.name))
                .description(format!("Its {games} are removed from this computer."))
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let db = cx.global::<GameDbHandle>().0.clone();
                    db.update(cx, |db, cx| db.delete(id.clone(), cx));
                    true
                })
        });
    }

    fn render_toolbar(&self, cx: &mut Context<Self>, palette: &Palette) -> impl IntoElement {
        let player = Self::text(&self.player_input, cx).is_some();
        let chosen = match self.result {
            None => 0,
            Some(GameDbResult::White) => 1,
            Some(GameDbResult::Draw) => 2,
            Some(GameDbResult::Black) => 3,
        };
        let results = widgets::pills(
            "database-result",
            RESULTS
                .iter()
                .map(|label| Pill {
                    icon: None,
                    label: SharedString::new_static(label),
                    badge: None,
                    disabled: false,
                })
                .collect(),
            chosen,
            {
                let page = cx.entity().downgrade();
                move |index, _, cx| {
                    page.update(cx, |this, cx| this.choose_result(index, cx))
                        .ok();
                }
            },
            true,
            palette,
        );
        let field = |input: &Entity<InputState>, width: f32, label: &'static str| {
            div()
                .w(px(width))
                .child(Input::new(input).small().cleanable(true).aria_label(label))
        };
        let select = |state: &Entity<SelectState<Vec<Choice>>>, width: f32, label: &'static str| {
            div()
                .w(px(width))
                .child(Select::new(state).small().accessibility_label(label))
        };
        h_flex()
            .flex_wrap()
            .gap(px(8.))
            .child(
                div().w(px(180.)).child(
                    Input::new(&self.player_input)
                        .small()
                        .cleanable(true)
                        .prefix(icon("search").size(px(14.)))
                        .aria_label("Player"),
                ),
            )
            .when(player, |row| {
                row.child(select(&self.side_select, 130., "Player's side"))
            })
            .child(field(&self.event_input, 160., "Event"))
            .child(field(&self.from_input, 96., "From year"))
            .child(field(&self.to_input, 96., "To year"))
            .child(select(&self.rating_select, 140., "Rating"))
            .child(results)
            .child(select(&self.sort_select, 160., "Order"))
            .when(self.filtered(cx), |row| {
                row.child(
                    Button::new("database-clear")
                        .ghost()
                        .small()
                        .icon(icon("x"))
                        .label("Clear filters")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.clear_filters(window, cx)),
                        ),
                )
            })
    }

    fn render_rows(&self, narrow: bool, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let muted = palette.text_muted;
        let column = |width: f32| div().w(px(width)).flex_none().truncate();
        let head = h_flex()
            .px(px(12.))
            .pb(px(8.))
            .gap(px(12.))
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(muted)
            .child(div().flex_1().child("WHITE"))
            .child(div().flex_1().child("BLACK"))
            .child(column(64.).child("RESULT"))
            .when(!narrow, |head| {
                head.child(div().flex_1().child("EVENT"))
                    .child(column(84.).child("DATE"))
                    .child(column(44.).child("ECO"))
                    .child(column(52.).text_right().child("MOVES"))
            });
        let player = |name: &str, elo: Option<u32>| {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.))
                .items_baseline()
                .child(div().truncate().child(name.to_string()))
                .children(elo.map(|elo| {
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(muted)
                        .child(elo.to_string())
                }))
        };
        let rows: Vec<AnyElement> = self
            .games
            .iter()
            .enumerate()
            .map(|(index, game)| {
                let chosen = game.clone();
                let label = format!("{} against {}, {}", game.white, game.black, game.result);
                h_flex()
                    .id(("database-game", index))
                    .test_support()
                    .role(Role::Button)
                    .keyboard_focus(palette.primary)
                    .aria_label(label)
                    .px(px(12.))
                    .py(px(8.))
                    .gap(px(12.))
                    .rounded(radii::BOX)
                    .text_size(px(13.))
                    .cursor_pointer()
                    .pressable(palette)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_game(&chosen, window, cx)),
                    )
                    .child(player(&game.white, game.white_elo))
                    .child(player(&game.black, game.black_elo))
                    .child(
                        column(64.)
                            .font_weight(FontWeight::MEDIUM)
                            .child(game.result.clone()),
                    )
                    .when(!narrow, |row| {
                        row.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(muted)
                                .child(game.event.clone().unwrap_or_default()),
                        )
                        .child(
                            column(84.)
                                .text_color(muted)
                                .child(game.date.clone().unwrap_or_default()),
                        )
                        .child(
                            column(44.)
                                .text_color(muted)
                                .child(game.eco.clone().unwrap_or_default()),
                        )
                        .child(
                            column(52.)
                                .text_right()
                                .text_color(muted)
                                .child(game.plies.div_ceil(2).to_string()),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        v_flex().child(head).children(rows).into_any_element()
    }
}

impl Focusable for DatabasePage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DatabasePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let narrow = window.viewport_size().width < NARROW;
        let databases = self.databases(cx).len();
        let busy = cx.global::<GameDbHandle>().0.read(cx).busy();
        let header = h_flex()
            .gap(px(12.))
            .flex_wrap()
            .when(databases > 0, |row| {
                row.child(
                    div().w(px(240.)).child(
                        Select::new(&self.database_select)
                            .small()
                            .accessibility_label("Database"),
                    ),
                )
                .children(self.shown_database(cx).map(|info| {
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(widgets::format_counted(info.games as f64, "game", "games"))
                }))
            })
            .child(div().flex_1())
            .when(databases > 0 && !busy, |row| {
                row.child(
                    Button::new("database-delete")
                        .ghost()
                        .small()
                        .icon(icon("trash-2"))
                        .label("Delete database")
                        .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                )
            })
            .child(game_db::import_card(cx, &palette));
        let body = if databases == 0 {
            widgets::card(&palette)
                .py(px(30.))
                .child(widgets::empty_state(
                    icon("database"),
                    "No game databases",
                    Some("Import a PGN file, or drop one here".into()),
                    &palette,
                ))
                .into_any_element()
        } else {
            let first_page = self.loading.busy() && self.games.is_empty();
            let content = if first_page {
                div()
                    .p(px(24.))
                    .child(widgets::loading("Loading games…", &palette))
                    .into_any_element()
            } else if let Some(error) = self.loading.error() {
                div()
                    .p(px(24.))
                    .text_color(palette.error)
                    .child(error.clone())
                    .into_any_element()
            } else if self.games.is_empty() {
                widgets::empty_state(
                    icon("search-x"),
                    "No games match these filters",
                    None,
                    &palette,
                )
                .into_any_element()
            } else {
                self.render_rows(narrow, cx, &palette)
            };
            let paged = !first_page && self.loading.error().is_none() && self.total > 0;
            let pages = self.total.div_ceil(self.page_size).max(1);
            let pager = div().pt(px(12.)).child(
                widgets::pager(
                    "database",
                    self.page,
                    pages,
                    cx.listener(|this, page: &usize, _, cx| {
                        this.page = *page;
                        this.load(cx);
                    }),
                )
                .range(self.page_size, self.total)
                .page_size(&self.size_select),
            );
            let position = self.position.clone().map(|fen| {
                h_flex()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(icon("crosshair").size(px(14.)))
                    .child(
                        div()
                            .truncate()
                            .child(format!("Games with the position {fen}")),
                    )
                    .child(
                        Button::new("database-position-clear")
                            .ghost()
                            .small()
                            .icon(icon("x"))
                            .accessibility_label("Any position")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.position = None;
                                this.reload(cx);
                            })),
                    )
            });
            let attribution = self
                .shown_database(cx)
                .and_then(|info| info.attribution.clone())
                .map(|text| {
                    div()
                        .text_size(px(11.))
                        .text_color(palette.text_muted)
                        .child(text)
                });
            v_flex()
                .gap(px(12.))
                .child(self.render_toolbar(cx, &palette))
                .children(position)
                .child(
                    widgets::card(&palette)
                        .p(px(12.))
                        .child(content)
                        .when(paged, |card| card.child(pager)),
                )
                .children(attribution)
                .into_any_element()
        };
        div()
            .id("database-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap(px(16.))
                    .pt(PAGE_TOP)
                    .pb(PAGE_BOTTOM)
                    .px(side_padding(window))
                    .child(header)
                    .child(body),
            )
    }
}

/// What the page offers to Quick Pick and the menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Import,
    ClearFilters,
}

impl PageAction for Action {
    fn info(&self) -> ActionInfo {
        match self {
            Action::Import => ActionInfo::new(
                "database:import",
                "Import PGN",
                "import",
                "pgn file games database",
            ),
            Action::ClearFilters => {
                ActionInfo::new("database:clear", "Clear filters", "x", "reset all")
            }
        }
    }
}

impl PageView for DatabasePage {
    const INFO: kchess_nav::page::PageInfo = kchess_nav::page::PageInfo {
        page: Page::Database,
        label: "Game database",
        short: "Database",
        icon: "database",
        group: Some(kchess_nav::page::NavGroup::Review),
        keywords: "games search pgn import masters chessbase",
    };

    type Action = Action;

    fn create(window: &mut Window, cx: &mut Context<Self>) -> Self {
        DatabasePage::new(window, cx)
    }

    fn search_fields(&self) -> Vec<Entity<InputState>> {
        vec![self.player_input.clone()]
    }

    fn actions(&self, cx: &App) -> Vec<Action> {
        let mut actions = Vec::new();
        if !cx.global::<GameDbHandle>().0.read(cx).busy() {
            actions.push(Action::Import);
        }
        if self.filtered(cx) {
            actions.push(Action::ClearFilters);
        }
        actions
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        match action {
            Action::Import => game_db::choose_and_import(window, cx),
            Action::ClearFilters => {
                this.update(cx, |page, cx| page.clear_filters(window, cx));
            }
        }
    }
}

/// The Database page listing the games that reached `fen` (the analysis board's position).
pub fn games_at(fen: String, window: &mut Window, cx: &mut App) {
    kchess_nav::shell::show::<DatabasePage>(window, cx, move |page, window, cx| {
        page.show_position(fen, window, cx)
    });
}

/// Import a PGN file dropped on the Database page.
pub fn import_dropped(path: std::path::PathBuf, cx: &mut App) {
    let db = cx.global::<GameDbHandle>().0.clone();
    db.update(cx, |db, cx| db.import(path, cx));
}
