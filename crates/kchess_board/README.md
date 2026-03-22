# kchess_board

A Rust-first, visual chessboard library with optional GPUI integration.

This crate intentionally does **not** validate chess legality. It is the visual board layer only:
- square and piece state,
- animated movement,
- pending move flow (for external validation),
- rollback/undo behavior.

## Why this architecture

This fits a clean 3-layer stack:
1. **`kchess_board` (this crate):** visual board and interactions.
2. **Engine/background app:** chess legality, FEN, PGN/UCI, clocks, etc.
3. **Wrapper app:** product UI/composition and orchestration.

That split keeps rendering responsive and makes it easier to swap engines or frontends later.

## Core usage (no GPUI required)

```rust
use std::time::Instant;
use kchess_board::{MoveRequest, Square, VisualBoard};

let mut board = VisualBoard::standard();
let now = Instant::now();

let from = Square::from_algebraic("e2").unwrap();
let to = Square::from_algebraic("e4").unwrap();

// Optimistic visual move.
let pending_id = board.apply_move_pending(MoveRequest::new(from, to), now).unwrap();

// Later, after your engine validates:
let move_is_legal = true;
if move_is_legal {
    board.confirm_pending(pending_id).unwrap();
} else {
    board.rollback_pending(pending_id, Instant::now()).unwrap();
}
```

## Rendering model for any UI toolkit

Use `render_pieces(now, perspective)` to get interpolated piece positions (`x`, `y` in board cells), including in-flight animation state.

## GPUI

Enable the `gpui` feature to use `ChessBoardView` and run the demo:

```bash
cargo run --example gpui_demo --features gpui
```

`ChessBoardView` supports:
- left-button drag-and-drop moves with pickup scaling,
- right-button drag to draw/remove arrows,
- automatic arrow kind selection:
  - straight arrows for normal gestures
  - `L` arrows for knight-style gestures/moves
- configurable perspective and square size,
- SVG piece sprites (cburnett set from lichess),
- optional auto-confirm pending moves,
- external `confirm_pending` / `rollback_pending` control.

## Engine-facing legal move API

`ChessBoardView` exposes a request/response pattern for legal moves:

1. User presses a piece with left mouse:
   - call `take_legal_moves_request()` from your app loop
2. Your engine computes legal squares for that source square
3. Feed highlights back with:
   - `set_legal_moves_for(source, targets)` or `set_legal_move_targets(targets)`
4. Clear with:
   - `clear_legal_moves()`

It also exposes arrow state:
- `arrows()`
- `set_arrows(...)`
- `clear_arrows()`

Types:
- `BoardArrow { from, to, kind }`
- `ArrowKind::{Straight, KnightL}`

Piece assets are stored in `assets/pieces/cburnett/` with attribution in:
- `assets/pieces/cburnett/ATTRIBUTION.md`

## Status

The repository environment used to generate this scaffold did not have Rust tooling (`cargo`) installed, so compilation was not executed here.
