//! The Database page: the games of a game database on this computer, searched by player, side,
//! event, years, rating, result and position, a page at a time, newest, oldest or best rated
//! first; each opened on the analysis board. PGN files, or zip and 7z archives of them, are imported here
//! (picked or dropped), a player's games downloaded from Lichess or Chess.com to prepare against
//! them (`player`), and a database deleted.

// A failure is an answer, never a panic: no `unwrap` or `expect` outside tests (`check`).
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

// Only for its features (`crates/kchess-hack`); naming it tells Cargo the dependency is used.
use kchess_hack as _;

use kchess_app::{core, loading, prefs};
use kchess_board::{game_db, game_db_list, live_game, widgets};
use kchess_ui::theme;

mod event;
mod lichess;
mod manage;
mod merge;
mod player;
mod report;
pub mod target;
mod tidy;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Selectable as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_board::analysis::{Open, Opening};
use kchess_board::game::played::{DatabaseGame, GameOrigin};
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, GameDbGameId, PgnSavePath};
use kchess_contracts::types::{
    DatabaseInfo, DatabaseSource, GameDbQuery, GameDbResult, GameDbRow, GameDbSide, GameDbSort,
    GameDbStructure, Speed,
};
use kchess_domain::gamedb::patterns::{Material, Placement};
use kchess_nav::page::{ActionInfo, Page, PageAction, PageView};
use kchess_nav::play_layout::{PAGE_BOTTOM, PAGE_TOP, side_padding};
use kchess_ui::icon;
use serde::{Deserialize, Serialize};

use crate::core::AppCore;
use crate::game_db::{Chosen, GameDbEvent, GameDbHandle};
use crate::live_game::Refused;
use crate::loading::Loading;
use crate::theme::{Palette, radii};
use crate::widgets::{Choice, Pill};

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
const SPEEDS: [(&str, &str); 7] = [
    ("any", "Any speed"),
    ("ultraBullet", "UltraBullet"),
    ("bullet", "Bullet"),
    ("blitz", "Blitz"),
    ("rapid", "Rapid"),
    ("classical", "Classical"),
    ("correspondence", "Correspondence"),
];
const STRUCTURES: [(&str, &str); 6] = [
    ("any", "Any structure"),
    ("isolatedQueenPawn", "Isolated queen pawn"),
    ("hangingPawns", "Hanging pawns"),
    ("carlsbad", "Carlsbad"),
    ("maroczyBind", "Maróczy bind"),
    ("stonewall", "Stonewall"),
];
/// The most moves a moves field takes.
const MOST_MOVES: u32 = 600;
/// The broadcast games: Lichess's broadcast archive, some 770 MB to download in all (2026-11).
const MASTERS_LABEL: &str = "Broadcast games · 770 MB";
const MASTERS_BYTES: u64 = 770 * 1024 * 1024;
/// TCEC's engine games: its compact release, some 27 MB to download (2026-06).
const TCEC_LABEL: &str = "Engine games · 27 MB";
const TCEC_BYTES: u64 = 27 * 1024 * 1024;
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
    /// The games count and last change of the database the games shown were read from: when
    /// either changes (games added, deleted or replaced), the page is read again.
    read_from: Option<(u64, i64)>,
    database_select: Entity<SelectState<Vec<Choice>>>,
    player_input: Entity<InputState>,
    side_select: Entity<SelectState<Vec<Choice>>>,
    event_input: Entity<InputState>,
    from_input: Entity<InputState>,
    to_input: Entity<InputState>,
    rating_select: Entity<SelectState<Vec<Choice>>>,
    /// The filters that replay each game, shown under the others.
    more: bool,
    min_moves_input: Entity<InputState>,
    max_moves_input: Entity<InputState>,
    speed_select: Entity<SelectState<Vec<Choice>>>,
    material_input: Entity<InputState>,
    pieces_input: Entity<InputState>,
    structure_select: Entity<SelectState<Vec<Choice>>>,
    speed: Option<Speed>,
    structure: Option<GameDbStructure>,
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
    /// Its games were refused while help was held back: listed once it is lent.
    refused: Refused,
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

fn speed_of(key: &str) -> Option<Speed> {
    Some(match key {
        "ultraBullet" => Speed::UltraBullet,
        "bullet" => Speed::Bullet,
        "blitz" => Speed::Blitz,
        "rapid" => Speed::Rapid,
        "classical" => Speed::Classical,
        "correspondence" => Speed::Correspondence,
        _ => return None,
    })
}

