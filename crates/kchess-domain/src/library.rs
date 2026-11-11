//! Library document decoders: semantic validation before a document reaches the library or a
//! board. Each returns the document normalized.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::js::{self, finite, integer, string, text};
use crate::misc::constants::HistoryKind;
use crate::replay;
use crate::tree;

pub const MAX_STUDIES: usize = 50;
pub const MAX_CHAPTERS: usize = 64;
pub const MAX_MISTAKES: usize = 500;

use crate::variant::{Setup, Variant};

fn is_variant(v: Option<&Value>) -> Option<Variant> {
    string(v).and_then(Variant::from_key)
}

fn object(v: Option<&Value>) -> Option<&Map<String, Value>> {
    match v {
        Some(Value::Object(map)) => Some(map),
        _ => None,
    }
}

/* ── Studies ── */

/// A study's chapter: its id, name and PGN.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(expecting = "an object")]
pub struct StudyChapter {
    pub id: String,
    pub name: String,
    pub pgn: String,
}

/// The Lichess study a local one is a copy of, and the PGN as it was downloaded.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StudyCloud {
    pub account: String,
    pub id: String,
    pub downloaded_pgn: String,
    /// Chapters were added, removed or moved since the download.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub structure_changed: bool,
}

/// A study as the library keeps it. Its `pgn` is its first chapter's.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Study {
    pub id: String,
    pub name: String,
    pub pgn: String,
    pub chapters: Vec<StudyChapter>,
    /// In milliseconds since the epoch, in the form it was written: whole when a command wrote
    /// it, fractional when read back from the profile.
    pub updated_at: serde_json::Number,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<StudyCloud>,
}

impl Study {
    pub fn chapter(&self, id: &str) -> Option<&StudyChapter> {
        self.chapters.iter().find(|chapter| chapter.id == id)
    }

    pub fn updated_at_ms(&self) -> f64 {
        self.updated_at.as_f64().unwrap_or_default()
    }
}

fn decode_chapters(raw: Option<&Value>, fallback: StudyChapter) -> Option<Vec<StudyChapter>> {
    let chapters = match raw {
        None | Some(Value::Null) => {
            return tree::valid_pgn(&fallback.pgn).then(|| vec![fallback]);
        }
        Some(Value::Array(chapters)) => chapters,
        Some(_) => return None,
    };
    if chapters.is_empty() || chapters.len() > MAX_CHAPTERS {
        return None;
    }
    chapters
        .iter()
        .map(|c| {
            let c = object(Some(c))?;
            let (id, name, pgn) = (
                string(c.get("id"))?,
                string(c.get("name"))?,
                string(c.get("pgn"))?,
            );
            tree::valid_pgn(pgn).then(|| StudyChapter {
                id: id.into(),
                name: name.into(),
                pgn: pgn.into(),
            })
        })
        .collect()
}

fn cloud_id(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A stored or sent study, checked: every chapter's PGN valid. A study from before chapters is
/// its one chapter, and a broken cloud link is dropped.
pub fn decode_study(raw: &Value) -> Option<Study> {
    let item = object(Some(raw))?;
    let id = string(item.get("id"))?;
    let name = string(item.get("name"))?;
    let pgn = string(item.get("pgn"))?;
    let updated_at = finite(item.get("updatedAt"))?;
    let chapters = decode_chapters(
        item.get("chapters"),
        StudyChapter {
            id: id.into(),
            name: name.into(),
            pgn: pgn.into(),
        },
    )?;
    let cloud = object(item.get("cloud")).and_then(|cloud| {
        let (account, cid, downloaded) = (
            string(cloud.get("account"))?,
            string(cloud.get("id"))?,
            string(cloud.get("downloadedPgn"))?,
        );
        cloud_id(cid).then(|| StudyCloud {
            account: account.into(),
            id: cid.into(),
            downloaded_pgn: downloaded.into(),
            structure_changed: cloud.get("structureChanged") == Some(&Value::Bool(true)),
        })
    });
    Some(Study {
        id: id.into(),
        name: name.into(),
        pgn: chapters[0].pgn.clone(),
        chapters,
        updated_at: float(updated_at),
        cloud,
    })
}

/// Invalid studies are left out rather than failing the whole library.
pub fn decode_studies(raw: &Value) -> Option<Vec<Study>> {
    let doc = object(Some(raw))?;
    let version = js::to_number(doc.get("version"));
    if !doc.contains_key("version")
        || !(version == 1.0 || version == 2.0)
        || !doc.contains_key("items")
    {
        return None;
    }
    let Value::Array(items) = doc.get("items")? else {
        return None;
    };
    Some(
        items
            .iter()
            .take(MAX_STUDIES)
            .filter_map(decode_study)
            .collect(),
    )
}

/* ── Played games ── */

/// A game played in KChess as Game history keeps it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayedGame {
    pub id: String,
    pub source: HistoryKind,
    /// When its first move was played, in milliseconds since the epoch.
    pub started_at: f64,
    pub updated_at: f64,
    pub white: String,
    pub black: String,
    /// `*` while it is unfinished, else `1-0`, `0-1` or `1/2-1/2`.
    pub result: String,
    pub reason: String,
    /// It will not be played on (a resumed game's entry is open until then).
    pub finished: bool,
    pub setup: Setup,
    /// UCI moves from the setup.
    pub moves: Vec<String>,
    pub time_control: String,
    /// The clocks at the end, for a chess clock game.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_summary: Option<String>,
}

