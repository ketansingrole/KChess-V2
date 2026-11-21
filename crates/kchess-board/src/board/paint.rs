//! What the board paints in one pass: squares and their highlights, pieces, arrows and circles,
//! and the glow of a king in check.

use super::*;

/// What one frame paints in the canvas.
pub(super) struct Frame {
    pub(super) state: BoardState,
    pub(super) pieces: Board,
    pub(super) selected: Option<Square>,
    /// The dragged piece's square: its piece is drawn as a ghost, not here.
    pub(super) lifted: Option<Square>,
    pub(super) hovered: Option<Square>,
    pub(super) show_dests: bool,
    pub(super) set: String,
    /// The queued premove's squares, and the targets of a piece picked for one.
    pub(super) queued: Option<(Square, Square)>,
    pub(super) premove_dests: Vec<Square>,
    /// Where the picked pocket piece may be dropped.
    pub(super) drop_dests: Vec<Square>,
    /// Squares whose pieces are sliding in, drawn on top as they move.
    pub(super) sliding: Vec<Square>,
}

pub(super) fn paint_frame(frame: Frame, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let side = bounds.size.width / 8.;
    let place = |square: Square| -> (u32, u32) {
        let (file, rank) = (u32::from(square.file()), u32::from(square.rank()));
        match frame.state.orientation {
            Color::White => (file, 7 - rank),
            Color::Black => (7 - file, rank),
        }
    };
    // A corner square's fill takes the board's rounded corner: GPUI does not clip to it.
    let rounded = |square: Square| -> Corners<Pixels> {
        let (column, row) = place(square);
        let corner = |at: bool| if at { radii::BOARD.min(side) } else { px(0.) };
        Corners {
            top_left: corner(column == 0 && row == 0),
            top_right: corner(column == 7 && row == 0),
            bottom_right: corner(column == 7 && row == 7),
            bottom_left: corner(column == 0 && row == 7),
        }
    };
    let at = |square: Square| -> Bounds<Pixels> {
        let (column, row) = place(square);
        Bounds::new(
            point(
                bounds.origin.x + side * column as f32,
                bounds.origin.y + side * row as f32,
            ),
            size(side, side),
        )
    };
    let fill = |window: &mut Window, square: Square, color: u32| {
        window.paint_quad(gpui_kit::fill(at(square), rgba(color)).corner_radii(rounded(square)));
    };
    if let Some((from, to)) = frame.state.last_move {
        fill(window, from, LAST_MOVE);
        fill(window, to, LAST_MOVE);
    }
    if let Some(selected) = frame.selected {
        fill(window, selected, SELECTED);
        if frame.show_dests {
            for &to in frame.state.dests.get(&selected).into_iter().flatten() {
                let cell = at(to);
                if frame.hovered == Some(to) {
                    fill(window, to, DEST_HOVER);
                } else if frame.pieces.piece_at(to).is_some() {
                    paint_corners(window, cell, rounded(to), rgba(DEST_CORNER));
                } else {
                    let diameter = side * 0.311;
                    let dot = Bounds::new(
                        point(
                            cell.center().x - diameter / 2.,
                            cell.center().y - diameter / 2.,
                        ),
                        size(diameter, diameter),
                    );
                    window.paint_quad(
                        gpui_kit::fill(dot, rgba(DEST_DOT)).corner_radii(diameter / 2.),
                    );
                }
            }
        }
    }
    if frame.show_dests {
        for &to in &frame.drop_dests {
            if frame.hovered == Some(to) {
                fill(window, to, DEST_HOVER);
                continue;
            }
            let cell = at(to);
            let diameter = side * 0.311;
            let dot = Bounds::new(
                point(
                    cell.center().x - diameter / 2.,
                    cell.center().y - diameter / 2.,
                ),
                size(diameter, diameter),
            );
            window.paint_quad(gpui_kit::fill(dot, rgba(DEST_DOT)).corner_radii(diameter / 2.));
        }
    }
    if let Some((from, to)) = frame.queued {
        fill(window, from, PREMOVE);
        fill(window, to, PREMOVE);
    }
    if frame.show_dests {
        for &to in &frame.premove_dests {
            let cell = at(to);
            let diameter = side * 0.311;
            let dot = Bounds::new(
                point(
                    cell.center().x - diameter / 2.,
                    cell.center().y - diameter / 2.,
                ),
                size(diameter, diameter),
            );
            window.paint_quad(gpui_kit::fill(dot, rgba(PREMOVE_DOT)).corner_radii(diameter / 2.));
        }
    }
    if let Some(square) = frame.state.hint {
        paint_hint(at(square), window);
    }
    if !frame.state.blindfold {
        paint_pieces(&frame, side, &at, window, cx);
    }
    for circle in &frame.state.circles {
        paint_circle(circle, at(circle.square), window);
    }
    for arrow in &frame.state.arrows {
        paint_arrow(
            arrow,
            side,
            at(arrow.from).center(),
            at(arrow.to).center(),
            window,
        );
    }
}

