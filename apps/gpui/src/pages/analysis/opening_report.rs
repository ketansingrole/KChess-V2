//! The opening report on the board's position (More → Opening report): through every game
//! database but Your games, how its games ended, its main lines, each played onto the board
//! when chosen, and its strongest games, each opened.

use super::*;
use gpui_kit::component::WindowExt as _;
use kchess_contracts::check::Fen;
use kchess_contracts::types::{
    GameDbFoundGame, GameDbOpeningLine, GameDbOpeningReport, LookupMove,
};
use kchess_domain::tree::pv_san;
use kchess_domain::variant::Variant;

/// What the report's result bar is given: a move's counts as the explorer draws them.
fn results(white: u64, draws: u64, black: u64) -> LookupMove {
    let count = |n: u64| Some(i64::try_from(n).unwrap_or(i64::MAX));
    LookupMove {
        white: count(white),
        draws: count(draws),
        black: count(black),
        ..LookupMove::default()
    }
}

/// A main line with its move numbers, from the position `fen`: "12. Nf3 Nc6 13. Bb5".
pub(super) fn line_text(fen: &str, line: &GameDbOpeningLine) -> String {
    let ucis: Vec<&str> = line.moves.iter().map(|m| m.uci.as_str()).collect();
    pv_san(fen, &ucis, ucis.len(), Variant::Standard)
        .into_iter()
        .map(|m| m.label)
        .collect::<Vec<_>>()
        .join(" ")
}

impl AnalysisPage {
    /// The report on the position shown.
    pub(super) fn open_opening_report(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fen = self.node().fen.clone();
        let page = cx.entity().downgrade();
        let report = cx.new(|cx| OpeningReport::new(page, fen, cx));
        widgets::open_dialog(window, cx, move |dialog, _, _| {
            dialog
                .title("Opening report")
                .w(px(620.))
                .child(report.clone())
        });
    }
}

pub(super) struct OpeningReport {
    page: WeakEntity<AnalysisPage>,
    fen: String,
    report: Option<GameDbOpeningReport>,
    loading: crate::loading::Loading,
}

impl OpeningReport {
    fn new(page: WeakEntity<AnalysisPage>, fen: String, cx: &mut Context<Self>) -> Self {
        let mut this = OpeningReport {
            page,
            fen: fen.clone(),
            report: None,
            loading: crate::loading::Loading::default(),
        };
        let request = cx
            .global::<AppCore>()
            .request(calls::GameDbOpeningReport { fen: Fen(fen) });
        this.loading.load(
            cx,
            "analysis",
            |this| &mut this.loading,
            request,
            |this, answer, _| this.report = answer.ok(),
        );
        this
    }

    fn play(&self, line: &GameDbOpeningLine, window: &mut Window, cx: &mut App) {
        let ucis: Vec<String> = line.moves.iter().map(|m| m.uci.clone()).collect();
        window.close_dialog(cx);
        self.page
            .update(cx, |page, cx| {
                page.play_line(&ucis, ucis.len().saturating_sub(1), cx)
            })
            .ok();
    }

    fn line_row(
        &self,
        index: usize,
        line: &GameDbOpeningLine,
        cx: &mut Context<Self>,
        palette: &Palette,
    ) -> impl IntoElement + use<> {
        let text = line_text(&self.fen, line);
        let games = line.white + line.draws + line.black;
        let chosen = line.clone();
        h_flex()
            .id(("opening-line", index))
            .test_support()
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(format!("Play {text}"))
            .gap(px(12.))
            .px(px(8.))
            .py(px(6.))
            .rounded(radii::BOX)
            .text_size(px(13.))
            .cursor_pointer()
            .pressable(palette)
            .on_click(cx.listener(move |this, _, window, cx| this.play(&chosen, window, cx)))
            .child(div().flex_1().min_w_0().child(text))
            .child(
                div()
                    .w(px(56.))
                    .flex_none()
                    .text_right()
                    .text_size(px(12.))
                    .text_color(palette.text_muted)
                    .child(widgets::format_count(games as f64)),
            )
            .child(
                div()
                    .w(px(140.))
                    .flex_none()
                    .flex()
                    .child(crate::explorer::result_bar(
                        &results(line.white, line.draws, line.black),
                        20,
                        palette,
                    )),
            )
    }