impl PlayedGame {
    /// The player's side in a computer game: the one `computer::players` named
    /// `computer::PLAYER`. A game at the board or on the clock has none.
    pub fn player_side(&self) -> Option<shakmaty::Color> {
        use crate::games::computer::PLAYER;
        if self.source != HistoryKind::Computer {
            None
        } else if self.white == PLAYER {
            Some(shakmaty::Color::White)
        } else if self.black == PLAYER {
            Some(shakmaty::Color::Black)
        } else {
            None
        }
    }

    /// How the game ended for its player: `win`, `loss` or `draw` for the player's side, the
    /// winning colour (`white`, `black`) or `draw` at the board; `unfinished`, `ongoing` or
    /// `clock` when it has no result.
    pub fn outcome(&self) -> &'static str {
        if self.source == HistoryKind::Clock {
            return "clock";
        }
        match self.result.as_str() {
            "*" if !self.finished => "ongoing",
            "*" => "unfinished",
            "1/2-1/2" => "draw",
            result => {
                let winner = if result == "1-0" {
                    shakmaty::Color::White
                } else {
                    shakmaty::Color::Black
                };
                match self.player_side() {
                    None => crate::color::name(winner),
                    Some(you) if you == winner => "win",
                    Some(_) => "loss",
                }
            }
        }
    }
}

/// A stored or sent played game, checked: every field in bounds and every move legal.
pub fn decode_archived_game(raw: &Value) -> Option<PlayedGame> {
    let g = object(Some(raw))?;
    let id = text(g.get("id"), 80)?;
    let source = string(g.get("source")).and_then(HistoryKind::from_key)?;
    let started_at = finite(g.get("startedAt"))?;
    let updated_at = finite(g.get("updatedAt"))?;
    let result = string(g.get("result")).filter(|s| ["*", "1-0", "0-1", "1/2-1/2"].contains(s))?;
    let Some(Value::Bool(finished)) = g.get("finished") else {
        return None;
    };
    let white = text(g.get("white"), 200)?;
    let black = text(g.get("black"), 200)?;
    let reason = text(g.get("reason"), 200)?;
    let time_control = text(g.get("timeControl"), 200)?;
    let clock_summary = match g.get("clockSummary") {
        None => None,
        some => Some(text(some, 200)?),
    };
    if js::falsy(g.get("setup")) {
        return None;
    }
    let setup = object(g.get("setup"))?;
    let variant = is_variant(setup.get("variant"))?;
    let fen = text(setup.get("fen"), 120)?;
    let Some(Value::Array(moves)) = g.get("moves") else {
        return None;
    };
    if moves.len() > 1024 {
        return None;
    }
    let moves: Vec<&str> = moves
        .iter()
        .map(|m| text(Some(m), 10))
        .collect::<Option<_>>()?;
    if replay::replay_setup_count(variant, fen, &moves) != Some(moves.len()) {
        return None;
    }
    Some(PlayedGame {
        id: id.into(),
        source,
        started_at,
        updated_at,
        white: white.into(),
        black: black.into(),
        result: result.into(),
        reason: reason.into(),
        finished: *finished,
        setup: Setup {
            variant,
            fen: fen.into(),
        },
        moves: moves.into_iter().map(String::from).collect(),
        time_control: time_control.into(),
        clock_summary: clock_summary.map(String::from),
    })
}

