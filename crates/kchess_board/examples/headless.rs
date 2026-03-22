use std::time::{Duration, Instant};

use kchess_board::{MoveRequest, Perspective, Square, VisualBoard};

fn main() {
    let mut board = VisualBoard::standard();

    let e2 = Square::from_algebraic("e2").unwrap();
    let e4 = Square::from_algebraic("e4").unwrap();

    let start = Instant::now();
    let pending_id = board
        .apply_move_pending(MoveRequest::new(e2, e4), start)
        .expect("visual move should apply");

    // Simulated engine response.
    let legal = true;
    if legal {
        board
            .confirm_pending(pending_id)
            .expect("pending should confirm");
    } else {
        board
            .rollback_pending(pending_id, Instant::now())
            .expect("pending should rollback");
    }

    let frame = board.render_pieces(start + Duration::from_millis(90), Perspective::White);
    println!("renderable pieces this frame: {}", frame.len());
}
