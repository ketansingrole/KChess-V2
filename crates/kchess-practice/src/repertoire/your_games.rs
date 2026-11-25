//! Practice → Repertoire → Check → Against your games: your games with the repertoire's side,
//! followed through it until they left it, and the places they left it listed, the most often
//! first: a move of yours the repertoire does not play, or your opponent's reply it has no answer
//! to. A listed move opens on the analysis board.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::{RepertoireSteps, Side};
use kchess_contracts::types::{GameDbRepertoireGames, GameDbRepertoireGap, RepertoireStep};
use shakmaty::Color;

use super::check::{Step, line_row, open_line};
use crate::core::AppCore;
use crate::loading::Loading;
use crate::theme::Palette;
use crate::widgets;

/// Why `gap` is listed: whose move it was, how its games went for you, the repertoire's move
/// instead when it was yours, and when you last played it.
pub fn reason(gap: &GameDbRepertoireGap, step: &Step) -> String {
    let games = widgets::format_counted(gap.games as f64, "game", "games");
    let decided = gap.won + gap.drawn + gap.lost;
    let mut parts = vec![if gap.yours {
        format!("You played it in {games}")
    } else {
        format!("Played against you in {games}")
    }];
    if decided > 0 {
        let points = gap.won as f64 + gap.drawn as f64 / 2.;
        parts.push(format!(
            "You scored {}%",
            (points * 100. / decided as f64).round()
        ));
    }
    if gap.yours {
        let prepared: Vec<&str> = step.moves.iter().map(|(_, san, _)| san.as_str()).collect();
        parts.push(format!("Repertoire: {}", prepared.join(", ")));
    } else {
        parts.push("No answer in the repertoire".to_string());
    }
    parts.extend(gap.last.as_ref().map(|date| format!("Last {date}")));
    parts.join(" · ")
}

/// Checks `steps` against your games as `color` in a dialog.
pub fn open(steps: Vec<Step>, color: Color, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| YourGamesCheck::new(steps, color, cx));
    widgets::open_dialog(window, cx, move |dialog, _, _| {
        dialog
            .title(SharedString::new_static("Check against your games"))
            .w(px(600.))
            .child(view.clone())
    });
}

pub struct YourGamesCheck {
    steps: Vec<Step>,
    color: Color,
    answer: Option<GameDbRepertoireGames>,
    loading: Loading,
}

impl YourGamesCheck {
    fn new(steps: Vec<Step>, color: Color, cx: &mut Context<Self>) -> Self {
        let sent = steps
            .iter()
            .map(|step| RepertoireStep {
                fen: step.fen.clone(),
                moves: step.moves.iter().map(|(uci, ..)| uci.clone()).collect(),
            })
            .collect();
        let mut this = YourGamesCheck {
            steps,
            color,
            answer: None,
            loading: Loading::default(),
        };
        let request = cx
            .global::<AppCore>()
            .request(calls::GameDbRepertoireGames {
                steps: RepertoireSteps(sent),
                color: Side(color),
            });
        this.loading.load(
            cx,
            "database",
            |this| &mut this.loading,
            request,
            |this, answer, _| this.answer = answer.ok(),
        );
        this
    }

    fn summary(&self, answer: &GameDbRepertoireGames) -> String {
        let side = if self.color == Color::White {
            "White"
        } else {
            "Black"
        };
        if answer.games == 0 {
            return format!("None of your games as {side}");
        }
        let left: u64 = answer.gaps.iter().map(|gap| gap.games).sum();
        [
            format!(
                "{} as {side}",
                widgets::format_counted(answer.games as f64, "game", "games")
            ),
            format!("{} in the repertoire", answer.reached),
            widgets::format_counted(left as f64, "left it", "left it"),
        ]
        .join(" · ")
    }
}

impl Render for YourGamesCheck {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let Some(answer) = &self.answer else {
            let body = match self.loading.error() {
                Some(error) => div()
                    .text_color(palette.error)
                    .child(error.clone())
                    .into_any_element(),
                None => widgets::loading("Going through your games…", &palette).into_any_element(),
            };
            return v_flex().gap(px(12.)).child(body);
        };
        let summary = self.summary(answer);
        let rows: Vec<AnyElement> = answer
            .gaps
            .iter()
            .enumerate()
            .filter_map(|(index, gap)| {
                let step = self.steps.get(gap.step as usize)?.clone();
                let uci = gap.uci.clone();
                Some(
                    line_row(
                        ("repertoire-games-move", index),
                        &step,
                        &gap.san,
                        &reason(gap, &step),
                        &palette,
                    )
                    .on_click(move |_, window, cx| open_line(&step, &uci, window, cx))
                    .test_support()
                    .into_any_element(),
                )
            })
            .collect();
        v_flex()
            .gap(px(12.))
            .child(
                h_flex().child(
                    div()
                        .id("repertoire-games-summary")
                        .test_support()
                        .aria_label(summary.clone())
                        .text_size(px(13.))
                        .text_color(palette.text_muted)
                        .child(summary),
                ),
            )
            .child(
                v_flex()
                    .id("repertoire-games-moves")
                    .max_h(px(440.))
                    .overflow_y_scroll()
                    .gap(px(2.))
                    .children(rows),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{GameDbRepertoireGap, Step, reason};

    fn step(moves: &[&str]) -> Step {
        Step {
            start: String::new(),
            fen: String::new(),
            ply: 2,
            line: Vec::new(),
            moves: moves
                .iter()
                .map(|san| (String::new(), san.to_string(), String::new()))
                .collect(),
        }
    }

    #[test]
    fn a_gap_says_whose_move_it_was_and_how_it_went() {
        let mut gap = GameDbRepertoireGap {
            san: "Nc3".into(),
            yours: true,
            games: 2,
            drawn: 1,
            lost: 1,
            last: Some("2024.01.06".into()),
            ..GameDbRepertoireGap::default()
        };
        assert_eq!(
            reason(&gap, &step(&["Nf3", "d4"])),
            "You played it in 2 games · You scored 25% · Repertoire: Nf3, d4 · Last 2024.01.06"
        );
        gap.yours = false;
        gap.last = None;
        (gap.won, gap.drawn, gap.lost) = (0, 0, 0);
        assert_eq!(
            reason(&gap, &step(&["Nf3"])),
            "Played against you in 2 games · No answer in the repertoire"
        );
    }
}
