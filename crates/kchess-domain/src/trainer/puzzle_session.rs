//! `core/src/domain/puzzleSession.ts`: the training session's transitions.
//!
//! The TypeScript class keeps its reactive state, runs the host call each transition returns
//! (`call`), and reports the settled reply back with the `resume` token it was given. Every
//! decision is made here: a reply counts only while its load generation, session version,
//! attempt and selection are still current, which is how late replies are ignored.
//!
//! Transitions take `{ state, internal, … }` and return `{ state, internal, call, warn }`.
//! `internal` holds what the class kept privately (load generation, session version, attempt).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::Out;

const OFFLINE_TEXT: &str = "You’re offline. Choose Downloaded to use puzzles on this device, or reconnect for online puzzles.";
const NO_PUZZLE_TEXT: &str = "No puzzle of that theme and difficulty in the local database.";
const REJECTED_TEXT: &str = "Lichess did not accept the result: reconnect your account.";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Selection {
    account: String,
    angle: String,
    difficulty: String,
    color: String,
    mode: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tally {
    solved: f64,
    failed: f64,
    rating_change: f64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    win: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rating_diff: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failed: Option<String>,
}

/// The observable state (`PuzzleSessionState` in TypeScript).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    phase: String,
    #[serde(default)]
    current: Option<Value>,
    puzzle_key: u64,
    is_retry: bool,
    train_error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rating: Option<f64>,
    session: Tally,
    #[serde(default)]
    last_report: Option<Report>,
    #[serde(default)]
    attempt_outcome: Option<bool>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attempt {
    key: u64,
    account: String,
    angle: String,
    mode: String,
}

/// What the class kept private: replies are matched against these counters.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Internal {
    load_gen: u64,
    session_version: u64,
    attempt: Option<Attempt>,
}

/// Carried by a host call and handed back with its reply.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Resume {
    /// `offline` (local puzzle), `draw` (Lichess puzzle), `anonymous` (practice retry), `solve`.
    stage: String,
    generation: u64,
    #[serde(default)]
    version: u64,
    #[serde(default)]
    key: u64,
    #[serde(default)]
    win: bool,
    #[serde(default)]
    puzzle_id: Value,
    selection: Selection,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Call {
    method: &'static str,
    args: Value,
    resume: Resume,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    state: State,
    internal: Internal,
    call: Option<Call>,
    warn: bool,
}

struct Session {
    state: State,
    internal: Internal,
    call: Option<Call>,
    warn: bool,
}

/// `OFFLINE_OFFSET` by difficulty; an unknown difficulty has no offset (NaN, as in TypeScript).
fn offset(difficulty: &str) -> f64 {
    match difficulty {
        "easiest" => -600.0,
        "easier" => -300.0,
        "normal" => 0.0,
        "harder" => 300.0,
        "hardest" => 600.0,
        _ => f64::NAN,
    }
}

/// `Math.round`: halves go up, towards positive infinity.
fn js_round(x: f64) -> f64 {
    let floor = x.floor();
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// `isReconnect`: a non-array object with a `needsReconnect` property.
pub(super) fn is_reconnect(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.contains_key("needsReconnect"))
}

fn initial_state() -> State {
    State {
        phase: "idle".into(),
        current: None,
        puzzle_key: 0,
        is_retry: false,
        train_error: String::new(),
        rating: None,
        session: Tally::default(),
        last_report: None,
        attempt_outcome: None,
    }
}

fn syncs_to_lichess(state: &State, selection: &Selection) -> bool {
    selection.mode == "rated" && !state.is_retry && !selection.account.is_empty()
}

/// The arguments of `puzzleNext`: the colour is left out when it is random.
fn draw_args(account: &str, selection: &Selection) -> Value {
    let mut args = Map::new();
    args.insert("account".into(), json!(account));
    args.insert("angle".into(), json!(selection.angle));
    args.insert("difficulty".into(), json!(selection.difficulty));
    if selection.color != "random" {
        args.insert("color".into(), json!(selection.color));
    }
    Value::Object(args)
}

