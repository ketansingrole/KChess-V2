//! Practice → Repertoire → Check → Against your games: your games with the repertoire's side,
//! followed through it until they left it, and the places they left it listed, the most often
//! first: a move of yours the repertoire does not play, or your opponent's reply it has no answer
//! to. Against a player (their database from Player's games): their games with the other side,
//! and the moves they chose where the repertoire waits for theirs, those it has no answer to
//! first. A listed move opens on the analysis board, and one the repertoire lacks can join it.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use kchess_contracts::calls;
use kchess_contracts::check::{DatabaseId, RepertoireSteps, Side};
use kchess_contracts::error::CoreError;
use kchess_contracts::types::{GameDbRepertoireGames, GameDbRepertoireGap, RepertoireStep};
use shakmaty::Color;

use super::check::{Step, Target, add_button, add_to_repertoire, line_row, open_line};
use crate::core::AppCore;
use crate::loading::Loading;
use crate::theme::Palette;
use crate::widgets;

/// Whose games a check follows.
#[derive(Clone, Debug, PartialEq)]
pub enum Whose {
    /// Yours, in Your games.
    Yours,
    /// A player's, in their database from Player's games: its id, and their name.
    Player { database: String, name: String },
}

/// A score in percent from `points` of `decided` games; none without one.
fn percent(points: f64, decided: u64) -> Option<String> {
    (decided > 0).then(|| format!("{}%", (points * 100. / decided as f64).round()))
}

/// Why `gap` is listed: whose move it was, how its games went, the repertoire's move instead when
/// it was yours or that it has none when it was not, and when it was last played.
pub fn reason(gap: &GameDbRepertoireGap, step: &Step, whose: &Whose) -> String {
    let games = widgets::format_counted(gap.games as f64, "game", "games");
    let decided = gap.won + gap.drawn + gap.lost;
    let mut parts = Vec::new();
    match whose {
        Whose::Yours => {
            parts.push(if gap.yours {
                format!("You played it in {games}")
            } else {
                format!("Played against you in {games}")
            });
            let points = gap.won as f64 + gap.drawn as f64 / 2.;
            parts.extend(percent(points, decided).map(|score| format!("You scored {score}")));
        }
        Whose::Player { name, .. } => {
            parts.push(format!("{name} played it in {games}"));
            // The results are the repertoire's side's: theirs the other way round.
            let points = gap.lost as f64 + gap.drawn as f64 / 2.;
            parts.extend(percent(points, decided).map(|score| format!("{name} scored {score}")));
        }
    }
    if gap.yours {
        let prepared: Vec<&str> = step.moves.iter().map(|(_, san, _)| san.as_str()).collect();
        parts.push(format!("Repertoire: {}", prepared.join(", ")));
    } else if !gap.prepared {
        parts.push("No answer in the repertoire".to_string());
    }
    parts.extend(gap.last.as_ref().map(|date| format!("Last\u{a0}{date}")));
    parts.join(" · ")
}

/// Checks `steps`, the repertoire of `color`, against `whose` games in a dialog.
pub fn open(
    steps: Vec<Step>,
    target: Target,
    color: Color,
    whose: Whose,
    window: &mut Window,
    cx: &mut App,
) {
    let title: SharedString = match &whose {
        Whose::Yours => SharedString::new_static("Check against your games"),
        Whose::Player { name, .. } => format!("Check against {name}").into(),
    };
    let view = cx.new(|cx| YourGamesCheck::new(steps, target, color, whose, cx));
    widgets::open_dialog(window, cx, move |dialog, _, _| {
        dialog.title(title.clone()).w(px(600.)).child(view.clone())
    });
}

pub struct YourGamesCheck {
    /// The scroll of the list inside it, kept to the list (`widgets::nested_scroll`).
    list_scroll: ScrollHandle,
    steps: Vec<Step>,
    target: Target,
    color: Color,
    whose: Whose,
    answer: Option<GameDbRepertoireGames>,
    loading: Loading,
}

