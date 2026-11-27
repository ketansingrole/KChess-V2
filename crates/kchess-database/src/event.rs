//! An event's table from the database shown (a game's menu → Event table): the standings of the
//! edition the game was played in, with each player's result against each other beside them while
//! there are at most `CROSSTABLE` players. A result opens its game on the analysis board, a player
//! their report.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, GameDbGameId};
use kchess_contracts::types::{GameDbEventPlayer, GameDbEventReport, GameDbEventResult};

use crate::DatabasePage;
use crate::core::AppCore;
use crate::loading::Loading;
use crate::theme::{Palette, radii};
use crate::widgets::{self, KeyboardFocus as _, Pressable as _};

/// The most players whose results are shown each against each.
pub const CROSSTABLE: usize = 16;
const PLACE: Pixels = px(28.);
const NAME: Pixels = px(168.);
const NUMBER: Pixels = px(48.);
const CELL: Pixels = px(26.);
const GAP: Pixels = px(2.);
/// Between the ratings and the crosstable.
const APART: Pixels = px(8.);

/// Whether an event of `players` players shows each one's result against each other.
fn crosstable(players: usize) -> bool {
    (2..=CROSSTABLE).contains(&players)
}

/// The dialog's width for an event of `players` players: the crosstable's, or the standings'.
fn width(players: usize) -> Pixels {
    let columns = if crosstable(players) {
        // Rating, the grid, then Points, SB and Perf.
        (PLACE + NAME + NUMBER * 4. + APART) + (CELL + GAP) * players as f32
    } else {
        PLACE + NAME + NUMBER * 5.
    };
    // The gaps between the columns, the rows' padding and the dialog's.
    (columns + GAP * 6. + px(56.)).max(px(560.))
}

/// Half points as a score: 5 is "2½", 1 "½".
pub fn points(halves: u32) -> String {
    match (halves / 2, halves % 2) {
        (0, 1) => "½".to_string(),
        (whole, 1) => format!("{whole}½"),
        (whole, _) => whole.to_string(),
    }
}

/// A Sonneborn–Berger score in quarter points: 11 is "2¾".
pub fn quarters(quarters: u32) -> String {
    let whole = quarters / 4;
    let part = ["", "¼", "½", "¾"][(quarters % 4) as usize];
    match (whole, part) {
        (0, "") => "0".to_string(),
        (0, part) => part.to_string(),
        (whole, part) => format!("{whole}{part}"),
    }
}

/// A result as a crosstable shows it.
fn mark(result: &GameDbEventResult) -> &'static str {
    match result.half_points {
        2 => "1",
        1 => "½",
        _ => "0",
    }
}