fn paint_pieces(
    frame: &Frame,
    side: Pixels,
    at: &dyn Fn(Square) -> Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let pixels = (side.as_f32() * window.scale_factor()).round() as u32;
    let viewer = window.current_view();
    for square in frame.pieces.occupied() {
        if Some(square) == frame.lifted || frame.sliding.contains(&square) {
            continue;
        }
        let Some(piece) = frame.pieces.piece_at(square) else {
            continue;
        };
        if let Some(image) = looks::piece_image(&frame.set, piece, pixels, viewer, cx) {
            let cell = at(square);
            let _ = window.paint_image(cell, cell, Corners::default(), image, 0, false);
        }
    }
}

/// Chessground's circle: a ring 4/64 of a square wide, 0.47 of a square from
/// its centre.
fn paint_circle(circle: &Circle, cell: Bounds<Pixels>, window: &mut Window) {
    let side = cell.size.width;
    let width = side * (4. / 64.);
    // The stroke is centred on the radius: the ring's outer edge is half a width further out.
    let outer = side * (0.5 - 2. / 64.) + width / 2.;
    let center = cell.center();
    window.paint_quad(
        gpui_kit::fill(
            Bounds::new(
                point(center.x - outer, center.y - outer),
                size(outer * 2., outer * 2.),
            ),
            Hsla::transparent_black(),
        )
        .corner_radii(outer)
        .border_widths(width)
        .border_color(rgb(circle.color)),
    );
}

/// The puzzle hint: a thick green ring on a soft white halo,
/// over a faint green tint, in a square's 100 units: radius 42, halo 14 wide, ring 9 wide.
fn paint_hint(cell: Bounds<Pixels>, window: &mut Window) {
    let unit = cell.size.width / 100.;
    let center = cell.center();
    let ring = |radius: f32, width: f32, fill_color: Hsla, border: Hsla, window: &mut Window| {
        let outer = unit * (radius + width / 2.);
        let bounds = Bounds::new(
            point(center.x - outer, center.y - outer),
            size(outer * 2., outer * 2.),
        );
        window.paint_quad(
            gpui_kit::fill(bounds, fill_color)
                .corner_radii(outer)
                .border_widths(unit * width)
                .border_color(border),
        );
    };
    let green = rgb(0x16a34a);
    let tint = Hsla::from(green).opacity(0.22);
    let disc = unit * 42.;
    window.paint_quad(
        gpui_kit::fill(
            Bounds::new(
                point(center.x - disc, center.y - disc),
                size(disc * 2., disc * 2.),
            ),
            tint,
        )
        .corner_radii(disc),
    );
    ring(
        42.,
        14.,
        Hsla::transparent_black(),
        Hsla::white().opacity(0.8),
        window,
    );
    ring(42., 9., Hsla::transparent_black(), green.into(), window);
}