fn resume(stage: &str, generation: u64, selection: &Selection) -> Resume {
    Resume {
        stage: stage.into(),
        generation,
        version: 0,
        key: 0,
        win: false,
        puzzle_id: Value::Null,
        selection: selection.clone(),
    }
}

/// A reply's outcome: `Ok(value)` or the failure's message.
type Reply = Result<Value, String>;

impl Session {
    fn invalidate(&mut self) {
        self.internal.load_gen += 1;
        self.internal.attempt = None;
        self.state.phase = "idle".into();
    }

    fn show(&mut self, puzzle: Value, selection: &Selection) {
        self.state.current = Some(puzzle);
        self.state.puzzle_key += 1;
        self.state.attempt_outcome = None;
        self.internal.attempt = Some(Attempt {
            key: self.state.puzzle_key,
            account: selection.account.clone(),
            angle: selection.angle.clone(),
            mode: selection.mode.clone(),
        });
        self.state.phase = "ready".into();
    }

    fn retry(&mut self, puzzle: Value, selection: &Selection) {
        self.internal.load_gen += 1;
        self.state.is_retry = true;
        self.state.last_report = None;
        self.show(puzzle, selection);
    }

    /// A failed load: offline without the database is a distinct phase; everything else shows
    /// the message.
    fn fail(&mut self, selection: &Selection, text: &str) {
        self.warn = true;
        if selection.mode == "offline" && text.contains("Download the puzzle database") {
            self.state.phase = "nodb".into();
        } else {
            self.state.train_error = text.into();
            self.state.phase = "error".into();
        }
    }

    fn load_begin(&mut self, selection: &Selection, online: bool) {
        self.internal.load_gen += 1;
        let generation = self.internal.load_gen;
        self.state.phase = "loading".into();
        self.state.train_error = String::new();
        self.state.last_report = None;
        self.state.is_retry = false;
        if selection.mode == "offline" {
            let centre = self.state.rating.unwrap_or(1500.0) + offset(&selection.difficulty);
            self.call = Some(Call {
                method: "localPuzzles",
                args: json!({
                    "theme": selection.angle,
                    "minRating": (centre - 150.0).max(0.0),
                    "maxRating": centre + 150.0,
                    "count": 1,
                }),
                resume: resume("offline", generation, selection),
            });
            return;
        }
        if selection.mode == "rated" && selection.account.is_empty() {
            self.state.phase = "noaccount".into();
            return;
        }
        if !online {
            self.fail(selection, OFFLINE_TEXT);
            return;
        }
        self.call = Some(Call {
            method: "puzzleNext",
            args: draw_args(&selection.account, selection),
            resume: resume("draw", generation, selection),
        });
    }

    /// A reply to a load. `now` is the host's selection at the reply.
    fn load_settled(&mut self, token: Resume, reply: Reply, now: &Selection) {
        if token.generation != self.internal.load_gen || token.selection != *now {
            return;
        }
        let selection = token.selection.clone();
        let value = match reply {
            Ok(value) => value,
            Err(text) => return self.fail(&selection, &text),
        };
        match token.stage.as_str() {
            "offline" => match value.as_array().and_then(|puzzles| puzzles.first()) {
                Some(puzzle) if !puzzle.is_null() => self.show(puzzle.clone(), &selection),
                _ => self.fail(&selection, NO_PUZZLE_TEXT),
            },
            "draw" => {
                if is_reconnect(&value) {
                    if selection.mode == "rated" {
                        self.state.phase = "reconnect".into();
                        return;
                    }
                    // Practice does not need the permission: take a puzzle without the account.
                    self.call = Some(Call {
                        method: "puzzleNext",
                        args: draw_args("", &selection),
                        resume: resume("anonymous", token.generation, &selection),
                    });
                    return;
                }
                if let Some(rating) = value
                    .get("glicko")
                    .and_then(|glicko| glicko.get("rating"))
                    .and_then(Value::as_f64)
                {
                    self.state.rating = Some(js_round(rating));
                }
                let puzzle = value.get("puzzle").cloned().unwrap_or(Value::Null);
                self.show(puzzle, &selection);
            }
            _ => {
                if !is_reconnect(&value) {
                    let puzzle = value.get("puzzle").cloned().unwrap_or(Value::Null);
                    self.show(puzzle, &selection);
                }
            }
        }
    }

