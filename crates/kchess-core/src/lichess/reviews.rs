//! Lichess's own analysis of a game, as a review (`reviewFromLichess`, `fetchLichessReviews` and
//! the review writes of the game sync).
//!
//! Reviews are JSON values shaped as `StoredReview`. Writing them
//! goes through `LichessStore::save_reviews` and `mark_checked`, so the review store stays behind
//! the storage trait the wiring provides.

use serde::Deserialize;
use serde_json::{Map, Number, Value, json};
use tokio_util::sync::CancellationToken;

use super::accounts::{Lichess, now_ms};
use super::client::{Failure, LichessClient};
use super::decode::{Loose, read};
use crate::error::{CoreError, Result as CoreResult};
use kchess_domain::{js, records, replay, review as domain_review};

/// The standard start, which Lichess scores as +0.15 when it works out accuracy.
const INITIAL_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
/// `fetchLichessReviews` asks about at most this many games at once.
pub const MAX_REVIEW_IDS: usize = 300;

/// One entry of Lichess's analysis: each field read when it has the right type.
#[derive(Default, Deserialize)]
struct RawAnalysis {
    #[serde(default)]
    mate: Loose<Number>,
    #[serde(default)]
    eval: Loose<Number>,
    #[serde(default)]
    best: Loose<String>,
    #[serde(default)]
    variation: Loose<String>,
    #[serde(default)]
    judgment: Loose<RawJudgment>,
}

#[derive(Deserialize)]
struct RawJudgment {
    #[serde(default)]
    name: Loose<String>,
}

#[derive(Deserialize)]
struct RawAccuracy {
    #[serde(default)]
    accuracy: Loose<Number>,
}

#[derive(Deserialize)]
struct RawPlayer {
    #[serde(default)]
    analysis: Loose<RawAccuracy>,
}

#[derive(Default, Deserialize)]
struct RawPlayers {
    #[serde(default)]
    white: Loose<RawPlayer>,
    #[serde(default)]
    black: Loose<RawPlayer>,
}

/// A game export with its analysis.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGame {
    #[serde(default)]
    status: Option<Value>,
    #[serde(default)]
    analysis: Loose<Vec<Loose<RawAnalysis>>>,
    #[serde(default)]
    variant: Loose<String>,
    #[serde(default)]
    moves: Loose<String>,
    #[serde(default)]
    initial_fen: Loose<String>,
    #[serde(default)]
    players: Loose<RawPlayers>,
    #[serde(default)]
    id: Loose<String>,
}

/// `reviewFromLichess(raw)`: Lichess's computer analysis of a game, as a review (scores from
/// White's side, its labels and accuracy). None when the game is unfinished, has no analysis,
/// or is a variant reviews cannot replay.
pub fn review_from_lichess(raw: &Value) -> Option<Value> {
    let game: RawGame = read(raw)?;
    let status = game.status.unwrap_or(Value::Null);
    let in_progress = records::call("isGameInProgress", &[status])
        .and_then(Result::ok)
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    if in_progress {
        return None;
    }
    let analysis = game.analysis.0?;
    if analysis.is_empty() {
        return None;
    }
    let variant = game.variant.0.as_deref();
    if variant != Some("standard") && variant != Some("fromPosition") {
        return None;
    }
    let moves_text = game.moves.0.as_deref().unwrap_or("");
    let initial = game.initial_fen.0.as_deref();
    let (fen, moves) = domain_review::lichess_line(moves_text, None, initial);
    if moves.is_empty() {
        return None;
    }
    let positions = replay::replay_positions(&fen, &moves);
    let mut evals: Vec<Option<Map<String, Value>>> = vec![None; moves.len() + 1];
    if fen == INITIAL_FEN {
        evals[0] = Some(Map::from_iter([("cp".to_string(), json!(15))]));
    }
    let mut judgments: Vec<Value> = vec![Value::Null; moves.len()];
    for (i, entry) in analysis.into_iter().take(moves.len()).enumerate() {
        // Entry i scores the position after move i+1; its `best` is what should have been played.
        // Only numbers are scores: a review is stored as Lichess sent it.
        let entry = entry.0.unwrap_or_default();
        let score = match (entry.mate.0, entry.eval.0) {
            (Some(mate), _) => Some(("mate", Value::Number(mate))),
            (None, Some(eval)) => Some(("cp", Value::Number(eval))),
            (None, None) => None,
        };
        if let Some((key, value)) = score {
            evals[i + 1]
                .get_or_insert_with(Map::new)
                .insert(key.to_string(), value);
        }
        if let Some(best) = entry.best.0.as_deref().filter(|best| !best.is_empty()) {
            let variation = entry.variation.0.as_deref().filter(|text| !text.is_empty());
            let pv: Vec<String> = match variation {
                Some(text) => {
                    let start = positions.get(i).map_or(fen.as_str(), |p| p.fen.as_str());
                    domain_review::san_to_uci(start, &split_whitespace(text))
                }
                None => Vec::new(),
            };
            let pv = if pv.is_empty() {
                vec![best.to_string()]
            } else {
                pv
            };
            let slot = evals[i].get_or_insert_with(Map::new);
            slot.insert("best".to_string(), json!(best));
            slot.insert("pv".to_string(), json!(pv));
        }
        if let Some(name) = entry
            .judgment
            .0
            .and_then(|judgment| judgment.name.0)
            .map(|name| name.to_lowercase())
            && matches!(name.as_str(), "inaccuracy" | "mistake" | "blunder")
        {
            judgments[i] = json!(name);
        }
    }
    let mut accuracy = Map::new();
    let players = game.players.0.unwrap_or_default();
    for (color, player) in [("white", players.white), ("black", players.black)] {
        let figure = player
            .0
            .and_then(|player| player.analysis.0)
            .and_then(|analysis| analysis.accuracy.0);
        if let Some(figure) = figure {
            accuracy.insert(color.to_string(), Value::Number(figure));
        }
    }
    let key = records::call("reviewKey", &[json!(fen), json!(moves)])
        .and_then(Result::ok)
        .and_then(|value| value.as_str().map(str::to_string))?;
    let evals: Vec<Value> = evals
        .into_iter()
        .map(|slot| slot.map_or(Value::Null, Value::Object))
        .collect();
    Some(json!({
        "key": key,
        "fen": fen,
        "moves": moves,
        "source": "lichess",
        "evals": evals,
        "judgments": judgments,
        "accuracy": accuracy,
        "depth": 0,
        "complete": true,
        "updatedAt": now_ms(),
        "gameId": game.id.0?,
    }))
}

