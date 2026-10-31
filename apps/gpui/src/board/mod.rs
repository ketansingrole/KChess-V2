//! The chessboard, as Chessground draws it in the Electron app: the chosen board
//! and pieces, the last move, check, the selected piece and where it can go, coordinates,
//! click-to-move and drag-and-drop, and the promotion picker. Squares, highlights and pieces are
//! painted in one pass; the page gives the board its position and hears its moves.

pub mod looks;

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use kchess_contracts::types::{CoordinateMode, PromotionMode};
use shakmaty::variant::VariantPosition;
use shakmaty::{Board, Color, File, Piece, Position as _, Rank, Role, Square};

use crate::settings::SettingsHandle;
use crate::theme::radii;

/// Chessground's square colors (`chessground.brown.css`).
const LAST_MOVE: u32 = 0x9bc70069; // rgba(155, 199, 0, 0.41)
const SELECTED: u32 = 0x14551e80; // rgba(20, 85, 30, 0.5)
const DEST_DOT: u32 = 0x14551e80; // rgba(20, 85, 30, 0.5)
const DEST_CORNER: u32 = 0x1455004d; // rgba(20, 85, 0, 0.3)
const DEST_HOVER: u32 = 0x14551e4d; // rgba(20, 85, 30, 0.3)
/// Chessground's queued premove and its targets (`current-premove`, `premove-dest`).
const PREMOVE: u32 = 0x141e5580; // rgba(20, 30, 85, 0.5)
const PREMOVE_DOT: u32 = 0x141e5580;
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
    /// A puzzle hint: the piece to move, ringed under it.
    pub hint: Option<Square>,
    /// Rings over squares (a drill's target, a right or wrong answer).
    pub circles: Vec<Circle>,
    /// Coordinates other than Settings' (the coordinate drills hide or show them themselves).
    pub coordinates: Option<CoordinateMode>,
    /// Every square's name on it (`coordinatesOnSquares`).
    pub square_names: bool,
    /// The board editor's tool: pieces move anywhere, are put down or taken off.
    pub edit: Option<EditTool>,
    /// A review's label on the move just played.
    pub badge: Option<Badge>,
    /// The side that may queue a move for its next turn while the other side is to move.
    pub premove: Option<Color>,
}

/// A move's label as a badge over its square's top-right corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    pub square: Square,
    pub glyph: &'static str,
    pub color: Hsla,
}

/// The board editor's tools (`EditorTool`): move pieces (dragged off the board, they are gone),
/// put one piece down (or take it off again where it already stands), or take pieces off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTool {
    Pointer,
    Place(Piece),
    Trash,
}

/// A spare piece dragged onto the editor's board.
pub struct SparePiece(pub Piece);

/// A ring over a square in a Chessground brush color (`yellow`, `green`, `red`).
#[derive(Clone, Debug, PartialEq)]
pub struct Circle {
    pub square: Square,
    pub color: u32,
}

impl Circle {
    /// A circle in one of Chessground's brushes.
    pub fn brush(brush: &str, square: Square) -> Circle {
        Circle {
            square,
            color: brush_color(brush),
        }
    }

    pub fn yellow(square: Square) -> Circle {
        Circle::brush("yellow", square)
    }

    pub fn green(square: Square) -> Circle {
        Circle::brush("green", square)
    }

    pub fn red(square: Square) -> Circle {
        Circle::brush("red", square)
    }
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

/// Chessground's brushes by name: `green`, `red`, `blue` and `yellow`.
pub fn brush_color(brush: &str) -> u32 {
    match brush {
        "red" => 0x882020,
        "blue" => 0x003088,
        "yellow" => 0xe68f00,
        _ => 0x15781b,
    }
}

impl Arrow {
    /// An arrow in one of Chessground's brushes (a lesson's drawings).
    pub fn brush(brush: &str, from: Square, to: Square) -> Arrow {
        Arrow {
            from,
            to,
            color: brush_color(brush),
            opacity: 1.,
            width: 10.,
        }
    }

    /// The solution's next move, shown (the `green` brush).
    pub fn solution(from: Square, to: Square) -> Arrow {
        Arrow::brush("green", from, to)
    }

