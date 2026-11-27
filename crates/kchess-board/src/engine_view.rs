//! The engine's evaluation as the analysis board and the broadcast board show it: the bar beside
//! the board and the lines under the score.

use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::EngineLine;
use shakmaty::Color;

use crate::game::tree::{format_eval, pv_san, winning_chances};
use crate::theme::{Palette, radii, tint};
use crate::widgets::KeyboardFocus as _;
use crate::widgets::Pressable as _;

/// The evaluation bar's width.
pub const EVAL_BAR: Pixels = px(14.);
/// One engine line's row.
const PV_ROW: Pixels = px(34.);
/// A score chip's and a move's height: the smallest a target may be (WCAG 2.5.8).
const TARGET: Pixels = px(24.);
/// How much of a waiting row its moves' placeholder fills, row by row, so the rows still to come
/// read as lines of moves rather than one block.
const PENDING_WIDTHS: [f32; 3] = [0.62, 0.48, 0.55];

/// Play an engine line's moves up to the one at an index.
pub type PlayLine = Rc<dyn Fn(&[String], usize, &mut Window, &mut App)>;

/// The evaluation bar: White's share fills from White's side of the board; stripes until there is
/// something to show.
pub fn eval_bar(line: Option<&EngineLine>, orientation: Color, result: Option<&str>) -> Canvas<()> {
    let known = line.is_some() || result.is_some();
    let white = match result {
        Some("1-0") => 1.,
        Some("0-1") => 0.,
        Some(_) => 0.5,
        None => 0.5 + 0.5 * winning_chances(line),
    };
    canvas(
        |_, _, _| {},
        move |bounds, (), window, _| {
            let dark = rgb(0x403d39);
            window.paint_quad(fill(bounds, dark).corner_radii(radii::SMALL));
            if !known {
                // Light stripes over the dark bar.
                let step = 8.;
                let height = bounds.size.height.as_f32();
                let width = bounds.size.width.as_f32();
                let mut offset = -width;
                while offset < height {
                    let mut stripe = PathBuilder::fill();
                    // Clear of the rounded ends, which GPUI does not clip to.
                    let end = radii::SMALL.as_f32();
                    let at = |x: f32, y: f32| {
                        point(
                            bounds.origin.x + px(x),
                            bounds.origin.y + px(y.clamp(end, height - end)),
                        )
                    };
                    stripe.move_to(at(0., offset + 4.));
                    stripe.line_to(at(0., offset + step));
                    stripe.line_to(at(width, offset + step - width));
                    stripe.line_to(at(width, offset + 4. - width));
                    stripe.close();
                    if let Ok(path) = stripe.build() {
                        window.paint_path(path, rgb(0x68645e));
                    }
                    offset += step;
                }
                return;
            }
            let fill_height = bounds.size.height * white;
            // Rounded at the bar's end only: square where it meets the dark part.
            let (white_part, ends) = if orientation == Color::White {
                (
                    Bounds::new(
                        point(bounds.origin.x, bounds.bottom() - fill_height),
                        size(bounds.size.width, fill_height),
                    ),
                    Corners {
                        bottom_left: radii::SMALL,
                        bottom_right: radii::SMALL,
                        ..Corners::default()
                    },
                )
            } else {
                (
                    Bounds::new(bounds.origin, size(bounds.size.width, fill_height)),
                    Corners {
                        top_left: radii::SMALL,
                        top_right: radii::SMALL,
                        ..Corners::default()
                    },
                )
            };
            window.paint_quad(fill(white_part, rgb(0xf2f0ea)).corner_radii(ends));
            let middle = Bounds::new(
                point(bounds.origin.x, bounds.center().y - px(1.)),
                size(bounds.size.width, px(2.)),
            );
            window.paint_quad(fill(middle, rgba(0xd64f008c)));
        },
    )
    .w(EVAL_BAR)
    .h_full()
    .flex_none()
}

/// What the engine panel lists (`engine_lines`): the `lines` searched from `fen` in `variant`, how
/// many rows wait for lines still to come (`pending`), and, when there is nothing to search, why
/// (`ended`). A move's preview board is seen as the board beside it is (`orientation`).
pub struct EngineLines<'a> {
    pub fen: &'a str,
    pub variant: kchess_domain::variant::Variant,
    pub orientation: Color,
    pub lines: &'a [EngineLine],
    pub pending: usize,
    pub ended: Option<&'static str>,
}

