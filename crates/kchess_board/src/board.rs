use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

pub const BOARD_SIDE: usize = 8;
pub const BOARD_SQUARES: usize = BOARD_SIDE * BOARD_SIDE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Square(u8);

impl Square {
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub const fn file(self) -> u8 {
        self.0 % 8
    }

    pub const fn rank(self) -> u8 {
        self.0 / 8
    }

    pub fn from_index(index: usize) -> Option<Self> {
        if index < BOARD_SQUARES {
            Some(Self(index as u8))
        } else {
            None
        }
    }

    pub fn from_file_rank(file: u8, rank: u8) -> Option<Self> {
        if file < 8 && rank < 8 {
            Some(Self(rank * 8 + file))
        } else {
            None
        }
    }

    pub fn from_algebraic(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() != 2 {
            return None;
        }

        let file = match bytes[0] {
            b'a'..=b'h' => bytes[0] - b'a',
            b'A'..=b'H' => bytes[0] - b'A',
            _ => return None,
        };

        let rank = match bytes[1] {
            b'1'..=b'8' => bytes[1] - b'1',
            _ => return None,
        };

        Self::from_file_rank(file, rank)
    }

    pub fn algebraic(self) -> String {
        let file = (b'a' + self.file()) as char;
        let rank = (b'1' + self.rank()) as char;
        format!("{file}{rank}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    White,
    Black,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PieceKind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Piece {
    pub side: Side,
    pub kind: PieceKind,
}

impl Piece {
    pub const fn unicode(self) -> char {
        match (self.side, self.kind) {
            (Side::White, PieceKind::Pawn) => '♙',
            (Side::White, PieceKind::Knight) => '♘',
            (Side::White, PieceKind::Bishop) => '♗',
            (Side::White, PieceKind::Rook) => '♖',
            (Side::White, PieceKind::Queen) => '♕',
            (Side::White, PieceKind::King) => '♔',
            (Side::Black, PieceKind::Pawn) => '♟',
            (Side::Black, PieceKind::Knight) => '♞',
            (Side::Black, PieceKind::Bishop) => '♝',
            (Side::Black, PieceKind::Rook) => '♜',
            (Side::Black, PieceKind::Queen) => '♛',
            (Side::Black, PieceKind::King) => '♚',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MoveId(u64);

impl MoveId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveRequest {
    pub from: Square,
    pub to: Square,
    pub promotion: Option<PieceKind>,
}

impl MoveRequest {
    pub const fn new(from: Square, to: Square) -> Self {
        Self {
            from,
            to,
            promotion: None,
        }
    }

    pub const fn with_promotion(from: Square, to: Square, promotion: PieceKind) -> Self {
        Self {
            from,
            to,
            promotion: Some(promotion),
        }
    }

    pub fn from_algebraic(from: &str, to: &str) -> Option<Self> {
        Some(Self::new(
            Square::from_algebraic(from)?,
            Square::from_algebraic(to)?,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    EaseInOutCubic,
}

impl Easing {
    fn apply(self, t: f32) -> f32 {
        match self {
            Self::Linear => t,
            Self::EaseInOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let f = -2.0 * t + 2.0;
                    1.0 - (f * f * f) / 2.0
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct AnimationState {
    pub piece: Piece,
    pub from: Square,
    pub to: Square,
    pub from_override_xy: Option<(f32, f32)>,
    pub started_at: Instant,
    pub duration: Duration,
    pub easing: Easing,
}

impl AnimationState {
    pub fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started_at);
        let raw = (elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        self.easing.apply(raw)
    }

    pub fn is_finished(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started_at) >= self.duration
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderPiece {
    pub piece: Piece,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Perspective {
    White,
    Black,
}

#[derive(Clone, Debug)]
pub struct AppliedMove {
    pub id: MoveId,
    pub request: MoveRequest,
    pub moved_piece_before: Piece,
    pub moved_piece_after: Piece,
    pub captured_piece: Option<Piece>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardError {
    NoPieceAtSource(Square),
    SourceEqualsDestination(Square),
    PendingMoveExists(MoveId),
    NoPendingMove,
    PendingMoveIdMismatch { expected: MoveId, got: MoveId },
    NoMoveToUndo,
}

impl fmt::Display for BoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoardError::NoPieceAtSource(square) => {
                write!(f, "no piece found at source square {}", square.algebraic())
            }
            BoardError::SourceEqualsDestination(square) => {
                write!(
                    f,
                    "source and destination are the same square: {}",
                    square.algebraic()
                )
            }
            BoardError::PendingMoveExists(id) => {
                write!(
                    f,
                    "cannot apply move while pending move {} exists",
                    id.get()
                )
            }
            BoardError::NoPendingMove => write!(f, "there is no pending move"),
            BoardError::PendingMoveIdMismatch { expected, got } => write!(
                f,
                "pending move id mismatch: expected {}, got {}",
                expected.get(),
                got.get()
            ),
            BoardError::NoMoveToUndo => write!(f, "move history is empty"),
        }
    }
}

impl Error for BoardError {}

#[derive(Clone, Debug)]
pub struct VisualBoard {
    squares: [Option<Piece>; BOARD_SQUARES],
    history: Vec<AppliedMove>,
    pending: Option<AppliedMove>,
    next_move_id: u64,
    active_animation: Option<AnimationState>,
    animation_duration: Duration,
    animation_easing: Easing,
}

impl Default for VisualBoard {
    fn default() -> Self {
        Self::standard()
    }
}

impl VisualBoard {
    pub fn empty() -> Self {
        Self {
            squares: [None; BOARD_SQUARES],
            history: Vec::new(),
            pending: None,
            next_move_id: 1,
            active_animation: None,
            animation_duration: Duration::from_millis(260),
            animation_easing: Easing::EaseInOutCubic,
        }
    }

    pub fn standard() -> Self {
        let mut board = Self::empty();

        for file in 0..8 {
            let white_pawn = Square::from_file_rank(file, 1).expect("valid square");
            let black_pawn = Square::from_file_rank(file, 6).expect("valid square");
            board.squares[white_pawn.index()] = Some(Piece {
                side: Side::White,
                kind: PieceKind::Pawn,
            });
            board.squares[black_pawn.index()] = Some(Piece {
                side: Side::Black,
                kind: PieceKind::Pawn,
            });
        }

        let back_rank = [
            PieceKind::Rook,
            PieceKind::Knight,
            PieceKind::Bishop,
            PieceKind::Queen,
            PieceKind::King,
            PieceKind::Bishop,
            PieceKind::Knight,
            PieceKind::Rook,
        ];

        for (file, kind) in back_rank.into_iter().enumerate() {
            let white_square = Square::from_file_rank(file as u8, 0).expect("valid square");
            let black_square = Square::from_file_rank(file as u8, 7).expect("valid square");

            board.squares[white_square.index()] = Some(Piece {
                side: Side::White,
                kind,
            });
            board.squares[black_square.index()] = Some(Piece {
                side: Side::Black,
                kind,
            });
        }

        board
    }

    pub fn set_animation_profile(&mut self, duration: Duration, easing: Easing) {
        self.animation_duration = duration.max(Duration::from_millis(16));
        self.animation_easing = easing;
    }

    pub fn clear(&mut self) {
        self.squares = [None; BOARD_SQUARES];
        self.history.clear();
        self.pending = None;
        self.next_move_id = 1;
        self.active_animation = None;
    }

    pub fn piece_at(&self, square: Square) -> Option<Piece> {
        self.squares[square.index()]
    }

    pub fn set_piece(&mut self, square: Square, piece: Option<Piece>) {
        self.squares[square.index()] = piece;
    }

    pub fn squares(&self) -> &[Option<Piece>; BOARD_SQUARES] {
        &self.squares
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn pending_move_id(&self) -> Option<MoveId> {
        self.pending.as_ref().map(|pending| pending.id)
    }

    pub fn active_animation(&self) -> Option<&AnimationState> {
        self.active_animation.as_ref()
    }

    pub fn prune_finished_animation(&mut self, now: Instant) {
        if self
            .active_animation
            .as_ref()
            .is_some_and(|animation| animation.is_finished(now))
        {
            self.active_animation = None;
        }
    }

    pub fn apply_move(&mut self, request: MoveRequest, now: Instant) -> Result<MoveId, BoardError> {
        if let Some(pending) = self.pending.as_ref() {
            return Err(BoardError::PendingMoveExists(pending.id));
        }

        let applied = self.apply_move_inner(request, now, None)?;
        let id = applied.id;
        self.history.push(applied);
        Ok(id)
    }

    pub fn apply_move_with_animation_from(
        &mut self,
        request: MoveRequest,
        now: Instant,
        from_xy: (f32, f32),
    ) -> Result<MoveId, BoardError> {
        if let Some(pending) = self.pending.as_ref() {
            return Err(BoardError::PendingMoveExists(pending.id));
        }

        let applied = self.apply_move_inner(request, now, Some(from_xy))?;
        let id = applied.id;
        self.history.push(applied);
        Ok(id)
    }

    pub fn apply_move_pending(
        &mut self,
        request: MoveRequest,
        now: Instant,
    ) -> Result<MoveId, BoardError> {
        if let Some(pending) = self.pending.as_ref() {
            return Err(BoardError::PendingMoveExists(pending.id));
        }

        let applied = self.apply_move_inner(request, now, None)?;
        let id = applied.id;
        self.pending = Some(applied);
        Ok(id)
    }

    pub fn apply_move_pending_with_animation_from(
        &mut self,
        request: MoveRequest,
        now: Instant,
        from_xy: (f32, f32),
    ) -> Result<MoveId, BoardError> {
        if let Some(pending) = self.pending.as_ref() {
            return Err(BoardError::PendingMoveExists(pending.id));
        }

        let applied = self.apply_move_inner(request, now, Some(from_xy))?;
        let id = applied.id;
        self.pending = Some(applied);
        Ok(id)
    }

    pub fn confirm_pending(&mut self, id: MoveId) -> Result<(), BoardError> {
        let pending = self.pending.take().ok_or(BoardError::NoPendingMove)?;

        if pending.id != id {
            let expected = pending.id;
            self.pending = Some(pending);
            return Err(BoardError::PendingMoveIdMismatch { expected, got: id });
        }

        self.history.push(pending);
        Ok(())
    }

    pub fn rollback_pending(&mut self, id: MoveId, now: Instant) -> Result<(), BoardError> {
        let pending = self.pending.take().ok_or(BoardError::NoPendingMove)?;

        if pending.id != id {
            let expected = pending.id;
            self.pending = Some(pending);
            return Err(BoardError::PendingMoveIdMismatch { expected, got: id });
        }

        self.revert_applied_move(&pending, now);
        Ok(())
    }

    pub fn undo_last(&mut self, now: Instant) -> Result<MoveId, BoardError> {
        if let Some(pending) = self.pending.as_ref() {
            return Err(BoardError::PendingMoveExists(pending.id));
        }

        let applied = self.history.pop().ok_or(BoardError::NoMoveToUndo)?;
        let id = applied.id;
        self.revert_applied_move(&applied, now);
        Ok(id)
    }

    pub fn render_pieces(&self, now: Instant, perspective: Perspective) -> Vec<RenderPiece> {
        let animation = self
            .active_animation
            .as_ref()
            .filter(|animation| !animation.is_finished(now));

        let mut pieces = Vec::with_capacity(32);

        for (index, piece) in self.squares.iter().enumerate() {
            let Some(piece) = piece else {
                continue;
            };
            let square = Square::from_index(index).expect("index is in board range");

            if animation.is_some_and(|state| state.to == square) {
                continue;
            }

            let (x, y) = square_to_xy(square, perspective);
            pieces.push(RenderPiece {
                piece: *piece,
                x,
                y,
            });
        }

        if let Some(animation) = animation {
            let progress = animation.progress(now);
            let (x0, y0) = animation
                .from_override_xy
                .map(|point| board_xy_to_screen_xy(point, perspective))
                .unwrap_or_else(|| square_to_xy(animation.from, perspective));
            let (x1, y1) = square_to_xy(animation.to, perspective);
            pieces.push(RenderPiece {
                piece: animation.piece,
                x: x0 + (x1 - x0) * progress,
                y: y0 + (y1 - y0) * progress,
            });
        }

        pieces
    }

    fn apply_move_inner(
        &mut self,
        request: MoveRequest,
        now: Instant,
        from_override_xy: Option<(f32, f32)>,
    ) -> Result<AppliedMove, BoardError> {
        if request.from == request.to {
            return Err(BoardError::SourceEqualsDestination(request.from));
        }

        let moving_piece =
            self.squares[request.from.index()].ok_or(BoardError::NoPieceAtSource(request.from))?;

        let moved_piece_after = if let Some(promotion) = request.promotion {
            Piece {
                side: moving_piece.side,
                kind: promotion,
            }
        } else {
            moving_piece
        };

        let captured_piece = self.squares[request.to.index()];
        self.squares[request.from.index()] = None;
        self.squares[request.to.index()] = Some(moved_piece_after);

        let id = MoveId(self.next_move_id);
        self.next_move_id += 1;

        self.active_animation = Some(AnimationState {
            piece: moved_piece_after,
            from: request.from,
            to: request.to,
            from_override_xy,
            started_at: now,
            duration: self.animation_duration,
            easing: self.animation_easing,
        });

        Ok(AppliedMove {
            id,
            request,
            moved_piece_before: moving_piece,
            moved_piece_after,
            captured_piece,
        })
    }

    fn revert_applied_move(&mut self, applied: &AppliedMove, now: Instant) {
        self.squares[applied.request.to.index()] = applied.captured_piece;
        self.squares[applied.request.from.index()] = Some(applied.moved_piece_before);

        self.active_animation = Some(AnimationState {
            piece: applied.moved_piece_before,
            from: applied.request.to,
            to: applied.request.from,
            from_override_xy: None,
            started_at: now,
            duration: self.animation_duration,
            easing: self.animation_easing,
        });
    }
}

fn square_to_xy(square: Square, perspective: Perspective) -> (f32, f32) {
    let file = square.file() as f32;
    let rank = square.rank() as f32;

    match perspective {
        Perspective::White => (file, 7.0 - rank),
        Perspective::Black => (7.0 - file, rank),
    }
}

fn board_xy_to_screen_xy((x, y): (f32, f32), perspective: Perspective) -> (f32, f32) {
    match perspective {
        Perspective::White => (x, y),
        Perspective::Black => (7.0 - x, 7.0 - y),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn standard_position_has_32_pieces() {
        let board = VisualBoard::standard();
        let count = board
            .squares()
            .iter()
            .filter(|piece| piece.is_some())
            .count();
        assert_eq!(count, 32);

        let e1 = Square::from_algebraic("e1").unwrap();
        let e8 = Square::from_algebraic("e8").unwrap();
        assert_eq!(board.piece_at(e1).unwrap().kind, PieceKind::King);
        assert_eq!(board.piece_at(e8).unwrap().kind, PieceKind::King);
    }

    #[test]
    fn pending_move_can_be_rolled_back() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();
        let e2 = Square::from_algebraic("e2").unwrap();
        let e4 = Square::from_algebraic("e4").unwrap();

        let id = board
            .apply_move_pending(MoveRequest::new(e2, e4), now)
            .unwrap();
        assert!(board.piece_at(e2).is_none());
        assert!(board.piece_at(e4).is_some());

        board
            .rollback_pending(id, now + Duration::from_millis(30))
            .unwrap();
        assert!(board.piece_at(e2).is_some());
        assert!(board.piece_at(e4).is_none());
    }

    #[test]
    fn confirmed_move_can_be_undone() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();
        let e2 = Square::from_algebraic("e2").unwrap();
        let e4 = Square::from_algebraic("e4").unwrap();

        let id = board
            .apply_move_pending(MoveRequest::new(e2, e4), now)
            .unwrap();
        board.confirm_pending(id).unwrap();

        board.undo_last(now + Duration::from_millis(25)).unwrap();
        assert!(board.piece_at(e2).is_some());
        assert!(board.piece_at(e4).is_none());
    }

    #[test]
    fn undo_fails_while_pending_move_exists() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();
        let e2 = Square::from_algebraic("e2").unwrap();
        let e4 = Square::from_algebraic("e4").unwrap();

        board
            .apply_move_pending(MoveRequest::new(e2, e4), now)
            .unwrap();
        let err = board
            .undo_last(now + Duration::from_millis(10))
            .unwrap_err();

        assert!(matches!(err, BoardError::PendingMoveExists(_)));
    }

    #[test]
    fn render_pieces_tracks_animation_without_duplicates() {
        let mut board = VisualBoard::standard();
        board.set_animation_profile(Duration::from_millis(200), Easing::Linear);

        let now = Instant::now();
        let e2 = Square::from_algebraic("e2").unwrap();
        let e4 = Square::from_algebraic("e4").unwrap();
        board.apply_move(MoveRequest::new(e2, e4), now).unwrap();

        let frame = board.render_pieces(now + Duration::from_millis(100), Perspective::White);
        let e4_count = frame
            .iter()
            .filter(|piece| piece.x == 4.0 && piece.y == 4.0)
            .count();
        assert_eq!(
            e4_count, 0,
            "destination square must not render duplicate piece"
        );

        let moving_piece = frame
            .iter()
            .find(|piece| piece.piece.kind == PieceKind::Pawn && (piece.y - 5.0).abs() < 0.001)
            .is_some();
        assert!(moving_piece, "moving pawn should be between e2 and e4");
    }

    #[test]
    fn animation_can_start_from_custom_board_position() {
        let mut board = VisualBoard::standard();
        board.set_animation_profile(Duration::from_millis(200), Easing::Linear);

        let now = Instant::now();
        let e2 = Square::from_algebraic("e2").unwrap();
        let e4 = Square::from_algebraic("e4").unwrap();
        board
            .apply_move_with_animation_from(MoveRequest::new(e2, e4), now, (4.0, 5.3))
            .unwrap();

        let frame = board.render_pieces(now, Perspective::White);
        let piece = frame
            .iter()
            .find(|piece| {
                piece.piece.side == Side::White
                    && piece.piece.kind == PieceKind::Pawn
                    && (piece.x - 4.0).abs() < 0.001
            })
            .unwrap();

        assert!((piece.y - 5.3).abs() < 0.001);
    }
}
