//! The chessboard, as Chessground draws it in the Electron app (`ChessBoard.vue`): the chosen board
//! and pieces, the last move, check, the selected piece and where it can go, coordinates,
//! click-to-move and drag-and-drop, and the promotion picker. Squares, highlights and pieces are
//! painted in one pass; the page gives the board its position and hears its moves.

pub mod looks;

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{CoordinateMode, PromotionMode};
use shakmaty::variant::VariantPosition;
use shakmaty::{Board, Color, File, Piece, Position as _, Rank, Role, Square};

use crate::settings::SettingsHandle;

/// Chessground's square colors (`chessground.brown.css`).
const LAST_MOVE: u32 = 0x9bc70069; // rgba(155, 199, 0, 0.41)
const SELECTED: u32 = 0x14551e80; // rgba(20, 85, 30, 0.5)
const DEST_DOT: u32 = 0x14551e80; // rgba(20, 85, 30, 0.5)
const DEST_CORNER: u32 = 0x1455004d; // rgba(20, 85, 0, 0.3)
const DEST_HOVER: u32 = 0x14551e4d; // rgba(20, 85, 30, 0.3)
/// `--board-radius`.
const BOARD_RADIUS: Pixels = px(14.);
/// A press moves this far before it is a drag rather than a click.
const DRAG_THRESHOLD: Pixels = px(4.);

/// What the page shows on the board.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardState {
    pub fen: String,
    pub orientation: Color,
    /// The side whose pieces may move now; `None` while nothing moves.
    pub movable: Option<Color>,
    /// Each piece's destinations, as `position::dests` lists them for a board.
    pub dests: HashMap<Square, Vec<Square>>,
    pub last_move: Option<(Square, Square)>,
    /// The king in check.
    pub check: Option<Square>,
    /// Hide the pieces (blindfold play); highlights and moves stay.
    pub blindfold: bool,
    /// A small picture of a position: no coordinates, nothing to touch.
    pub preview: bool,
    /// Arrows over the pieces (the engine's choices), drawn as Chessground draws them.
    pub arrows: Vec<Arrow>,
}

/// An arrow from one square to another: Chessground's brush color and opacity, and its line
/// width in 64ths of a square.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrow {
    pub from: Square,
    pub to: Square,
    pub color: u32,
    pub opacity: f32,
    pub width: f32,
}

impl Arrow {
    /// The analysis board's arrows (`paleBlue` for the best line, `paleGrey` for the rest):
    /// the best one bold, the others thinner.
    pub fn engine(from: Square, to: Square, rank: usize) -> Arrow {
        let (color, opacity) = if rank == 0 {
            (0x003088, 0.4)
        } else {
            (0x4a4a4a, 0.35)
        };
        Arrow {
            from,
            to,
            color,
            opacity,
            width: if rank == 0 {
                12.
            } else {
                (9. - rank as f32 * 2.).max(4.)
            },
        }
    }
}

impl BoardState {
    /// A position as the board shows it; `movable` is the side the player may move.
    pub fn of(
        position: &VariantPosition,
        orientation: Color,
        movable: Option<Color>,
        last_move: Option<(Square, Square)>,
    ) -> BoardState {
        let can_move = movable == Some(position.turn());
        let dests = if can_move {
            kchess_domain::position::dests(position, kchess_domain::position::DestMode::Board)
                .into_iter()
                .collect()
        } else {
            HashMap::new()
        };
        let check = position
            .is_check()
            .then(|| position.board().king_of(position.turn()))
            .flatten();
        BoardState {
            fen: kchess_domain::rules::make_fen(position),
            orientation,
            movable: can_move.then_some(position.turn()),
            dests,
            last_move,
            check,
            blindfold: false,
            preview: false,
            arrows: Vec::new(),
        }
    }
}

/// What the board tells its page.
pub enum BoardEvent {
    /// A move the player made, in UCI (`e2e4`, `e7e8q`, castling as king to rook or two squares).
    Move(String),
}

struct Drag {
    from: Square,
    piece: Piece,
    /// Where the pointer is, in window coordinates.
    position: Point<Pixels>,
    start: Point<Pixels>,
    moved: bool,
}

