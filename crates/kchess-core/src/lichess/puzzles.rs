//! Lichess puzzles (the puzzle endpoints of `core/src/services/lichess.ts`): the next puzzle, a
//! solved puzzle reported back, the daily puzzle, the account's dashboard and activity, and the
//! public Storm dashboard.
//!
//! Puzzles are read with `kchess_domain` (`puzzleFromApi`). Requests that need a login run through
//! `Lichess::as_account`, so a refused login answers `Reply::NeedsReconnect`.

use std::time::Duration;

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::accounts::{Lichess, Reply, path_segment};
use super::client::{Failure, LichessError, authorize, lichess_error};
use super::policy::Admitted;
use super::reviews::collect_lines;
use crate::error::{CoreError, Result as CoreResult};
use kchess_domain::training;

/// The request for a puzzle (`PuzzleRequest`). An empty account trains anonymously.
#[derive(Clone, Debug, PartialEq)]
pub struct PuzzleRequest {
    pub account: String,
    pub angle: String,
    pub difficulty: String,
    pub color: Option<String>,
}

/// A puzzle drawn for the user (`PuzzleDraw`): the puzzle and, for an account, its rating.
#[derive(Clone, Debug, PartialEq)]
pub struct PuzzleDraw {
    pub puzzle: Value,
    pub glicko: Option<Value>,
}

/// A puzzle result to report (`PuzzleSolveRequest`).
#[derive(Clone, Debug, PartialEq)]
pub struct PuzzleSolveRequest {
    pub account: String,
    pub angle: String,
    pub id: String,
    pub win: bool,
    pub rated: bool,
}

/// The rating change Lichess applied to a rated result (`PuzzleSolveResult`).
#[derive(Clone, Debug, PartialEq)]
pub struct PuzzleSolveResult {
    pub rating_diff: Option<Value>,
}

/// One result of the activity feed (`PuzzleActivityEntry`).
#[derive(Clone, Debug, PartialEq)]
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

/// `puzzleNext(request)`: with an account, a puzzle chosen for its rating and one it has not seen;
/// without, a random puzzle.
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
        let raw: Value = lichess
            .client()
            .policy()
            .with_usage("", "puzzles", async {
                lichess
                    .client()
                    .get_json("/api/puzzle/next", &query, None, &cancel)
                    .await
            })
            .await?;
        return Ok(Reply::Done(PuzzleDraw {
            puzzle: readable_puzzle(&raw)?,
            glicko: None,
        }));
    }
    let batch_query: Vec<(&str, String)> = {
        let mut list = vec![("nb", "1".to_string())];
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
            let first = batch
                .get("puzzles")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .ok_or_else(|| {
                    Failure::Core(CoreError::new(
                        "Lichess has no more puzzles for this theme and difficulty.",
                    ))
                })?;
            Ok(PuzzleDraw {
                puzzle: readable_puzzle(first)?,
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
            let round = result
                .get("rounds")
                .and_then(Value::as_array)
                .and_then(|rounds| {
                    rounds
                        .iter()
                        .find(|round| round.get("id").and_then(Value::as_str) == Some(&request.id))
                });
            Ok(PuzzleSolveResult {
                rating_diff: if request.rated {
                    round.and_then(|round| round.get("ratingDiff")).cloned()
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
                let mut puzzle = raw.get("puzzle").cloned().unwrap_or_else(|| json!({}));
                if let Some(object) = puzzle.as_object_mut() {
                    object.insert("initialPly".into(), json!(0));
                }
                let read = puzzle_from_api(&json!({ "game": {}, "puzzle": puzzle }));
                if let Some(read) = read {
                    entries.push(PuzzleActivityEntry {
                        date: raw.get("date").cloned().unwrap_or(Value::Null),
                        win: raw.get("win").and_then(Value::as_bool).unwrap_or(false),
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
    let text = match tokio::time::timeout(timeout, response.text()).await {
        Ok(Ok(text)) => text,
        Ok(Err(cause)) => {
            drop(hold);
            return Err(Failure::Core(CoreError::new(cause.to_string())));
        }
        Err(_) => {
            drop(hold);
            return Err(Failure::Core(CoreError::new(
                "Lichess request timed out. Retry or reconnect.",
            )));
        }
    };
    drop(hold);
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