/// A pawn structure's name, as the Pawn structure filter lists it.
pub fn structure_name(structure: GameDbStructure) -> &'static str {
    STRUCTURES
        .iter()
        .find(|(key, _)| *key == structure.as_str())
        .map_or(structure.as_str(), |(_, name)| name)
}

fn structure_of(key: &str) -> Option<GameDbStructure> {
    Some(match key {
        "isolatedQueenPawn" => GameDbStructure::IsolatedQueenPawn,
        "hangingPawns" => GameDbStructure::HangingPawns,
        "carlsbad" => GameDbStructure::Carlsbad,
        "maroczyBind" => GameDbStructure::MaroczyBind,
        "stonewall" => GameDbStructure::Stonewall,
        _ => return None,
    })
}

/// A number of moves typed in a moves field; anything else is none.
fn moves(text: &str) -> Option<u32> {
    text.trim()
        .parse::<u32>()
        .ok()
        .filter(|moves| *moves <= MOST_MOVES)
}

/// Material typed in its field, once it reads as each side's pieces ("R vs B"): until then, as a
/// year half typed, it filters nothing.
fn material(text: &str) -> Option<String> {
    Material::parse(text).map(|_| text.trim().to_string())
}

/// Pieces typed in their field, once they read as each side's ("Nd5 vs e6"): until then, as
/// material half typed, they filter nothing.
fn pieces(text: &str) -> Option<String> {
    Placement::parse(text).map(|_| text.trim().to_string())
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
        let speed_select = cx.new(|cx| SelectState::new(choices(&SPEEDS), None, window, cx));
        let structure_select =
            cx.new(|cx| SelectState::new(choices(&STRUCTURES), None, window, cx));
        for (select, value) in [
            (&side_select, "either".to_string()),
            (&rating_select, "0".to_string()),
            (&speed_select, "any".to_string()),
            (&structure_select, "any".to_string()),
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
        let min_moves_input = field("Min moves", window, cx);
        let max_moves_input = field("Max moves", window, cx);
        let material_input = field("Material, e.g. R vs B", window, cx);
        let pieces_input = field("Pieces, e.g. Nd5 vs e6", window, cx);
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
                &speed_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.speed = speed_of(value);
                        this.reload(cx);
                    }
                },
            ),
            cx.subscribe(
                &structure_select,
                |this, _, event: &SelectEvent<Vec<Choice>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.structure = structure_of(value);
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
        for input in [
            &player_input,
            &event_input,
            &from_input,
            &to_input,
            &min_moves_input,
            &max_moves_input,
            &material_input,
            &pieces_input,
        ] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                // Typing keeps the last answer's games until the next one comes, where a choice
                // clears them (`reload`): cleared, they flashed the loading line at each key.
                if let InputEvent::Change = event {
                    this.page = 0;
                    this.load(cx);
                }
            }));
        }
        // Opened as the app starts, the page asks before Lichess has answered whether a game is
        // in progress: refused, its games are listed once help is lent.
        subscriptions.push(Refused::watch(cx, |this| &mut this.refused, Self::load));
        let databases = cx.global::<GameDbHandle>().0.clone();
        subscriptions.push(cx.observe_in(&databases, window, |this, _, window, cx| {
            this.sync_databases(window, cx)
        }));
        subscriptions.push(cx.subscribe_in(
            &databases,
            window,
            |this, _, event: &GameDbEvent, window, cx| match event {
                GameDbEvent::PlayerInstalled { database, username } => {
                    this.show_database(database.clone(), username.clone(), window, cx)
                }
                GameDbEvent::LichessInstalled { database } => {
                    this.show_database(database.clone(), String::new(), window, cx)
                }
                GameDbEvent::Copied { database, added } => window.push_notification(
                    Notification::success(format!(
                        "{} copied to {}",
                        widgets::format_counted(*added as f64, "game", "games"),
                        database.name
                    )),
                    cx,
                ),
                GameDbEvent::DuplicatesRemoved { removed } => window.push_notification(
                    Notification::success(format!(
                        "{} removed",
                        widgets::format_counted(*removed as f64, "duplicate", "duplicates")
                    )),
                    cx,
                ),
            },
        ));
        let mut page = DatabasePage {
            focus: cx.focus_handle(),
            database: kept.database,
            listed: Vec::new(),
            read_from: None,
            database_select,
            player_input,
            side_select,
            event_input,
            from_input,
            to_input,
            rating_select,
            more: false,
            min_moves_input,
            max_moves_input,
            speed_select,
            material_input,
            pieces_input,
            structure_select,
            speed: None,
            structure: None,
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
            refused: Refused::default(),
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
            // Its games changed: the same page is read again, the last one if it went.
            if self.edited(cx) != self.read_from {
                self.load(cx);
            }
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
            min_moves: Self::text(&self.min_moves_input, cx).and_then(|text| moves(&text)),
            max_moves: Self::text(&self.max_moves_input, cx).and_then(|text| moves(&text)),
            speed: self.speed,
            material: Self::text(&self.material_input, cx).and_then(|text| material(&text)),
            structure: self.structure,
            pieces: Self::text(&self.pieces_input, cx).and_then(|text| pieces(&text)),
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
            || self.more_filtered(cx)
    }

    /// Whether a filter under More filters is set.
    fn more_filtered(&self, cx: &App) -> bool {
        [
            &self.min_moves_input,
            &self.max_moves_input,
            &self.material_input,
            &self.pieces_input,
        ]
        .iter()
        .any(|input| Self::text(input, cx).is_some())
            || self.speed.is_some()
            || self.structure.is_some()
    }

    /// The first page of what the filters now match. The last search's games go at once: kept
    /// until the answer came, they and the pager under them jumped to the new search's size.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.page = 0;
        self.games.clear();
        self.total = 0;
        self.load(cx);
    }

    /// The shown database's games count and last change.
    fn edited(&self, cx: &App) -> Option<(u64, i64)> {
        self.shown_database(cx)
            .map(|info| (info.games, info.updated_at))
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.read_from = self.edited(cx);
        self.refused.asking();
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
            |this, answer, cx| {
                this.refused
                    .answered(&answer, |this| &mut this.refused, Self::load, cx);
                match answer {
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
            &self.min_moves_input,
            &self.max_moves_input,
            &self.material_input,
            &self.pieces_input,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        for (select, value) in [
            (&self.side_select, "either"),
            (&self.rating_select, "0"),
            (&self.speed_select, "any"),
            (&self.structure_select, "any"),
        ] {
            select.update(cx, |state, cx| {
                state.set_selected_value(&SharedString::new_static(value), window, cx)
            });
        }
        self.side = GameDbSide::Either;
        self.min_elo = None;
        self.speed = None;
        self.structure = None;
        self.result = None;
        self.position = None;
        self.reload(cx);
    }

    /// A player's database once their games are in: their games, the player's name as the
    /// filter, so that a side splits them into their games with White and with Black.
    /// Database `database`'s games, every filter cleared but the player's, `player`.
    fn show_database(
        &mut self,
        database: String,
        player: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_filters(window, cx);
        let kept = database.clone();
        store(cx, |prefs| prefs.database = Some(kept));
        self.database_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(database.clone()), window, cx)
        });
        self.database = Some(database);
        self.player_input
            .update(cx, |input, cx| input.set_value(player, window, cx));
        self.reload(cx);
    }

    /// Database `database`'s games of `player` that reach `structure`, every other filter
    /// cleared, with More filters open to show it.
    pub fn show_structure(
        &mut self,
        database: String,
        player: String,
        structure: GameDbStructure,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_database(database, player, window, cx);
        self.structure_select.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::new_static(structure.as_str()), window, cx)
        });
        self.structure = Some(structure);
        self.more = true;
        self.reload(cx);
    }

    /// The games that reached `fen`, every other filter cleared.
    pub fn show_position(&mut self, fen: String, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_filters(window, cx);
        self.position = Some(fen);
        self.reload(cx);
    }

    fn open_game(&mut self, row: &GameDbRow, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(database) = self.database.clone() {
            open_game(database, row, window, cx);
        }
    }

    /// Every game the filters find, whatever its page.
    fn found(&self, cx: &App) -> Option<Chosen> {
        let database = self.database.clone()?;
        Some(Chosen::Search(Box::new(GameDbQuery {
            offset: 0,
            ..self.query(database, cx)
        })))
    }

    /// Asks, then removes the `chosen` games (`count` of them, `game` when it is one) from the
    /// database shown.
    fn ask_delete_games(
        &self,
        chosen: Chosen,
        count: u64,
        game: Option<&GameDbRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(info) = self.shown_database(cx).cloned() else {
            return;
        };
        let (title, description) = match game {
            Some(game) => (
                "Delete this game?".to_string(),
                format!(
                    "{} against {} is removed from {}.",
                    game.white, game.black, info.name
                ),
            ),
            None => (
                format!(
                    "Delete {}?",
                    widgets::format_counted(count as f64, "game", "games")
                ),
                format!(
                    "Every game this search finds is removed from {}.",
                    info.name
                ),
            ),
        };
        widgets::confirm_danger(window, cx, title, description, "Delete", move |_, cx| {
            let db = cx.global::<GameDbHandle>().0.clone();
            db.update(cx, |db, cx| {
                db.delete_games(info.id.clone(), chosen.clone(), cx)
            });
        });
    }

    /// Copies the `chosen` games to a database picked in a dialog.
    /// Merges the `chosen` games into a chapter of a study picked in a dialog.
    fn merge_games(&self, chosen: Chosen, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(info) = self.shown_database(cx).cloned() {
            merge::merge(info, chosen, window, cx);
        }
    }

    fn copy_games(&self, chosen: Chosen, count: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(info) = self.shown_database(cx).cloned() {
            target::copy(info, chosen, count, window, cx);
        }
    }

    /// Asks where, then saves the `chosen` games of the database shown as one PGN file named
    /// `name` at first.
    fn export(&self, chosen: Chosen, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(database) = self.database.clone() else {
            return;
        };
        let place = kchess_board::export::choose_new_path(&format!("{name}.pgn"), cx);
        cx.spawn_in(window, async move |_, cx| {
            let saved = match place.await {
                None => return,
                Some(Err(cause)) => Err(cause),
                Some(Ok(path)) => {
                    let (games, search) = chosen.parts();
                    let export = calls::GameDbExport {
                        database: DatabaseId(database),
                        path: PgnSavePath(path.to_string_lossy().into_owned()),
                        games,
                        search,
                    };
                    // The window closed while the place was asked: nothing was asked of the core.
                    let Ok(request) = cx.update(|_, cx| cx.global::<AppCore>().request(export))
                    else {
                        return;
                    };
                    request
                        .await
                        .map_err(|cause| loading::failure("database", &cause))
                }
            };
            cx.update(|window, cx| {
                let notification = match saved {
                    Ok(saved) => Notification::success(format!(
                        "{} saved",
                        widgets::format_counted(saved as f64, "game", "games")
                    )),
                    Err(cause) => Notification::error(cause).title("Could not save the games"),
                };
                window.push_notification(notification, cx)
            })
            .ok();
        })
        .detach();
    }

    /// What may be done with `chosen` (`count` games, `game` when it is one).
    fn games_menu(
        &self,
        chosen: Chosen,
        count: u64,
        game: Option<GameDbRow>,
        cx: &mut Context<Self>,
    ) -> GamesMenu {
        let name = match (&game, self.shown_database(cx)) {
            (Some(game), _) => kchess_board::export::game_file_name(&game.white, &game.black),
            (None, Some(info)) => kchess_board::export::file_name(&info.name),
            (None, None) => "games".to_string(),
        };
        GamesMenu {
            chosen,
            count,
            game,
            name,
            busy: cx.global::<GameDbHandle>().0.read(cx).busy(),
            editable: self
                .shown_database(cx)
                .is_some_and(|info| info.source != DatabaseSource::Own),
            page: cx.entity().downgrade(),
        }
    }

    /// The report on the player `name` of the database shown.
    fn event_table(
        &mut self,
        game: u64,
        event: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(database) = self.database.clone() {
            event::open(cx.entity().downgrade(), database, game, event, window, cx);
        }
    }

    /// Guess game `row`'s moves on Practice, as the side that won (White after a draw), as the
    /// explorer's master games are guessed.
    fn guess(&mut self, row: &GameDbRow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(database) = self.database.clone() else {
            return;
        };
        let request = cx.global::<AppCore>().request(calls::GameDbGame {
            database: DatabaseId(database),
            game: GameDbGameId(row.id),
        });
        let side = if row.result == "0-1" {
            shakmaty::Color::Black
        } else {
            shakmaty::Color::White
        };
        cx.spawn_in(window, async move |_, cx| {
            let pgn = request.await;
            cx.update(|window, cx| match pgn {
                Ok(pgn) => kchess_practice::guess_moves(&pgn, Some(side), window, cx),
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

    fn about(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(database) = self.database.clone() {
            report::open(cx.entity().downgrade(), database, name, window, cx);
        }
    }

    /// Asks, then deletes the database shown.
    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(info) = self.shown_database(cx).cloned() {
            manage::ask_delete(info, window, cx);
        }
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
                .child(Input::new(input).cleanable(true).aria_label(label))
        };
        let select = |state: &Entity<SelectState<Vec<Choice>>>, width: f32, label: &'static str| {
            div()
                .w(px(width))
                .child(Select::new(state).accessibility_label(label))
        };
        let more = self.more || self.more_filtered(cx);
        let toolbar = h_flex()
            .flex_wrap()
            .gap(px(8.))
            .child(
                div().w(px(180.)).child(
                    Input::new(&self.player_input)
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
            .child(
                Button::new("database-more")
                    .ghost()
                    .icon(icon("sliders-horizontal"))
                    .label("More filters")
                    .selected(more)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.more = !this.more;
                        cx.notify();
                    })),
            )
            .when(self.filtered(cx), |row| {
                row.child(
                    Button::new("database-clear")
                        .ghost()
                        .icon(icon("x"))
                        .label("Clear filters")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.clear_filters(window, cx)),
                        ),
                )
            });
        v_flex().gap(px(8.)).child(toolbar).when(more, |column| {
            column.child(
                h_flex()
                    .id("database-more-filters")
                    .flex_wrap()
                    .gap(px(8.))
                    .child(field(&self.min_moves_input, 96., "Min moves"))
                    .child(field(&self.max_moves_input, 96., "Max moves"))
                    .child(select(&self.speed_select, 150., "Time control"))
                    .child(field(&self.material_input, 180., "Material"))
                    .child(select(&self.structure_select, 170., "Pawn structure"))
                    .child(field(&self.pieces_input, 180., "Pieces")),
            )
        })
    }

    fn render_rows(&self, narrow: bool, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let rows: Vec<AnyElement> = self
            .games
            .iter()
            .enumerate()
            .map(|(index, game)| {
                let chosen = game.clone();
                let label = game_db_list::label(game);
                let menu = self.games_menu(Chosen::Game(game.id), 1, Some(game.clone()), cx);
                let actions = Button::new(("database-game-actions", index))
                    .ghost()
                    .small()
                    .icon(icon("ellipsis"))
                    .accessibility_label(format!("Actions for {label}"))
                    // Aligned to the button's end, so it stays in the window.
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |popup, _, _| {
                        menu.build(popup)
                    });
                game_db_list::row(
                    ("database-game", index),
                    game,
                    narrow,
                    Some(actions.into_any_element()),
                    cx.listener(move |this, _, window, cx| this.open_game(&chosen, window, cx)),
                    palette,
                )
            })
            .collect();
        v_flex()
            .child(game_db_list::head(narrow, palette))
            .children(rows)
            .into_any_element()
    }
}