pub struct ChessBoard {
    state: BoardState,
    pieces: Board,
    selected: Option<Square>,
    drag: Option<Drag>,
    /// A pawn move waiting for its promotion piece.
    promotion: Option<(Square, Square)>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl EventEmitter<BoardEvent> for ChessBoard {}

impl ChessBoard {
    pub fn new(state: BoardState) -> ChessBoard {
        ChessBoard {
            pieces: board_of(&state.fen),
            state,
            selected: None,
            drag: None,
            promotion: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
        }
    }

    /// Show another position (after a move, a takeback, a new game).
    pub fn set_state(&mut self, state: BoardState, cx: &mut Context<Self>) {
        if state == self.state {
            return;
        }
        if state.fen != self.state.fen {
            self.pieces = board_of(&state.fen);
            self.promotion = None;
        }
        // A selection survives only while its piece can still move.
        if self
            .selected
            .is_some_and(|square| !state.dests.contains_key(&square))
        {
            self.selected = None;
        }
        self.state = state;
        cx.notify();
    }

    fn square_size(&self) -> Pixels {
        self.bounds.get().size.width / 8.
    }

    /// Where a square is drawn, relative to the board: column and row from the top left.
    fn cell(&self, square: Square) -> (u32, u32) {
        let (file, rank) = (u32::from(square.file()), u32::from(square.rank()));
        match self.state.orientation {
            Color::White => (file, 7 - rank),
            Color::Black => (7 - file, rank),
        }
    }

    /// The square under a point in window coordinates.
    fn square_at(&self, point: Point<Pixels>) -> Option<Square> {
        let bounds = self.bounds.get();
        let side = self.square_size();
        if side <= px(0.) || !bounds.contains(&point) {
            return None;
        }
        let column = ((point.x - bounds.origin.x) / side).floor() as u32;
        let row = ((point.y - bounds.origin.y) / side).floor() as u32;
        if column > 7 || row > 7 {
            return None;
        }
        let (file, rank) = match self.state.orientation {
            Color::White => (column, 7 - row),
            Color::Black => (7 - column, row),
        };
        Some(Square::from_coords(File::new(file), Rank::new(rank)))
    }

    fn can_move_from(&self, square: Square) -> bool {
        self.state
            .dests
            .get(&square)
            .is_some_and(|targets| !targets.is_empty())
    }

    fn reaches(&self, from: Square, to: Square) -> bool {
        self.state
            .dests
            .get(&from)
            .is_some_and(|targets| targets.contains(&to))
    }

    fn press(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.promotion.is_some() || self.state.preview {
            return;
        }
        let Some(square) = self.square_at(event.position) else {
            return;
        };
        if let Some(from) = self.selected
            && from != square
            && self.reaches(from, square)
        {
            self.selected = None;
            self.play(from, square, cx);
            return;
        }
        match self.pieces.piece_at(square) {
            Some(piece)
                if Some(piece.color) == self.state.movable && self.can_move_from(square) =>
            {
                self.drag = Some(Drag {
                    from: square,
                    piece,
                    position: event.position,
                    start: event.position,
                    moved: false,
                });
                // A second press on the selected piece lets go of it, unless it is dragged.
                if self.selected != Some(square) {
                    self.selected = Some(square);
                }
            }
            _ => self.selected = None,
        }
        cx.notify();
    }

    fn pointer_moved(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        drag.position = position;
        if far(drag.start, position) {
            drag.moved = true;
        }
        cx.notify();
    }

    fn release(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let target = self.square_at(position);
        // The moves of a quick drag may all have come before the board was listening.
        if drag.moved || far(drag.start, position) {
            match target {
                Some(to) if self.reaches(drag.from, to) => {
                    self.selected = None;
                    self.play(drag.from, to, cx);
                }
                // Dropped back on its square: still selected, as Chessground keeps it.
                Some(to) if to == drag.from => {}
                _ => self.selected = None,
            }
        } else if target == Some(drag.from)
            && self.selected == Some(drag.from)
            && drag.start != drag.position
        {
            self.selected = None;
        }
        cx.notify();
    }

