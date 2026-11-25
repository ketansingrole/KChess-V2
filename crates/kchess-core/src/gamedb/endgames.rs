//! The endgames of Your games that slipped away (`gameDbYourEndgames`): each of your games is
//! replayed, and at each of your moves from a position with five pieces or fewer the endgame
//! tablebases judge the position before it and after it. A win that became a draw or a loss, or
//! a draw that became a loss, is listed: the first in a game, since what follows it comes of it.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::Connection;
use shakmaty::fen::Fen;
use shakmaty::san::SanPlus;
use shakmaty::variant::VariantPosition;
use shakmaty::{Chess, Color, EnPassantMode, Position as _, Role};
use shakmaty_syzygy::Tablebase;

use super::search;
use crate::store::sql::Sql as _;
use crate::tablebase::outcome;
use kchess_contracts::error::{CoreError, Result};
use kchess_contracts::types::YourEndgame;
use kchess_domain::gamedb;

/// The most endgames an answer lists.
pub const MOST: usize = 200;

/// The pieces of `pos`, `color`'s first: "KRP vs KR".
pub fn material(pos: &Chess, color: Color) -> String {
    let side = |color: Color| {
        let mut text = String::from("K");
        for (role, letter) in [
            (Role::Queen, 'Q'),
            (Role::Rook, 'R'),
            (Role::Bishop, 'B'),
            (Role::Knight, 'N'),
            (Role::Pawn, 'P'),
        ] {
            let count = (pos.board().by_role(role) & pos.board().by_color(color)).count();
            text.extend(std::iter::repeat_n(letter, count));
        }
        text
    };
    format!("{} vs {}", side(color), side(!color))
}

/// The games of Your games (`db`) where one of `names` (in any case) played, the newest first,
/// each replayed for the first of their moves that gave away a win or a draw in `tables`. Answers
/// the games gone through and at most `MOST` such moves.
pub fn your_endgames(
    db: &Connection,
    names: &[String],
    tables: &Tablebase<Chess>,
    stop: &AtomicBool,
) -> Result<(u64, Vec<YourEndgame>)> {
    let names: HashSet<String> = names
        .iter()
        .map(|name| name.trim().to_lowercase())
        .collect();
    let mut statement = db
        .prepare(
            "SELECT g.id, w.name, b.name, g.fen, g.moves, g.date FROM games g
             JOIN players w ON w.id = g.white JOIN players b ON b.id = g.black
             WHERE g.import_id <= ?1 ORDER BY g.date_key DESC, g.id DESC",
        )
        .sql()?;
    let mut rows = statement.query([search::visible(db)?]).sql()?;
    let (mut games, mut found) = (0, Vec::new());
    while let Some(row) = rows.next().sql()? {
        if stop.load(Ordering::Relaxed) {
            return Err(CoreError::new("Looking through your endgames stopped."));
        }
        let (white, black): (String, String) = (row.get(1).sql()?, row.get(2).sql()?);
        let (you, opponent) = if names.contains(&white.trim().to_lowercase()) {
            (Color::White, black)
        } else if names.contains(&black.trim().to_lowercase()) {
            (Color::Black, white)
        } else {
            continue;
        };
        games += 1;
        let fen: Option<String> = row.get(3).sql()?;
        let code: Vec<u8> = row.get(4).sql()?;
        let Some((start, moves)) = gamedb::start_of(fen.as_deref())
            .and_then(|start| Some((start.clone(), gamedb::decode_line(&start, &code)?)))
        else {
            continue;
        };
        let VariantPosition::Chess(mut pos) = start else {
            continue;
        };
        for m in moves {
            if pos.turn() == you
                && pos.board().occupied().count() <= crate::tablebase::PIECES as usize
            {
                let mut after = pos.clone();
                after.play_unchecked(m);
                let judge = |pos: &Chess| {
                    outcome(tables, pos).map_err(|cause| {
                        CoreError::new(format!("A tablebase probe failed: {cause}"))
                    })
                };
                // After your move it is the opponent's turn: their outcome, turned round.
                let slipped = match (judge(&pos)?, judge(&after)?.map(|theirs| -theirs)) {
                    (Some(2), Some(-2)) => Some(("win", "loss")),
                    (Some(2), Some(now)) if now <= 0 => Some(("win", "draw")),
                    (Some(0), Some(-2)) => Some(("draw", "loss")),
                    _ => None,
                };
                if let Some((goal, became)) = slipped {
                    let id: i64 = row.get(0).sql()?;
                    found.push(YourEndgame {
                        game: id.to_string(),
                        fen: Fen::from_position(&pos, EnPassantMode::Legal).to_string(),
                        side: if you == Color::White {
                            "white"
                        } else {
                            "black"
                        }
                        .into(),
                        goal: goal.into(),
                        san: SanPlus::from_move_and_play_unchecked(&mut pos.clone(), m).to_string(),
                        became: became.into(),
                        material: material(&pos, you),
                        number: pos.fullmoves().get(),
                        opponent,
                        date: row.get(5).sql()?,
                    });
                    break;
                }
            }
            pos.play_unchecked(m);
        }
        if found.len() >= MOST {
            break;
        }
    }
    Ok((games, found))
}
