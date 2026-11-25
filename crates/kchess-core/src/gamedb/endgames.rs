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
use shakmaty::{Chess, Color, EnPassantMode, Move, Position as _, Role};
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

/// What one of your games came to in the tablebases.
#[derive(Default)]
pub struct Scanned {
    /// The first position at your move the tables decide: its outcome for you (2 won, 0 drawn,
    /// -2 lost) and its pieces.
    pub reached: Option<(i8, String)>,
    /// The first of your moves that let a win or a draw go.
    pub slip: Option<Slip>,
}

/// A move of yours that let a win or a draw go.
pub struct Slip {
    /// The position before it.
    pub pos: Chess,
    pub played: Move,
    /// "win" or "draw", and what it became: "draw" or "loss".
    pub goal: &'static str,
    pub became: &'static str,
}

impl Slip {
    /// The position as written in Your endgames, and in Insights' endgames that lead there.
    pub fn fen(&self) -> String {
        Fen::from_position(&self.pos, EnPassantMode::Legal).to_string()
    }

    /// The slip as Your endgames lists it, of `game` against `opponent` on `date`.
    fn endgame(
        self,
        game: String,
        you: Color,
        opponent: String,
        date: Option<String>,
    ) -> YourEndgame {
        YourEndgame {
            game,
            fen: self.fen(),
            side: if you == Color::White {
                "white"
            } else {
                "black"
            }
            .into(),
            goal: self.goal.into(),
            san: SanPlus::from_move_and_play_unchecked(&mut self.pos.clone(), self.played)
                .to_string(),
            became: self.became.into(),
            material: material(&self.pos, you),
            number: self.pos.fullmoves().get(),
            opponent,
            date,
        }
    }
}

/// `moves` from `pos`, played by `you` and the opponent, judged at each of your moves from five
/// pieces or fewer: the first position the tables decide, and the first move that let a win or
/// a draw go (what follows it comes of it).
pub fn scan(
    mut pos: Chess,
    moves: &[Move],
    you: Color,
    tables: &Tablebase<Chess>,
) -> Result<Scanned> {
    let judge = |pos: &Chess| {
        outcome(tables, pos)
            .map_err(|cause| CoreError::new(format!("A tablebase probe failed: {cause}")))
    };
    let mut scanned = Scanned::default();
    for m in moves {
        if pos.turn() == you && pos.board().occupied().count() <= crate::tablebase::PIECES as usize
        {
            let before = judge(&pos)?;
            if scanned.reached.is_none()
                && let Some(decided @ (2 | 0 | -2)) = before
            {
                scanned.reached = Some((decided, material(&pos, you)));
            }
            if matches!(before, Some(2 | 0)) {
                let mut after = pos.clone();
                after.play_unchecked(*m);
                // After your move it is the opponent's turn: their outcome, turned round.
                let slipped = match (before, judge(&after)?.map(|theirs| -theirs)) {
                    (Some(2), Some(-2)) => Some(("win", "loss")),
                    (Some(2), Some(now)) if now <= 0 => Some(("win", "draw")),
                    (Some(0), Some(-2)) => Some(("draw", "loss")),
                    _ => None,
                };
                if let Some((goal, became)) = slipped {
                    scanned.slip = Some(Slip {
                        pos,
                        played: *m,
                        goal,
                        became,
                    });
                    return Ok(scanned);
                }
            }
        }
        pos.play_unchecked(*m);
    }
    Ok(scanned)
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
        let VariantPosition::Chess(pos) = start else {
            continue;
        };
        let id: i64 = row.get(0).sql()?;
        if let Some(slip) = scan(pos, &moves, you, tables)?.slip {
            found.push(slip.endgame(id.to_string(), you, opponent, row.get(5).sql()?));
        }
        if found.len() >= MOST {
            break;
        }
    }
    Ok((games, found))
}
