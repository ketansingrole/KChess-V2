//! The app's line chart (`LineChart.vue` as `RatingChart.vue` draws it): a value over time with its
//! axes, the area under it, and a crosshair with the value and date under the pointer.

use gpui_kit::component::v_flex;
use gpui_kit::*;

use crate::theme::{Palette, tint};

/// Room for the value labels left of the plot, the dates under it and the top.
const LEFT: f32 = 40.;
const BOTTOM: f32 = 20.;
const TOP: f32 = 8.;
const RIGHT: f32 = 8.;

/// One point: its value, and its date short (an axis label) and long (the tooltip).
#[derive(Clone)]
pub struct ChartPoint {
    pub value: f64,
    pub short: SharedString,
    pub long: SharedString,
}

/// `niceTicks`: about `count` round values (1, 2 or 5 × 10ⁿ apart) inside the domain.
pub fn nice_ticks((lo, hi): (f64, f64), count: usize) -> Vec<f64> {
    if hi <= lo || count == 0 || !lo.is_finite() || !hi.is_finite() {
        return Vec::new();
    }
    let raw = (hi - lo) / count as f64;
    let mut step = 10f64.powf(raw.log10().floor());
    let ratio = raw / step;
    if ratio >= 7.5 {
        step *= 10.;
    } else if ratio >= 3.5 {
        step *= 5.;
    } else if ratio >= 1.5 {
        step *= 2.;
    }
    let first = (lo / step - 1e-9).ceil() as i64;
    let last = (hi / step + 1e-9).floor() as i64;
    (first..=last)
        .map(|index| (index as f64 * step * 1e12).round() / 1e12)
        .collect()
}

pub struct LineChart {
    points: Vec<ChartPoint>,
    domain: (f64, f64),
    height: Pixels,
    format: fn(f64) -> String,
    /// What a screen reader hears for the chart.
    summary: SharedString,
    hovered: Option<usize>,
    bounds: Bounds<Pixels>,
}

impl LineChart {
    pub fn new(height: Pixels, format: fn(f64) -> String) -> LineChart {
        LineChart {
            points: Vec::new(),
            domain: (0., 1.),
            height,
            format,
            summary: SharedString::default(),
            hovered: None,
            bounds: Bounds::default(),
        }
    }

    pub fn set(
        &mut self,
        points: Vec<ChartPoint>,
        domain: (f64, f64),
        summary: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.points = points;
        self.domain = domain;
        self.summary = summary.into();
        self.hovered = None;
        cx.notify();
    }

    fn x(&self, index: usize, width: f32) -> f32 {
        let span = (width - LEFT - RIGHT).max(1.);
        if self.points.len() < 2 {
            return LEFT + span / 2.;
        }
        LEFT + span * index as f32 / (self.points.len() - 1) as f32
    }

    fn y(&self, value: f64, height: f32) -> f32 {
        let (lo, hi) = self.domain;
        let bottom = height - BOTTOM;
        if hi <= lo {
            return (TOP + bottom) / 2.;
        }
        bottom - ((value - lo) / (hi - lo)) as f32 * (bottom - TOP)
    }

    /// About five dates along the bottom, evenly spaced.
    fn date_ticks(&self) -> Vec<usize> {
        let count = self.points.len();
        if count == 0 {
            return Vec::new();
        }
        let wanted = 5.min(count);
        let mut ticks: Vec<usize> = (0..wanted)
            .map(|step| {
                if wanted == 1 {
                    0
                } else {
                    (step * (count - 1) + (wanted - 1) / 2) / (wanted - 1)
                }
            })
            .collect();
        ticks.dedup();
        ticks
    }
}

