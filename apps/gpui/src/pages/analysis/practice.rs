//! Practising a reviewed game's mistakes on the analysis board (`stores/reviewPractice.ts`,
//! `ReviewPractice.vue`), as Lichess's "learn from your mistakes" does: the game's positions in
//! practice, in move order, each with the game's moves and chart around it. Answers schedule the
//! positions as Practice → Your mistakes does (`answerMistake`).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_core::Level;
use serde_json::{Value, json};
use shakmaty::{Color, Square};

use super::AnalysisPage;
use super::review::{classification_look, eval_chart};
use crate::assets::icon;
use crate::core::AppCore;
use crate::game::tree::path_of;
use crate::theme::{Palette, judgment_color};
use crate::widgets;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Find,
    Wrong,
    Solved,
    Revealed,
    Done,
}

/// What the board draws while a position is being solved: the game's move, and the piece to move
/// once hinted.
pub struct PracticeShapes {
    pub played: Option<(Square, Square)>,
    pub hint: Option<Square>,
}

pub struct Practice {
    /// The review being practised.
    pub key: String,
    /// Its positions in practice, in move order: the mistake items and their plies.
    positions: Vec<(usize, Value)>,
    index: usize,
    pub status: Status,
    /// The last wrong move tried here, as SAN.
    tried: String,
    hinted: bool,
    /// A wrong move or a hint here: the position is not counted as found.
    helped: bool,
    /// Per position: found without help.
    found: Vec<bool>,
    /// The board's orientation before practice, given back when it stops.
    orientation: Color,
}

/// A move's SAN from a FEN.
fn san(fen: &str, uci: &str) -> Option<String> {
    let position = kchess_domain::rules::setup_start("standard", fen)?;
    kchess_domain::position::play_uci(&position, uci).map(|played| played.san)
}

impl Practice {
    fn current(&self) -> Option<&(usize, Value)> {
        self.positions.get(self.index)
    }

    /// The solution's first move: the one to find.
    fn answer(&self) -> Option<String> {
        self.current()?.1["solution"][0]
            .as_str()
            .map(str::to_string)
    }

    /// Solving: the engine (and anything else that knows) would tell.
    pub fn hides_engine(&self) -> bool {
        matches!(self.status, Status::Find | Status::Wrong)
    }
}

impl AnalysisPage {
    pub fn start_practice(&mut self, key: &str, cx: &mut Context<Self>) {
        let prefix = format!("{key}:");
        let mut positions: Vec<(usize, Value)> = self
            .review
            .mistakes
            .iter()
            .filter_map(|item| {
                let ply = item["id"].as_str()?.strip_prefix(&prefix)?.parse().ok()?;
                Some((ply, item.clone()))
            })
            .collect();
        positions.sort_by_key(|(ply, _)| *ply);
        if positions.is_empty() {
            self.stop_practice(cx);
            return;
        }
        let orientation = self
            .practice
            .as_ref()
            .map_or(self.orientation, |practice| practice.orientation);
        self.practice = Some(Practice {
            key: key.to_string(),
            positions,
            index: 0,
            status: Status::Find,
            tried: String::new(),
            hinted: false,
            helped: false,
            found: Vec::new(),
            orientation,
        });
        self.begin_position(cx);
    }

    pub(super) fn stop_practice(&mut self, cx: &mut Context<Self>) {
        if let Some(practice) = self.practice.take() {
            self.orientation = practice.orientation;
            self.redraw(cx);
        }
    }

    /// Another game on the board, or a live game starting, ends practice. Moves added after the
    /// end of the game keep its review, and so its practice.
    pub(super) fn check_practice(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.practice.as_ref().map(|practice| practice.key.clone()) else {
            return;
        };
        let shown = self
            .review(cx)
            .and_then(|review| review["key"].as_str().map(str::to_string));
        if (self.mainline_key() != key && shown.as_deref() != Some(key.as_str()))
            || !Self::assistance_allowed(cx)
        {
            self.stop_practice(cx);
        }
    }