pub const MAX_ARCHIVED_GAMES: usize = 500;

/* ── Mistake drills ── */

/// A position from a reviewed game to practise: the move that was missed and the line after it.
/// Its numbers keep the form they were written in (a drill answered since it was read writes
/// whole numbers, one read back from the profile fractional ones): the `*_ms`, `streak` and
/// `attempts` methods read them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Drill {
    /// The review's key and the ply, `key:ply`.
    pub id: String,
    pub fen: String,
    /// The best move, then the line after it (UCI).
    pub solution: Vec<String>,
    /// `inaccuracy`, `mistake` or `blunder`.
    pub judgment: String,
    pub due_at: serde_json::Number,
    /// Right answers in a row.
    pub streak: serde_json::Number,
    pub attempts: serde_json::Number,
    /// Played by this variant's rules; standard chess when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<Variant>,
}

impl Drill {
    /// When it is next due, in milliseconds since the epoch.
    pub fn due_at_ms(&self) -> f64 {
        self.due_at.as_f64().unwrap_or_default()
    }

    pub fn streak(&self) -> f64 {
        self.streak.as_f64().unwrap_or_default()
    }

    pub fn attempts(&self) -> f64 {
        self.attempts.as_f64().unwrap_or_default()
    }
}

/// A finite number, written as `serde_json` writes an `f64` (with a fraction).
fn float(n: f64) -> serde_json::Number {
    serde_json::Number::from_f64(n).unwrap_or_else(|| 0.into())
}