    /// Play a move the player made; a pawn reaching the last rank asks which piece it becomes.
    fn play(&mut self, from: Square, to: Square, cx: &mut Context<Self>) {
        let promotes = self.pieces.piece_at(from).is_some_and(|piece| {
            piece.role == Role::Pawn && matches!(to.rank(), Rank::First | Rank::Eighth)
        });
        if promotes {
            let mode = cx
                .global::<SettingsHandle>()
                .0
                .read(cx)
                .settings
                .frontend
                .promotion;
            if mode == PromotionMode::Queen {
                cx.emit(BoardEvent::Move(format!("{from}{to}q")));
            } else {
                self.promotion = Some((from, to));
            }
        } else {
            // The piece lands at once; the page's next position confirms or undoes it.
            if let Some(piece) = self.pieces.remove_piece_at(from) {
                self.pieces.set_piece_at(to, piece);
            }
            cx.emit(BoardEvent::Move(format!("{from}{to}")));
        }
        cx.notify();
    }

    fn promote(&mut self, role: Role, cx: &mut Context<Self>) {
        if let Some((from, to)) = self.promotion.take() {
            cx.emit(BoardEvent::Move(format!("{from}{to}{}", role.char())));
            cx.notify();
        }
    }

    fn cancel_promotion(&mut self, cx: &mut Context<Self>) {
        self.promotion = None;
        cx.notify();
    }

    fn render_coordinates(
        &self,
        mode: CoordinateMode,
        look: &looks::BoardLook,
        cx: &App,
    ) -> Vec<AnyElement> {
        if mode == CoordinateMode::None {
            return Vec::new();
        }
        let inside = mode == CoordinateMode::Inside;
        let muted = cx.theme().muted_foreground;
        // A label on a light square is drawn in the dark square's color, and the other way round.
        let color = |square: Square| {
            if !inside {
                muted
            } else if square.is_light() {
                looks::hex(&look.coord_black)
            } else {
                looks::hex(&look.coord_white)
            }
        };
        let (edge_file, edge_rank) = match self.state.orientation {
            Color::White => (File::H, Rank::First),
            Color::Black => (File::A, Rank::Eighth),
        };
        // Chessground sets rank labels 39% down their square, below a 2px inset.
        let rank_offset = self.square_size() * 0.39 + px(2.);
        let mut labels = Vec::new();
        for rank in Rank::ALL {
            let square = Square::from_coords(edge_file, rank);
            let (_, row) = self.cell(square);
            labels.push(
                div()
                    .absolute()
                    .top(relative(row as f32 / 8.))
                    .when(inside, |label| label.right_0().w(px(10.)).mt(rank_offset))
                    .when(!inside, |label| {
                        label
                            .right(px(-15.))
                            .h(relative(0.125))
                            .flex()
                            .items_center()
                    })
                    .text_size(px(11.))
                    .font_weight(if inside {
                        FontWeight::BOLD
                    } else {
                        FontWeight::SEMIBOLD
                    })
                    .text_color(color(square))
                    .opacity(0.8)
                    .child(rank.char().to_string())
                    .into_any_element(),
            );
        }
        for file in File::ALL {
            let square = Square::from_coords(file, edge_rank);
            let (column, _) = self.cell(square);
            labels.push(
                div()
                    .absolute()
                    .left(relative(column as f32 / 8.))
                    .when(inside, |label| label.bottom_0().h(px(15.)).pl(px(4.)))
                    .when(!inside, |label| {
                        label
                            .bottom(px(-17.))
                            .w(relative(0.125))
                            .flex()
                            .justify_center()
                    })
                    .text_size(px(11.))
                    .font_weight(if inside {
                        FontWeight::BOLD
                    } else {
                        FontWeight::SEMIBOLD
                    })
                    .text_color(color(square))
                    .opacity(0.8)
                    .child(file.char().to_ascii_uppercase().to_string())
                    .into_any_element(),
            );
        }
        labels
    }

