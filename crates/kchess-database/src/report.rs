//! A player's report from the database shown (a game's menu → About): their games and years, their
//! rating by month, their results and openings as White and as Black, and their best wins, each
//! opened on the analysis board. Show games lists all their games on the page.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, PlayerName};
use kchess_contracts::types::{GameDbPlayerReport, GameDbRecord, GameDbRow, GameDbSideReport};
use kchess_ui::chart::{ChartPoint, LineChart};
use kchess_ui::icon;

use crate::DatabasePage;
use crate::core::AppCore;
use crate::loading::Loading;
use crate::theme::{Palette, radii};
use crate::widgets::{self, KeyboardFocus as _, Pressable as _};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// What a record reads as: "Won 12 · Drawn 5 · Lost 3 · 72%".
pub fn record_text(record: &GameDbRecord) -> String {
    let parts = [
        ("Won", record.wins),
        ("Drawn", record.draws),
        ("Lost", record.losses),
    ]
    .map(|(label, count)| format!("{label} {count}"))
    .join(" · ");
    match score(record) {
        Some(score) => format!("{parts} · {score}%"),
        None => parts,
    }
}

/// The share of the points won in the games with a result, as a whole percent.
pub fn score(record: &GameDbRecord) -> Option<u32> {
    let decided = record.wins + record.draws + record.losses;
    (decided > 0).then(|| {
        ((record.wins as f64 + record.draws as f64 / 2.) * 100. / decided as f64).round() as u32
    })
}

/// "2014–2024", or one year.
fn years(report: &GameDbPlayerReport) -> Option<String> {
    match (report.first_year, report.last_year) {
        (Some(first), Some(last)) if first != last => Some(format!("{first}–{last}")),
        (Some(year), _) | (_, Some(year)) => Some(year.to_string()),
        _ => None,
    }
}

/// Opens the report on `name` from database `database` of `page`.
pub fn open(
    page: WeakEntity<DatabasePage>,
    database: String,
    name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let title = SharedString::from(name.clone());
    let report = cx.new(|cx| PlayerReport::new(page, database, name, cx));
    widgets::open_dialog(window, cx, move |dialog, _, _| {
        dialog
            .title(title.clone())
            .w(px(680.))
            .child(report.clone())
    });
}

pub struct PlayerReport {
    page: WeakEntity<DatabasePage>,
    database: String,
    name: String,
    report: Option<GameDbPlayerReport>,
    loading: Loading,
    chart: Entity<LineChart>,
}

impl PlayerReport {
    fn new(
        page: WeakEntity<DatabasePage>,
        database: String,
        name: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let chart = cx.new(|_| LineChart::new(px(140.), |rating| format!("{}", rating.round())));
        let mut this = PlayerReport {
            page,
            database: database.clone(),
            name: name.clone(),
            report: None,
            loading: Loading::default(),
            chart,
        };
        let request = cx.global::<AppCore>().request(calls::GameDbPlayer {
            database: DatabaseId(database),
            name: PlayerName(name),
        });
        this.loading.load(
            cx,
            "database",
            |this| &mut this.loading,
            request,
            |this, answer, cx| {
                let report = answer.ok();
                if let Some(report) = &report {
                    this.chart_ratings(report, cx);
                }
                this.report = report;
            },
        );
        this
    }

    fn chart_ratings(&self, report: &GameDbPlayerReport, cx: &mut Context<Self>) {
        let points: Vec<ChartPoint> = report
            .ratings
            .iter()
            .map(|point| {
                let month = MONTHS[usize::from(point.month.clamp(1, 12)) - 1];
                ChartPoint {
                    value: f64::from(point.rating),
                    short: format!("{month} {:02}", point.year.rem_euclid(100)).into(),
                    long: format!("{month} {}", point.year).into(),
                }
            })
            .collect();
        let domain =
            kchess_board::rating_history::rating_domain(points.iter().map(|point| point.value));
        let summary = match (report.ratings.first(), report.ratings.last()) {
            (Some(first), Some(last)) => format!(
                "Rating from {} in {} to {} in {}",
                first.rating, first.year, last.rating, last.year
            ),
            _ => String::new(),
        };
        self.chart
            .update(cx, |chart, cx| chart.set(points, domain, summary, cx));
    }

    fn show_games(&self, window: &mut Window, cx: &mut App) {
        let (database, name) = (self.database.clone(), self.name.clone());
        window.close_dialog(cx);
        self.page
            .update(cx, |page, cx| {
                page.show_database(database, name, window, cx)
            })
            .ok();
    }

    fn open_game(&self, row: GameDbRow, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
        self.page
            .update(cx, |page, cx| page.open_game(&row, window, cx))
            .ok();
    }

