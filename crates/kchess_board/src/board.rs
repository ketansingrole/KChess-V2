use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

pub const BOARD_SIDE: usize = 8;
pub const BOARD_SQUARES: usize = BOARD_SIDE * BOARD_SIDE;

const PROMOTION_PIECES: [PieceKind; 4] = [
    PieceKind::Queen,
    PieceKind::Rook,
    PieceKind::Bishop,
    PieceKind::Knight,
];

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

impl Side {
    pub const fn opposite(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }
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

impl PieceKind {
    fn san_symbol(self) -> Option<char> {
        match self {
            Self::Pawn => None,
            Self::Knight => Some('N'),
            Self::Bishop => Some('B'),
            Self::Rook => Some('R'),
            Self::Queen => Some('Q'),
            Self::King => Some('K'),
        }
    }

    fn promotion_symbol(self) -> char {
        self.san_symbol().unwrap_or('Q')
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveHistoryCell {
    pub id: MoveId,
    pub notation: String,
    pub is_latest: bool,
    pub is_current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveHistoryRow {
    pub move_number: usize,
    pub white: Option<MoveHistoryCell>,
    pub black: Option<MoveHistoryCell>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CastleSide {
    KingSide,
    QueenSide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CastlingRights {
    white_king_side: bool,
    white_queen_side: bool,
    black_king_side: bool,
    black_queen_side: bool,
}

impl CastlingRights {
    const fn standard() -> Self {
        Self {
            white_king_side: true,
            white_queen_side: true,
            black_king_side: true,
            black_queen_side: true,
        }
    }

    const fn empty() -> Self {
        Self {
            white_king_side: false,
            white_queen_side: false,
            black_king_side: false,
            black_queen_side: false,
        }
    }

    const fn can_castle(self, side: Side, castle_side: CastleSide) -> bool {
        match (side, castle_side) {
            (Side::White, CastleSide::KingSide) => self.white_king_side,
            (Side::White, CastleSide::QueenSide) => self.white_queen_side,
            (Side::Black, CastleSide::KingSide) => self.black_king_side,
            (Side::Black, CastleSide::QueenSide) => self.black_queen_side,
        }
    }

    fn disable_king_side(&mut self, side: Side) {
        match side {
            Side::White => self.white_king_side = false,
            Side::Black => self.black_king_side = false,
        }
    }

    fn disable_queen_side(&mut self, side: Side) {
        match side {
            Side::White => self.white_queen_side = false,
            Side::Black => self.black_queen_side = false,
        }
    }

    fn disable_both(&mut self, side: Side) {
        self.disable_king_side(side);
        self.disable_queen_side(side);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RookMove {
    from: Square,
    to: Square,
    piece: Piece,
}

#[derive(Clone, Debug)]
pub struct AppliedMove {
    pub id: MoveId,
    pub request: MoveRequest,
    pub moved_piece_before: Piece,
    pub moved_piece_after: Piece,
    pub captured_piece: Option<Piece>,
    pub side: Side,
    pub san: String,
    captured_square: Option<Square>,
    rook_move: Option<RookMove>,
    previous_en_passant_target: Option<Square>,
    previous_castling_rights: CastlingRights,
    previous_side_to_move: Side,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardError {
    NoPieceAtSource(Square),
    SourceEqualsDestination(Square),
    WrongSideToMove { expected: Side, found: Side },
    IllegalMove { from: Square, to: Square },
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
            BoardError::WrongSideToMove { expected, found } => {
                write!(
                    f,
                    "wrong side to move: expected {expected:?}, found {found:?}"
                )
            }
            BoardError::IllegalMove { from, to } => {
                write!(
                    f,
                    "illegal move from {} to {}",
                    from.algebraic(),
                    to.algebraic()
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
struct ValidatedMove {
    request: MoveRequest,
    side: Side,
    moved_piece_before: Piece,
    moved_piece_after: Piece,
    captured_piece: Option<Piece>,
    captured_square: Option<Square>,
    rook_move: Option<RookMove>,
    castle_side: Option<CastleSide>,
    previous_en_passant_target: Option<Square>,
    new_en_passant_target: Option<Square>,
    previous_castling_rights: CastlingRights,
    new_castling_rights: CastlingRights,
    previous_side_to_move: Side,
}

#[derive(Clone, Debug)]
struct PreparedMove {
    validated: ValidatedMove,
    san: String,
}

#[derive(Clone, Debug)]
pub struct VisualBoard {
    squares: [Option<Piece>; BOARD_SQUARES],
    history: Vec<AppliedMove>,
    pending: Option<AppliedMove>,
    next_move_id: u64,
    active_animation: Option<AnimationState>,
    animation_duration: Duration,
    animation_easing: Easing,
    side_to_move: Side,
    en_passant_target: Option<Square>,
    castling_rights: CastlingRights,
    view_ply: usize,
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
            side_to_move: Side::White,
            en_passant_target: None,
            castling_rights: CastlingRights::empty(),
            view_ply: 0,
        }
    }

    pub fn standard() -> Self {
        let mut board = Self::empty();
        board.reset_to_start();
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
        self.side_to_move = Side::White;
        self.en_passant_target = None;
        self.castling_rights = CastlingRights::empty();
        self.view_ply = 0;
    }

    pub fn reset_to_start(&mut self) {
        self.clear();
        self.castling_rights = CastlingRights::standard();

        for file in 0..8 {
            let white_pawn = Square::from_file_rank(file, 1).expect("valid square");
            let black_pawn = Square::from_file_rank(file, 6).expect("valid square");
            self.squares[white_pawn.index()] = Some(Piece {
                side: Side::White,
                kind: PieceKind::Pawn,
            });
            self.squares[black_pawn.index()] = Some(Piece {
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

            self.squares[white_square.index()] = Some(Piece {
                side: Side::White,
                kind,
            });
            self.squares[black_square.index()] = Some(Piece {
                side: Side::Black,
                kind,
            });
        }
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

    pub fn history(&self) -> &[AppliedMove] {
        &self.history
    }

    pub fn side_to_move(&self) -> Side {
        self.side_to_move
    }

    pub fn displayed_ply(&self) -> usize {
        self.view_ply
    }

    pub fn is_viewing_latest(&self) -> bool {
        self.view_ply == self.history.len()
    }

    pub fn displayed_side_to_move(&self) -> Side {
        if self.is_viewing_latest() {
            return self.side_to_move;
        }

        if self.view_ply == 0 {
            return Side::White;
        }

        self.history
            .get(self.view_ply)
            .map(|entry| entry.previous_side_to_move)
            .unwrap_or(self.side_to_move)
    }

    pub fn step_back_view(&mut self) -> bool {
        if self.pending.is_some() || self.view_ply == 0 {
            return false;
        }
        self.view_ply -= 1;
        true
    }

    pub fn step_forward_view(&mut self) -> bool {
        if self.pending.is_some() || self.view_ply >= self.history.len() {
            return false;
        }
        self.view_ply += 1;
        true
    }

    pub fn jump_to_move(&mut self, id: MoveId) -> bool {
        if self.pending.is_some() {
            return false;
        }

        let Some(index) = self.history.iter().position(|entry| entry.id == id) else {
            return false;
        };

        let new_view_ply = index + 1;
        if self.view_ply == new_view_ply {
            return false;
        }

        self.view_ply = new_view_ply;
        true
    }

    pub fn jump_to_start(&mut self) {
        self.view_ply = 0;
    }

    pub fn jump_to_latest(&mut self) {
        self.view_ply = self.history.len();
    }

    pub fn move_history(&self) -> Vec<MoveHistoryRow> {
        let latest_id = self.history.last().map(|entry| entry.id);
        let current_id = self
            .view_ply
            .checked_sub(1)
            .and_then(|index| self.history.get(index))
            .map(|entry| entry.id);
        let mut rows = Vec::with_capacity((self.history.len() + 1) / 2);

        for (index, applied) in self.history.iter().enumerate() {
            let row_index = index / 2;
            if rows.len() <= row_index {
                rows.push(MoveHistoryRow {
                    move_number: row_index + 1,
                    white: None,
                    black: None,
                });
            }

            let cell = MoveHistoryCell {
                id: applied.id,
                notation: applied.san.clone(),
                is_latest: latest_id == Some(applied.id),
                is_current: current_id == Some(applied.id),
            };

            if applied.side == Side::White {
                rows[row_index].white = Some(cell);
            } else {
                rows[row_index].black = Some(cell);
            }
        }

        rows
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

        let prepared = self.prepare_move(request)?;
        let applied = self.apply_prepared_move(prepared, now, None);
        let id = applied.id;
        self.history.push(applied);
        self.view_ply = self.history.len();
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

        let prepared = self.prepare_move(request)?;
        let applied = self.apply_prepared_move(prepared, now, Some(from_xy));
        let id = applied.id;
        self.history.push(applied);
        self.view_ply = self.history.len();
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

        let prepared = self.prepare_move(request)?;
        let applied = self.apply_prepared_move(prepared, now, None);
        let id = applied.id;
        self.pending = Some(applied);
        self.view_ply = self.history.len();
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

        let prepared = self.prepare_move(request)?;
        let applied = self.apply_prepared_move(prepared, now, Some(from_xy));
        let id = applied.id;
        self.pending = Some(applied);
        self.view_ply = self.history.len();
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
        self.view_ply = self.history.len();
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
        self.view_ply = self.history.len();
        Ok(())
    }

    pub fn undo_last(&mut self, now: Instant) -> Result<MoveId, BoardError> {
        if let Some(pending) = self.pending.as_ref() {
            return Err(BoardError::PendingMoveExists(pending.id));
        }

        let applied = self.history.pop().ok_or(BoardError::NoMoveToUndo)?;
        let id = applied.id;
        self.revert_applied_move(&applied, now);
        self.view_ply = self.history.len();
        Ok(id)
    }

    pub fn render_pieces(&self, now: Instant, perspective: Perspective) -> Vec<RenderPiece> {
        if !self.is_viewing_latest() {
            let squares = self.display_squares_for_view();
            let mut pieces = Vec::with_capacity(32);
            for (index, piece) in squares.iter().enumerate() {
                let Some(piece) = piece else {
                    continue;
                };
                let square = Square::from_index(index).expect("index is in board range");
                let (x, y) = square_to_xy(square, perspective);
                pieces.push(RenderPiece {
                    piece: *piece,
                    x,
                    y,
                });
            }
            return pieces;
        }

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

    fn display_squares_for_view(&self) -> [Option<Piece>; BOARD_SQUARES] {
        if self.is_viewing_latest() {
            return self.squares;
        }

        let mut squares = self.squares;
        for applied in self.history[self.view_ply..].iter().rev() {
            if let Some(rook_move) = applied.rook_move {
                squares[rook_move.to.index()] = None;
                squares[rook_move.from.index()] = Some(rook_move.piece);
            }

            squares[applied.request.to.index()] = None;
            if let Some(captured_square) = applied.captured_square {
                squares[captured_square.index()] = applied.captured_piece;
            }
            squares[applied.request.from.index()] = Some(applied.moved_piece_before);
        }

        squares
    }

    fn prepare_move(&self, request: MoveRequest) -> Result<PreparedMove, BoardError> {
        let validated = self.validate_request_for_side(request, self.side_to_move)?;
        let san = self.format_san(&validated);
        Ok(PreparedMove { validated, san })
    }

    fn apply_prepared_move(
        &mut self,
        prepared: PreparedMove,
        now: Instant,
        from_override_xy: Option<(f32, f32)>,
    ) -> AppliedMove {
        let id = MoveId(self.next_move_id);
        self.next_move_id += 1;

        let piece = prepared.validated.moved_piece_after;
        let from = prepared.validated.request.from;
        let to = prepared.validated.request.to;

        self.apply_validated_state(&prepared.validated);
        self.active_animation = Some(AnimationState {
            piece,
            from,
            to,
            from_override_xy,
            started_at: now,
            duration: self.animation_duration,
            easing: self.animation_easing,
        });

        AppliedMove {
            id,
            request: prepared.validated.request,
            moved_piece_before: prepared.validated.moved_piece_before,
            moved_piece_after: prepared.validated.moved_piece_after,
            captured_piece: prepared.validated.captured_piece,
            side: prepared.validated.side,
            san: prepared.san,
            captured_square: prepared.validated.captured_square,
            rook_move: prepared.validated.rook_move,
            previous_en_passant_target: prepared.validated.previous_en_passant_target,
            previous_castling_rights: prepared.validated.previous_castling_rights,
            previous_side_to_move: prepared.validated.previous_side_to_move,
        }
    }

    fn validate_request_for_side(
        &self,
        request: MoveRequest,
        side: Side,
    ) -> Result<ValidatedMove, BoardError> {
        if request.from == request.to {
            return Err(BoardError::SourceEqualsDestination(request.from));
        }

        let moving_piece = self
            .piece_at(request.from)
            .ok_or(BoardError::NoPieceAtSource(request.from))?;
        if moving_piece.side != side {
            return Err(BoardError::WrongSideToMove {
                expected: side,
                found: moving_piece.side,
            });
        }

        let request = self.normalize_request(request, moving_piece)?;
        let target_piece = self.piece_at(request.to);
        if target_piece.is_some_and(|piece| piece.side == side) {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }
        if target_piece.is_some_and(|piece| piece.kind == PieceKind::King) {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }

        let validated = match moving_piece.kind {
            PieceKind::Pawn => self.validate_pawn_move(request, moving_piece),
            PieceKind::Knight => self.validate_knight_move(request, moving_piece),
            PieceKind::Bishop => self.validate_sliding_move(
                request,
                moving_piece,
                &[(1, 1), (1, -1), (-1, 1), (-1, -1)],
            ),
            PieceKind::Rook => self.validate_sliding_move(
                request,
                moving_piece,
                &[(1, 0), (-1, 0), (0, 1), (0, -1)],
            ),
            PieceKind::Queen => self.validate_sliding_move(
                request,
                moving_piece,
                &[
                    (1, 1),
                    (1, -1),
                    (-1, 1),
                    (-1, -1),
                    (1, 0),
                    (-1, 0),
                    (0, 1),
                    (0, -1),
                ],
            ),
            PieceKind::King => self.validate_king_move(request, moving_piece),
        }?;

        let mut simulated = self.analysis_clone();
        simulated.apply_validated_state(&validated);
        if simulated.is_in_check(side) {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }

        Ok(validated)
    }

    fn normalize_request(
        &self,
        mut request: MoveRequest,
        moving_piece: Piece,
    ) -> Result<MoveRequest, BoardError> {
        if moving_piece.kind != PieceKind::Pawn {
            if request.promotion.is_some() {
                return Err(BoardError::IllegalMove {
                    from: request.from,
                    to: request.to,
                });
            }
            return Ok(request);
        }

        let promotion_rank = match moving_piece.side {
            Side::White => 7,
            Side::Black => 0,
        };
        let reaches_promotion_rank = request.to.rank() == promotion_rank;

        match (reaches_promotion_rank, request.promotion) {
            (true, Some(PieceKind::King | PieceKind::Pawn)) => Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            }),
            (true, None) => {
                request.promotion = Some(PieceKind::Queen);
                Ok(request)
            }
            (true, Some(_)) => Ok(request),
            (false, Some(_)) => Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            }),
            (false, None) => Ok(request),
        }
    }

    fn validate_pawn_move(
        &self,
        request: MoveRequest,
        moving_piece: Piece,
    ) -> Result<ValidatedMove, BoardError> {
        let direction = match moving_piece.side {
            Side::White => 1,
            Side::Black => -1,
        };
        let start_rank = match moving_piece.side {
            Side::White => 1,
            Side::Black => 6,
        };
        let file_delta = request.to.file() as i8 - request.from.file() as i8;
        let rank_delta = request.to.rank() as i8 - request.from.rank() as i8;
        let mut captured_piece = None;
        let mut captured_square = None;
        let mut new_en_passant_target = None;

        if file_delta == 0 {
            if rank_delta == direction {
                if self.piece_at(request.to).is_some() {
                    return Err(BoardError::IllegalMove {
                        from: request.from,
                        to: request.to,
                    });
                }
            } else if rank_delta == direction * 2 && request.from.rank() == start_rank {
                let intermediate = square_offset(request.from, 0, direction)
                    .expect("double pawn push has middle square");
                if self.piece_at(intermediate).is_some() || self.piece_at(request.to).is_some() {
                    return Err(BoardError::IllegalMove {
                        from: request.from,
                        to: request.to,
                    });
                }
                new_en_passant_target = Some(intermediate);
            } else {
                return Err(BoardError::IllegalMove {
                    from: request.from,
                    to: request.to,
                });
            }
        } else if file_delta.abs() == 1 && rank_delta == direction {
            if let Some(target) = self.piece_at(request.to) {
                if target.side == moving_piece.side || target.kind == PieceKind::King {
                    return Err(BoardError::IllegalMove {
                        from: request.from,
                        to: request.to,
                    });
                }
                captured_piece = Some(target);
                captured_square = Some(request.to);
            } else if self.en_passant_target == Some(request.to) {
                let en_passant_square = square_offset(request.to, 0, -direction)
                    .expect("en passant capture square exists");
                let target = self.piece_at(en_passant_square).filter(|piece| {
                    piece.side == moving_piece.side.opposite() && piece.kind == PieceKind::Pawn
                });
                if let Some(target) = target {
                    captured_piece = Some(target);
                    captured_square = Some(en_passant_square);
                } else {
                    return Err(BoardError::IllegalMove {
                        from: request.from,
                        to: request.to,
                    });
                }
            } else {
                return Err(BoardError::IllegalMove {
                    from: request.from,
                    to: request.to,
                });
            }
        } else {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }

        let moved_piece_after = Piece {
            side: moving_piece.side,
            kind: request.promotion.unwrap_or(PieceKind::Pawn),
        };

        Ok(self.finish_validated_move(
            request,
            moving_piece,
            moved_piece_after,
            captured_piece,
            captured_square,
            None,
            None,
            new_en_passant_target,
        ))
    }

    fn validate_knight_move(
        &self,
        request: MoveRequest,
        moving_piece: Piece,
    ) -> Result<ValidatedMove, BoardError> {
        let file_delta = (request.to.file() as i8 - request.from.file() as i8).abs();
        let rank_delta = (request.to.rank() as i8 - request.from.rank() as i8).abs();
        if !matches!((file_delta, rank_delta), (1, 2) | (2, 1)) {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }

        Ok(self.finish_validated_move(
            request,
            moving_piece,
            moving_piece,
            self.piece_at(request.to),
            self.piece_at(request.to).map(|_| request.to),
            None,
            None,
            None,
        ))
    }

    fn validate_sliding_move(
        &self,
        request: MoveRequest,
        moving_piece: Piece,
        directions: &[(i8, i8)],
    ) -> Result<ValidatedMove, BoardError> {
        let file_delta = request.to.file() as i8 - request.from.file() as i8;
        let rank_delta = request.to.rank() as i8 - request.from.rank() as i8;
        let step = direction_step(file_delta, rank_delta).ok_or(BoardError::IllegalMove {
            from: request.from,
            to: request.to,
        })?;

        if !directions.contains(&step) {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }

        let mut current = square_offset(request.from, step.0, step.1);
        while let Some(square) = current {
            if square == request.to {
                break;
            }
            if self.piece_at(square).is_some() {
                return Err(BoardError::IllegalMove {
                    from: request.from,
                    to: request.to,
                });
            }
            current = square_offset(square, step.0, step.1);
        }

        if current != Some(request.to) {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }

        Ok(self.finish_validated_move(
            request,
            moving_piece,
            moving_piece,
            self.piece_at(request.to),
            self.piece_at(request.to).map(|_| request.to),
            None,
            None,
            None,
        ))
    }

    fn validate_king_move(
        &self,
        request: MoveRequest,
        moving_piece: Piece,
    ) -> Result<ValidatedMove, BoardError> {
        let file_delta = request.to.file() as i8 - request.from.file() as i8;
        let rank_delta = request.to.rank() as i8 - request.from.rank() as i8;

        if file_delta.abs() <= 1 && rank_delta.abs() <= 1 {
            return Ok(self.finish_validated_move(
                request,
                moving_piece,
                moving_piece,
                self.piece_at(request.to),
                self.piece_at(request.to).map(|_| request.to),
                None,
                None,
                None,
            ));
        }

        if rank_delta != 0 || file_delta.abs() != 2 {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }

        let castle_side = match request.to.file() {
            6 => CastleSide::KingSide,
            2 => CastleSide::QueenSide,
            _ => {
                return Err(BoardError::IllegalMove {
                    from: request.from,
                    to: request.to,
                })
            }
        };

        let expected_from = king_home_square(moving_piece.side);
        if request.from != expected_from
            || !self
                .castling_rights
                .can_castle(moving_piece.side, castle_side)
        {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }

        let rook_from = rook_home_square(moving_piece.side, castle_side);
        let rook_to = rook_castle_target(moving_piece.side, castle_side);
        let rook = self
            .piece_at(rook_from)
            .filter(|piece| piece.side == moving_piece.side && piece.kind == PieceKind::Rook)
            .ok_or(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            })?;

        let between_squares = match castle_side {
            CastleSide::KingSide => [
                square_offset(request.from, 1, 0).unwrap(),
                square_offset(request.from, 2, 0).unwrap(),
            ]
            .to_vec(),
            CastleSide::QueenSide => [
                square_offset(request.from, -1, 0).unwrap(),
                square_offset(request.from, -2, 0).unwrap(),
                square_offset(request.from, -3, 0).unwrap(),
            ]
            .to_vec(),
        };

        for square in &between_squares {
            if self.piece_at(*square).is_some() {
                return Err(BoardError::IllegalMove {
                    from: request.from,
                    to: request.to,
                });
            }
        }

        let transit_squares = match castle_side {
            CastleSide::KingSide => [
                request.from,
                square_offset(request.from, 1, 0).unwrap(),
                request.to,
            ],
            CastleSide::QueenSide => [
                request.from,
                square_offset(request.from, -1, 0).unwrap(),
                request.to,
            ],
        };

        if transit_squares
            .into_iter()
            .any(|square| self.is_square_attacked(square, moving_piece.side.opposite()))
        {
            return Err(BoardError::IllegalMove {
                from: request.from,
                to: request.to,
            });
        }

        Ok(self.finish_validated_move(
            request,
            moving_piece,
            moving_piece,
            None,
            None,
            Some(RookMove {
                from: rook_from,
                to: rook_to,
                piece: rook,
            }),
            Some(castle_side),
            None,
        ))
    }

    fn finish_validated_move(
        &self,
        request: MoveRequest,
        moved_piece_before: Piece,
        moved_piece_after: Piece,
        captured_piece: Option<Piece>,
        captured_square: Option<Square>,
        rook_move: Option<RookMove>,
        castle_side: Option<CastleSide>,
        new_en_passant_target: Option<Square>,
    ) -> ValidatedMove {
        let previous_castling_rights = self.castling_rights;
        let mut new_castling_rights = previous_castling_rights;
        update_castling_rights_for_move(
            &mut new_castling_rights,
            moved_piece_before,
            request.from,
            captured_piece,
            captured_square,
        );

        ValidatedMove {
            request,
            side: moved_piece_before.side,
            moved_piece_before,
            moved_piece_after,
            captured_piece,
            captured_square,
            rook_move,
            castle_side,
            previous_en_passant_target: self.en_passant_target,
            new_en_passant_target,
            previous_castling_rights,
            new_castling_rights,
            previous_side_to_move: self.side_to_move,
        }
    }

    fn apply_validated_state(&mut self, validated: &ValidatedMove) {
        self.squares[validated.request.from.index()] = None;
        if let Some(captured_square) = validated.captured_square {
            self.squares[captured_square.index()] = None;
        }
        if let Some(rook_move) = validated.rook_move {
            self.squares[rook_move.from.index()] = None;
            self.squares[rook_move.to.index()] = Some(rook_move.piece);
        }
        self.squares[validated.request.to.index()] = Some(validated.moved_piece_after);
        self.en_passant_target = validated.new_en_passant_target;
        self.castling_rights = validated.new_castling_rights;
        self.side_to_move = validated.side.opposite();
    }

    fn revert_applied_move(&mut self, applied: &AppliedMove, now: Instant) {
        if let Some(rook_move) = applied.rook_move {
            self.squares[rook_move.to.index()] = None;
            self.squares[rook_move.from.index()] = Some(rook_move.piece);
        }

        self.squares[applied.request.to.index()] = None;
        if let Some(captured_square) = applied.captured_square {
            self.squares[captured_square.index()] = applied.captured_piece;
        }
        self.squares[applied.request.from.index()] = Some(applied.moved_piece_before);
        self.en_passant_target = applied.previous_en_passant_target;
        self.castling_rights = applied.previous_castling_rights;
        self.side_to_move = applied.previous_side_to_move;
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

    fn format_san(&self, validated: &ValidatedMove) -> String {
        let mut san = String::new();

        if let Some(castle_side) = validated.castle_side {
            san.push_str(match castle_side {
                CastleSide::KingSide => "O-O",
                CastleSide::QueenSide => "O-O-O",
            });
        } else {
            match validated.moved_piece_before.kind {
                PieceKind::Pawn => {
                    if validated.captured_piece.is_some() {
                        san.push(file_char(validated.request.from.file()));
                    }
                }
                piece_kind => {
                    if let Some(symbol) = piece_kind.san_symbol() {
                        san.push(symbol);
                    }
                    match self.disambiguation(validated) {
                        Disambiguation::File => san.push(file_char(validated.request.from.file())),
                        Disambiguation::Rank => san.push(rank_char(validated.request.from.rank())),
                        Disambiguation::Both => {
                            san.push(file_char(validated.request.from.file()));
                            san.push(rank_char(validated.request.from.rank()));
                        }
                        Disambiguation::None => {}
                    }
                }
            }

            if validated.captured_piece.is_some() {
                san.push('x');
            }
            san.push_str(&validated.request.to.algebraic());
            if let Some(promotion) = validated.request.promotion {
                san.push('=');
                san.push(promotion.promotion_symbol());
            }
        }

        let mut simulated = self.analysis_clone();
        simulated.apply_validated_state(validated);
        let opponent = validated.side.opposite();
        if simulated.is_in_check(opponent) {
            san.push(if simulated.has_any_legal_move(opponent) {
                '+'
            } else {
                '#'
            });
        }

        san
    }

    fn disambiguation(&self, validated: &ValidatedMove) -> Disambiguation {
        if validated.moved_piece_before.kind == PieceKind::Pawn {
            return Disambiguation::None;
        }

        let mut others = Vec::new();
        for index in 0..BOARD_SQUARES {
            let square = Square::from_index(index).expect("index in range");
            if square == validated.request.from {
                continue;
            }

            let Some(piece) = self.piece_at(square) else {
                continue;
            };
            if piece.side != validated.side || piece.kind != validated.moved_piece_before.kind {
                continue;
            }

            let candidate = MoveRequest {
                from: square,
                to: validated.request.to,
                promotion: validated.request.promotion,
            };
            if self
                .validate_request_for_side(candidate, validated.side)
                .is_ok()
            {
                others.push(square);
            }
        }

        if others.is_empty() {
            return Disambiguation::None;
        }

        let same_file = others
            .iter()
            .any(|square| square.file() == validated.request.from.file());
        let same_rank = others
            .iter()
            .any(|square| square.rank() == validated.request.from.rank());

        if !same_file {
            Disambiguation::File
        } else if !same_rank {
            Disambiguation::Rank
        } else {
            Disambiguation::Both
        }
    }

    fn has_any_legal_move(&self, side: Side) -> bool {
        for index in 0..BOARD_SQUARES {
            let from = Square::from_index(index).expect("index in range");
            let Some(piece) = self.piece_at(from) else {
                continue;
            };
            if piece.side != side {
                continue;
            }

            for target_index in 0..BOARD_SQUARES {
                let to = Square::from_index(target_index).expect("index in range");
                if to == from {
                    continue;
                }

                if piece.kind == PieceKind::Pawn && reaches_promotion_rank(piece.side, to.rank()) {
                    for promotion in PROMOTION_PIECES {
                        let request = MoveRequest::with_promotion(from, to, promotion);
                        if self.validate_request_for_side(request, side).is_ok() {
                            return true;
                        }
                    }
                } else {
                    let request = MoveRequest::new(from, to);
                    if self.validate_request_for_side(request, side).is_ok() {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn is_in_check(&self, side: Side) -> bool {
        let Some(king_square) = self.king_square(side) else {
            return false;
        };
        self.is_square_attacked(king_square, side.opposite())
    }

    fn king_square(&self, side: Side) -> Option<Square> {
        self.squares.iter().enumerate().find_map(|(index, piece)| {
            let piece = (*piece)?;
            (piece.side == side && piece.kind == PieceKind::King)
                .then(|| Square::from_index(index).expect("index in range"))
        })
    }

    fn is_square_attacked(&self, square: Square, by_side: Side) -> bool {
        for index in 0..BOARD_SQUARES {
            let from = Square::from_index(index).expect("index in range");
            let Some(piece) = self.piece_at(from) else {
                continue;
            };
            if piece.side != by_side {
                continue;
            }
            if self.piece_attacks_square(from, piece, square) {
                return true;
            }
        }
        false
    }

    fn piece_attacks_square(&self, from: Square, piece: Piece, target: Square) -> bool {
        let file_delta = target.file() as i8 - from.file() as i8;
        let rank_delta = target.rank() as i8 - from.rank() as i8;
        match piece.kind {
            PieceKind::Pawn => {
                let direction = match piece.side {
                    Side::White => 1,
                    Side::Black => -1,
                };
                rank_delta == direction && file_delta.abs() == 1
            }
            PieceKind::Knight => matches!((file_delta.abs(), rank_delta.abs()), (1, 2) | (2, 1)),
            PieceKind::Bishop => {
                self.sliding_attacks(from, target, &[(1, 1), (1, -1), (-1, 1), (-1, -1)])
            }
            PieceKind::Rook => {
                self.sliding_attacks(from, target, &[(1, 0), (-1, 0), (0, 1), (0, -1)])
            }
            PieceKind::Queen => self.sliding_attacks(
                from,
                target,
                &[
                    (1, 1),
                    (1, -1),
                    (-1, 1),
                    (-1, -1),
                    (1, 0),
                    (-1, 0),
                    (0, 1),
                    (0, -1),
                ],
            ),
            PieceKind::King => file_delta.abs() <= 1 && rank_delta.abs() <= 1,
        }
    }

    fn sliding_attacks(&self, from: Square, target: Square, directions: &[(i8, i8)]) -> bool {
        let file_delta = target.file() as i8 - from.file() as i8;
        let rank_delta = target.rank() as i8 - from.rank() as i8;
        let Some(step) = direction_step(file_delta, rank_delta) else {
            return false;
        };
        if !directions.contains(&step) {
            return false;
        }

        let mut current = square_offset(from, step.0, step.1);
        while let Some(square) = current {
            if square == target {
                return true;
            }
            if self.piece_at(square).is_some() {
                return false;
            }
            current = square_offset(square, step.0, step.1);
        }
        false
    }

    fn analysis_clone(&self) -> Self {
        Self {
            squares: self.squares,
            history: Vec::new(),
            pending: None,
            next_move_id: self.next_move_id,
            active_animation: None,
            animation_duration: self.animation_duration,
            animation_easing: self.animation_easing,
            side_to_move: self.side_to_move,
            en_passant_target: self.en_passant_target,
            castling_rights: self.castling_rights,
            view_ply: self.view_ply,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disambiguation {
    None,
    File,
    Rank,
    Both,
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

fn square_offset(square: Square, file_delta: i8, rank_delta: i8) -> Option<Square> {
    let file = square.file() as i8 + file_delta;
    let rank = square.rank() as i8 + rank_delta;
    if !(0..=7).contains(&file) || !(0..=7).contains(&rank) {
        return None;
    }
    Square::from_file_rank(file as u8, rank as u8)
}

fn direction_step(file_delta: i8, rank_delta: i8) -> Option<(i8, i8)> {
    let file_sign = file_delta.signum();
    let rank_sign = rank_delta.signum();
    let abs_file = file_delta.abs();
    let abs_rank = rank_delta.abs();

    if file_delta == 0 && rank_delta == 0 {
        None
    } else if file_delta == 0 {
        Some((0, rank_sign))
    } else if rank_delta == 0 {
        Some((file_sign, 0))
    } else if abs_file == abs_rank {
        Some((file_sign, rank_sign))
    } else {
        None
    }
}

fn update_castling_rights_for_move(
    rights: &mut CastlingRights,
    moved_piece: Piece,
    from: Square,
    captured_piece: Option<Piece>,
    captured_square: Option<Square>,
) {
    match moved_piece.kind {
        PieceKind::King => rights.disable_both(moved_piece.side),
        PieceKind::Rook => {
            if from == rook_home_square(moved_piece.side, CastleSide::KingSide) {
                rights.disable_king_side(moved_piece.side);
            }
            if from == rook_home_square(moved_piece.side, CastleSide::QueenSide) {
                rights.disable_queen_side(moved_piece.side);
            }
        }
        _ => {}
    }

    if let (Some(piece), Some(square)) = (captured_piece, captured_square) {
        if piece.kind == PieceKind::Rook {
            if square == rook_home_square(piece.side, CastleSide::KingSide) {
                rights.disable_king_side(piece.side);
            }
            if square == rook_home_square(piece.side, CastleSide::QueenSide) {
                rights.disable_queen_side(piece.side);
            }
        }
    }
}

fn king_home_square(side: Side) -> Square {
    match side {
        Side::White => Square::from_algebraic("e1").unwrap(),
        Side::Black => Square::from_algebraic("e8").unwrap(),
    }
}

fn rook_home_square(side: Side, castle_side: CastleSide) -> Square {
    match (side, castle_side) {
        (Side::White, CastleSide::KingSide) => Square::from_algebraic("h1").unwrap(),
        (Side::White, CastleSide::QueenSide) => Square::from_algebraic("a1").unwrap(),
        (Side::Black, CastleSide::KingSide) => Square::from_algebraic("h8").unwrap(),
        (Side::Black, CastleSide::QueenSide) => Square::from_algebraic("a8").unwrap(),
    }
}

fn rook_castle_target(side: Side, castle_side: CastleSide) -> Square {
    match (side, castle_side) {
        (Side::White, CastleSide::KingSide) => Square::from_algebraic("f1").unwrap(),
        (Side::White, CastleSide::QueenSide) => Square::from_algebraic("d1").unwrap(),
        (Side::Black, CastleSide::KingSide) => Square::from_algebraic("f8").unwrap(),
        (Side::Black, CastleSide::QueenSide) => Square::from_algebraic("d8").unwrap(),
    }
}

fn reaches_promotion_rank(side: Side, rank: u8) -> bool {
    matches!((side, rank), (Side::White, 7) | (Side::Black, 0))
}

fn file_char(file: u8) -> char {
    (b'a' + file) as char
}

fn rank_char(rank: u8) -> char {
    (b'1' + rank) as char
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use pretty_assertions::assert_eq;

    fn square(name: &str) -> Square {
        Square::from_algebraic(name).unwrap()
    }

    #[test]
    fn standard_position_has_32_pieces() {
        let board = VisualBoard::standard();
        let count = board
            .squares()
            .iter()
            .filter(|piece| piece.is_some())
            .count();
        assert_eq!(count, 32);

        assert_eq!(board.piece_at(square("e1")).unwrap().kind, PieceKind::King);
        assert_eq!(board.piece_at(square("e8")).unwrap().kind, PieceKind::King);
    }

    #[test]
    fn pending_move_can_be_rolled_back() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();

        let id = board
            .apply_move_pending(MoveRequest::new(square("e2"), square("e4")), now)
            .unwrap();
        assert!(board.piece_at(square("e2")).is_none());
        assert!(board.piece_at(square("e4")).is_some());

        board
            .rollback_pending(id, now + Duration::from_millis(30))
            .unwrap();
        assert!(board.piece_at(square("e2")).is_some());
        assert!(board.piece_at(square("e4")).is_none());
        assert_eq!(board.side_to_move(), Side::White);
    }

    #[test]
    fn confirmed_move_can_be_undone() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();

        let id = board
            .apply_move_pending(MoveRequest::new(square("e2"), square("e4")), now)
            .unwrap();
        board.confirm_pending(id).unwrap();

        board.undo_last(now + Duration::from_millis(25)).unwrap();
        assert!(board.piece_at(square("e2")).is_some());
        assert!(board.piece_at(square("e4")).is_none());
        assert_eq!(board.side_to_move(), Side::White);
    }

    #[test]
    fn undo_fails_while_pending_move_exists() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();

        board
            .apply_move_pending(MoveRequest::new(square("e2"), square("e4")), now)
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
        board
            .apply_move(MoveRequest::new(square("e2"), square("e4")), now)
            .unwrap();

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
        board
            .apply_move_with_animation_from(
                MoveRequest::new(square("e2"), square("e4")),
                now,
                (4.0, 5.3),
            )
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

    #[test]
    fn legal_opening_sequence_builds_move_rows() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();
        let moves = [("e2", "e4"), ("e7", "e5"), ("g1", "f3"), ("b8", "c6")];

        for (ix, (from, to)) in moves.into_iter().enumerate() {
            board
                .apply_move(
                    MoveRequest::new(square(from), square(to)),
                    now + Duration::from_millis((ix * 5) as u64),
                )
                .unwrap();
        }

        let history = board.move_history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].white.as_ref().unwrap().notation, "e4");
        assert_eq!(history[0].black.as_ref().unwrap().notation, "e5");
        assert_eq!(history[1].white.as_ref().unwrap().notation, "Nf3");
        assert_eq!(history[1].black.as_ref().unwrap().notation, "Nc6");
        assert!(history[1].black.as_ref().unwrap().is_latest);
        assert!(history[1].black.as_ref().unwrap().is_current);
    }

    #[test]
    fn jump_to_middle_move_reconstructs_position_and_side() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();

        let _m1 = board
            .apply_move(MoveRequest::new(square("e2"), square("e4")), now)
            .unwrap();
        let m2 = board
            .apply_move(
                MoveRequest::new(square("e7"), square("e5")),
                now + Duration::from_millis(5),
            )
            .unwrap();
        board
            .apply_move(
                MoveRequest::new(square("g1"), square("f3")),
                now + Duration::from_millis(10),
            )
            .unwrap();
        board
            .apply_move(
                MoveRequest::new(square("b8"), square("c6")),
                now + Duration::from_millis(15),
            )
            .unwrap();

        assert!(board.jump_to_move(m2));
        assert_eq!(board.displayed_ply(), 2);
        assert!(!board.is_viewing_latest());
        assert_eq!(board.displayed_side_to_move(), Side::White);

        let rendered = board.render_pieces(now + Duration::from_millis(16), Perspective::White);
        let has_knight_on_g1 = rendered.iter().any(|piece| {
            piece.piece.side == Side::White
                && piece.piece.kind == PieceKind::Knight
                && (piece.x - 6.0).abs() < 0.001
                && (piece.y - 7.0).abs() < 0.001
        });
        let has_knight_on_f3 = rendered.iter().any(|piece| {
            piece.piece.side == Side::White
                && piece.piece.kind == PieceKind::Knight
                && (piece.x - 5.0).abs() < 0.001
                && (piece.y - 5.0).abs() < 0.001
        });
        assert!(has_knight_on_g1);
        assert!(!has_knight_on_f3);
    }

    #[test]
    fn step_view_respects_bounds_with_start_position() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();
        board
            .apply_move(MoveRequest::new(square("e2"), square("e4")), now)
            .unwrap();
        board
            .apply_move(
                MoveRequest::new(square("e7"), square("e5")),
                now + Duration::from_millis(5),
            )
            .unwrap();

        board.jump_to_start();
        assert_eq!(board.displayed_ply(), 0);
        assert!(!board.step_back_view());
        assert!(board.step_forward_view());
        assert_eq!(board.displayed_ply(), 1);
        assert!(board.step_forward_view());
        assert_eq!(board.displayed_ply(), 2);
        assert!(!board.step_forward_view());
    }

    #[test]
    fn move_history_marks_current_and_latest_separately() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();
        board
            .apply_move(MoveRequest::new(square("e2"), square("e4")), now)
            .unwrap();
        let m2 = board
            .apply_move(
                MoveRequest::new(square("e7"), square("e5")),
                now + Duration::from_millis(5),
            )
            .unwrap();
        board
            .apply_move(
                MoveRequest::new(square("g1"), square("f3")),
                now + Duration::from_millis(10),
            )
            .unwrap();

        assert!(board.jump_to_move(m2));
        let history = board.move_history();

        assert!(history[0].black.as_ref().unwrap().is_current);
        assert!(!history[0].black.as_ref().unwrap().is_latest);
        assert!(history[1].white.as_ref().unwrap().is_latest);
        assert!(!history[1].white.as_ref().unwrap().is_current);
    }

    #[test]
    fn jumping_view_does_not_mutate_history_entries() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();

        let ids = [
            board
                .apply_move(MoveRequest::new(square("e2"), square("e4")), now)
                .unwrap(),
            board
                .apply_move(
                    MoveRequest::new(square("e7"), square("e5")),
                    now + Duration::from_millis(5),
                )
                .unwrap(),
            board
                .apply_move(
                    MoveRequest::new(square("g1"), square("f3")),
                    now + Duration::from_millis(10),
                )
                .unwrap(),
            board
                .apply_move(
                    MoveRequest::new(square("b8"), square("c6")),
                    now + Duration::from_millis(15),
                )
                .unwrap(),
        ];

        let expected_len = board.history_len();
        let expected_ids: Vec<u64> = board.history().iter().map(|entry| entry.id.get()).collect();
        let expected_san: Vec<String> = board
            .history()
            .iter()
            .map(|entry| entry.san.clone())
            .collect();

        assert!(board.jump_to_move(ids[1]));
        assert!(board.step_back_view());
        assert!(board.step_forward_view());
        board.jump_to_latest();

        assert_eq!(board.history_len(), expected_len);
        assert_eq!(
            board
                .history()
                .iter()
                .map(|entry| entry.id.get())
                .collect::<Vec<_>>(),
            expected_ids
        );
        assert_eq!(
            board
                .history()
                .iter()
                .map(|entry| entry.san.clone())
                .collect::<Vec<_>>(),
            expected_san
        );
    }

    #[test]
    fn rejects_move_by_wrong_side() {
        let mut board = VisualBoard::standard();
        let err = board
            .apply_move(MoveRequest::new(square("e7"), square("e5")), Instant::now())
            .unwrap_err();
        assert!(matches!(err, BoardError::WrongSideToMove { .. }));
    }

    #[test]
    fn rejects_illegal_piece_movement_pattern() {
        let mut board = VisualBoard::standard();
        let err = board
            .apply_move(MoveRequest::new(square("b1"), square("b3")), Instant::now())
            .unwrap_err();
        assert!(matches!(err, BoardError::IllegalMove { .. }));
    }

    #[test]
    fn rejects_move_that_leaves_king_in_check() {
        let mut board = VisualBoard::empty();
        board.set_piece(
            square("e1"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::King,
            }),
        );
        board.set_piece(
            square("e8"),
            Some(Piece {
                side: Side::Black,
                kind: PieceKind::King,
            }),
        );
        board.set_piece(
            square("e2"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::Rook,
            }),
        );
        board.set_piece(
            square("e7"),
            Some(Piece {
                side: Side::Black,
                kind: PieceKind::Rook,
            }),
        );
        board.side_to_move = Side::White;

        let err = board
            .apply_move(MoveRequest::new(square("e2"), square("f2")), Instant::now())
            .unwrap_err();
        assert!(matches!(err, BoardError::IllegalMove { .. }));
    }

    #[test]
    fn castling_is_recorded_in_san() {
        let mut board = VisualBoard::empty();
        board.set_piece(
            square("e1"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::King,
            }),
        );
        board.set_piece(
            square("h1"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::Rook,
            }),
        );
        board.set_piece(
            square("e8"),
            Some(Piece {
                side: Side::Black,
                kind: PieceKind::King,
            }),
        );
        board.castling_rights = CastlingRights {
            white_king_side: true,
            white_queen_side: false,
            black_king_side: false,
            black_queen_side: false,
        };

        board
            .apply_move(MoveRequest::new(square("e1"), square("g1")), Instant::now())
            .unwrap();

        assert_eq!(board.history()[0].san, "O-O");
        assert_eq!(board.piece_at(square("g1")).unwrap().kind, PieceKind::King);
        assert_eq!(board.piece_at(square("f1")).unwrap().kind, PieceKind::Rook);
    }

    #[test]
    fn capture_notation_is_generated() {
        let mut board = VisualBoard::empty();
        board.set_piece(
            square("e1"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::King,
            }),
        );
        board.set_piece(
            square("e8"),
            Some(Piece {
                side: Side::Black,
                kind: PieceKind::King,
            }),
        );
        board.set_piece(
            square("e4"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::Pawn,
            }),
        );
        board.set_piece(
            square("d5"),
            Some(Piece {
                side: Side::Black,
                kind: PieceKind::Pawn,
            }),
        );

        board
            .apply_move(MoveRequest::new(square("e4"), square("d5")), Instant::now())
            .unwrap();
        assert_eq!(board.history()[0].san, "exd5");
    }

    #[test]
    fn disambiguation_is_generated_for_same_piece_type() {
        let mut board = VisualBoard::empty();
        board.set_piece(
            square("e1"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::King,
            }),
        );
        board.set_piece(
            square("e8"),
            Some(Piece {
                side: Side::Black,
                kind: PieceKind::King,
            }),
        );
        board.set_piece(
            square("a1"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::Rook,
            }),
        );
        board.set_piece(
            square("a3"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::Rook,
            }),
        );

        board
            .apply_move(MoveRequest::new(square("a1"), square("a2")), Instant::now())
            .unwrap();
        assert_eq!(board.history()[0].san, "R1a2");
    }