    /// A move shown on request (the `blue` brush).
    pub fn hint(from: Square, to: Square) -> Arrow {
        Arrow::brush("blue", from, to)
    }

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
            hint: None,
            circles: Vec::new(),
            coordinates: None,
            square_names: false,
            edit: None,
            badge: None,
            premove: None,
        }
    }
}

impl BoardState {
    /// A position only looked at (a live game, a broadcast board): its FEN, which may be just the
    /// placement and the side to move, and the last move in UCI.
    pub fn shown(fen: &str, variant: &str, orientation: Color, last: Option<&str>) -> BoardState {
        let fields = fen.split_whitespace().count();
        let full = if fields >= 4 {
            fen.to_string()
        } else {
            format!(
                "{} - - 0 1",
                fen.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
            )
        };
        let last = last.and_then(|uci| {
            uci.get(0..2)
                .and_then(|from| from.parse().ok())
                .zip(uci.get(2..4).and_then(|to| to.parse().ok()))
        });
        match kchess_domain::rules::setup_start(variant, &full)
            .or_else(|| kchess_domain::rules::setup_start("standard", &full))
        {
            Some(position) => BoardState::of(&position, orientation, None, last),
            None => {
                let mut state = BoardState::of(
                    &VariantPosition::Chess(shakmaty::Chess::default()),
                    orientation,
                    None,
                    last,
                );
                state.fen = fen.to_string();
                state.check = None;
                state
            }
        }
    }
}

/// What the board tells its page.
pub enum BoardEvent {
    /// A move the player made, in UCI (`e2e4`, `e7e8q`, castling as king to rook or two squares).
    Move(String),
    /// A square was clicked (`select`): the coordinate drills' answers.
    Select(Square),
    /// The editor's pieces changed: their placement (a FEN's first field).
    Edited(String),
}

struct Drag {
    from: Square,
    piece: Piece,
    /// Where the pointer is, in window coordinates.
    position: Point<Pixels>,
    start: Point<Pixels>,
    moved: bool,
}

/// Pieces sliding from one square to another.
struct Animation {
    moves: Vec<(Piece, Square, Square)>,
    start: std::time::Instant,
    duration: std::time::Duration,
}

impl Animation {
    /// How far along, eased as Chessground eases (cubic in, then out).
    fn progress(&self) -> f32 {
        let t = (self.start.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.);
        if t < 0.5 {
            4. * t * t * t
        } else {
            (t - 1.) * (2. * t - 2.) * (2. * t - 2.) + 1.
        }
    }
}

/// The pieces that moved from `old` to `new`: each piece that left a square, paired with the
/// nearest square a like piece arrived on. More than a few is a new position, not a move.
fn slides(old: &Board, new: &Board) -> Vec<(Piece, Square, Square)> {
    let left: Vec<(Square, Piece)> = old
        .occupied()
        .into_iter()
        .filter_map(|square| {
            let piece = old.piece_at(square)?;
            (new.piece_at(square) != Some(piece)).then_some((square, piece))
        })
        .collect();
    let mut arrived: Vec<(Square, Piece)> = new
        .occupied()
        .into_iter()
        .filter_map(|square| {
            let piece = new.piece_at(square)?;
            (old.piece_at(square) != Some(piece)).then_some((square, piece))
        })
        .collect();
    let mut moves = Vec::new();
    for (from, piece) in left {
        let nearest = arrived
            .iter()
            .enumerate()
            .filter(|(_, (_, p))| *p == piece)
            .min_by_key(|(_, (to, _))| from.distance(*to))
            .map(|(index, _)| index);
        if let Some(index) = nearest {
            let (to, _) = arrived.remove(index);
            moves.push((piece, from, to));
        }
    }
    if moves.len() > 4 { Vec::new() } else { moves }
}