    /// The move played instead: its numbered SAN, its label and who played it.
    fn played(&self) -> Option<(String, String, String, Color)> {
        let (ply, item) = self.practice.as_ref()?.current()?;
        let uci = self.mainline().get(*ply)?.clone();
        let fen = item["fen"].as_str()?;
        let mut fields = fen.split(' ');
        let color = if fields.nth(1) == Some("b") {
            Color::Black
        } else {
            Color::White
        };
        let number = fen.split(' ').nth(5).unwrap_or("1");
        let label = format!(
            "{number}{} {}",
            if color == Color::Black { "…" } else { "." },
            san(fen, &uci)?
        );
        Some((
            uci,
            label,
            item["judgment"].as_str().unwrap_or("mistake").to_string(),
            color,
        ))
    }

    /// The board is at the position before the mistake (on the main line, which is the game).
    fn at_position(&self) -> bool {
        self.practice
            .as_ref()
            .and_then(Practice::current)
            .is_some_and(|(ply, _)| self.path == path_of(&self.mainline()[..*ply]))
    }

    /// The game's move, drawn on the board while a better one is found, and the piece to move once
    /// a hint is asked for.
    pub(super) fn practice_shapes(&self) -> Option<PracticeShapes> {
        let practice = self
            .practice
            .as_ref()
            .filter(|practice| practice.hides_engine())?;
        if !self.at_position() {
            return Some(PracticeShapes {
                played: None,
                hint: None,
            });
        }
        let square = |uci: &str, range: std::ops::Range<usize>| uci.get(range)?.parse().ok();
        let played = self
            .played()
            .and_then(|(uci, ..)| Some((square(&uci, 0..2)?, square(&uci, 2..4)?)));
        let hint = practice
            .hinted
            .then(|| practice.answer())
            .flatten()
            .and_then(|answer| square(&answer, 0..2));
        Some(PracticeShapes { played, hint })
    }

    fn show_position(&mut self, cx: &mut Context<Self>) {
        let Some(ply) = self
            .practice
            .as_ref()
            .and_then(Practice::current)
            .map(|(ply, _)| *ply)
        else {
            return;
        };
        if let Some((_, _, _, color)) = self.played() {
            self.orientation = color;
        }
        self.go_to_ply(ply, cx);
    }

    fn begin_position(&mut self, cx: &mut Context<Self>) {
        if let Some(practice) = &mut self.practice {
            practice.status = Status::Find;
            practice.tried.clear();
            practice.hinted = false;
            practice.helped = false;
        }
        self.show_position(cx);
    }

    /// A move made on the board at the position being solved: `Some(kept)` when practice took it
    /// (the answer is played as a variation; a wrong move is not kept), `None` to play it as usual.
    pub(super) fn practice_attempt(&mut self, uci: &str, cx: &mut Context<Self>) -> Option<bool> {
        if !self.at_position() || !self.practice.as_ref().is_some_and(Practice::hides_engine) {
            return None;
        }
        let practice = self.practice.as_mut()?;
        let (_, item) = practice.current()?.clone();
        if Some(uci.to_string()) == practice.answer() {
            let clean = !practice.helped;
            let index = practice.index;
            if practice.found.len() <= index {
                practice.found.resize(index + 1, false);
            }
            practice.found[index] = clean;
            practice.status = Status::Solved;
            self.answer_mistake(&item, clean, cx);
            return Some(self.play_move(uci, cx));
        }
        practice.tried = item["fen"]
            .as_str()
            .and_then(|fen| san(fen, uci))
            .unwrap_or_else(|| uci.to_string());
        practice.helped = true;
        practice.status = Status::Wrong;
        self.redraw(cx);
        Some(false)
    }