pub fn decode_mistakes(raw: &Value) -> Option<Vec<Drill>> {
    let doc = object(Some(raw))?;
    if finite(doc.get("version")) != Some(1.0) {
        return None;
    }
    let Some(Value::Array(items)) = doc.get("items") else {
        return None;
    };
    Some(
        items
            .iter()
            .take(MAX_MISTAKES)
            .filter_map(|raw| {
                if js::falsy(Some(raw)) {
                    return None;
                }
                let item = object(Some(raw))?;
                let id = string(item.get("id"))?;
                let fen = string(item.get("fen"))?;
                let Some(Value::Array(solution)) = item.get("solution") else {
                    return None;
                };
                let solution: Vec<&str> = solution
                    .iter()
                    .map(|m| string(Some(m)))
                    .collect::<Option<_>>()?;
                // A drill from a variant game is played by that variant's rules.
                let variant = match item.get("variant") {
                    None | Some(Value::Null) => None,
                    Some(variant) => Some(string(Some(variant)).and_then(Variant::from_key)?),
                };
                let playable = match variant {
                    Some(variant) => replay::replay_setup_count(variant, fen, &solution)? + 1,
                    None => replay::replay_positions_count(fen, &solution),
                };
                if playable != solution.len() + 1 {
                    return None;
                }
                let due_at = finite(item.get("dueAt"))?;
                let streak = integer(item.get("streak"))?;
                let attempts = integer(item.get("attempts"))?;
                Some(Drill {
                    id: id.into(),
                    fen: fen.into(),
                    solution: solution.into_iter().map(String::from).collect(),
                    judgment: js::to_string(item.get("judgment")),
                    due_at: float(due_at),
                    streak: float(streak),
                    attempts: float(attempts),
                    variant,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn game(moves: &[&str]) -> Value {
        json!({
            "id": "g1", "source": "board", "startedAt": 1, "updatedAt": 2,
            "white": "A", "black": "B", "result": "*", "reason": "", "finished": false,
            "setup": { "variant": "standard", "fen": START }, "moves": moves, "timeControl": "-",
        })
    }

    #[test]
    fn played_games_end_for_their_player_or_by_colour() {
        let computer = |white: &str, black: &str, result: &str| {
            let mut game = decode_archived_game(&game(&[])).unwrap();
            game.source = HistoryKind::Computer;
            (game.white, game.black, game.result) = (white.into(), black.into(), result.into());
            game.finished = true;
            game
        };
        assert_eq!(computer("You", "Stockfish", "1-0").outcome(), "win");
        assert_eq!(computer("You", "Stockfish", "0-1").outcome(), "loss");
        assert_eq!(computer("Stockfish", "You", "1-0").outcome(), "loss");
        assert_eq!(computer("Stockfish", "You", "0-1").outcome(), "win");
        assert_eq!(computer("You", "Stockfish", "1/2-1/2").outcome(), "draw");
        assert_eq!(computer("White", "Black", "0-1").outcome(), "black");
        assert_eq!(computer("You", "Stockfish", "*").outcome(), "unfinished");
        let board = decode_archived_game(&game(&[])).unwrap();
        assert_eq!(board.outcome(), "ongoing");
        let clock = PlayedGame {
            source: HistoryKind::Clock,
            result: "1-0".into(),
            ..board
        };
        assert_eq!(clock.outcome(), "clock");
        assert_eq!(
            computer("White", "You", "*").player_side(),
            Some(shakmaty::Color::Black)
        );
        assert_eq!(computer("White", "Black", "*").player_side(), None);
        let mut named_at_the_board = computer("You", "Black", "1-0");
        named_at_the_board.source = HistoryKind::Board;
        assert_eq!(named_at_the_board.player_side(), None);
        assert_eq!(named_at_the_board.outcome(), "white");
    }

    #[test]
    fn archived_games_need_every_move_legal() {
        assert!(decode_archived_game(&game(&["e2e4", "e7e5"])).is_some());
        assert!(decode_archived_game(&game(&["e2e4", "e2e4"])).is_none());
    }

    #[test]
    fn studies_drop_invalid_chapters_and_keep_cloud_links() {
        let doc = json!({ "version": 2, "items": [
            { "id": "s", "name": "S", "pgn": "1. e4 *", "updatedAt": 1,
              "cloud": { "account": "a", "id": "abcdEFGH", "downloadedPgn": "", "structureChanged": true } },
            { "id": "bad", "name": "B", "pgn": "1. e5 *", "updatedAt": 1 },
        ]});
        let studies = decode_studies(&doc).unwrap();
        assert_eq!(studies.len(), 1);
        assert!(studies[0].cloud.as_ref().unwrap().structure_changed);
        assert_eq!(studies[0].chapters[0].id, "s");
    }

    /// A study from before chapters is its one chapter, read back in the library's key order with
    /// its time fractional and a link without `structureChanged` written without it; a broken
    /// link is dropped and the study kept.
    #[test]
    fn a_stored_study_reads_back_as_the_library_writes_it() {
        let stored = json!({ "updatedAt": 7, "pgn": "1. e4 *", "name": "S", "id": "s",
            "cloud": { "account": "a", "id": "abcdEFGH", "downloadedPgn": "1. e4 *", "structureChanged": false } });
        assert_eq!(
            serde_json::to_string(&decode_study(&stored).unwrap()).unwrap(),
            r#"{"id":"s","name":"S","pgn":"1. e4 *","chapters":[{"id":"s","name":"S","pgn":"1. e4 *"}],"updatedAt":7.0,"cloud":{"account":"a","id":"abcdEFGH","downloadedPgn":"1. e4 *"}}"#
        );
        let broken = json!({ "id": "s", "name": "S", "pgn": "*", "updatedAt": 1, "cloud": { "id": "short" } });
        assert_eq!(decode_study(&broken).unwrap().cloud, None);
    }

    #[test]
    fn mistakes_need_a_playable_solution() {
        let doc = json!({ "version": 1, "items": [
            { "id": "m", "fen": START, "solution": ["e2e4"], "judgment": 3, "dueAt": 0, "streak": 0, "attempts": 1.0 },
            { "id": "x", "fen": START, "solution": ["e2e5"], "dueAt": 0, "streak": 0, "attempts": 0 },
        ]});
        let items = decode_mistakes(&doc).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].judgment, "3");
    }
}