/// Where a piece could go next turn, whatever stands in the way (Chessground's `premove`).
fn premove_targets(pieces: &Board, from: Square) -> Vec<Square> {
    let Some(piece) = pieces.piece_at(from) else {
        return Vec::new();
    };
    let (x1, y1) = (i32::from(from.file()), i32::from(from.rank()));
    let forward = if piece.color == Color::White { 1 } else { -1 };
    let start_rank = if piece.color == Color::White { 1 } else { 6 };
    let fits = |x2: i32, y2: i32| -> bool {
        let (dx, dy) = ((x2 - x1).abs(), (y2 - y1).abs());
        match piece.role {
            Role::Pawn => {
                (dx == 0 && y2 - y1 == forward)
                    || (dx == 0 && y1 == start_rank && y2 - y1 == 2 * forward)
                    || (dx == 1 && y2 - y1 == forward)
            }
            Role::Knight => (dx == 1 && dy == 2) || (dx == 2 && dy == 1),
            Role::Bishop => dx == dy,
            Role::Rook => dx == 0 || dy == 0,
            Role::Queen => dx == dy || dx == 0 || dy == 0,
            Role::King => {
                let home = if piece.color == Color::White { 0 } else { 7 };
                let castling = y1 == home && y2 == home && x1 == 4 && (x2 == 2 || x2 == 6) && {
                    let rook = if x2 == 6 { 7 } else { 0 };
                    pieces.piece_at(Square::from_coords(File::new(rook), Rank::new(home as u32)))
                        == Some(Piece {
                            color: piece.color,
                            role: Role::Rook,
                        })
                };
                (dx <= 1 && dy <= 1) || castling
            }
        }
    };
    Square::ALL
        .into_iter()
        .filter(|&to| to != from && fits(i32::from(to.file()), i32::from(to.rank())))
        .collect()
}

/// A shape the player drew with the right button: an arrow, or a circle where `from == to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Drawn {
    from: Square,
    to: Square,
    brush: &'static str,
}

/// Chessground's brush for a right-button drawing (`eventBrush`): Shift or Ctrl red, Alt or ⌘
/// blue, both yellow, else green.
fn drawing_brush(modifiers: &Modifiers) -> &'static str {
    let a = modifiers.shift || modifiers.control;
    let b = modifiers.alt || modifiers.platform;
    ["green", "red", "blue", "yellow"][usize::from(a) + 2 * usize::from(b)]
}

/// Draw `shape`: the same shape again takes it off, the same squares in another brush recolour
/// it (Chessground's `addShape`).
fn toggle_shape(shapes: &mut Vec<Drawn>, shape: Drawn) {
    match shapes
        .iter()
        .position(|s| s.from == shape.from && s.to == shape.to)
    {
        Some(index) if shapes[index].brush == shape.brush => {
            shapes.remove(index);
        }
        Some(index) => shapes[index] = shape,
        None => shapes.push(shape),
    }
}

pub struct ChessBoard {
    state: BoardState,
    pieces: Board,
    /// The player's right-button drawings, until a left click clears them.
    drawn: Vec<Drawn>,
    /// A drawing under way: where it started, its brush, and the square under the pointer.
    drawing: Option<(Square, &'static str, Option<Square>)>,
    /// A move queued for the next turn (a premove).
    queued: Option<(Square, Square)>,
    /// Pieces sliding to where the new position put them.
    animation: Option<Animation>,
    _animating: Option<Task<()>>,
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
            drawn: Vec::new(),
            drawing: None,
            queued: None,
            animation: None,
            _animating: None,
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
            let next = board_of(&state.fen);
            if !state.preview {
                self.animate(&next, state.orientation, cx);
            }
            self.pieces = next;
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
        self.play_queued(cx);
        cx.notify();
    }

    /// Put the pieces back as the state has them: a move the board played at once and the page
    /// refused goes back, which `set_state` cannot see when the position did not change.
    pub fn undo_move(&mut self, cx: &mut Context<Self>) {
        self.pieces = board_of(&self.state.fen);
        self.promotion = None;
        cx.notify();
    }