    fn side(
        id: &'static str,
        title: &'static str,
        side: &GameDbSideReport,
        palette: &Palette,
    ) -> Div {
        let muted = palette.text_muted;
        let openings = side.openings.iter().enumerate().map(|(index, opening)| {
            let score = score(&opening.record)
                .map(|score| format!("{score}%"))
                .unwrap_or_default();
            let said = format!(
                "{}, {}, {}",
                opening.name,
                widgets::format_counted(opening.record.games as f64, "game", "games"),
                record_text(&opening.record)
            );
            h_flex()
                .id((id, index))
                .test_support()
                .aria_label(said)
                .gap(px(8.))
                .text_size(px(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(opening.name.clone()),
                )
                .child(
                    div()
                        .w(px(40.))
                        .flex_none()
                        .text_right()
                        .text_color(muted)
                        .child(opening.record.games.to_string()),
                )
                .child(div().w(px(40.)).flex_none().text_right().child(score))
        });
        v_flex()
            .flex_1()
            .min_w(px(240.))
            .gap(px(6.))
            .child(widgets::heading(title, palette))
            .child(
                div()
                    .id(SharedString::new_static(id))
                    .test_support()
                    .aria_label(format!(
                        "{title}: {}",
                        widgets::format_counted(side.record.games as f64, "game", "games")
                    ))
                    .text_size(px(12.))
                    .text_color(muted)
                    .child(format!(
                        "{} · {}",
                        widgets::format_counted(side.record.games as f64, "game", "games"),
                        record_text(&side.record)
                    )),
            )
            .children(openings)
    }

    fn best(&self, report: &GameDbPlayerReport, cx: &mut Context<Self>, palette: &Palette) -> Div {
        let rows = report.best.iter().enumerate().map(|(index, row)| {
            let white = row.white == report.name;
            let (opponent, rating) = if white {
                (&row.black, row.black_elo)
            } else {
                (&row.white, row.white_elo)
            };
            let detail = [
                rating.map(|rating| rating.to_string()),
                row.event.clone(),
                row.date
                    .as_deref()
                    .map(|date| date.chars().take(4).collect()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<String>>()
            .join(" · ");
            let chosen = row.clone();
            h_flex()
                .id(("report-best", index))
                .test_support()
                .role(Role::Button)
                .keyboard_focus(palette.primary)
                .aria_label(format!("Won against {opponent}"))
                .gap(px(8.))
                .px(px(8.))
                .py(px(6.))
                .rounded(radii::BOX)
                .text_size(px(12.))
                .cursor_pointer()
                .pressable(palette)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_game(chosen.clone(), window, cx)
                }))
                .child(div().flex_none().child(if white { "1-0" } else { "0-1" }))
                .child(div().flex_none().child(opponent.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(palette.text_muted)
                        .child(detail),
                )
        });
        v_flex()
            .gap(px(2.))
            .child(widgets::heading("Best wins", palette))
            .children(rows)
    }
}

impl Render for PlayerReport {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let Some(report) = self.report.clone() else {
            let body = match self.loading.error() {
                Some(error) => div()
                    .text_color(palette.error)
                    .child(error.clone())
                    .into_any_element(),
                None => widgets::loading("Reading their games…", &palette).into_any_element(),
            };
            return v_flex().gap(px(12.)).child(body);
        };
        let games = report.white.record.games + report.black.record.games;
        let summary = [
            Some(widgets::format_counted(games as f64, "game", "games")),
            years(&report),
            report
                .ratings
                .last()
                .map(|last| format!("Rated {}", last.rating)),
            report
                .ratings
                .iter()
                .map(|point| point.rating)
                .max()
                .filter(|_| report.ratings.len() > 1)
                .map(|peak| format!("Peak {peak}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<String>>()
        .join(" · ");
        v_flex()
            .gap(px(16.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .id("report-summary")
                            .test_support()
                            .aria_label(summary.clone())
                            .flex_1()
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(summary),
                    )
                    .child(
                        Button::new("report-games")
                            .outline()
                            .small()
                            .icon(icon("search"))
                            .label("Show games")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.show_games(window, cx)),
                            ),
                    ),
            )
            .when(report.ratings.len() > 1, |body| {
                body.child(self.chart.clone())
            })
            .child(
                h_flex()
                    .flex_wrap()
                    .items_start()
                    .gap(px(24.))
                    .child(Self::side(
                        "report-white",
                        "As White",
                        &report.white,
                        &palette,
                    ))
                    .child(Self::side(
                        "report-black",
                        "As Black",
                        &report.black,
                        &palette,
                    )),
            )
            .when(!report.best.is_empty(), |body| {
                body.child(self.best(&report, cx, &palette))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{record_text, score};
    use kchess_contracts::types::GameDbRecord;

    #[test]
    fn a_record_reads_with_its_score() {
        let record = GameDbRecord {
            games: 21,
            wins: 12,
            draws: 5,
            losses: 3,
        };
        assert_eq!(record_text(&record), "Won 12 · Drawn 5 · Lost 3 · 73%");
        assert_eq!(score(&GameDbRecord::default()), None);
        assert_eq!(
            record_text(&GameDbRecord {
                games: 1,
                ..GameDbRecord::default()
            }),
            "Won 0 · Drawn 0 · Lost 0"
        );
    }
}