    fn game_row(
        index: usize,
        found: &GameDbFoundGame,
        palette: &Palette,
    ) -> impl IntoElement + use<> {
        let game = &found.game;
        let player = |name: &str, elo: Option<u32>| match elo {
            Some(elo) => format!("{name} {elo}"),
            None => name.to_string(),
        };
        let detail = [
            game.event.clone(),
            game.date
                .as_deref()
                .map(|date| date.chars().take(4).collect()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<String>>()
        .join(" · ");
        let (database, row) = (found.database.clone(), game.clone());
        h_flex()
            .id(("opening-best", index))
            .test_support()
            .role(Role::Button)
            .keyboard_focus(palette.primary)
            .aria_label(format!(
                "{} against {}, {}",
                game.white, game.black, game.result
            ))
            .gap(px(12.))
            .px(px(8.))
            .py(px(6.))
            .rounded(radii::BOX)
            .text_size(px(12.))
            .cursor_pointer()
            .pressable(palette)
            .on_click(move |_, window, cx| {
                window.close_dialog(cx);
                crate::pages::database::open_game(database.clone(), &row, window, cx);
            })
            .child(div().flex_1().min_w_0().truncate().child(format!(
                "{} – {}",
                player(&game.white, game.white_elo),
                player(&game.black, game.black_elo)
            )))
            .child(div().flex_none().child(game.result.clone()))
            .child(
                div()
                    .w(px(180.))
                    .flex_none()
                    .truncate()
                    .text_color(palette.text_muted)
                    .child(detail),
            )
    }
}

impl Render for OpeningReport {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let Some(report) = self.report.clone() else {
            let body = match self.loading.error() {
                Some(error) => div()
                    .text_color(palette.error)
                    .child(error.clone())
                    .into_any_element(),
                None => widgets::loading("Reading the databases…", &palette).into_any_element(),
            };
            return v_flex().child(body);
        };
        let games = report.white + report.draws + report.black;
        if games == 0 {
            return v_flex().child(widgets::empty_state(
                icon("database"),
                "No games reached this position",
                None,
                &palette,
            ));
        }
        let summary = widgets::format_counted(games as f64, "game", "games");
        let lines: Vec<AnyElement> = report
            .lines
            .iter()
            .enumerate()
            .map(|(index, line)| self.line_row(index, line, cx, &palette).into_any_element())
            .collect();
        let best: Vec<AnyElement> = report
            .best
            .iter()
            .enumerate()
            .map(|(index, found)| Self::game_row(index, found, &palette).into_any_element())
            .collect();
        v_flex()
            .gap(px(16.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .id("opening-games")
                            .test_support()
                            .aria_label(summary.clone())
                            .text_size(px(13.))
                            .text_color(palette.text_muted)
                            .child(summary),
                    )
                    .child(div().flex_1().flex().child(crate::explorer::result_bar(
                        &results(report.white, report.draws, report.black),
                        8,
                        &palette,
                    ))),
            )
            .child(
                v_flex()
                    .gap(px(2.))
                    .child(widgets::heading("Main lines", &palette))
                    .children(lines),
            )
            .when(!best.is_empty(), |body| {
                body.child(
                    v_flex()
                        .gap(px(2.))
                        .child(widgets::heading("Strongest games", &palette))
                        .children(best),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::line_text;
    use kchess_contracts::types::{GameDbOpeningLine, NoveltyMove};

    #[test]
    fn a_main_line_reads_with_its_move_numbers() {
        let line = |ucis: &[&str]| GameDbOpeningLine {
            moves: ucis
                .iter()
                .map(|uci| NoveltyMove {
                    san: String::new(),
                    uci: uci.to_string(),
                    games: 1,
                })
                .collect(),
            white: 1,
            draws: 0,
            black: 0,
        };
        let start = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
        assert_eq!(
            line_text(start, &line(&["e2e4", "e7e5", "g1f3"])),
            "1. e4 e5 2. Nf3"
        );
        let black = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
        assert_eq!(line_text(black, &line(&["c7c5", "g1f3"])), "1… c5 2. Nf3");
    }
}