impl YourGamesCheck {
    fn new(
        steps: Vec<Step>,
        target: Target,
        color: Color,
        whose: Whose,
        cx: &mut Context<Self>,
    ) -> Self {
        let sent = steps
            .iter()
            .map(|step| RepertoireStep {
                fen: step.fen.clone(),
                moves: step.moves.iter().map(|(uci, ..)| uci.clone()).collect(),
            })
            .collect();
        type Answer =
            std::pin::Pin<Box<dyn Future<Output = Result<GameDbRepertoireGames, CoreError>>>>;
        let core = cx.global::<AppCore>();
        let request: Answer = match &whose {
            Whose::Yours => Box::pin(core.request(calls::GameDbRepertoireGames {
                steps: RepertoireSteps(sent),
                color: Side(color),
            })),
            Whose::Player { database, .. } => {
                Box::pin(core.request(calls::GameDbRepertoirePlayer {
                    steps: RepertoireSteps(sent),
                    color: Side(color),
                    database: DatabaseId(database.clone()),
                }))
            }
        };
        let mut this = YourGamesCheck {
            list_scroll: ScrollHandle::new(),
            steps,
            target,
            color,
            whose,
            answer: None,
            loading: Loading::default(),
        };
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
        let side = |color: Color| {
            if color == Color::White {
                "White"
            } else {
                "Black"
            }
        };
        match &self.whose {
            Whose::Yours => {
                let side = side(self.color);
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
            Whose::Player { name, .. } => {
                let side = side(!self.color);
                if answer.games == 0 {
                    return format!("None of {name}'s games as {side}");
                }
                let unanswered = answer.gaps.iter().filter(|gap| !gap.prepared).count();
                [
                    format!(
                        "{} as {side}",
                        widgets::format_counted(answer.games as f64, "game", "games")
                    ),
                    format!("{} in the repertoire", answer.reached),
                    widgets::format_counted(
                        unanswered as f64,
                        "move without an answer",
                        "moves without an answer",
                    ),
                ]
                .join(" · ")
            }
        }
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
                None => {
                    let text = match &self.whose {
                        Whose::Yours => "Going through your games…".to_string(),
                        Whose::Player { name, .. } => format!("Going through {name}'s games…"),
                    };
                    widgets::loading(text, &palette).into_any_element()
                }
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
                let row = line_row(
                    ("repertoire-games-move", index),
                    &step,
                    &gap.san,
                    &reason(gap, &step, &self.whose),
                    &palette,
                )
                .on_click({
                    let (step, uci) = (step.clone(), uci.clone());
                    move |_, window, cx| open_line(&step, &uci, window, cx)
                })
                .test_support();
                // A move the repertoire already has needs no adding.
                let target = self.target.clone();
                let add = (!gap.prepared).then(|| {
                    add_button(("repertoire-games-add", index), &gap.san).on_click(
                        move |_, window, cx| add_to_repertoire(&target, &step, &uci, window, cx),
                    )
                });
                Some(
                    h_flex()
                        .gap(px(4.))
                        .child(row.flex_1().min_w_0())
                        .children(add)
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
                widgets::nested_scroll(
                    v_flex().id("repertoire-games-moves").max_h(px(440.)),
                    &self.list_scroll,
                )
                .gap(px(2.))
                .children(rows),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{GameDbRepertoireGap, Step, Whose, reason};

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
            reason(&gap, &step(&["Nf3", "d4"]), &Whose::Yours),
            "You played it in 2 games · You scored 25% · Repertoire: Nf3, d4 · Last\u{a0}2024.01.06"
        );
        gap.yours = false;
        gap.last = None;
        (gap.won, gap.drawn, gap.lost) = (0, 0, 0);
        assert_eq!(
            reason(&gap, &step(&["Nf3"]), &Whose::Yours),
            "Played against you in 2 games · No answer in the repertoire"
        );
    }

    #[test]
    fn a_players_move_says_how_they_scored_and_whether_it_is_prepared() {
        let whose = Whose::Player {
            database: "db".into(),
            name: "Rival".into(),
        };
        let mut gap = GameDbRepertoireGap {
            san: "c5".into(),
            games: 4,
            won: 1,
            drawn: 1,
            lost: 2,
            ..GameDbRepertoireGap::default()
        };
        // Results are the repertoire's: Rival won 2 and drew 1 of 4.
        assert_eq!(
            reason(&gap, &step(&["Nf3"]), &whose),
            "Rival played it in 4 games · Rival scored 63% · No answer in the repertoire"
        );
        gap.prepared = true;
        assert_eq!(
            reason(&gap, &step(&["Nf3"]), &whose),
            "Rival played it in 4 games · Rival scored 63%"
        );
    }
}