    fn render_promotion(&self, set: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, to) = self.promotion?;
        let color = if to.rank() == Rank::Eighth {
            Color::White
        } else {
            Color::Black
        };
        let theme = cx.theme();
        let (elevated, border, background) = (theme.muted, theme.border, theme.background);
        let choices = [
            (Role::Queen, "Queen"),
            (Role::Rook, "Rook"),
            (Role::Bishop, "Bishop"),
            (Role::Knight, "Knight"),
        ];
        Some(
            div()
                .id("promotion")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(BOARD_RADIUS)
                .bg(hsla(0., 0., 0., 0.5))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.cancel_promotion(cx)),
                )
                .child(
                    div()
                        .id("promotion-choices")
                        .flex()
                        .gap(px(6.))
                        .p(px(8.))
                        .rounded(px(12.))
                        .bg(elevated)
                        .border_1()
                        .border_color(border)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .children(choices.into_iter().map(|(role, label)| {
                            div()
                                .id(label)
                                .size(px(60.))
                                .p(px(4.))
                                .rounded(px(8.))
                                .bg(background)
                                .border_1()
                                .border_color(border)
                                .cursor_pointer()
                                .hover(|style| style.bg(elevated))
                                .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
                                .on_click(cx.listener(move |this, _, _, cx| this.promote(role, cx)))
                                .child(
                                    img(SharedString::from(looks::piece_path(
                                        set,
                                        Piece { color, role },
                                    )))
                                    .size_full(),
                                )
                        })),
                )
                .into_any_element(),
        )
    }
}

/// Whether the pointer went far enough from where it was pressed to be a drag.
fn far(start: Point<Pixels>, now: Point<Pixels>) -> bool {
    let (dx, dy) = ((now.x - start.x).as_f32(), (now.y - start.y).as_f32());
    (dx * dx + dy * dy).sqrt() > DRAG_THRESHOLD.as_f32()
}

fn board_of(fen: &str) -> Board {
    let placement = fen.split(' ').next().unwrap_or_default();
    Board::from_ascii_board_fen(placement.as_bytes()).unwrap_or_else(|_| Board::empty())
}

use gpui_kit::component::tooltip::Tooltip;

/// What one frame paints in the canvas.
struct Frame {
    state: BoardState,
    pieces: Board,
    selected: Option<Square>,
    /// The dragged piece's square: its piece is drawn as a ghost, not here.
    lifted: Option<Square>,
    hovered: Option<Square>,
    show_dests: bool,
    set: String,
}