/// Chessground's arrow, in one fill so its overlaps are not darker: a
/// shaft with a round tail, stopping 10/64 of a square short of the target's centre, and a head
/// 4 widths wide and 3 long whose point sits 0.95 widths past that end.
fn paint_arrow(
    arrow: &Arrow,
    side: Pixels,
    from: Point<Pixels>,
    to: Point<Pixels>,
    window: &mut Window,
) {
    let side = side.as_f32();
    let width = arrow.width / 64. * side;
    let (dx, dy) = (
        to.x.as_f32() - from.x.as_f32(),
        to.y.as_f32() - from.y.as_f32(),
    );
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f32::EPSILON {
        return;
    }
    let (ux, uy) = (dx / length, dy / length);
    // Perpendicular, to the left of the arrow's direction.
    let (nx, ny) = (-uy, ux);
    let end = (
        to.x.as_f32() - ux * 10. / 64. * side,
        to.y.as_f32() - uy * 10. / 64. * side,
    );
    let base = (end.0 - ux * 2.05 * width, end.1 - uy * 2.05 * width);
    let tip = (end.0 + ux * 0.95 * width, end.1 + uy * 0.95 * width);
    let at = |(x, y): (f32, f32), offset: f32| point(px(x + nx * offset), px(y + ny * offset));
    let start = (from.x.as_f32(), from.y.as_f32());
    let half = width / 2.;
    let mut path = PathBuilder::fill();
    path.move_to(at(start, half));
    path.line_to(at(base, half));
    path.line_to(at(base, 2. * width));
    path.line_to(at(tip, 0.));
    path.line_to(at(base, -2. * width));
    path.line_to(at(base, -half));
    path.line_to(at(start, -half));
    // The round tail.
    path.arc_to(
        point(px(half), px(half)),
        px(0.),
        false,
        false,
        at(start, half),
    );
    path.close();
    if let Ok(path) = path.build() {
        let mut color = rgba((arrow.color << 8) | 0xff);
        color.a = arrow.opacity;
        window.paint_path(path, color);
    }
}

