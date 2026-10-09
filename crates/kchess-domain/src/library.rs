//! Library document decoders (`core/src/domain/library.ts`): semantic validation before a
//! document reaches the library or a board. Each accepts and rejects exactly what the
//! TypeScript decoder does and produces the same normalized value.

use serde_json::{Map, Value, json};

use crate::js::{self, finite, integer, string, text};
use crate::replay;
use crate::rules;
use crate::tree;

pub const MAX_STUDIES: usize = 50;
pub const MAX_CHAPTERS: usize = 64;
pub const MAX_MISTAKES: usize = 500;

const VARIANTS: [&str; 8] = [
    "standard",
    "chess960",
    "kingOfTheHill",
    "threeCheck",
    "antichess",
    "atomic",
    "horde",
    "racingKings",
];

fn is_variant(v: Option<&Value>) -> Option<&str> {
    string(v).filter(|s| VARIANTS.contains(s))
}

fn object(v: Option<&Value>) -> Option<&Map<String, Value>> {
    match v {
        Some(Value::Object(map)) => Some(map),
        _ => None,
    }
}

fn number(n: f64) -> Value {
    serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
}

/* ── Studies ── */

fn decode_chapters(raw: Option<&Value>, fallback: Value) -> Option<Vec<Value>> {
    let owned;
    let chapters = match raw {
        None | Some(Value::Null) => {
            owned = Value::Array(vec![fallback]);
            &owned
        }
        Some(value) => value,
    };
    let Value::Array(chapters) = chapters else {
        return None;
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
            tree::valid_pgn(pgn).then(|| json!({ "id": id, "name": name, "pgn": pgn }))
        })
        .collect()
}

fn cloud_id(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// `decodeStudy`
pub fn decode_study(raw: &Value) -> Option<Value> {
    let item = object(Some(raw))?;
    let id = string(item.get("id"))?;
    let name = string(item.get("name"))?;
    let pgn = string(item.get("pgn"))?;
    let updated_at = finite(item.get("updatedAt"))?;
    let chapters = decode_chapters(
        item.get("chapters"),
        json!({ "id": id, "name": name, "pgn": pgn }),
    )?;
    let mut study = Map::new();
    study.insert("id".into(), id.into());
    study.insert("name".into(), name.into());
    study.insert("pgn".into(), chapters[0]["pgn"].clone());
    study.insert("chapters".into(), Value::Array(chapters));
    study.insert("updatedAt".into(), number(updated_at));
    if !js::falsy(item.get("cloud"))
        && let Some(cloud) = object(item.get("cloud"))
        && let (Some(account), Some(cid), Some(downloaded)) = (
            string(cloud.get("account")),
            string(cloud.get("id")),
            string(cloud.get("downloadedPgn")),
        )
        && cloud_id(cid)
    {
        let mut value = Map::new();
        value.insert("account".into(), account.into());
        value.insert("id".into(), cid.into());
        value.insert("downloadedPgn".into(), downloaded.into());
        if cloud.get("structureChanged") == Some(&Value::Bool(true)) {
            value.insert("structureChanged".into(), true.into());
        }
        study.insert("cloud".into(), Value::Object(value));
    }
    Some(Value::Object(study))
}

/// `decodeStudies`: invalid studies are left out rather than failing the whole library.
pub fn decode_studies(raw: &Value) -> Option<Vec<Value>> {
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

/// `decodeArchivedGame`
pub fn decode_archived_game(raw: &Value) -> Option<Value> {
    let g = object(Some(raw))?;
    let id = text(g.get("id"), 80)?;
    let source = string(g.get("source")).filter(|s| ["computer", "board", "clock"].contains(s))?;
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
    let mut game = Map::new();
    game.insert("id".into(), id.into());
    game.insert("source".into(), source.into());
    game.insert("startedAt".into(), number(started_at));
    game.insert("updatedAt".into(), number(updated_at));
    game.insert("white".into(), white.into());
    game.insert("black".into(), black.into());
    game.insert("result".into(), result.into());
    game.insert("reason".into(), reason.into());
    game.insert("finished".into(), (*finished).into());
    game.insert("setup".into(), json!({ "variant": variant, "fen": fen }));
    game.insert("moves".into(), moves.into());
    game.insert("timeControl".into(), time_control.into());
    if let Some(summary) = clock_summary {
        game.insert("clockSummary".into(), summary.into());
    }
    Some(Value::Object(game))
}

pub const MAX_ARCHIVED_GAMES: usize = 500;

/// `decodeArchive`: played games as one document (earlier releases); any invalid or duplicated
/// game rejects the whole document.
pub fn decode_archive(raw: &Value) -> Option<Vec<Value>> {
    let doc = object(Some(raw))?;
    if finite(doc.get("version")) != Some(1.0) {
        return None;
    }
    let Some(Value::Array(games)) = doc.get("games") else {
        return None;
    };
    if games.len() > MAX_ARCHIVED_GAMES {
        return None;
    }
    let mut ids = std::collections::HashSet::new();
    games
        .iter()
        .map(|raw| {
            let game = decode_archived_game(raw)?;
            ids.insert(game["id"].as_str()?.to_string()).then_some(game)
        })
        .collect()
}

/* ── Mistake drills ── */

/// `decodeMistakes`
pub fn decode_mistakes(raw: &Value) -> Option<Vec<Value>> {
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
                if replay::replay_positions_count(fen, &solution) != solution.len() + 1 {
                    return None;
                }
                let due_at = finite(item.get("dueAt"))?;
                let streak = integer(item.get("streak"))?;
                let attempts = integer(item.get("attempts"))?;
                Some(json!({
                    "id": id,
                    "fen": fen,
                    "solution": solution,
                    "judgment": js::to_string(item.get("judgment")),
                    "dueAt": number(due_at),
                    "streak": number(streak),
                    "attempts": number(attempts),
                }))
            })
            .collect(),
    )
}

/// Whether a variant key is one KChess can play.
pub fn known_variant(key: &str) -> bool {
    rules::lichess_variant(key).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

    fn game(moves: &[&str]) -> Value {
        json!({
            "id": "g1", "source": "board", "startedAt": 1, "updatedAt": 2,
            "white": "A", "black": "B", "result": "*", "reason": "", "finished": false,
            "setup": { "variant": "standard", "fen": START }, "moves": moves, "timeControl": "-",
        })
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
        assert_eq!(studies[0]["cloud"]["structureChanged"], json!(true));
        assert_eq!(studies[0]["chapters"][0]["id"], json!("s"));
    }

    #[test]
    fn mistakes_need_a_playable_solution() {
        let doc = json!({ "version": 1, "items": [
            { "id": "m", "fen": START, "solution": ["e2e4"], "judgment": 3, "dueAt": 0, "streak": 0, "attempts": 1.0 },
            { "id": "x", "fen": START, "solution": ["e2e5"], "dueAt": 0, "streak": 0, "attempts": 0 },
        ]});
        let items = decode_mistakes(&doc).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["judgment"], json!("3"));
    }
}
