//! Lichess puzzles (the puzzle endpoints of `crates/kchess-node/js/lichess.ts`): the next puzzle, a
//! solved puzzle reported back, the daily puzzle, the account's dashboard and activity, and the
//! public Storm dashboard.
//!
//! Puzzles are read with `kchess_domain` (`puzzleFromApi`). Requests that need a login run through
//! `Lichess::as_account`, so a refused login answers `Reply::NeedsReconnect`.

use std::collections::VecDeque;
use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::accounts::{Lichess, Reply, path_segment};
use super::client::{Failure, LichessError, authorize, lichess_error, read_with_deadline};
use super::decode::{Loose, present, read};
use super::policy::Admitted;
use super::reviews::collect_lines;
use crate::error::{CoreError, Result as CoreResult};
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
    rating_diff: Option<Value>,
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
    date: Option<Value>,
    #[serde(default)]
    win: Loose<bool>,
}

/// The request for a puzzle (`PuzzleRequest`). An empty account trains anonymously.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleRequest {
    pub account: String,
    pub angle: String,
    pub difficulty: String,
    pub color: Option<String>,
}

/// A puzzle drawn for the user (`PuzzleDraw`): the puzzle and, for an account, its rating. The
/// puzzle is null when Lichess has none left for the theme and difficulty.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleDraw {
    #[serde(skip_serializing_if = "Value::is_null")]
    pub puzzle: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glicko: Option<Value>,
}

/// A puzzle result to report (`PuzzleSolveRequest`).
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleSolveRequest {
    pub account: String,
    pub angle: String,
    pub id: String,
    pub win: bool,
    pub rated: bool,
}

/// The rating change Lichess applied to a rated result (`PuzzleSolveResult`).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleSolveResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating_diff: Option<Value>,
}

/// One result of the activity feed (`PuzzleActivityEntry`).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PuzzleActivityEntry {
    pub date: Value,
    pub win: bool,
    pub puzzle: Value,
}

/// `puzzleFromApi(raw)`: a Lichess puzzle as KChess reads it.
pub fn puzzle_from_api(raw: &Value) -> Option<Value> {
    let result = training::call("puzzleFromApi", std::slice::from_ref(raw))?.ok()?;
    if result.is_null() { None } else { Some(result) }
}