impl Render for LineChart {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = *cx.global::<Palette>();
        let (width, height) = (self.bounds.size.width.as_f32(), self.height.as_f32());
        let entity = cx.entity().downgrade();
        let values: Vec<f64> = self.points.iter().map(|point| point.value).collect();
        let xs: Vec<f32> = (0..values.len())
            .map(|index| self.x(index, width))
            .collect();
        let ys: Vec<f32> = values.iter().map(|value| self.y(*value, height)).collect();
        let ticks = nice_ticks(self.domain, 4);
        let tick_ys: Vec<f32> = ticks.iter().map(|tick| self.y(*tick, height)).collect();
        let hovered = self.hovered.filter(|index| *index < values.len());
        let (primary, border) = (palette.primary, palette.border);
        let label = |text: String| {
            div()
                .absolute()
                .text_size(px(10.))
                .text_color(palette.text_muted)
                .child(text)
        };
        let y_labels = ticks.iter().zip(&tick_ys).map(|(tick, y)| {
            label((self.format)(*tick))
                .w(px(LEFT - 6.))
                .left_0()
                .top(px(y - 7.))
                .text_right()
        });
        let ticks_x = self.date_ticks();
        let last_tick = ticks_x.last().copied();
        // The first and last dates end at the plot's edges; the others are centred.
        let x_labels = ticks_x.into_iter().map(|index| {
            let x = self.x(index, width);
            let text = label(self.points[index].short.to_string())
                .w(px(60.))
                .top(px(height - BOTTOM + 4.))
                .flex();
            if index == 0 {
                text.left(px(x - 8.)).justify_start()
            } else if Some(index) == last_tick {
                text.left(px(x - 52.)).justify_end()
            } else {
                text.left(px(x - 30.)).justify_center()
            }
        });
        let tooltip = hovered.map(|index| {
            let point = &self.points[index];
            let at = xs[index];
            let flip = at > (LEFT + width - RIGHT) / 2.;
            v_flex()
                .absolute()
                .top(px(TOP))
                .when_flip(flip, at, width)
                .px(px(8.))
                .py(px(4.))
                .rounded(px(6.))
                .border_1()
                .border_color(palette.border)
                .bg(palette.elevated)
                .text_size(px(12.))
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .child((self.format)(point.value)),
                )
                .child(
                    div()
                        .text_color(palette.text_muted)
                        .child(point.long.clone()),
                )
        });
        let painted = (xs.clone(), ys.clone(), tick_ys.clone(), hovered);
        div()
            .id("line-chart")
            .relative()
            .w_full()
            .h(self.height)
            .aria_label(self.summary.clone())
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                let count = this.points.len();
                let width = this.bounds.size.width.as_f32();
                if count == 0 || width <= LEFT + RIGHT {
                    return;
                }
                let x = (event.position.x - this.bounds.origin.x).as_f32();
                let ratio = ((x - LEFT) / (width - LEFT - RIGHT)).clamp(0., 1.);
                let index = (ratio * (count - 1) as f32).round() as usize;
                if this.hovered != Some(index) {
                    this.hovered = Some(index);
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && this.hovered.take().is_some() {
                    cx.notify();
                }
            }))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            if this.bounds != bounds {
                                this.bounds = bounds;
                                cx.notify();
                            }
                        });
                    },
                    move |bounds, (), window, _| {
                        let (xs, ys, tick_ys, hovered) = &painted;
                        let origin = bounds.origin;
                        let at = |x: f32, y: f32| point(origin.x + px(x), origin.y + px(y));
                        let right = bounds.size.width.as_f32() - RIGHT;
                        let bottom = bounds.size.height.as_f32() - BOTTOM;
                        for y in tick_ys {
                            window.paint_quad(fill(
                                Bounds::new(at(LEFT, *y), size(px(right - LEFT), px(1.))),
                                tint(border, 60.),
                            ));
                        }
                        window.paint_quad(fill(
                            Bounds::new(at(LEFT, bottom), size(px(right - LEFT), px(1.))),
                            border,
                        ));
                        if xs.len() >= 2 {
                            let mut area = PathBuilder::fill();
                            area.move_to(at(xs[0], bottom));
                            for (x, y) in xs.iter().zip(ys) {
                                area.line_to(at(*x, *y));
                            }
                            area.line_to(at(xs[xs.len() - 1], bottom));
                            area.close();
                            if let Ok(path) = area.build() {
                                window.paint_path(path, primary.opacity(0.14));
                            }
                            let mut line = PathBuilder::stroke(px(2.));
                            line.move_to(at(xs[0], ys[0]));
                            for (x, y) in xs.iter().zip(ys).skip(1) {
                                line.line_to(at(*x, *y));
                            }
                            if let Ok(path) = line.build() {
                                window.paint_path(path, primary);
                            }
                        }
                        if let Some(index) = hovered {
                            let (x, y) = (xs[*index], ys[*index]);
                            window.paint_quad(fill(
                                Bounds::new(at(x, TOP), size(px(1.), px(bottom - TOP))),
                                border,
                            ));
                            let radius = 4.;
                            window.paint_quad(
                                fill(
                                    Bounds::new(
                                        at(x - radius, y - radius),
                                        size(px(radius * 2.), px(radius * 2.)),
                                    ),
                                    primary,
                                )
                                .corner_radii(px(radius)),
                            );
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(y_labels)
            .children(x_labels)
            .children(tooltip)
    }
}

/// Places the tooltip right of the crosshair, or left of it past the middle.
trait FlipPlacement: Styled + Sized {
    fn when_flip(self, flip: bool, at: f32, width: f32) -> Self {
        if flip {
            self.right(px(width - at + 12.))
        } else {
            self.left(px(at + 12.))
        }
    }
}

impl<E: Styled> FlipPlacement for E {}

#[cfg(test)]
mod tests {
    use super::nice_ticks;

    #[test]
    fn ticks_are_round_values_inside_the_domain() {
        assert_eq!(
            nice_ticks((0., 100.), 4),
            vec![0., 20., 40., 60., 80., 100.]
        );
        assert_eq!(nice_ticks((0., 11.5), 4), vec![0., 2., 4., 6., 8., 10.]);
        assert_eq!(nice_ticks((0.1, 0.3), 4), vec![0.1, 0.15, 0.2, 0.25, 0.3]);
        assert!(nice_ticks((1., 1.), 4).is_empty());
    }
}