/// A menu of what may be done with some games: for one, its players' and event's reports and
/// guessing its moves; copy them, save them as PGN, merge a search's into a study, or delete them;
/// from Your games only copy and save, and only save while an import runs.
#[derive(Clone)]
struct GamesMenu {
    chosen: Chosen,
    count: u64,
    /// The game, when the menu is one game's.
    game: Option<GameDbRow>,
    /// The PGN file's name at first.
    name: String,
    busy: bool,
    editable: bool,
    page: WeakEntity<DatabasePage>,
}

impl GamesMenu {
    fn build(&self, menu: PopupMenu) -> PopupMenu {
        let (copy, export, merge, delete) =
            (self.clone(), self.clone(), self.clone(), self.clone());
        // A game's players, each with their report.
        let players: Vec<String> = self
            .game
            .iter()
            .flat_map(|game| [game.white.clone(), game.black.clone()])
            .filter(|name| !name.is_empty() && name != "?")
            .collect();
        let menu = players.into_iter().fold(menu, |menu, name| {
            let page = self.page.clone();
            menu.item(
                PopupMenuItem::new(format!("About {name}"))
                    .icon(icon("user"))
                    .on_click(move |_, window, cx| {
                        let name = name.clone();
                        page.update(cx, |page, cx| page.about(name, window, cx))
                            .ok();
                    }),
            )
        });
        let event = self
            .game
            .as_ref()
            .and_then(|game| Some((game.id, game.event.clone()?)))
            .filter(|(_, event)| !event.is_empty() && event != "?");
        let menu = match event {
            Some((game, event)) => {
                let page = self.page.clone();
                menu.item(
                    PopupMenuItem::new("Event table")
                        .icon(icon("trophy"))
                        .on_click(move |_, window, cx| {
                            let event = event.clone();
                            page.update(cx, |page, cx| page.event_table(game, event, window, cx))
                                .ok();
                        }),
                )
            }
            None => menu,
        };
        // A game with moves can be guessed through.
        let menu = match self.game.clone().filter(|game| game.plies > 0) {
            Some(game) => {
                let page = self.page.clone();
                menu.item(
                    PopupMenuItem::new("Guess the move")
                        .icon(icon("target"))
                        .on_click(move |_, window, cx| {
                            page.update(cx, |page, cx| page.guess(&game, window, cx))
                                .ok();
                        }),
                )
            }
            None => menu,
        };
        let menu = if self.game.is_some() {
            menu.separator()
        } else {
            menu
        };
        menu.when(!self.busy, |menu| {
            menu.item(
                PopupMenuItem::new("Copy to database…")
                    .icon(icon("copy"))
                    .on_click(move |_, window, cx| {
                        copy.page
                            .update(cx, |page, cx| {
                                page.copy_games(copy.chosen.clone(), copy.count, window, cx)
                            })
                            .ok();
                    }),
            )
        })
        .item(
            PopupMenuItem::new("Save as PGN…")
                .icon(icon("download"))
                .on_click(move |_, window, cx| {
                    export
                        .page
                        .update(cx, |page, cx| {
                            page.export(export.chosen.clone(), export.name.clone(), window, cx)
                        })
                        .ok();
                }),
        )
        // Games found, as one chapter: one game alone has nothing to merge.
        .when(self.game.is_none(), |menu| {
            menu.item(
                PopupMenuItem::new("Merge into a study…")
                    .icon(icon("book-plus"))
                    .on_click(move |_, window, cx| {
                        merge
                            .page
                            .update(cx, |page, cx| {
                                page.merge_games(merge.chosen.clone(), window, cx)
                            })
                            .ok();
                    }),
            )
        })
        .when(!self.busy && self.editable, |menu| {
            menu.separator().item(
                PopupMenuItem::new(if self.game.is_some() {
                    "Delete game"
                } else {
                    "Delete these games…"
                })
                .icon(icon("trash-2"))
                .on_click(move |_, window, cx| {
                    delete
                        .page
                        .update(cx, |page, cx| {
                            page.ask_delete_games(
                                delete.chosen.clone(),
                                delete.count,
                                delete.game.as_ref(),
                                window,
                                cx,
                            )
                        })
                        .ok();
                }),
            )
        })
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
        let masters = self
            .databases(cx)
            .iter()
            .any(|info| info.source == DatabaseSource::Master);
        let engines = self
            .databases(cx)
            .iter()
            .any(|info| info.source == DatabaseSource::Tcec);
        // The database's own row sits under the downloads, so the downloads stay put when a
        // database is listed after the page first draws ("Your games" is built as games sync).
        let chosen = (databases > 0).then(|| {
            h_flex()
                .gap(px(12.))
                .flex_wrap()
                .child(
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
                        .child(format!(
                            "{} · {}",
                            widgets::format_counted(info.games as f64, "game", "games"),
                            widgets::format_bytes(info.bytes as f64)
                        ))
                }))
                .child(div().flex_1())
                .children(
                    self.shown_database(cx)
                        .and_then(|info| info.player.clone())
                        .filter(|_| !busy)
                        .map(|earlier| {
                            Button::new("database-update-player")
                                .ghost()
                                .small()
                                .icon(icon("refresh-cw"))
                                .label("Update games")
                                .on_click(move |_, window, cx| {
                                    player::open(Some(earlier.clone()), window, cx)
                                })
                        }),
                )
                .children(
                    self.shown_database(cx)
                        .and_then(|info| info.lichess_rating)
                        .filter(|_| !busy)
                        .map(|rating| {
                            Button::new("database-update-lichess")
                                .ghost()
                                .small()
                                .icon(icon("plus"))
                                .label("Add a month")
                                .on_click(move |_, window, cx| {
                                    lichess::open(Some(rating), window, cx)
                                })
                        }),
                )
                .children(
                    self.shown_database(cx)
                        .filter(|info| !busy && info.source != DatabaseSource::Own)
                        .cloned()
                        .map(|info| {
                            let (copies, names) = (info.clone(), info);
                            Button::new("database-tidy")
                                .ghost()
                                .small()
                                .icon(icon("eraser"))
                                .label("Tidy up")
                                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                                    let (copies, names) = (copies.clone(), names.clone());
                                    menu.item(
                                        PopupMenuItem::new("Remove duplicate games…")
                                            .icon(icon("copy"))
                                            .on_click(move |_, window, cx| {
                                                tidy::remove_duplicates(copies.clone(), window, cx)
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new("Merge player names…")
                                            .icon(icon("users"))
                                            .on_click(move |_, window, cx| {
                                                tidy::merge_names(names.clone(), window, cx)
                                            }),
                                    )
                                })
                        }),
                )
                .when(!busy, |row| {
                    row.child(
                        Button::new("database-delete")
                            .ghost()
                            .small()
                            .icon(icon("trash-2"))
                            .label("Delete database")
                            .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                    )
                })
        });
        // Import is the page's main action; the downloads share one menu beside it, their sizes
        // in their names, so the row keeps three weights rather than five equal buttons.
        let downloads = h_flex()
            .gap(px(12.))
            .flex_wrap()
            .child(game_db::import_card(cx, true, &palette))
            .when(!busy, |row| {
                row.child(
                    Button::new("database-add")
                        .outline()
                        .small()
                        .icon(icon("download"))
                        .label("Add games")
                        .dropdown_caret(true)
                        .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                            menu.item(
                                PopupMenuItem::new("Player's games…")
                                    .icon(icon("user"))
                                    .on_click(|_, window, cx| player::open(None, window, cx)),
                            )
                            .item(
                                PopupMenuItem::new("Lichess games…")
                                    .icon(icon("download"))
                                    .on_click(|_, window, cx| lichess::open(None, window, cx)),
                            )
                            .separator()
                            .item(
                                PopupMenuItem::new(if masters {
                                    SharedString::new_static("Update broadcast games")
                                } else {
                                    SharedString::new_static(MASTERS_LABEL)
                                })
                                .icon(icon("download"))
                                .on_click(|_, window, cx| install_masters(window, cx)),
                            )
                            .item(
                                PopupMenuItem::new(if engines {
                                    SharedString::new_static("Update engine games")
                                } else {
                                    SharedString::new_static(TCEC_LABEL)
                                })
                                .icon(icon("download"))
                                .on_click(|_, window, cx| install_tcec(window, cx)),
                            )
                        }),
                )
            })
            // Always there, so what the databases take is found before the first download.
            .child(div().flex_1())
            .child(
                Button::new("database-manage")
                    .ghost()
                    .small()
                    .icon(icon("hard-drive"))
                    .label("Manage")
                    .tooltip("Every database, its size and the space they take")
                    .on_click(|_, window, cx| manage::open(window, cx)),
            );
        let header = v_flex().gap(px(12.)).child(downloads).children(chosen);
        let body = if databases == 0 {
            // Drawn as the place a file goes, which is what it asks for.
            v_flex()
                .id("database-empty")
                .test_support()
                .py(px(30.))
                .px(px(20.))
                .rounded(radii::BOX)
                .border_2()
                .border_dashed()
                .border_color(palette.border)
                .child(widgets::empty_state(
                    icon("database"),
                    "No game databases",
                    Some("Drop a PGN file here".into()),
                    &palette,
                ))
                .into_any_element()
        } else {
            let first_page = self.loading.busy() && self.games.is_empty();
            let content = if first_page {
                div()
                    .p(px(24.))
                    .child(widgets::loading_rows(
                        "database-games-loading",
                        "Loading games…",
                        6,
                        &palette,
                    ))
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
            // Every game the search finds, whatever its page: copied, saved or deleted at once.
            let found = self.found(cx).filter(|_| paged).map(|chosen| {
                let count = self.total as u64;
                let menu = self.games_menu(chosen, count, None, cx);
                h_flex().justify_end().pb(px(4.)).child(
                    Button::new("database-found")
                        .ghost()
                        .small()
                        .icon(icon("ellipsis"))
                        .label(format!(
                            "All {}",
                            widgets::format_counted(count as f64, "game", "games")
                        ))
                        .dropdown_menu_with_anchor(Anchor::TopRight, move |popup, _, _| {
                            menu.build(popup)
                        }),
                )
            });
            v_flex()
                .gap(px(12.))
                .child(self.render_toolbar(cx, &palette))
                .children(position)
                .child(
                    widgets::card(&palette)
                        .p(px(12.))
                        .children(found)
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
    Player,
    Lichess,
    Masters,
    Tcec,
    Manage,
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
            Action::Player => ActionInfo::new(
                "database:player",
                "Download a player's games",
                "user",
                "opponent prepare preparation lichess chess.com scout",
            ),
            Action::Lichess => ActionInfo::new(
                "database:lichess",
                "Download strong Lichess games",
                "download",
                "lichess rated month elite strong rating 2400 games",
            ),
            Action::Masters => ActionInfo::new(
                "database:masters",
                "Download broadcast games",
                "download",
                "masters broadcasts otb grandmaster update",
            ),
            Action::Tcec => ActionInfo::new(
                "database:tcec",
                "Download engine games",
                "download",
                "tcec engines stockfish leela computer championship update",
            ),
            Action::Manage => ActionInfo::new(
                "database:manage",
                "Manage game databases",
                "hard-drive",
                "storage disk space size delete free",
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
            actions.push(Action::Player);
            actions.push(Action::Lichess);
            actions.push(Action::Masters);
            actions.push(Action::Tcec);
        }
        actions.push(Action::Manage);
        if self.filtered(cx) {
            actions.push(Action::ClearFilters);
        }
        actions
    }

    fn act(this: &Entity<Self>, action: Action, window: &mut Window, cx: &mut App) {
        match action {
            Action::Import => game_db::choose_and_import(window, cx),
            Action::Player => player::open(None, window, cx),
            Action::Lichess => lichess::open(None, window, cx),
            Action::Masters => install_masters(window, cx),
            Action::Tcec => install_tcec(window, cx),
            Action::Manage => manage::open(window, cx),
            Action::ClearFilters => {
                this.update(cx, |page, cx| page.clear_filters(window, cx));
            }
        }
    }
}

/// The Database page listing the games that reached `fen` (the analysis board's position).
/// Opens game `row` of database `database` on the analysis board.
pub fn open_game(database: String, row: &GameDbRow, window: &mut Window, cx: &mut App) {
    open_game_by_id(
        database,
        row.id,
        row.white.clone(),
        row.black.clone(),
        window,
        cx,
    );
}

/// Opens game `game` of database `database`, between `white` and `black`, on the analysis board.
fn open_game_by_id(
    database: String,
    game: u64,
    white: String,
    black: String,
    window: &mut Window,
    cx: &mut App,
) {
    let request = cx.global::<AppCore>().request(calls::GameDbGame {
        database: DatabaseId(database.clone()),
        game: GameDbGameId(game),
    });
    let origin = GameOrigin {
        white: Some(white),
        black: Some(black),
        database: Some(DatabaseGame {
            database: database.clone(),
            game,
        }),
        ..GameOrigin::default()
    };
    window
        .spawn(cx, async move |cx| {
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

pub fn games_at(fen: String, window: &mut Window, cx: &mut App) {
    kchess_nav::shell::show::<DatabasePage>(window, cx, move |page, window, cx| {
        page.show_position(fen, window, cx)
    });
}

/// The Database page with database `database`'s games of `player` that reach `structure`.
pub fn games_with_structure(
    database: String,
    player: String,
    structure: GameDbStructure,
    window: &mut Window,
    cx: &mut App,
) {
    kchess_nav::shell::show::<DatabasePage>(window, cx, move |page, window, cx| {
        page.show_structure(database, player, structure, window, cx)
    });
}

/// The Database page with the table of game `game`'s event `event` from database `database`.
pub fn event_table(database: String, game: u64, event: String, window: &mut Window, cx: &mut App) {
    kchess_nav::shell::show::<DatabasePage>(window, cx, move |_, window, cx| {
        event::open(cx.entity().downgrade(), database, game, event, window, cx)
    });
}

/// The Database page with its databases listed to manage (from Settings → Data).
pub fn manage_databases(window: &mut Window, cx: &mut App) {
    kchess_nav::shell::show::<DatabasePage>(window, cx, |_, window, cx| manage::open(window, cx));
}

pub use manage::total_bytes;

/// Import a PGN file dropped on the Database page, once the disk has room for it.
pub fn import_dropped(path: std::path::PathBuf, window: &mut Window, cx: &mut App) {
    game_db::import_with_room(path, window, cx);
}

/// Download the broadcast games (or their newer months), once the disk has room for them.
pub(crate) fn install_masters(window: &mut Window, cx: &mut App) {
    game_db::with_room(
        Some(MASTERS_BYTES),
        "Download anyway",
        window,
        cx,
        |_, cx| {
            let db = cx.global::<GameDbHandle>().0.clone();
            db.update(cx, |db, cx| db.install_masters(cx));
        },
    );
}

/// Download TCEC's engine games, once the disk has room for them.
pub(crate) fn install_tcec(window: &mut Window, cx: &mut App) {
    game_db::with_room(Some(TCEC_BYTES), "Download anyway", window, cx, |_, cx| {
        let db = cx.global::<GameDbHandle>().0.clone();
        db.update(cx, |db, cx| db.install_tcec(cx));
    });
}
