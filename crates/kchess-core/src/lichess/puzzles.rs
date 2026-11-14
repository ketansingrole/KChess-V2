//! Lichess puzzles: the next puzzle, a
//! solved puzzle reported back, the daily puzzle, the account's dashboard and activity, and the
//! public Storm dashboard.
//!
//! Puzzles are read with `kchess_domain`. Requests that need a login run through
//! `Lichess::as_account`, so a refused login answers `Reply::NeedsReconnect`.

use std::collections::VecDeque;

use kchess_contracts::types::{
    Puzzle, PuzzleActivityEntry, PuzzleDraw, PuzzleGlicko, PuzzleRequest, PuzzleSolveRequest,
    PuzzleSolveResult, UsageKind,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::accounts::{Lichess, Reply, no_such_user, path_segment};
use super::client::Lane;
use super::decode::{Loose, present, read};
use kchess_contracts::error::{CoreError, ErrorKind, Result as CoreResult};
use kchess_domain::training;

#[derive(Deserialize)]
struct PuzzleId {
    #[serde(default)]
    id: Loose<String>,
}

/// An entry of a puzzle batch; only the puzzle's id is read here.
#[derive(Deserialize)]
struct BatchItem {
    #[serde(default)]
    puzzle: Loose<PuzzleId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SolveRound {
    #[serde(default)]
    id: Loose<String>,
    /// Passed on as Lichess sent it, null included.
    #[serde(default, deserialize_with = "present")]
    rating_diff: Option<Option<Loose<i64>>>,
}

/// Lichess's answer to solved puzzles.
#[derive(Deserialize)]
struct SolveAnswer {
    #[serde(default)]
    rounds: Loose<Vec<Loose<SolveRound>>>,
}

/// A line of the puzzle activity; the puzzle is read by `puzzle_from_api`.
#[derive(Default, Deserialize)]
struct ActivityEntry {
    #[serde(default, deserialize_with = "present")]
    puzzle: Option<Value>,
    #[serde(default)]
    date: Loose<i64>,
    #[serde(default)]
    win: Loose<bool>,
}

/// A Lichess puzzle as KChess reads it.
pub fn puzzle_from_api(raw: &Value) -> Option<Puzzle> {
    serde_json::from_value(training::puzzle::from_api(raw)?).ok()
}

fn readable_puzzle(raw: &Value) -> CoreResult<Puzzle> {
    puzzle_from_api(raw)
        .ok_or_else(|| CoreError::new("Lichess sent a puzzle KChess could not read."))
}

/// Puzzles asked for per draw. Lichess serves its next unplayed puzzles until a result reaches it,
/// so a puzzle left with Next (or drawn again before its result arrived) would come back first.
const DRAW_BATCH: usize = 10;
/// Puzzle ids remembered per account as recently drawn.
const RECENT_DRAWS: usize = 50;

/// The index in `ids` to show: the first not drawn recently, else the one drawn longest ago.
/// Records it in `recent` (oldest first).
fn pick_unseen(recent: &mut VecDeque<String>, ids: &[&str]) -> usize {
    let age = |id: &str| recent.iter().position(|seen| seen == id);
    let chosen = ids
        .iter()
        .position(|id| age(id).is_none())
        .or_else(|| (0..ids.len()).min_by_key(|&index| age(ids[index])))
        .unwrap_or(0);
    if let Some(id) = ids.get(chosen).filter(|id| !id.is_empty()) {
        recent.retain(|seen| seen != id);
        recent.push_back((*id).to_string());
        while recent.len() > RECENT_DRAWS {
            recent.pop_front();
        }
    }
    chosen
}

/// With an account, a puzzle chosen for its rating and one it has not seen;
/// without, a random puzzle.
///
/// Lichess picks an account's puzzles near its rating plus the difficulty's offset (Harder is +300)
/// and skips puzzles it has played, so a narrow theme such as one opening can run out at one
/// difficulty while others still have puzzles. That is an empty draw, not a failure: an empty
/// batch, or a 404 for the anonymous `/api/puzzle/next`.
pub async fn puzzle_next(
    lichess: &Lichess,
    request: &PuzzleRequest,
) -> CoreResult<Reply<PuzzleDraw>> {
    let mut query: Vec<(&str, String)> = Vec::new();
    query.push(("angle", request.angle.clone()));
    query.push(("difficulty", request.difficulty.as_str().to_string()));
    if let Some(color) = request.color {
        query.push(("color", kchess_domain::color::name(color).to_string()));
    }
    if request.account.is_empty() {
        let raw: CoreResult<Value> = lichess
            .client()
            .get("/api/puzzle/next", &lichess.anonymous(UsageKind::Puzzles))
            .query(&query)
            .read_json()
            .await;
        let puzzle = match raw {
            Err(error) if error.kind == Some(ErrorKind::NotFound) => None,
            raw => Some(readable_puzzle(&raw?)?),
        };
        return Ok(Reply::Done(PuzzleDraw {
            puzzle,
            glicko: None,
        }));
    }
    let batch_query: Vec<(&str, String)> = {
        let mut list = vec![("nb", DRAW_BATCH.to_string())];
        list.extend(query.iter().skip(1).cloned());
        list
    };
    lichess
        .as_account(&request.account, UsageKind::Puzzles, |auth| async move {
            let path = format!("/api/puzzle/batch/{}", path_segment(&request.angle));
            let batch: Value = lichess
                .client()
                .get(&path, &auth)
                .query(&batch_query)
                .read_json()
                .await?;
            let Some(items) = batch
                .get("puzzles")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
            else {
                return Ok(PuzzleDraw {
                    puzzle: None,
                    glicko: glicko(&batch),
                });
            };
            let ids: Vec<String> = items
                .iter()
                .map(|item| {
                    read::<BatchItem>(item)
                        .and_then(|item| item.puzzle.0)
                        .and_then(|puzzle| puzzle.id.0)
                        .unwrap_or_default()
                })
                .collect();
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            let chosen = {
                let mut drawn = lichess.drawn_puzzles();
                let recent = drawn.entry(request.account.to_lowercase()).or_default();
                pick_unseen(recent, &ids)
            };
            Ok(PuzzleDraw {
                puzzle: Some(readable_puzzle(items.get(chosen).unwrap_or(&Value::Null))?),
                glicko: glicko(&batch),
            })
        })
        .await
}

/// The account's puzzle rating in a batch, when Lichess sent one KChess can read.
fn glicko(batch: &Value) -> Option<PuzzleGlicko> {
    read(batch.get("glicko")?)
}

/// Tells Lichess how a puzzle went. A rated result moves the account's
/// Lichess puzzle rating; the rating change comes back.
pub async fn puzzle_solve(
    lichess: &Lichess,
    request: &PuzzleSolveRequest,
) -> CoreResult<Reply<PuzzleSolveResult>> {
    lichess
        .as_account(&request.account, UsageKind::Puzzles, |auth| async move {
            let path = format!("/api/puzzle/batch/{}", path_segment(&request.angle));
            let body = json!({
                "solutions": [{ "id": request.id, "win": request.win, "rated": request.rated }]
            });
            let result: Value = lichess
                .client()
                .post(&path, &auth)
                .json(&body)
                .read_json()
                .await?;
            let rounds = read::<SolveAnswer>(&result)
                .and_then(|answer| answer.rounds.0)
                .unwrap_or_default();
            let round = rounds
                .into_iter()
                .filter_map(|round| round.0)
                .find(|round| round.id.0.as_deref() == Some(request.id.as_str()));
            Ok(PuzzleSolveResult {
                rating_diff: if request.rated {
                    round
                        .and_then(|round| round.rating_diff)
                        .map(|diff| diff.and_then(|diff| diff.0))
                } else {
                    None
                },
            })
        })
        .await
}

/// Today's puzzle, with no login needed.
pub async fn puzzle_daily(lichess: &Lichess) -> CoreResult<Puzzle> {
    let raw: Value = lichess
        .client()
        .get("/api/puzzle/daily", &lichess.anonymous(UsageKind::Puzzles))
        .read_json()
        .await?;
    readable_puzzle(&raw)
}

/// One puzzle, as a lichess.org/training link names it; no login needed.
pub async fn puzzle_by_id(lichess: &Lichess, id: &str) -> CoreResult<Puzzle> {
    let path = format!("/api/puzzle/{}", path_segment(id));
    let raw: Value = lichess
        .client()
        .get(&path, &lichess.anonymous(UsageKind::Puzzles))
        .read_json()
        .await?;
    readable_puzzle(&raw)
}

/// The account's results over the last `days` days, as Lichess
/// reports them.
pub async fn puzzle_dashboard(
    lichess: &Lichess,
    account: &str,
    days: u32,
) -> CoreResult<Reply<Value>> {
    lichess
        .as_account(account, UsageKind::Puzzles, |auth| async move {
            let path = format!("/api/puzzle/dashboard/{days}");
            lichess.client().get(&path, &auth).read_json().await
        })
        .await
}

/// The account's most recent puzzle results, newest first.
/// Entries whose puzzle KChess cannot read are left out.
pub async fn puzzle_activity(
    lichess: &Lichess,
    account: &str,
    max: u32,
) -> CoreResult<Reply<Vec<PuzzleActivityEntry>>> {
    lichess
        .as_account(account, UsageKind::Puzzles, |auth| async move {
            let lines = lichess
                .client()
                .get("/api/puzzle/activity", &auth)
                .query(&[("max", max.to_string())])
                .lane(Lane::Feed)
                .records("puzzle", usize::MAX, Some::<Value>)
                .await?;
            let mut entries = Vec::new();
            for raw in &lines {
                let activity: ActivityEntry = read(raw).unwrap_or_default();
                let mut puzzle = activity.puzzle.unwrap_or_else(|| json!({}));
                if let Some(object) = puzzle.as_object_mut() {
                    object.insert("initialPly".into(), json!(0));
                }
                let read = puzzle_from_api(&json!({ "game": {}, "puzzle": puzzle }));
                if let Some(read) = read {
                    entries.push(PuzzleActivityEntry {
                        date: activity.date.0,
                        win: activity.win.0.unwrap_or(false),
                        puzzle: read,
                    });
                }
            }
            Ok(entries)
        })
        .await
}

/// Anyone's public Storm results; needs no login.
pub async fn storm_dashboard(lichess: &Lichess, username: &str, days: u32) -> CoreResult<Value> {
    let path = format!("/api/storm/dashboard/{}", path_segment(username));
    lichess
        .client()
        .get(&path, &lichess.anonymous(UsageKind::Puzzles))
        .query(&[("days", days.to_string())])
        .read_json()
        .await
        .map_err(|error| no_such_user(username, error))
}