fn readable_puzzle(raw: &Value) -> CoreResult<Value> {
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

/// `puzzleNext(request)`: with an account, a puzzle chosen for its rating and one it has not seen;
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
    let cancel = lichess.lifetime().clone();
    let mut query: Vec<(&str, String)> = Vec::new();
    query.push(("angle", request.angle.clone()));
    query.push(("difficulty", request.difficulty.clone()));
    if let Some(color) = &request.color {
        query.push(("color", color.clone()));
    }
    if request.account.is_empty() {
        let raw: Result<Value, Failure> = lichess
            .client()
            .policy()
            .with_usage("", "puzzles", async {
                lichess
                    .client()
                    .get_json("/api/puzzle/next", &query, None, &cancel)
                    .await
            })
            .await;
        let puzzle = match raw {
            Err(Failure::Lichess(error)) if error.status == 404 => Value::Null,
            raw => readable_puzzle(&raw?)?,
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
        .as_account(&request.account, "puzzles", |token| async move {
            let path = format!("/api/puzzle/batch/{}", path_segment(&request.angle));
            let batch: Value = lichess
                .client()
                .get_json(&path, &batch_query, Some(&authorize(&token)), &cancel)
                .await?;
            let Some(items) = batch
                .get("puzzles")
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
            else {
                return Ok(PuzzleDraw {
                    puzzle: Value::Null,
                    glicko: batch.get("glicko").cloned(),
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
                puzzle: readable_puzzle(items.get(chosen).unwrap_or(&Value::Null))?,
                glicko: batch.get("glicko").cloned(),
            })
        })
        .await
}

/// `puzzleSolve(request)`: tells Lichess how a puzzle went. A rated result moves the account's
/// Lichess puzzle rating; the rating change comes back.
pub async fn puzzle_solve(
    lichess: &Lichess,
    request: &PuzzleSolveRequest,
) -> CoreResult<Reply<PuzzleSolveResult>> {
    let cancel = lichess.lifetime().clone();
    lichess
        .as_account(&request.account, "puzzles", |token| async move {
            let path = format!("/api/puzzle/batch/{}", path_segment(&request.angle));
            let body = json!({
                "solutions": [{ "id": request.id, "win": request.win, "rated": request.rated }]
            });
            let result = post_json(lichess, &path, &body, &authorize(&token), &cancel).await?;
            let rounds = read::<SolveAnswer>(&result)
                .and_then(|answer| answer.rounds.0)
                .unwrap_or_default();
            let round = rounds
                .into_iter()
                .filter_map(|round| round.0)
                .find(|round| round.id.0.as_deref() == Some(request.id.as_str()));
            Ok(PuzzleSolveResult {
                rating_diff: if request.rated {
                    round.and_then(|round| round.rating_diff)
                } else {
                    None
                },
            })
        })
        .await
}

/// `puzzleDaily()`: today's puzzle, with no login needed.
pub async fn puzzle_daily(lichess: &Lichess) -> CoreResult<Value> {
    let cancel = lichess.lifetime().clone();
    let raw: Value = lichess
        .client()
        .policy()
        .with_usage("", "puzzles", async {
            lichess
                .client()
                .get_json("/api/puzzle/daily", &[], None, &cancel)
                .await
        })
        .await?;
    readable_puzzle(&raw)
}

/// `puzzleDashboard(account, days)`: the account's results over the last `days` days, as Lichess
/// reports them.
pub async fn puzzle_dashboard(
    lichess: &Lichess,
    account: &str,
    days: u32,
) -> CoreResult<Reply<Value>> {
    let cancel = lichess.lifetime().clone();
    lichess
        .as_account(account, "puzzles", |token| async move {
            let path = format!("/api/puzzle/dashboard/{days}");
            lichess
                .client()
                .get_json(&path, &[], Some(&authorize(&token)), &cancel)
                .await
        })
        .await
}

/// `puzzleActivity(account, max)`: the account's most recent puzzle results, newest first.
/// Entries whose puzzle KChess cannot read are left out.
pub async fn puzzle_activity(
    lichess: &Lichess,
    account: &str,
    max: u32,
) -> CoreResult<Reply<Vec<PuzzleActivityEntry>>> {
    let cancel = lichess.lifetime().clone();
    lichess
        .as_account(account, "puzzles", |token| async move {
            let auth = authorize(&token);
            let lines = collect_lines(
                lichess.client(),
                reqwest::Method::GET,
                "/api/puzzle/activity",
                &[("max", max.to_string())],
                None,
                Some(auth.as_str()),
                &cancel,
            )
            .await?;
            let mut entries = Vec::new();
            for line in &lines {
                let raw: Value = serde_json::from_str(line).map_err(|cause| {
                    Failure::Core(CoreError::new(format!(
                        "Lichess sent an unreadable puzzle record: {cause}"
                    )))
                })?;
                let activity: ActivityEntry = read(&raw).unwrap_or_default();
                let mut puzzle = activity.puzzle.unwrap_or_else(|| json!({}));
                if let Some(object) = puzzle.as_object_mut() {
                    object.insert("initialPly".into(), json!(0));
                }
                let read = puzzle_from_api(&json!({ "game": {}, "puzzle": puzzle }));
                if let Some(read) = read {
                    entries.push(PuzzleActivityEntry {
                        date: activity.date.unwrap_or(Value::Null),
                        win: activity.win.0.unwrap_or(false),
                        puzzle: read,
                    });
                }
            }
            Ok(entries)
        })
        .await
}

/// `stormDashboard(username, days)`: anyone's public Storm results; needs no login.
pub async fn storm_dashboard(lichess: &Lichess, username: &str, days: u32) -> CoreResult<Value> {
    let cancel = lichess.lifetime().clone();
    let path = format!("/api/storm/dashboard/{}", path_segment(username));
    let result = lichess
        .client()
        .policy()
        .with_usage("", "puzzles", async {
            lichess
                .client()
                .get_json::<Value>(&path, &[("days", days.to_string())], None, &cancel)
                .await
        })
        .await;
    match result {
        Ok(value) => Ok(value),
        Err(Failure::Lichess(error)) if error.status == 404 => Err(CoreError::new(format!(
            "Lichess 404 from {}: no such Lichess user \"{username}\"",
            error.endpoint
        ))),
        Err(failure) => Err(failure.into()),
    }
}

/// A JSON `POST` admitted through the policy (the client only sends form bodies), read within the
/// request deadline. A non-2xx answer is the same `LichessError` a `GET` gives.
async fn post_json(
    lichess: &Lichess,
    path: &str,
    body: &Value,
    auth: &str,
    cancel: &CancellationToken,
) -> Result<Value, Failure> {
    let url = reqwest::Url::parse(&format!("{}{path}", lichess.client().base()))
        .map_err(|cause| Failure::Core(CoreError::new(format!("Invalid Lichess URL: {cause}"))))?;
    let mut request = reqwest::Request::new(reqwest::Method::POST, url);
    let bad_header = |cause: reqwest::header::InvalidHeaderValue| {
        Failure::Core(CoreError::new(format!("Invalid Lichess request: {cause}")))
    };
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(auth).map_err(bad_header)?,
    );
    request
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    request
        .headers_mut()
        .insert(ACCEPT, HeaderValue::from_static("application/json"));
    *request.body_mut() = Some(serde_json::to_vec(body).unwrap_or_default().into());
    let Admitted { response, hold } = lichess.client().policy().send(request, cancel).await?;
    let status = response.status().as_u16();
    let timeout: Duration = lichess.client().policy().timeout();
    let text = read_with_deadline(response, cancel, false, timeout, None).await;
    drop(hold);
    let text = text?;
    let endpoint = format!("POST {path}");
    if !(200..300).contains(&status) {
        let detail = if text.is_empty() {
            None
        } else {
            Some(serde_json::from_str(&text).unwrap_or(Value::String(text.clone())))
        };
        return Err(Failure::Lichess(lichess_error(status, detail, &endpoint)));
    }
    serde_json::from_str(&text).map_err(|cause| {
        Failure::Lichess(LichessError {
            status,
            endpoint,
            detail: format!("unreadable response: {cause}"),
        })
    })
}