    #[test]
    fn promotion_records_queen_suffix() {
        let mut board = VisualBoard::empty();
        board.set_piece(
            square("e1"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::King,
            }),
        );
        board.set_piece(
            square("e8"),
            Some(Piece {
                side: Side::Black,
                kind: PieceKind::King,
            }),
        );
        board.set_piece(
            square("e7"),
            Some(Piece {
                side: Side::White,
                kind: PieceKind::Pawn,
            }),
        );

        board
            .apply_move(MoveRequest::new(square("e7"), square("e8")), Instant::now())
            .unwrap_err();

        board.set_piece(square("e8"), None);
        board.set_piece(
            square("a8"),
            Some(Piece {
                side: Side::Black,
                kind: PieceKind::King,
            }),
        );
        board
            .apply_move(MoveRequest::new(square("e7"), square("e8")), Instant::now())
            .unwrap();

        assert_eq!(board.history()[0].san, "e8=Q+");
        assert_eq!(board.piece_at(square("e8")).unwrap().kind, PieceKind::Queen);
    }

    #[test]
    fn reset_restores_standard_position_and_clears_state() {
        let mut board = VisualBoard::standard();
        let now = Instant::now();
        board
            .apply_move(MoveRequest::new(square("e2"), square("e4")), now)
            .unwrap();
        assert_eq!(board.history_len(), 1);

        board.reset_to_start();
        assert_eq!(board.history_len(), 0);
        assert_eq!(board.side_to_move(), Side::White);
        assert!(board.active_animation().is_none());
        assert_eq!(board.pending_move_id(), None);
        assert_eq!(board.piece_at(square("e2")).unwrap().kind, PieceKind::Pawn);
        assert_eq!(board.piece_at(square("e4")), None);
    }
}