    fn report_begin(&mut self, win: bool, selection: &Selection) {
        let attempt = self.internal.attempt.clone();
        let puzzle = self.state.current.clone().filter(|value| !value.is_null());
        let (Some(puzzle), Some(started)) = (puzzle, attempt) else {
            return;
        };
        if self.state.is_retry
            || self.state.phase != "ready"
            || self.state.attempt_outcome.is_some()
            || started.account != selection.account
            || started.mode != selection.mode
            || started.angle != selection.angle
        {
            return;
        }
        self.state.attempt_outcome = Some(win);
        if win {
            self.state.session.solved += 1.0;
        } else {
            self.state.session.failed += 1.0;
        }
        if !syncs_to_lichess(&self.state, selection) {
            return;
        }
        let puzzle_id = puzzle.get("id").cloned().unwrap_or(Value::Null);
        let mut args = Map::new();
        args.insert("account".into(), json!(started.account));
        args.insert("angle".into(), json!(started.angle));
        if let Some(id) = puzzle.get("id") {
            args.insert("id".into(), id.clone());
        }
        args.insert("win".into(), json!(win));
        args.insert("rated".into(), json!(true));
        let mut token = resume("solve", self.internal.load_gen, selection);
        token.version = self.internal.session_version;
        token.key = started.key;
        token.win = win;
        token.puzzle_id = puzzle_id;
        self.call = Some(Call {
            method: "puzzleSolve",
            args: Value::Object(args),
            resume: token,
        });
    }

    /// A reply to a rated result: it counts only while the attempt, session and selection still are.
    fn report_settled(&mut self, token: Resume, reply: Reply, now: &Selection) {
        let attempt_current = self
            .internal
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.key == token.key);
        let current = token.generation == self.internal.load_gen
            && token.version == self.internal.session_version
            && attempt_current
            && now.account == token.selection.account
            && now.mode == token.selection.mode
            && now.angle == token.selection.angle;
        if !current {
            return;
        }
        let win = token.win;
        let result = match reply {
            Ok(result) => result,
            Err(text) => {
                self.warn = true;
                self.state.last_report = Some(Report {
                    win,
                    rating_diff: None,
                    failed: Some(text),
                });
                return;
            }
        };
        if is_reconnect(&result) {
            self.state.phase = "reconnect".into();
            self.state.last_report = Some(Report {
                win,
                rating_diff: None,
                failed: Some(REJECTED_TEXT.into()),
            });
            return;
        }
        let rating_diff = result.get("ratingDiff").and_then(Value::as_f64);
        if let Some(diff) = rating_diff {
            self.state.session.rating_change += diff;
            if let Some(rating) = self.state.rating {
                self.state.rating = Some(rating + diff);
            }
        }
        let same_puzzle = self
            .state
            .current
            .as_ref()
            .and_then(|puzzle| puzzle.get("id"))
            .is_some_and(|id| *id == token.puzzle_id);
        if same_puzzle {
            self.state.last_report = Some(Report {
                win,
                rating_diff,
                failed: None,
            });
        }
    }

    fn finish(self) -> Out {
        serde_json::to_value(Step {
            state: self.state,
            internal: self.internal,
            call: self.call,
            warn: self.warn,
        })
        .map_err(|error| error.to_string())
    }
}

fn take<T: DeserializeOwned>(arg: &Map<String, Value>, key: &str) -> Result<T, String> {
    serde_json::from_value(arg.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|error| format!("{key}: {error}"))
}