/// Each line's score and its moves, a click playing the line up to the move (`on_play`), and the
/// position a move reaches under the lines while the pointer rests on it (`LinePreview`).
pub fn engine_lines(
    shown: EngineLines<'_>,
    on_play: PlayLine,
    palette: &Palette,
    figurine: crate::notation::Figurine,
) -> AnyElement {
    let EngineLines {
        fen,
        variant,
        lines,
        pending,
        ended,
        orientation,
    } = shown;
    LinesView {
        fen: fen.to_string(),
        variant,
        orientation,
        lines: lines.to_vec(),
        pending,
        ended,
        on_play,
        palette: *palette,
        figurine,
    }
    .into_any_element()
}

/// Where the moves start in a line's row: past its padding, the score chip and the gap after it.
const MOVES_X: Pixels = px(8. + 55. + 8.);

/// The move of an engine line the pointer last rested on, and the board showing the position it
/// reaches. It shows at once, under the lines, and stays as the pointer crosses from move to move
/// and over the gaps between them, the board moving on to each move rather than a tooltip closing
/// and opening again; it goes when the pointer leaves the lines.
#[derive(Default)]
struct LinePreview {
    /// The line's index, and its moves up to the one shown.
    shown: Option<(usize, Vec<String>)>,
    board: Option<Entity<crate::board::ChessBoard>>,
}

impl LinePreview {
    fn show(
        &mut self,
        line: usize,
        moves: Vec<String>,
        state: crate::board::BoardState,
        cx: &mut Context<Self>,
    ) {
        if self.shown.as_ref() == Some(&(line, moves.clone())) {
            return;
        }
        self.shown = Some((line, moves));
        match &self.board {
            Some(board) => board.update(cx, |board, cx| board.set_state(state, cx)),
            None => self.board = Some(cx.new(|_| crate::board::ChessBoard::new(state))),
        }
        cx.notify();
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        if self.shown.take().is_some() {
            cx.notify();
        }
    }
}

#[derive(IntoElement)]
struct LinesView {
    fen: String,
    variant: kchess_domain::variant::Variant,
    orientation: Color,
    lines: Vec<EngineLine>,
    pending: usize,
    ended: Option<&'static str>,
    on_play: PlayLine,
    palette: Palette,
    figurine: crate::notation::Figurine,
}