    fn answer_mistake(&mut self, item: &Value, clean: bool, cx: &mut Context<Self>) {
        let answered = cx
            .global::<AppCore>()
            .call("answerMistake", vec![item["id"].clone(), json!(clean)]);
        cx.spawn(async move |this, cx| match answered.await {
            Ok(items) => {
                let _ = this.update(cx, |this, _| {
                    this.review.mistakes = items.as_array().cloned().unwrap_or_default();
                });
            }
            Err(cause) => crate::log::write(
                Level::Warn,
                "mistakes",
                &format!("saving drills failed: {}", cause.message),
            ),
        })
        .detach();
    }

    fn practice_hint(&mut self, cx: &mut Context<Self>) {
        if let Some(practice) = &mut self.practice {
            practice.hinted = true;
            practice.helped = true;
        }
        if self.at_position() {
            self.redraw(cx);
        } else {
            self.show_position(cx);
        }
    }

    fn practice_reveal(&mut self, cx: &mut Context<Self>) {
        let Some(practice) = &mut self.practice else {
            return;
        };
        let (Some((_, item)), Some(answer)) = (practice.current().cloned(), practice.answer())
        else {
            return;
        };
        let index = practice.index;
        if practice.found.len() <= index {
            practice.found.resize(index + 1, false);
        }
        practice.found[index] = false;
        practice.status = Status::Revealed;
        self.answer_mistake(&item, false, cx);
        self.show_position(cx);
        self.play_move(&answer, cx);
    }

    fn practice_next(&mut self, cx: &mut Context<Self>) {
        let Some(practice) = &mut self.practice else {
            return;
        };
        if practice.index + 1 >= practice.positions.len() {
            practice.status = Status::Done;
            self.redraw(cx);
            return;
        }
        practice.index += 1;
        self.begin_position(cx);
    }

    fn practice_again(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.practice.as_ref().map(|practice| practice.key.clone()) {
            self.start_practice(&key, cx);
        }
    }