/// The `{ state, internal }` input shared by every transition.
fn session_from(args: &[Value]) -> Result<(Map<String, Value>, Session), String> {
    let arg = args
        .first()
        .and_then(Value::as_object)
        .ok_or("the session input must be an object")?
        .clone();
    let session = Session {
        state: take(&arg, "state")?,
        internal: take(&arg, "internal")?,
        call: None,
        warn: false,
    };
    Ok((arg, session))
}

/// A reply as `{ ok: value }` or `{ error: message }`; an unreadable shape is an error.
fn reply(arg: &Map<String, Value>) -> Result<Reply, String> {
    let outcome = arg
        .get("outcome")
        .and_then(Value::as_object)
        .ok_or("outcome must be an object")?;
    if let Some(text) = outcome.get("error") {
        return Ok(Err(text.as_str().unwrap_or_default().to_string()));
    }
    Ok(Ok(outcome.get("ok").cloned().unwrap_or(Value::Null)))
}

/// `{ state, selection }` → whether the session syncs its result to Lichess.
fn syncs_call(args: &[Value]) -> Out {
    let arg = args
        .first()
        .and_then(Value::as_object)
        .ok_or("the input must be an object")?;
    let state: State = take(arg, "state")?;
    let selection: Selection = take(arg, "selection")?;
    Ok(json!(syncs_to_lichess(&state, &selection)))
}

/// This module's methods for `api::call`; None when the method is not one of them.
pub(super) fn call(method: &str, args: &[Value]) -> Option<Out> {
    let result = match method {
        "puzzleSessionInitial" => Ok(json!({
            "state": initial_state(),
            "internal": Internal::default(),
        })),
        "puzzleSessionInvalidate" => session_from(args).and_then(|(_, mut session)| {
            session.invalidate();
            session.finish()
        }),
        "puzzleSessionLoadBegin" => session_from(args).and_then(|(arg, mut session)| {
            let selection: Selection = take(&arg, "selection")?;
            let online = arg.get("online").and_then(Value::as_bool).unwrap_or(false);
            session.load_begin(&selection, online);
            session.finish()
        }),
        "puzzleSessionLoadSettled" => session_from(args).and_then(|(arg, mut session)| {
            let token: Resume = take(&arg, "resume")?;
            let now: Selection = take(&arg, "selection")?;
            let outcome = reply(&arg)?;
            session.load_settled(token, outcome, &now);
            session.finish()
        }),
        "puzzleSessionRetry" | "puzzleSessionShow" => {
            session_from(args).and_then(|(arg, mut session)| {
                let puzzle = arg.get("puzzle").cloned().unwrap_or(Value::Null);
                let selection: Selection = take(&arg, "selection")?;
                if method == "puzzleSessionRetry" {
                    session.retry(puzzle, &selection);
                } else {
                    session.show(puzzle, &selection);
                }
                session.finish()
            })
        }
        "puzzleSessionReportBegin" => session_from(args).and_then(|(arg, mut session)| {
            let win = arg.get("win").and_then(Value::as_bool).unwrap_or(false);
            let selection: Selection = take(&arg, "selection")?;
            session.report_begin(win, &selection);
            session.finish()
        }),
        "puzzleSessionReportSettled" => session_from(args).and_then(|(arg, mut session)| {
            let token: Resume = take(&arg, "resume")?;
            let now: Selection = take(&arg, "selection")?;
            let outcome = reply(&arg)?;
            session.report_settled(token, outcome, &now);
            session.finish()
        }),
        "puzzleSessionResetSession" => session_from(args).and_then(|(_, mut session)| {
            session.internal.session_version += 1;
            session.state.session = Tally::default();
            session.finish()
        }),
        "puzzleSessionSyncs" => syncs_call(args),
        "isReconnect" => Ok(json!(args.first().is_some_and(is_reconnect))),
        _ => return None,
    };
    Some(result)
}