fn paint_frame(frame: Frame, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let side = bounds.size.width / 8.;
    let at = |square: Square| -> Bounds<Pixels> {
        let (file, rank) = (u32::from(square.file()), u32::from(square.rank()));
        let (column, row) = match frame.state.orientation {
            Color::White => (file, 7 - rank),
            Color::Black => (7 - file, rank),
        };
        Bounds::new(
            point(
                bounds.origin.x + side * column as f32,
                bounds.origin.y + side * row as f32,
            ),
            size(side, side),
        )
    };
    let fill = |window: &mut Window, square: Square, color: u32| {
        window.paint_quad(gpui_kit::fill(at(square), rgba(color)));
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
                    paint_corners(window, cell, rgba(DEST_CORNER));
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
    if !frame.state.blindfold {
        paint_pieces(&frame, side, &at, window, cx);
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
    for square in frame.pieces.occupied() {
        if Some(square) == frame.lifted {
            continue;
        }
        let Some(piece) = frame.pieces.piece_at(square) else {
            continue;
        };
        if let Some(image) = looks::piece_image(&frame.set, piece, pixels, cx) {
            let cell = at(square);
            let _ = window.paint_image(cell, cell, Corners::default(), image, 0, false);
        }
    }
}

/// Chessground's arrow (`svg.js`, `renderArrow`), in one fill so its overlaps are not darker: a
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

/// Chessground's mark on a square the piece can take: the corners outside a circle.
fn paint_corners(window: &mut Window, cell: Bounds<Pixels>, color: Rgba) {
    let side = cell.size.width;
    // The circle reaches 80% of the way to the corners; it cuts each edge this far from a corner.
    let radius = side * 0.5657;
    let cut = side * (0.5 - (0.5657f32.powi(2) - 0.25).sqrt());
    let corners = [
        (cell.origin, 1., 1.),
        (point(cell.origin.x + side, cell.origin.y), -1., 1.),
        (point(cell.origin.x + side, cell.origin.y + side), -1., -1.),
        (point(cell.origin.x, cell.origin.y + side), 1., -1.),
    ];
    for (corner, dx, dy) in corners {
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

impl Render for ChessBoard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frontend = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .clone();
        let look = looks::board(&frontend.board_theme);
        let set = looks::piece_set(&frontend.piece_set).to_string();
        let lifted = self
            .drag
            .as_ref()
            .filter(|drag| drag.moved)
            .map(|drag| drag.from);
        let hovered = self
            .drag
            .as_ref()
            .filter(|drag| drag.moved)
            .and_then(|drag| self.square_at(drag.position));
        let frame = Frame {
            state: self.state.clone(),
            pieces: self.pieces.clone(),
            selected: self.selected,
            lifted,
            hovered,
            show_dests: frontend.show_legal_moves,
            set: set.clone(),
        };
        let bounds = Rc::clone(&self.bounds);
        let side = self.square_size();
        let origin = self.bounds.get().origin;
        let entity = cx.entity().downgrade();
        let dragging = self.drag.is_some();

        // The king in check glows red, as Chessground's radial gradient does.
        let check = self.state.check.map(|square| {
            let (column, row) = self.cell(square);
            div()
                .absolute()
                .left(relative(column as f32 / 8.))
                .top(relative(row as f32 / 8.))
                .size(relative(0.125))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(relative(0.5))
                        .rounded_full()
                        .bg(rgb(0xff0000))
                        .shadow(vec![BoxShadow {
                            color: rgba(0xe70000cc).into(),
                            offset: point(px(0.), px(0.)),
                            blur_radius: side * 0.3,
                            spread_radius: side * 0.08,
                            inset: false,
                        }]),
                )
        });
        // The piece's own square keeps a faint ghost while it is dragged (Chessground's `ghost`).
        let ghost = lifted.and_then(|square| {
            let piece = self.pieces.piece_at(square)?;
            let (column, row) = self.cell(square);
            Some(
                img(SharedString::from(looks::piece_path(&set, piece)))
                    .absolute()
                    .left(relative(column as f32 / 8.))
                    .top(relative(row as f32 / 8.))
                    .size(relative(0.125))
                    .opacity(0.3),
            )
        });
        let held = self.drag.as_ref().filter(|drag| drag.moved).map(|drag| {
            deferred(
                img(SharedString::from(looks::piece_path(&set, drag.piece)))
                    .absolute()
                    .left(drag.position.x - origin.x - side / 2.)
                    .top(drag.position.y - origin.y - side / 2.)
                    .size(side)
                    .cursor_grabbing(),
            )
            .with_priority(1)
        });
        let coordinates = if self.state.preview {
            Vec::new()
        } else {
            self.render_coordinates(frontend.coordinates, look, cx)
        };
        let promotion = self.render_promotion(&set, cx);

        div()
            .id("chess-board")
            .relative()
            .w_full()
            .aspect_ratio(1.)
            .child(
                img(SharedString::from(format!("boards/{}", look.image)))
                    .absolute()
                    .size_full()
                    .object_fit(ObjectFit::Fill)
                    .rounded(BOARD_RADIUS),
            )
            .children(check)
            .child(
                canvas(
                    move |canvas_bounds, _, _| bounds.set(canvas_bounds),
                    move |canvas_bounds, (), window, cx| {
                        paint_frame(frame, canvas_bounds, window, cx);
                        if dragging {
                            // A drag follows the pointer anywhere in the window, not only over the board.
                            let moving = entity.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase == DispatchPhase::Bubble {
                                    let _ = moving.update(cx, |this, cx| {
                                        // A release this listener missed (it arrived before
                                        // this frame) ends the drag at the next move.
                                        if event.pressed_button == Some(MouseButton::Left) {
                                            this.pointer_moved(event.position, cx)
                                        } else {
                                            this.release(event.position, cx)
                                        }
                                    });
                                }
                            });
                            let releasing = entity.clone();
                            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Bubble
                                    && event.button == MouseButton::Left
                                {
                                    let _ = releasing
                                        .update(cx, |this, cx| this.release(event.position, cx));
                                }
                            });
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(ghost)
            .children(coordinates)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, _, cx| this.press(event, cx)),
            )
            // The press's own frame has no window-wide listeners yet: a quick release lands here.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.release(event.position, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.release(event.position, cx)),
            )
            .children(held)
            .children(promotion)
            .cursor(if self.state.movable.is_some() {
                CursorStyle::PointingHand
            } else {
                CursorStyle::Arrow
            })
    }
}
