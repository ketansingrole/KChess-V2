use std::time::Instant;
use std::{error::Error, fmt};

pub use kchess_board::{
    AppliedMove, BoardError, Easing, MoveId, MoveRequest, Perspective, Piece, PieceKind,
    RenderPiece, Side, Square, VisualBoard,
};

#[derive(Debug, Clone)]
pub struct BoardApi {
    board: VisualBoard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    InvalidSquareNotation { from: String, to: String },
    Board(BoardError),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSquareNotation { from, to } => {
                write!(f, "invalid move notation: from={from}, to={to}")
            }
            Self::Board(err) => write!(f, "{err}"),
        }
    }
}

impl Error for ApiError {}

impl From<BoardError> for ApiError {
    fn from(value: BoardError) -> Self {
        Self::Board(value)
    }
}

impl Default for BoardApi {
    fn default() -> Self {
        Self::standard()
    }
}

impl BoardApi {
    pub fn empty() -> Self {
        Self {
            board: VisualBoard::empty(),
        }
    }

    pub fn standard() -> Self {
        Self {
            board: VisualBoard::standard(),
        }
    }

    pub fn board(&self) -> &VisualBoard {
        &self.board
    }

    pub fn board_mut(&mut self) -> &mut VisualBoard {
        &mut self.board
    }

    pub fn apply_algebraic_move(
        &mut self,
        from: &str,
        to: &str,
        now: Instant,
    ) -> Result<MoveId, ApiError> {
        let request = parse_move(from, to).ok_or(ApiError::InvalidSquareNotation {
            from: from.to_owned(),
            to: to.to_owned(),
        })?;
        self.board.apply_move(request, now).map_err(Into::into)
    }

    pub fn undo_last(&mut self, now: Instant) -> Result<MoveId, BoardError> {
        self.board.undo_last(now)
    }

    pub fn render_pieces(&self, now: Instant, perspective: Perspective) -> Vec<RenderPiece> {
        self.board.render_pieces(now, perspective)
    }
}

pub fn parse_move(from: &str, to: &str) -> Option<MoveRequest> {
    MoveRequest::from_algebraic(from, to)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    #[test]
    fn applies_move_by_square_notation() {
        let mut api = super::BoardApi::standard();
        let id = api
            .apply_algebraic_move("e2", "e4", Instant::now())
            .expect("move should be accepted");

        assert_eq!(id.get(), 1);
    }

    #[test]
    fn parses_move() {
        let request = super::parse_move("a2", "a4").expect("valid move");
        assert_eq!(request.from.algebraic(), "a2");
        assert_eq!(request.to.algebraic(), "a4");
    }
}
