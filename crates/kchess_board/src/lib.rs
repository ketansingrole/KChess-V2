pub mod board;

pub use board::{
    AnimationState, AppliedMove, BoardError, Easing, MoveHistoryCell, MoveHistoryRow, MoveId,
    MoveRequest, Perspective, Piece, PieceKind, RenderPiece, Side, Square, VisualBoard,
};

#[cfg(feature = "gpui")]
pub mod gpui_view;

#[cfg(feature = "gpui")]
pub use gpui_view::{ArrowKind, BoardArrow, BoardTheme, ChessBoardView};