/// "Hastings ENG · 2023.12.28–2024.01.03 · 45 games · 10 players".
fn summary(report: &GameDbEventReport) -> String {
    let dates = match (&report.first_date, &report.last_date) {
        (Some(first), Some(last)) if first != last => Some(format!("{first}–{last}")),
        (Some(date), _) | (_, Some(date)) => Some(date.clone()),
        _ => None,
    };
    [
        report.site.clone(),
        dates,
        Some(widgets::format_counted(
            report.games as f64,
            "game",
            "games",
        )),
        Some(widgets::format_counted(
            report.players.len() as f64,
            "player",
            "players",
        )),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<String>>()
    .join(" · ")
}

/// Opens the table of game `game`'s event from database `database` of `page`.
pub fn open(
    page: WeakEntity<DatabasePage>,
    database: String,
    game: u64,
    event: String,
    window: &mut Window,
    cx: &mut App,
) {
    let title = SharedString::from(event);
    let table = cx.new(|cx| EventTable::new(page, database, game, cx));
    widgets::open_dialog(window, cx, move |dialog, _, cx| {
        let players = table
            .read(cx)
            .report
            .as_ref()
            .map_or(0, |report| report.players.len());
        dialog
            .title(title.clone())
            .w(width(players))
            .child(table.clone())
    });
}

pub struct EventTable {
    /// The scroll of the list inside it, kept to the list (`widgets::nested_scroll`).
    list_scroll: ScrollHandle,
    page: WeakEntity<DatabasePage>,
    database: String,
    report: Option<GameDbEventReport>,
    loading: Loading,
}

impl EventTable {
    fn new(
        page: WeakEntity<DatabasePage>,
        database: String,
        game: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = EventTable {
            list_scroll: ScrollHandle::new(),
            page,
            database: database.clone(),
            report: None,
            loading: Loading::default(),
        };
        let request = cx.global::<AppCore>().request(calls::GameDbEvent {
            database: DatabaseId(database),
            game: GameDbGameId(game),
        });
        this.loading.load(
            cx,
            "database",
            |this| &mut this.loading,
            request,
            |this, answer, _| this.report = answer.ok(),
        );
        this
    }

    fn about(&self, name: String, window: &mut Window, cx: &mut App) {
        let (page, database) = (self.page.clone(), self.database.clone());
        window.close_dialog(cx);
        crate::report::open(page, database, name, window, cx);
    }

    fn open_game(
        &self,
        game: u64,
        white: String,
        black: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.close_dialog(cx);
        crate::open_game_by_id(self.database.clone(), game, white, black, window, cx);
    }

    /// A player's name and place, which opens their report.
    fn player_cell(
        &self,
        place: usize,
        player: &GameDbEventPlayer,
        label: String,
        cx: &Context<Self>,
        palette: &Palette,
    ) -> impl IntoElement + use<> {
        let name = player.name.clone();
        h_flex()
            .id(("event-player", place))
            .test_support()
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(label)
            .w(PLACE + NAME)
            .flex_none()
            .gap(px(4.))
            .px(px(4.))
            .py(px(4.))
            .rounded(radii::BOX)
            .cursor_pointer()
            .pressable(palette)
            .on_click(cx.listener(move |this, _, window, cx| this.about(name.clone(), window, cx)))
            .child(
                div()
                    .w(PLACE - px(4.))
                    .flex_none()
                    .text_color(palette.text_muted)
                    .child(format!("{}", place + 1)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(player.name.clone()),
            )
    }

    /// The standings, with each player's result against each other while they are few enough.
    fn table(&self, report: &GameDbEventReport, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let muted = palette.text_muted;
        let count = report.players.len();
        let grid = crosstable(count);
        let number = |text: SharedString| div().w(NUMBER).flex_none().text_right().child(text);
        let header = h_flex()
            .gap(GAP)
            .px(px(4.))
            .text_color(muted)
            .child(div().w(PLACE + NAME).flex_none().child("Player"))
            .child(number(SharedString::new_static("Rating")))
            .when(grid, |row| {
                row.child(div().w(APART).flex_none())
                    .children((1..=count).map(|column| {
                        div()
                            .w(CELL)
                            .flex_none()
                            .text_center()
                            .child(column.to_string())
                    }))
            })
            .child(number(SharedString::new_static("Points")))
            .when(!grid, |row| {
                row.child(number(SharedString::new_static("W–D–L")))
            })
            .child(number(SharedString::new_static("SB")))
            .child(number(SharedString::new_static("Perf.")));
        let rows = report.players.iter().enumerate().map(|(place, player)| {
            let label = format!(
                "{}. {}, {} points, won {}, drew {}, lost {}",
                place + 1,
                player.name,
                points(player.half_points),
                player.wins,
                player.draws,
                player.losses
            );
            let optional = |value: Option<u32>| -> SharedString {
                value
                    .map(|value| value.to_string())
                    .unwrap_or_default()
                    .into()
            };
            let record = format!("{}–{}–{}", player.wins, player.draws, player.losses);
            h_flex()
                .gap(GAP)
                .child(self.player_cell(place, player, label, cx, palette))
                .child(number(optional(player.rating)).text_color(muted))
                .when(grid, |row| {
                    row.child(div().w(APART).flex_none()).children(
                        (0..count).map(|opponent| self.cell(report, place, opponent, cx, palette)),
                    )
                })
                .child(number(points(player.half_points).into()))
                .when(!grid, |row| {
                    row.child(number(record.into()).text_color(muted))
                })
                .child(number(quarters(player.tiebreak_quarters).into()).text_color(muted))
                .child(number(optional(player.performance)).text_color(muted))
        });
        v_flex()
            .gap(px(2.))
            .text_size(px(12.))
            .child(header)
            .children(rows)
    }

    /// Player `place`'s results against player `opponent`, each opening its game.
    fn cell(
        &self,
        report: &GameDbEventReport,
        place: usize,
        opponent: usize,
        cx: &Context<Self>,
        palette: &Palette,
    ) -> AnyElement {
        let cell = div()
            .w(CELL)
            .flex_none()
            .h(px(24.))
            .rounded(radii::BOX)
            .flex()
            .justify_center()
            .items_center();
        if opponent == place {
            return cell.bg(palette.accented).into_any_element();
        }
        let player = &report.players[place];
        let other = report.players[opponent].name.clone();
        let results = player
            .results
            .iter()
            .filter(|result| result.opponent as usize == opponent)
            .map(|result| {
                let (white, black) = if result.white {
                    (player.name.clone(), other.clone())
                } else {
                    (other.clone(), player.name.clone())
                };
                let game = result.game;
                let label = format!(
                    "{} against {}{}: {}",
                    player.name,
                    other,
                    result
                        .round
                        .as_deref()
                        .map(|round| format!(", round {round}"))
                        .unwrap_or_default(),
                    points(u32::from(result.half_points))
                );
                div()
                    .id(SharedString::from(format!("event-result-{place}-{game}")))
                    .test_support()
                    .role(Role::Button)
                    .keyboard_focus(palette.primary)
                    .aria_label(label)
                    .px(px(3.))
                    .rounded(radii::BOX)
                    .cursor_pointer()
                    .pressable(palette)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_game(game, white.clone(), black.clone(), window, cx)
                    }))
                    .child(mark(result))
            });
        cell.children(results).into_any_element()
    }
}

impl Render for EventTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let Some(report) = self.report.clone() else {
            let body = match self.loading.error() {
                Some(error) => div()
                    .text_color(palette.error)
                    .child(error.clone())
                    .into_any_element(),
                None => widgets::loading("Reading the event…", &palette).into_any_element(),
            };
            return v_flex().gap(px(12.)).child(body);
        };
        let summary = summary(&report);
        v_flex()
            .gap(px(16.))
            .child(
                div()
                    .id("event-summary")
                    .test_support()
                    .aria_label(summary.clone())
                    .text_size(px(13.))
                    .text_color(palette.text_muted)
                    .child(summary),
            )
            .child(
                widgets::nested_scroll(
                    v_flex().id("event-tables").max_h(px(560.)),
                    &self.list_scroll,
                )
                .gap(px(16.))
                .child(self.table(&report, cx, &palette)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{points, quarters};

    #[test]
    fn scores_read_with_halves_and_quarters() {
        assert_eq!(points(0), "0");
        assert_eq!(points(1), "½");
        assert_eq!(points(5), "2½");
        assert_eq!(points(6), "3");
        assert_eq!(quarters(0), "0");
        assert_eq!(quarters(1), "¼");
        assert_eq!(quarters(11), "2¾");
        assert_eq!(quarters(8), "2");
    }
}
