//! Replying to a correspondence game from its notification: its "your move" notification takes a
//! typed move, which comes back as a second notification to confirm it before it is sent (Play or
//! Cancel), so nothing typed there is played unseen. Only correspondence games are offered this:
//! a live game is played on the board, where its clock and the opponent are in sight.
//!
//! The core sends the move (`playCorrespondence`) only while the game is still on the account's
//! move at the position the reply was typed for, and only once.

use kchess_contracts::calls;
use std::collections::HashMap;

use gpui_kit::{App, Global};
use serde_json::{Value, json};
use shakmaty::Square;

use crate::core::AppCore;

/// A move typed in reply, legal where it was typed, waiting for Play or Cancel.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub account: String,
    pub id: String,
    pub opponent: String,
    /// The position it was typed for, as the core checks it again before sending.
    pub fen: String,
    pub uci: String,
    pub san: String,
}

/// The replies waiting for Play, by their confirmation's tag.
#[derive(Default)]
struct Pending(HashMap<String, Reply>);

impl Global for Pending {}

/// The position of an ongoing correspondence game on your move, as a full FEN: Lichess lists only
/// its placement, so the side is yours and castling is allowed where king and rook still stand on
/// their squares. Lichess refuses a castling the game no longer allows, which the reply reports.
pub fn position(game: &Value) -> Option<String> {
    let placement = game["fen"].as_str()?.split_whitespace().next()?;
    let board: shakmaty::Board = placement.parse().ok()?;
    let side = if game["color"] == json!("black") {
        "b"
    } else {
        "w"
    };
    let piece = |square: Square, letter: char| {
        board
            .piece_at(square)
            .is_some_and(|piece| piece.char() == letter)
    };
    let mut castling = String::new();
    for (king, rook, king_letter, rook_letter, right) in [
        (Square::E1, Square::H1, 'K', 'R', 'K'),
        (Square::E1, Square::A1, 'K', 'R', 'Q'),
        (Square::E8, Square::H8, 'k', 'r', 'k'),
        (Square::E8, Square::A8, 'k', 'r', 'q'),
    ] {
        if piece(king, king_letter) && piece(rook, rook_letter) {
            castling.push(right);
        }
    }
    if castling.is_empty() {
        castling.push('-');
    }
    Some(format!("{placement} {side} {castling} - 0 1"))
}

/// `text` as a legal move of `game` on your move: SAN or UCI, as the move box takes it.
pub fn read(game: &Value, text: &str) -> Option<Reply> {
    let fen = position(game)?;
    let variant = game["variant"].as_str().unwrap_or("standard");
    let start = kchess_domain::rules::setup_start(variant, &fen)?;
    let text = text.trim();
    let uci = crate::game::view::typed_uci(&json!({ "variant": variant }), &start, text)?;
    let played = kchess_domain::position::play_uci(&start, &uci)?;
    Some(Reply {
        account: game["account"].as_str()?.to_string(),
        id: game["gameId"].as_str()?.to_string(),
        opponent: game["opponent"]["name"]
            .as_str()
            .unwrap_or("your opponent")
            .to_string(),
        fen,
        uci: played.uci,
        san: played.san,
    })
}

/// Keep `reply` for its confirmation's Play.
pub fn hold(tag: &str, reply: Reply, cx: &mut App) {
    cx.default_global::<Pending>()
        .0
        .insert(tag.to_string(), reply);
}

/// The reply waiting under `tag`, taken: Play or Cancel answers it once.
pub fn take(tag: &str, cx: &mut App) -> Option<Reply> {
    cx.default_global::<Pending>().0.remove(tag)
}

/// Send `reply`; the answer is the core's, an error naming why it was not played.
pub fn send(reply: &Reply, cx: &mut App) -> gpui_kit::Task<Result<Value, String>> {
    let call = cx.global::<AppCore>().request(calls::PlayCorrespondence {
        account: json!(reply.account),
        id: json!(reply.id),
        fen: json!(reply.fen),
        r#move: json!(reply.uci),
    });
    cx.background_executor()
        .spawn(async move { call.await.map_err(|cause| cause.message) })
}

#[cfg(test)]
mod tests {
    use super::{position, read};
    use serde_json::json;

    fn game(fen: &str, color: &str) -> serde_json::Value {
        json!({
            "gameId": "Corr1234",
            "account": "Tester",
            "opponent": { "name": "Bob" },
            "color": color,
            "fen": fen,
            "variant": "standard",
            "speed": "correspondence",
            "isMyTurn": true,
        })
    }

    /// Lichess lists only the placement: the side is the player's, castling where king and
    /// rook stand on their squares.
    #[test]
    fn the_listed_placement_becomes_the_position_on_your_move() {
        let start = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR";
        assert_eq!(
            position(&game(start, "black")).as_deref(),
            Some("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1")
        );
        let moved = "rnbq1bnr/ppppkppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBN1";
        assert_eq!(
            position(&game(moved, "white")).as_deref(),
            Some("rnbq1bnr/ppppkppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBN1 w Q - 0 1")
        );
        assert_eq!(position(&game("not a board", "white")), None);
    }

    /// A reply is read as the move box reads a move, SAN or UCI, and only when legal.
    #[test]
    fn a_reply_is_a_legal_move_in_san_or_uci() {
        let after_e4 = game("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR", "black");
        let reply = read(&after_e4, " Nf6 ").expect("a legal reply");
        assert_eq!((reply.uci.as_str(), reply.san.as_str()), ("g8f6", "Nf6"));
        assert_eq!(
            (reply.account.as_str(), reply.id.as_str()),
            ("Tester", "Corr1234")
        );
        assert_eq!(
            read(&after_e4, "e7e5").map(|reply| reply.san),
            Some("e5".into())
        );
        assert_eq!(read(&after_e4, "Ke2"), None);
        assert_eq!(read(&after_e4, "hello"), None);
    }
}