/// `fetchLichessReviews(account, ids)`: asks Lichess for its analysis of games already synced (up
/// to 300 per request), saves each one it has, and remembers every game asked about. Nothing is
/// written when the account's sync was cancelled meanwhile. Returns the stored reviews.
pub async fn fetch_lichess_reviews(
    lichess: &Lichess,
    account: &str,
    ids: &[String],
) -> CoreResult<Vec<Value>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let epoch = lichess.account_epoch(account);
    let batch: Vec<String> = ids.iter().take(MAX_REVIEW_IDS).cloned().collect();
    let body = batch.join(",");
    let query = [
        ("moves", "true".to_string()),
        ("evals", "true".to_string()),
        ("accuracy", "true".to_string()),
    ];
    let cancel = lichess.lifetime().clone();
    let lines = lichess
        .client()
        .policy()
        .with_usage(account, "games", async {
            lichess
                .as_owner(account, |auth| {
                    let body = body.clone();
                    let cancel = cancel.clone();
                    let query = query.clone();
                    async move {
                        collect_lines(
                            lichess.client(),
                            reqwest::Method::POST,
                            "/api/games/export/_ids",
                            &query,
                            Some(body),
                            auth.as_deref(),
                            &cancel,
                        )
                        .await
                    }
                })
                .await
        })
        .await
        .map_err(CoreError::from)?;
    let mut parsed = Vec::new();
    for line in &lines {
        let raw = parse_record(line)?;
        if let Some(review) = review_from_lichess(&raw) {
            parsed.push(review);
        }
    }
    // Nothing is written until the download is over: a logout meanwhile keeps it all off disk.
    if epoch != lichess.account_epoch(account) {
        return Err(CoreError::new("Sync was cancelled."));
    }
    let current = || lichess.account_epoch(account) == epoch;
    let found = lichess
        .store()
        .save_reviews(account, &parsed, &current)
        .await?;
    lichess.store().mark_checked(&batch, &current).await?;
    Ok(found)
}

/// Parses one game record of an NDJSON export.
pub(crate) fn parse_record(line: &str) -> CoreResult<Value> {
    serde_json::from_str(line)
        .map_err(|cause| CoreError::new(format!("Lichess sent an unreadable game record: {cause}")))
}

/// The lines of an NDJSON response, read to the end (the request is admitted by the policy).
pub(crate) async fn collect_lines(
    client: &LichessClient,
    method: reqwest::Method,
    path: &str,
    query: &[(&str, String)],
    body: Option<String>,
    auth: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Vec<String>, Failure> {
    let mut lines = Vec::new();
    client
        .ndjson(method, path, query, body, auth, cancel, |line| {
            lines.push(line.to_string());
            Ok(())
        })
        .await?;
    Ok(lines)
}

/// `split(/\s+/)` of JavaScript: runs of whitespace separate words, and a leading or trailing run
/// leaves an empty word, as the regular expression does.
pub(crate) fn split_whitespace(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut run: Option<usize> = None;
    for (index, c) in text.char_indices() {
        if js::is_space(c) {
            run.get_or_insert(index);
        } else if let Some(run_start) = run.take() {
            parts.push(&text[start..run_start]);
            start = index;
        }
    }
    match run {
        Some(run_start) => {
            parts.push(&text[start..run_start]);
            parts.push("");
        }
        None => parts.push(&text[start..]),
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_javascript_regular_expressions() {
        assert_eq!(split_whitespace("a  b"), ["a", "b"]);
        assert_eq!(split_whitespace(" a"), ["", "a"]);
        assert_eq!(split_whitespace("a "), ["a", ""]);
        assert_eq!(split_whitespace(""), [""]);
    }
}