    /// Slide the pieces from `self.pieces` to `next` (Chessground's `animate`): each piece that
    /// left a square goes to the nearest square a like piece arrived on.
    fn animate(&mut self, next: &Board, orientation: Color, cx: &mut Context<Self>) {
        let choice = cx
            .global::<SettingsHandle>()
            .0
            .read(cx)
            .settings
            .frontend
            .piece_animation;
        let choice = serde_json::to_value(choice)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "normal".into());
        let duration = looks::animation_ms(&choice);
        let moves = slides(&self.pieces, next);
        // Same side up: a flipped board is redrawn, not slid.
        if duration <= 0. || moves.is_empty() || orientation != self.state.orientation {
            self.animation = None;
            return;
        }
        self.animation = Some(Animation {
            moves,
            start: std::time::Instant::now(),
            duration: std::time::Duration::from_secs_f64(duration / 1000.),
        });
        self._animating = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let going = this
                    .update(cx, |this, cx| {
                        cx.notify();
                        let done = this.animation.as_ref().is_none_or(|animation| {
                            animation.start.elapsed() >= animation.duration
                        });
                        if done {
                            this.animation = None;
                        }
                        !done
                    })
                    .unwrap_or(false);
                if !going {
                    return;
                }
            }
        }));
    }

    /// A queued premove plays once its side is to move and it is legal; otherwise it is dropped.
    fn play_queued(&mut self, cx: &mut Context<Self>) {
        let Some((from, to)) = self.queued else {
            return;
        };
        let mine = self.pieces.piece_at(from).map(|piece| piece.color);
        if self.state.movable.is_some() && self.state.movable == mine {
            self.queued = None;
            if self.reaches(from, to) {
                self.play_move(from, to, true, cx);
            }
        } else if self.state.premove.is_none() || self.state.premove != mine {
            self.queued = None;
        }
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

    /// A premove from `from` to `to` can be queued.
    fn premoves_to(&self, from: Square, to: Square) -> bool {
        self.state.movable.is_none()
            && self
                .pieces
                .piece_at(from)
                .is_some_and(|piece| Some(piece.color) == self.state.premove)
            && premove_targets(&self.pieces, from).contains(&to)
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
        // A left click clears the drawings (`eraseOnClick`); Shift keeps them.
        if !event.modifiers.shift && !self.drawn.is_empty() {
            self.drawn.clear();
            cx.notify();
        }
        let Some(square) = self.square_at(event.position) else {
            return;
        };
        if let Some(tool) = self.state.edit {
            return self.edit_press(tool, square, event.position, cx);
        }
        cx.emit(BoardEvent::Select(square));
        // A press while a premove waits takes it back (Chessground's `cancelPremove`).
        if self.queued.take().is_some() {
            self.selected = None;
            cx.notify();
            return;
        }
        if let Some(from) = self.selected
            && from != square
            && self.reaches(from, square)
        {
            self.selected = None;
            self.play(from, square, cx);
            return;
        }
        if let Some(from) = self.selected
            && from != square
            && self.premoves_to(from, square)
        {
            self.selected = None;
            self.queued = Some((from, square));
            cx.notify();
            return;
        }
        match self.pieces.piece_at(square) {
            Some(piece)
                if (Some(piece.color) == self.state.movable && self.can_move_from(square))
                    || (Some(piece.color) == self.state.premove
                        && self.state.movable.is_none()) =>
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

    /// The right button starts a drawing on a square.
    fn start_drawing(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if self.state.preview || self.state.edit.is_some() {
            return;
        }
        // The right button also takes back a queued premove, as on Lichess.
        if self.queued.take().is_some() {
            cx.notify();
        }
        if let Some(square) = self.square_at(event.position) {
            self.drawing = Some((square, drawing_brush(&event.modifiers), Some(square)));
            cx.notify();
        }
    }

    fn draw_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let square = self.square_at(position);
        if let Some((_, _, current)) = &mut self.drawing
            && *current != square
        {
            *current = square;
            cx.notify();
        }
    }

    /// Letting go draws the arrow (or the circle, on the same square), or takes it off.
    fn finish_drawing(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((from, brush, _)) = self.drawing.take() else {
            return;
        };
        if let Some(to) = self.square_at(position) {
            toggle_shape(&mut self.drawn, Drawn { from, to, brush });
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

    /// The editor's press: put a piece down or take it off, or pick one up to move it.
    fn edit_press(
        &mut self,
        tool: EditTool,
        square: Square,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        match tool {
            EditTool::Place(piece) => {
                if self.pieces.piece_at(square) == Some(piece) {
                    self.pieces.discard_piece_at(square);
                } else {
                    self.pieces.set_piece_at(square, piece);
                }
                self.edited(cx);
            }
            EditTool::Trash => {
                if self.pieces.remove_piece_at(square).is_some() {
                    self.edited(cx);
                }
            }
            EditTool::Pointer => {
                if let Some(piece) = self.pieces.piece_at(square) {
                    self.drag = Some(Drag {
                        from: square,
                        piece,
                        position,
                        start: position,
                        moved: false,
                    });
                }
            }
        }
        cx.notify();
    }

    /// A spare piece dropped on the board.
    fn drop_spare(&mut self, piece: Piece, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.state.edit.is_none() {
            return;
        }
        if let Some(square) = self.square_at(position) {
            self.pieces.set_piece_at(square, piece);
            self.edited(cx);
        }
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        cx.emit(BoardEvent::Edited(self.pieces.to_string()));
        cx.notify();
    }

    fn release(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let target = self.square_at(position);
        if self.state.edit.is_some() {
            // Dragged anywhere on the board, the piece moves there; dragged off it, it is gone.
            if drag.moved || far(drag.start, position) {
                match target {
                    Some(to) if to == drag.from => {}
                    Some(to) => {
                        self.pieces.discard_piece_at(drag.from);
                        self.pieces.set_piece_at(to, drag.piece);
                        self.edited(cx);
                    }
                    None => {
                        self.pieces.discard_piece_at(drag.from);
                        self.edited(cx);
                    }
                }
            }
            cx.notify();
            return;
        }
        // The moves of a quick drag may all have come before the board was listening.
        if drag.moved || far(drag.start, position) {
            match target {
                Some(to) if self.reaches(drag.from, to) => {
                    self.selected = None;
                    self.play(drag.from, to, cx);
                }
                Some(to) if to != drag.from && self.premoves_to(drag.from, to) => {
                    self.selected = None;
                    self.queued = Some((drag.from, to));
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
        self.play_move(from, to, false, cx);
    }

    /// Play a move; a premove's promotion becomes a queen when Settings say so for premoves.
    fn play_move(&mut self, from: Square, to: Square, premove: bool, cx: &mut Context<Self>) {
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
            if mode == PromotionMode::Queen || (premove && mode == PromotionMode::Premove) {
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

    /// `coords.squares`: each square's name in its top corner, in the other square color.
    fn render_square_names(&self, look: &looks::BoardLook) -> Vec<AnyElement> {
        Square::ALL
            .into_iter()
            .map(|square| {
                let (column, row) = self.cell(square);
                div()
                    .absolute()
                    .left(relative(column as f32 / 8.))
                    .top(relative(row as f32 / 8.))
                    .size(relative(0.125))
                    .flex()
                    .justify_end()
                    .pt(relative(0.06 / 8.))
                    .pr(relative(0.04 / 8.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if square.is_light() {
                        looks::hex(&look.coord_black)
                    } else {
                        looks::hex(&look.coord_white)
                    })
                    .opacity(0.8)
                    .child(square.to_string())
                    .into_any_element()
            })
            .collect()
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
                    .child(file.char().to_string())
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
                .rounded(radii::XL)
                .bg(hsla(0., 0., 0., 0.5))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.cancel_promotion(cx)),
                )
                .child(
                    div()
                        .id("promotion-choices")
                        .role(gpui_kit::Role::Dialog)
                        .aria_label("Choose promotion piece")
                        .flex()
                        .gap(px(6.))
                        .p(px(8.))
                        .rounded(radii::XL)
                        .bg(elevated)
                        .border_1()
                        .border_color(border)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .children(choices.into_iter().map(|(role, label)| {
                            div()
                                .id(label)
                                .role(gpui_kit::Role::Button)
                                .aria_label(label)
                                .size(px(60.))
                                .p(px(4.))
                                // Concentric with the picker's corners, 8px further out.
                                .rounded(radii::SM)
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
    /// The queued premove's squares, and the targets of a piece picked for one.
    queued: Option<(Square, Square)>,
    premove_dests: Vec<Square>,
    /// Squares whose pieces are sliding in, drawn on top as they move.
    sliding: Vec<Square>,
}

fn paint_frame(frame: Frame, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
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
        let corner = |at: bool| if at { radii::XL.min(side) } else { px(0.) };
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

/// Chessground's circle (`renderCircle`): a ring 4/64 of a square wide, 0.47 of a square from
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

/// The puzzle hint (`HINT_MARK`): a thick green ring on a soft white halo,
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

/// The board as a cached view (`Entity::cached`): its last frame is reused while only its parent
/// redraws (a clock tick, an engine line) and drawn again when the board itself changes
/// (`set_state`, a drag, an animation step) or the windows refresh (settings, preferences). It
/// fills its slot's width and is square, as its own root is.
pub fn cached(board: &Entity<ChessBoard>) -> AnyElement {
    let mut style = StyleRefinement::default();
    style.size.width = Some(relative(1.).into());
    style.aspect_ratio = Some(1.);
    board.clone().cached(style).into_any_element()
}

/// What a screen reader says of the board: the last move and
/// the side to move.
fn describe(state: &BoardState) -> String {
    let to_move = if state.fen.split_whitespace().nth(1) == Some("b") {
        "Black to move"
    } else {
        "White to move"
    };
    match state.last_move {
        Some((from, to)) => format!("Last move {from}–{to} · {to_move}"),
        None => to_move.to_string(),
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
        let mut shown = self.state.clone();
        let preview = self
            .drawing
            .and_then(|(from, brush, to)| to.map(|to| Drawn { from, to, brush }));
        for shape in self.drawn.iter().chain(preview.iter()) {
            if shape.from == shape.to {
                shown.circles.push(Circle::brush(shape.brush, shape.from));
            } else {
                shown
                    .arrows
                    .push(Arrow::brush(shape.brush, shape.from, shape.to));
            }
        }
        let premove_dests = self
            .selected
            .filter(|&square| {
                self.state.movable.is_none()
                    && self
                        .pieces
                        .piece_at(square)
                        .is_some_and(|piece| Some(piece.color) == self.state.premove)
            })
            .map(|square| premove_targets(&self.pieces, square))
            .unwrap_or_default();
        let sliding: Vec<Square> = self
            .animation
            .as_ref()
            .map(|animation| animation.moves.iter().map(|(_, _, to)| *to).collect())
            .unwrap_or_default();
        let frame = Frame {
            state: shown,
            pieces: self.pieces.clone(),
            selected: self.selected,
            lifted,
            hovered,
            show_dests: frontend.show_legal_moves,
            set: set.clone(),
            queued: self.queued,
            premove_dests,
            sliding,
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
        // Sliding pieces, between where they were and where they land.
        let sliding: Vec<AnyElement> = if self.state.blindfold {
            Vec::new()
        } else {
            self.animation
                .as_ref()
                .map(|animation| {
                    let t = animation.progress();
                    animation
                        .moves
                        .iter()
                        .map(|(piece, from, to)| {
                            let (c1, r1) = self.cell(*from);
                            let (c2, r2) = self.cell(*to);
                            let column = c1 as f32 + (c2 as f32 - c1 as f32) * t;
                            let row = r1 as f32 + (r2 as f32 - r1 as f32) * t;
                            img(SharedString::from(looks::piece_path(&set, *piece)))
                                .absolute()
                                .left(relative(column / 8.))
                                .top(relative(row / 8.))
                                .size(relative(0.125))
                                .into_any_element()
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
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
        let mut coordinates = if self.state.preview {
            Vec::new()
        } else {
            self.render_coordinates(
                self.state.coordinates.unwrap_or(frontend.coordinates),
                look,
                cx,
            )
        };
        if self.state.square_names {
            coordinates.extend(self.render_square_names(look));
        }
        let promotion = self.render_promotion(&set, cx);
        // The badge overhangs the square's top-right corner, except where that would leave the
        // board.
        let badge = self
            .state
            .badge
            .as_ref()
            .filter(|_| !self.state.preview)
            .map(|badge| {
                let (column, row) = self.cell(badge.square);
                let diameter = (side * 0.42).max(px(16.));
                let x = if column == 7 {
                    side - diameter
                } else {
                    side - diameter * 0.68
                };
                let y = if row == 0 { px(0.) } else { -diameter * 0.32 };
                div()
                    .absolute()
                    .left(side * column as f32 + x)
                    .top(side * row as f32 + y)
                    .size(diameter)
                    .rounded_full()
                    .bg(badge.color)
                    .border_1()
                    .border_color(rgb(0xffffff))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(diameter * 0.55)
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0xffffff))
                    .child(badge.glyph)
            });

        let description = (!self.state.preview).then(|| describe(&self.state));
        div()
            .id("chess-board")
            .test_support()
            // Pictures in a list are named by what holds them; a board to play on is a group, and
            // says each move as it is played (an `aria-live` line).
            .when_some(description, |board, description| {
                board
                    .role(gpui_kit::Role::Group)
                    .aria_label("Chess board")
                    .aria_description(description.clone())
                    .child(crate::widgets::live_region("board-live", description))
            })
            .relative()
            .w_full()
            .aspect_ratio(1.)
            .child(
                img(SharedString::from(format!("boards/{}", look.image)))
                    .absolute()
                    .size_full()
                    .object_fit(ObjectFit::Fill)
                    .rounded(radii::XL),
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
            .children(sliding)
            .children(coordinates)
            .children(badge)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, _, cx| this.press(event, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event, _, cx| {
                    cx.stop_propagation();
                    this.start_drawing(event, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.drawing.is_some() {
                    this.draw_to(event.position, cx)
                }
            }))
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.finish_drawing(event.position, cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.finish_drawing(event.position, cx)
                }),
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
            .on_drop(cx.listener(|this, spare: &SparePiece, window, cx| {
                this.drop_spare(spare.0, window.mouse_position(), cx)
            }))
            .cursor(match self.state.edit {
                Some(EditTool::Place(_) | EditTool::Trash) => CursorStyle::Crosshair,
                Some(EditTool::Pointer) => CursorStyle::PointingHand,
                None if self.state.movable.is_some() => CursorStyle::PointingHand,
                None => CursorStyle::Arrow,
            })
    }
}

#[cfg(test)]
mod drawing_tests {
    use super::{Drawn, drawing_brush, toggle_shape};
    use gpui_kit::Modifiers;
    use shakmaty::Square;

    #[test]
    fn modifiers_pick_chessgrounds_brushes() {
        let none = Modifiers::default();
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let both = Modifiers {
            shift: true,
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(drawing_brush(&none), "green");
        assert_eq!(drawing_brush(&shift), "red");
        assert_eq!(drawing_brush(&alt), "blue");
        assert_eq!(drawing_brush(&both), "yellow");
    }

    #[test]
    fn drawing_again_takes_off_or_recolours() {
        let arrow = |brush| Drawn {
            from: Square::E2,
            to: Square::E4,
            brush,
        };
        let mut shapes = Vec::new();
        toggle_shape(&mut shapes, arrow("green"));
        assert_eq!(shapes, [arrow("green")]);
        toggle_shape(&mut shapes, arrow("red"));
        assert_eq!(shapes, [arrow("red")]);
        toggle_shape(&mut shapes, arrow("red"));
        assert!(shapes.is_empty());
    }
}

#[cfg(test)]
mod premove_tests {
    use super::{board_of, premove_targets, slides};
    use shakmaty::Square;

    #[test]
    fn premoves_ignore_what_stands_in_the_way() {
        let start = board_of("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let rook = premove_targets(&start, Square::A1);
        assert!(rook.contains(&Square::A8) && rook.contains(&Square::H1));
        let pawn = premove_targets(&start, Square::E2);
        assert_eq!(pawn.len(), 4); // e3, e4, d3, f3
        let king = premove_targets(&start, Square::E1);
        assert!(king.contains(&Square::G1) && king.contains(&Square::C1));
    }

    #[test]
    fn a_move_slides_its_piece_and_a_new_game_jumps() {
        let before = board_of("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let after = board_of("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1");
        let moves = slides(&before, &after);
        assert_eq!(moves.len(), 1);
        assert_eq!((moves[0].1, moves[0].2), (Square::E2, Square::E4));
        let castled = board_of("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQ1RK1 b kq - 0 1");
        let castle = board_of("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQK2R w KQkq - 0 1");
        assert_eq!(slides(&castle, &castled).len(), 2);
        let empty = board_of("8/8/8/8/8/8/8/8 w - - 0 1");
        assert!(slides(&empty, &before).is_empty());
    }
}

#[cfg(test)]
mod description_tests {
    use super::{BoardState, describe};
    use shakmaty::Color;

    /// A screen reader hears the last move and whose turn it is, as Electron's status line says.
    #[test]
    fn the_board_says_the_last_move_and_whose_turn_it_is() {
        let start = BoardState::shown(
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "standard",
            Color::White,
            None,
        );
        assert_eq!(describe(&start), "White to move");
        let after = BoardState::shown(
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
            "standard",
            Color::White,
            Some("e2e4"),
        );
        assert_eq!(describe(&after), "Last move e2–e4 · Black to move");
    }
}