    /// The practice card (`ReviewPractice.vue`), in place of the engine's lines.
    pub(super) fn render_practice(&self, cx: &mut Context<Self>, palette: &Palette) -> AnyElement {
        let Some(practice) = &self.practice else {
            return div().into_any_element();
        };
        let player = self
            .review
            .origin
            .as_ref()
            .and_then(|origin| origin["player"].as_str())
            .map(|player| {
                if player == "black" {
                    Color::Black
                } else {
                    Color::White
                }
            });
        let named = self.review.origin.is_some()
            || !self.header("White").is_empty()
            || !self.header("Black").is_empty();
        let players = named.then(|| {
            h_flex()
                .flex_wrap()
                .gap_x(px(16.))
                .gap_y(px(4.))
                .text_size(px(13.))
                .children([Color::White, Color::Black].map(|color| {
                    h_flex()
                        .gap(px(6.))
                        .min_w_0()
                        .child(widgets::side_dot(color))
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(self.player_name(color)),
                        )
                        .when(player == Some(color), |row| {
                            row.child(
                                div()
                                    .px(px(5.))
                                    .rounded(px(4.))
                                    .bg(palette.accented)
                                    .text_size(px(11.))
                                    .text_color(palette.text_muted)
                                    .child("you"),
                            )
                        })
                }))
        });
        let count = practice.positions.len();
        let head = h_flex()
            .gap(px(8.))
            .min_h(px(24.))
            .text_size(px(13.))
            .child(div().font_weight(FontWeight::BOLD).child("Practice"))
            .when(practice.status != Status::Done, |head| {
                head.child(
                    div()
                        .text_size(px(12.))
                        .text_color(palette.text_muted)
                        .child(format!("{} of {count}", practice.index + 1)),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("stop-practice")
                    .ghost()
                    .xsmall()
                    .icon(icon("x"))
                    .tooltip("Stop practice")
                    .accessibility_label("Stop practice")
                    .on_click(cx.listener(|this, _, _, cx| this.stop_practice(cx))),
            );
        let status_line = |text: AnyElement, color: Hsla| {
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(text)
        };
        let answer_san = practice
            .current()
            .and_then(|(_, item)| san(item["fen"].as_str()?, &practice.answer()?))
            .unwrap_or_default();
        let body: Vec<AnyElement> = if practice.status == Status::Done {
            let found = practice.found.iter().filter(|found| **found).count();
            vec![
                status_line(
                    format!("You found {found} of {count}.").into_any_element(),
                    palette.text,
                )
                .into_any_element(),
                h_flex()
                    .gap(px(6.))
                    .child(
                        Button::new("practice-again")
                            .primary()
                            .small()
                            .label("Practice again")
                            .on_click(cx.listener(|this, _, _, cx| this.practice_again(cx))),
                    )
                    .child(
                        Button::new("practice-done")
                            .outline()
                            .small()
                            .label("Done")
                            .on_click(cx.listener(|this, _, _, cx| this.stop_practice(cx))),
                    )
                    .into_any_element(),
            ]
        } else {
            let played = self.played().map(|(_, label, judgment, color)| {
                let (_, glyph, tone) = classification_look(&judgment, palette.dark);
                let mover = if player == Some(color) {
                    "You".to_string()
                } else {
                    self.player_name(color)
                };
                h_flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .text_size(px(13.))
                    .text_color(palette.text_muted)
                    .child(format!("{mover} played"))
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(tone)
                            .child(format!("{label}{glyph}")),
                    )
                    .child(format!(
                        ", {} {}.",
                        if judgment == "inaccuracy" { "an" } else { "a" },
                        judgment
                    ))
                    .into_any_element()
            });
            let status = match practice.status {
                Status::Find => status_line("Find a better move.".into_any_element(), palette.text),
                Status::Wrong => status_line(
                    format!("{} isn't the best move. Try again.", practice.tried)
                        .into_any_element(),
                    judgment_color("blunder", palette.dark),
                ),
                Status::Solved => status_line(
                    h_flex()
                        .gap(px(4.))
                        .child(icon("check").size(px(16.)))
                        .child(format!("{answer_san} is the best move."))
                        .into_any_element(),
                    palette.success,
                ),
                _ => status_line(
                    format!("The best move was {answer_san}.").into_any_element(),
                    palette.text,
                ),
            };
            let last = practice.index + 1 >= count;
            let actions = if practice.hides_engine() {
                if self.at_position() {
                    h_flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(
                            Button::new("practice-hint")
                                .small()
                                .when(practice.hinted, |button| button.primary())
                                .when(!practice.hinted, |button| button.outline())
                                .label("Hint")
                                .on_click(cx.listener(|this, _, _, cx| this.practice_hint(cx))),
                        )
                        .child(
                            Button::new("practice-solution")
                                .outline()
                                .small()
                                .label("Show solution")
                                .on_click(cx.listener(|this, _, _, cx| this.practice_reveal(cx))),
                        )
                        .child(
                            Button::new("practice-skip")
                                .ghost()
                                .small()
                                .label("Skip")
                                .on_click(cx.listener(|this, _, _, cx| this.practice_next(cx))),
                        )
                } else {
                    h_flex().child(
                        Button::new("practice-back")
                            .primary()
                            .small()
                            .label("Back to the position")
                            .on_click(cx.listener(|this, _, _, cx| this.show_position(cx))),
                    )
                }
            } else {
                h_flex().child(
                    Button::new("practice-next")
                        .primary()
                        .small()
                        .label(if last { "Finish" } else { "Next mistake" })
                        .on_click(cx.listener(|this, _, _, cx| this.practice_next(cx))),
                )
            };
            played
                .into_iter()
                .chain([status.into_any_element(), actions.into_any_element()])
                .collect()
        };
        let chart = self.review_chart(cx).map(|(points, judgments)| {
            let current = self
                .review(cx)
                .and_then(|review| self.review_ply(self.reviewed_plies(&review)));
            eval_chart(points, judgments, current, cx.entity().downgrade(), palette)
        });
        v_flex()
            .gap(px(8.))
            .child(head)
            .children(players)
            .children(body)
            .children(chart)
            .into_any_element()
    }
}
