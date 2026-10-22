//! What the game pages show of a game: the position at a move, the moves in SAN, the last move,
//! a typed move and the board's state, from a setup (`{ variant, fen }`) and its UCI moves.

use serde_json::Value;
use shakmaty::variant::VariantPosition;
use shakmaty::{Chess, Color, Position as _, Square};

use crate::board::BoardState;

/// The setup's variant and FEN.
fn setup_parts(setup: &Value) -> (&str, &str) {
    (
        setup["variant"].as_str().unwrap_or("standard"),
        setup["fen"].as_str().unwrap_or_default(),
    )
}

/// The position after the first `ply` moves (`setupPositionAfter`).
pub fn position_at(setup: &Value, moves: &[String], ply: usize) -> Option<VariantPosition> {
    let (variant, fen) = setup_parts(setup);
    let replay = kchess_domain::replay::replay_setup(variant, fen, &moves[..ply.min(moves.len())])?;
    kchess_domain::rules::setup_start(variant, &replay.fen)
}

/// Each move in SAN (`setupSanHistory`).
pub fn sans(setup: &Value, moves: &[String]) -> Vec<String> {
    let (variant, fen) = setup_parts(setup);
    kchess_domain::replay::replay_setup(variant, fen, moves)
        .map(|replay| replay.played.into_iter().map(|played| played.san).collect())
        .unwrap_or_default()
}

/// The squares of the move before `ply` (`lastMoveKeys`).
pub fn last_move(moves: &[String], ply: usize) -> Option<(Square, Square)> {
    let uci = moves.get(ply.checked_sub(1)?)?;
    let from = uci.get(0..2)?.parse::<Square>().ok()?;
    let to = uci.get(2..4)?.parse::<Square>().ok()?;
    Some((from, to))
}

/// Whether the game began with Black to move (the move list then starts with "1. …").
pub fn black_first(setup: &Value) -> bool {
    setup_parts(setup).1.split(' ').nth(1) == Some("b")
}

/// A move typed as SAN or UCI, as UCI when it is legal here (`keyboardMove`).
pub fn typed_uci(setup: &Value, position: &VariantPosition, text: &str) -> Option<String> {
    let (variant, _) = setup_parts(setup);
    let pos =
        kchess_domain::rules::setup_start(variant, &kchess_domain::rules::make_fen(position))?;
    let bytes = text.as_bytes();
    let is_uci = bytes.len() >= 4
        && bytes[0].is_ascii_lowercase()
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_lowercase()
        && bytes[3].is_ascii_digit();
    if is_uci {
        kchess_domain::position::play_uci(&pos, text).map(|_| text.to_lowercase())
    } else {
        kchess_domain::position::play_san(&pos, text).map(|played| played.uci)
    }
}

/// The board for the game at `ply`, with `movable` the side that may move now.
pub fn board_state(
    setup: &Value,
    moves: &[String],
    ply: usize,
    orientation: Color,
    movable: Option<Color>,
) -> BoardState {
    let position =
        position_at(setup, moves, ply).unwrap_or_else(|| VariantPosition::Chess(Chess::default()));
    BoardState::of(&position, orientation, movable, last_move(moves, ply))
}

/// Whose turn it is in a position.
pub fn turn(position: Option<&VariantPosition>) -> Color {
    position.map_or(Color::White, |position| position.turn())
}

/// Where an arrow key moves a move list's position (←/→ one ply, ↑/↓ first/last), as
/// `navigatePly` does; `None` for other keys.
pub fn navigate(key: &str, ply: usize, max: usize) -> Option<usize> {
    match key {
        "left" => Some(ply.saturating_sub(1)),
        "right" => Some((ply + 1).min(max)),
        "up" => Some(0),
        "down" => Some(max),
        _ => None,
    }
}