impl RenderOnce for LinesView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let LinesView {
            fen,
            variant,
            orientation,
            lines,
            pending,
            ended,
            on_play,
            palette,
            figurine,
        } = self;
        let palette = &palette;
        let preview =
            window.use_keyed_state("engine-line-preview", cx, |_, _| LinePreview::default());
        // The move previewed, while its line still starts with the moves that led to it: a
        // deeper search may have changed the line under the pointer.
        let previewed = match (&preview.read(cx).shown, ended) {
            (Some((line, moves)), None)
                if lines
                    .get(*line)
                    .is_some_and(|shown| shown.pv.starts_with(moves)) =>
            {
                Some((*line, moves.len() - 1))
            }
            _ => None,
        };
        let board = previewed.and(preview.read(cx).board.clone());
        let previewed_move = previewed.and_then(|(line, at)| {
            pv_san(&fen, &lines[line].pv, 12, variant)
                .into_iter()
                .nth(at)
                .map(|played| played.label)
        });
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(ended) = ended {
            rows.push(
                h_flex()
                    .h(PV_ROW * pending.max(1) as f32)
                    .justify_center()
                    .text_color(palette.text_muted)
                    .child(ended)
                    .into_any_element(),
            );
        } else {
            for (index, line) in lines.iter().enumerate() {
                let pv = line.pv.clone();
                let moves = pv_san(&fen, &pv, 12, variant);
                let eval_pv = pv.clone();
                rows.push(
                    h_flex()
                        .h(PV_ROW)
                        .gap(px(8.))
                        .px(px(8.))
                        .when(index > 0, |row| {
                            row.border_t_1().border_color(palette.border)
                        })
                        .child(
                            div()
                                .id(("pv-eval", index))
                                .role(Role::Button)
                                .keyboard_focus(palette.primary)
                                .aria_label(format!("Play line {}", format_eval(Some(line))))
                                .w(px(55.))
                                .flex_none()
                                .h(TARGET)
                                .flex()
                                .items_center()
                                .justify_center()
                                .px(px(6.))
                                .rounded(radii::BOX)
                                .bg(palette.accented)
                                .font_weight(FontWeight::MEDIUM)
                                // The score changes as the search deepens; it holds still in its
                                // chip.
                                .font_features(crate::widgets::tabular())
                                .text_center()
                                .cursor_pointer()
                                .pressable_fill(tint(palette.primary, 24.), palette)
                                .on_click({
                                    let on_play = on_play.clone();
                                    move |_, window, cx| on_play(&eval_pv, 0, window, cx)
                                })
                                .child(format_eval(Some(line))),
                        )
                        .child(h_flex().min_w_0().overflow_hidden().children(
                            moves.into_iter().enumerate().map(|(at, played)| {
                                let (pv, on_play) = (pv.clone(), on_play.clone());
                                let fill = tint(palette.primary, 24.);
                                let state = crate::board::BoardState::preview(
                                    &played.fen,
                                    variant,
                                    orientation,
                                    Some(&played.uci),
                                );
                                let reached = pv[..(at + 1).min(pv.len())].to_vec();
                                let preview = preview.clone();
                                div()
                                    .id(SharedString::from(format!("pv-{index}-{at}")))
                                    .test_support()
                                    .role(Role::Button)
                                    .keyboard_focus(palette.primary)
                                    .aria_label(played.label.clone())
                                    .flex_none()
                                    // A 24 px target, as every button is.
                                    .h(TARGET)
                                    .min_w(TARGET)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .px(px(2.))
                                    .rounded(radii::SMALL)
                                    .text_color(palette.toned)
                                    .cursor_pointer()
                                    .when(previewed == Some((index, at)), |shown| shown.bg(fill))
                                    .pressable_fill(fill, palette)
                                    .on_hover(move |&hovered, _, cx| {
                                        if hovered {
                                            preview.update(cx, |preview, cx| {
                                                preview.show(
                                                    index,
                                                    reached.clone(),
                                                    state.clone(),
                                                    cx,
                                                )
                                            });
                                        }
                                    })
                                    .on_click(move |_, window, cx| on_play(&pv, at, window, cx))
                                    .child(crate::notation::shown(
                                        &played.label,
                                        &figurine
                                            .as_ref()
                                            .map(|figurine| figurine.in_color(palette.toned)),
                                    ))
                            }),
                        ))
                        .into_any_element(),
                );
            }
            // A line still to come holds its place in the shape of one: a score chip and moves,
            // pulsing (still, with reduced motion), so the row does not move when the line arrives.
            for index in lines.len()..pending {
                rows.push(
                    h_flex()
                        .id(("pv-pending", index))
                        .test_support()
                        .role(Role::Status)
                        .aria_label("Line still calculating")
                        .h(PV_ROW)
                        .gap(px(8.))
                        .px(px(8.))
                        .when(index > 0, |row| {
                            row.border_t_1().border_color(palette.border)
                        })
                        .child(
                            Skeleton::new()
                                .w(px(55.))
                                .h(px(18.))
                                .flex_none()
                                .rounded(radii::BOX),
                        )
                        .child(
                            Skeleton::new()
                                .secondary()
                                .w(relative(PENDING_WIDTHS[index % PENDING_WIDTHS.len()]))
                                .h(px(10.))
                                .rounded(radii::SMALL),
                        )
                        .into_any_element(),
                );
            }
        }
        let hide = preview.clone();
        v_flex()
            .flex_none()
            .child(
                v_flex()
                    .id("engine-lines")
                    .text_size(px(13.))
                    .border_1()
                    .border_color(palette.border)
                    .rounded(radii::BOX)
                    .bg(palette.bg)
                    .overflow_hidden()
                    // Kept while the pointer is anywhere over the lines, gaps included.
                    .on_hover(move |&hovered, _, cx| {
                        if !hovered {
                            hide.update(cx, |preview, cx| preview.hide(cx));
                        }
                    })
                    .children(rows),
            )
            // Under the lines, in line with the moves, so it holds still from move to move.
            .children(board.zip(previewed_move).map(|(board, played)| {
                div().h_0().child(
                    deferred(
                        anchored()
                            .offset(point(MOVES_X, px(4.)))
                            .snap_to_window_with_margin(px(8.))
                            .child(
                                div()
                                    .id("pv-preview")
                                    .test_support()
                                    .aria_label(format!("Position after {played}"))
                                    .w(crate::board::PREVIEW_TIP + px(14.))
                                    .p(px(6.))
                                    .bg(palette.elevated)
                                    .border_1()
                                    .border_color(palette.border)
                                    .rounded(radii::BOX)
                                    .shadow_md()
                                    .child(board),
                            ),
                    )
                    .with_priority(1),
                )
            }))
    }
}