/// Chessground's mark on a square the piece can take: the corners outside a circle, but for one
/// on the board's rounded corner (`board`), which would stand out past it.
fn paint_corners(window: &mut Window, cell: Bounds<Pixels>, board: Corners<Pixels>, color: Rgba) {
    let side = cell.size.width;
    // The circle reaches 80% of the way to the corners; it cuts each edge this far from a corner.
    let radius = side * 0.5657;
    let cut = side * (0.5 - (0.5657f32.powi(2) - 0.25).sqrt());
    let corners = [
        (cell.origin, 1., 1., board.top_left),
        (
            point(cell.origin.x + side, cell.origin.y),
            -1.,
            1.,
            board.top_right,
        ),
        (
            point(cell.origin.x + side, cell.origin.y + side),
            -1.,
            -1.,
            board.bottom_right,
        ),
        (
            point(cell.origin.x, cell.origin.y + side),
            1.,
            -1.,
            board.bottom_left,
        ),
    ];
    for (corner, dx, dy, rounded) in corners {
        if rounded > px(0.) {
            continue;
        }
        let mut path = PathBuilder::fill();
        path.move_to(corner);
        path.line_to(point(corner.x + cut * dx, corner.y));
        path.arc_to(
            point(radius, radius),
            px(0.),
            false,
            (dx * dy) < 0.,
            point(corner.x, corner.y + cut * dy),
        );
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

/// Discs from the outermost in, each with the opacity that makes those stacked over a radius add
/// up to Chessground's check gradient there (`radial-gradient(ellipse at center, #ff0000 0%,
/// #e70000 25%, transparent 89%)` over a square, its ellipse reaching the corners).
pub(super) fn check_rings(side: Pixels) -> Vec<(Pixels, Hsla)> {
    const BANDS: usize = 28;
    let reach = side * std::f32::consts::FRAC_1_SQRT_2;
    let (solid, gone) = (reach * 0.25, reach * 0.89);
    let mut rings = Vec::with_capacity(BANDS + 1);
    let mut below = 0f32;
    for band in 0..BANDS {
        let outer = gone - (gone - solid) * (band as f32 / BANDS as f32);
        let middle = gone - (gone - solid) * ((band as f32 + 0.5) / BANDS as f32);
        let wanted = (gone - middle) / (gone - solid);
        // Over what the discs outside it already cover, this one brings the rest.
        let alpha = 1. - (1. - wanted) / (1. - below);
        below = wanted;
        // Darkening toward #a90000 as it fades out, as the gradient's stops do.
        let mut color = red_between(0xa90000, 0xe70000, wanted);
        color.a = alpha.clamp(0., 1.);
        rings.push((outer, color));
    }
    // Solid inside a quarter of the way out, brightening to #ff0000 in the middle.
    const CORE: usize = 6;
    for step in 0..CORE {
        let share = step as f32 / CORE as f32;
        rings.push((solid * (1. - share), red_between(0xe70000, 0xff0000, share)));
    }
    rings
}

/// The red `share` of the way from `from` to `to`, opaque.
fn red_between(from: u32, to: u32, share: f32) -> Hsla {
    let (from, to) = (rgb(from), rgb(to));
    Rgba {
        r: from.r + (to.r - from.r) * share,
        g: from.g + (to.g - from.g) * share,
        b: from.b + (to.b - from.b) * share,
        a: 1.,
    }
    .into()
}

#[cfg(test)]
mod check_tests {
    use super::{BoardState, check_rings};
    use gpui_kit::px;
    use shakmaty::{Color, Square};

    fn state(fen: &str) -> BoardState {
        let position =
            kchess_domain::rules::setup_start(kchess_domain::variant::Variant::Standard, fen)
                .expect("a position");
        BoardState::of(&position, Color::White, None, None)
    }

    /// A checkmated king wears Lichess's red "#"; a king only in check is lit but unmarked.
    #[test]
    fn checkmate_marks_the_king() {
        // Fool's mate: White's king on e1 is mated.
        let mate = state("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3");
        assert_eq!(mate.check, Some(Square::E1));
        let badge = mate.badge.expect("the mate badge");
        assert_eq!((badge.square, badge.glyph), (Square::E1, "#"));
        let check = state("rnbqkbnr/ppp2ppp/3p4/1B2p3/4P3/8/PPPP1PPP/RNBQK1NR b KQkq - 1 3");
        assert_eq!(check.check, Some(Square::E8));
        assert!(check.badge.is_none());
    }

    /// Stacked, the discs fade as Chessground's gradient does: solid red in the middle, half
    /// way out half as strong, nothing past 89% of the half-diagonal, and all of it inside the
    /// square's corners.
    #[test]
    fn the_check_rings_add_up_to_the_lichess_gradient() {
        let side = px(80.);
        let rings = check_rings(side);
        let opacity = |radius: f32| {
            1. - rings
                .iter()
                .filter(|(outer, _)| f32::from(*outer) >= radius)
                .map(|(_, color)| 1. - color.a)
                .product::<f32>()
        };
        let reach = 80. * std::f32::consts::FRAC_1_SQRT_2;
        let (solid, gone) = (reach * 0.25, reach * 0.89);
        assert!((opacity(0.) - 1.).abs() < 1e-4);
        assert!((opacity(solid) - 1.).abs() < 1e-4);
        assert!(opacity(gone + 0.1) < 1e-4);
        let middle = (solid + gone) / 2.;
        assert!((opacity(middle) - 0.5).abs() < 0.08, "{}", opacity(middle));
        // Never brighter further out.
        let steps: Vec<f32> = (0..=50)
            .map(|step| opacity(gone * step as f32 / 50.))
            .collect();
        assert!(steps.windows(2).all(|pair| pair[1] <= pair[0] + 1e-4));
        // The faded edge reaches past the square's sides (40px) into its corners, as on Lichess.
        assert!(gone > 40.);
    }
}
