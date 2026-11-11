//! A game as a page hands it on: to its Export menu, and to the analysis board to open or
//! review it. One value says both, so the two never disagree about its players or result.

use kchess_domain::variant::Setup;
use kchess_games::Outcome;
use serde_json::Value;
use shakmaty::Color;

/// Where a game on the analysis board came from: its players, the side you played and its
/// Lichess (or Chess.com) game. A game set up or typed on the board has none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameOrigin {
    pub white: Option<String>,
    pub black: Option<String>,
    /// Your side, when you played the game: its review saves your mistakes for practice.
    pub player: Option<Color>,
    /// Its game id on Lichess or Chess.com, for that site's analysis and its links.
    pub game_id: Option<String>,
    /// The account that played it, for that account's analysis of the game.
    pub account: Option<String>,
}

impl GameOrigin {
    /// A Lichess game known by its id alone.
    pub fn lichess(id: impl Into<String>) -> GameOrigin {
        GameOrigin {
            game_id: Some(id.into()),
            ..GameOrigin::default()
        }
    }

    /// The players' names, as a document (a mistake drill's game, an explorer game) gives them.
    pub fn named(white: &Value, black: &Value) -> GameOrigin {
        GameOrigin {
            white: white.as_str().map(str::to_string),
            black: black.as_str().map(str::to_string),
            ..GameOrigin::default()
        }
    }

    /// A side's name, if known.
    pub fn name(&self, color: Color) -> Option<&str> {
        match color {
            Color::White => self.white.as_deref(),
            Color::Black => self.black.as_deref(),
        }
    }
}

/// A game a page shows: its start and moves, its players and result, how its board faces and
/// the move shown.
#[derive(Clone)]
pub struct PlayedGame {
    pub setup: Setup,
    pub moves: Vec<String>,
    pub orientation: Color,
    pub white: String,
    pub black: String,
    /// The position shown, for the PNG and for the analysis board; the last one by default.
    pub ply: Option<usize>,
    /// How it ended; `None` while it goes on.
    pub result: Option<Outcome>,
    /// Your side, when you played it.
    pub player: Option<Color>,
    /// The game's id when it was played on Lichess, so its link needs no upload.
    pub lichess_id: Option<String>,
    /// The Lichess account that played it.
    pub account: Option<String>,
    /// The document to save and upload as it is, with its variations, comments and headers,
    /// instead of the moves rebuilt as a PGN.
    pub pgn: Option<String>,
    /// The moves from `fen` to the position shown, when it is off `moves` (a variation).
    pub shown: Option<Vec<String>>,
}

impl PlayedGame {
    /// A game from `setup` and its moves, between `white` and `black`.
    pub fn new(
        setup: Setup,
        moves: Vec<String>,
        orientation: Color,
        white: impl Into<String>,
        black: impl Into<String>,
    ) -> PlayedGame {
        PlayedGame {
            setup,
            moves,
            orientation,
            white: white.into(),
            black: black.into(),
            ply: None,
            result: None,
            player: None,
            lichess_id: None,
            account: None,
            pgn: None,
            shown: None,
        }
    }

    /// Where it came from, as the analysis board keeps it.
    pub fn origin(&self) -> GameOrigin {
        GameOrigin {
            white: Some(self.white.clone()),
            black: Some(self.black.clone()),
            player: self.player,
            game_id: self.lichess_id.clone(),
            account: self.account.clone(),
        }
    }

    /// The game for the analysis board: its own document, or its moves from its start; `None`
    /// when that start is not a legal position.
    pub fn analysis_pgn(&self) -> Option<String> {
        match &self.pgn {
            Some(pgn) => Some(pgn.clone()),
            None => {
                kchess_domain::pgn::setup_pgn(self.setup.variant, &self.setup.fen, &self.moves, &[])
                    .ok()
            }
        }
    }
}
