//! The engine's evaluation as the analysis board and the broadcast board show it: the bar beside
//! the board and the lines under the score.

use std::rc::Rc;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use shakmaty::Color;

use crate::game::tree::{format_eval, pv_san, winning_chances};
use crate::theme::{Palette, radii, tint};
use crate::widgets::KeyboardFocus as _;

/// The evaluation bar's width.
pub const EVAL_BAR: Pixels = px(14.);
/// One engine line's row.
const PV_ROW: Pixels = px(34.);

/// Play an engine line's moves up to the one at an index.
pub type PlayLine = Rc<dyn Fn(&[String], usize, &mut Window, &mut App)>;

/// The evaluation bar: White's share fills from White's side of the board; stripes until there is
/// something to show.
pub fn eval_bar(line: Option<&Value>, orientation: Color, result: Option<&str>) -> Canvas<()> {
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
            window.paint_quad(fill(bounds, dark).corner_radii(radii::XS));
            if !known {
                // `repeating-linear-gradient(135deg, …)`: light stripes over the dark bar.
                let step = 8.;
                let height = bounds.size.height.as_f32();
                let width = bounds.size.width.as_f32();
                let mut offset = -width;
                while offset < height {
                    let mut stripe = PathBuilder::fill();
                    // Clear of the rounded ends, which GPUI does not clip to.
                    let end = radii::XS.as_f32();
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
                        bottom_left: radii::XS,
                        bottom_right: radii::XS,
                        ..Corners::default()
                    },
                )
            } else {
                (
                    Bounds::new(bounds.origin, size(bounds.size.width, fill_height)),
                    Corners {
                        top_left: radii::XS,
                        top_right: radii::XS,
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

/// Each line's score and its moves from `fen`, a click playing the line
/// up to the move (`on_play`); `pending` rows wait for lines still to come.
pub fn engine_lines(
    fen: &str,
    lines: &[Value],
    pending: usize,
    ended: Option<&'static str>,
    on_play: PlayLine,
    palette: &Palette,
) -> Div {
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
            let pv: Vec<String> = serde_json::from_value(line["pv"].clone()).unwrap_or_default();
            let moves = pv_san(fen, &pv, 12);
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
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(radii::SM)
                            .bg(palette.accented)
                            .font_weight(FontWeight::BOLD)
                            .text_center()
                            .cursor_pointer()
                            .hover(|eval| eval.bg(tint(palette.primary, 24.)))
                            .on_click({
                                let on_play = on_play.clone();
                                move |_, window, cx| on_play(&eval_pv, 0, window, cx)
                            })
                            .child(format_eval(Some(line))),
                    )
                    .child(h_flex().min_w_0().overflow_hidden().children(
                        moves.into_iter().enumerate().map(|(at, played)| {
                            let (pv, on_play) = (pv.clone(), on_play.clone());
                            div()
                                .id(SharedString::from(format!("pv-{index}-{at}")))
                                .role(Role::Button)
                                .keyboard_focus(palette.primary)
                                .aria_label(played.label.clone())
                                .flex_none()
                                .px(px(2.))
                                .py(px(1.))
                                .rounded(radii::XS)
                                .text_color(palette.toned)
                                .cursor_pointer()
                                .hover(|chip| chip.bg(tint(palette.primary, 24.)))
                                .on_click(move |_, window, cx| on_play(&pv, at, window, cx))
                                .child(played.label)
                        }),
                    ))
                    .into_any_element(),
            );
        }
        for index in lines.len()..pending {
            rows.push(
                h_flex()
                    .h(PV_ROW)
                    .px(px(8.))
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(palette.border)
                    })
                    .child(
                        div()
                            .w(px(55.))
                            .text_center()
                            .text_color(palette.dimmed)
                            .child("…"),
                    )
                    .into_any_element(),
            );
        }
    }
    v_flex()
        .flex_none()
        .text_size(px(12.5))
        .border_1()
        .border_color(palette.border)
        .rounded(radii::LG)
        .bg(palette.bg)
        .overflow_hidden()
        .children(rows)
}
