//! The puzzle training session, a typed state machine.
//!
//! The caller holds the session, runs the host call each step returns (`call`), and reports the
//! settled reply back with the `resume` token it was given. Every decision is made here: a reply
//! counts only while its load generation, session version, attempt and selection are still
//! current, which is how late replies are ignored. `day` is the host's calendar day
//! (`YYYY-MM-DD`), for an attempt that ends in the step.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::puzzle_progress::{Outcome, Verdict, classify, moves};
use crate::js;

const OFFLINE_TEXT: &str = "You’re offline. Choose Downloaded to use puzzles on this device, or reconnect for online puzzles.";
const REJECTED_TEXT: &str = "Lichess did not accept the result: reconnect your account.";

/// What the player chose to train: the account, theme, difficulty, colour and mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub account: String,
    pub angle: String,
    pub difficulty: String,
    pub color: String,
    pub mode: String,
}

/// The session's attempts by outcome (`puzzle_progress::Outcome`), and its player moves.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub solved: f64,
    pub hinted: f64,
    pub partial: f64,
    pub missed: f64,
    pub revealed: f64,
    /// Player moves found, and tried (found plus the wrong move that ended an attempt).
    pub moves_found: f64,
    pub moves_tried: f64,
    pub rating_change: f64,
}

impl Tally {
    fn count(&mut self, outcome: Outcome, found: u32, tried: u32) {
        let slot = match outcome {
            Outcome::Solved => &mut self.solved,
            Outcome::Hinted => &mut self.hinted,
            Outcome::Partial => &mut self.partial,
            Outcome::Missed => &mut self.missed,
            Outcome::Revealed => &mut self.revealed,
        };
        *slot += 1.0;
        self.moves_found += f64::from(found);
        self.moves_tried += f64::from(tried);
    }
}

/// How Lichess took the last result sent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub win: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_diff: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

/// What the page shows of the session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// `idle`, `loading`, `ready`, `exhausted`, `nodb`, `noaccount`, `reconnect` or `error`.
    pub phase: String,
    #[serde(default)]
    pub current: Option<Value>,
    pub puzzle_key: u64,
    pub is_retry: bool,
    pub train_error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    pub session: Tally,
    #[serde(default)]
    pub last_report: Option<Report>,
    #[serde(default)]
    pub attempt_outcome: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attempt {
    key: u64,
    account: String,
    angle: String,
    mode: String,
    /// Drawn without the account (practice when it lacks the puzzle permission): never reported.
    #[serde(default)]
    anonymous: bool,
    /// Tried again from the history: logged, not counted.
    #[serde(default)]
    retry: bool,
    /// Each player move's first try, by its number in the solution: 1 right, 2 wrong, 0 untried.
    #[serde(default)]
    tries: Vec<u8>,
    /// What was played at each wrong first try, for the log.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    wrong_moves: Vec<WrongMove>,
    /// The verdict's facts, once given; taken when the attempt ends and is logged.
    #[serde(default)]
    verdict: Option<VerdictFacts>,
}

/// A wrong first try: the index in the solution of the move it replaced, and the move played.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WrongMove {
    index: usize,
    uci: String,
}

/// What the board knew at the first verdict.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerdictFacts {
    win: bool,
    revealed: bool,
    hinted: bool,
    total: u32,
}

/// Whether Lichess rates an attempt: as on its own site, a hint leaves the result but not the rating.
fn counts_for_rating(mode: &str, hinted: bool) -> bool {
    mode == "rated" && !hinted
}

/// The most player moves an attempt scores; longer solutions are not real puzzles.
const MAX_TRIES: usize = 32;

/// What the session keeps private: replies are matched against these counters.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Internal {
    load_gen: u64,
    session_version: u64,
    attempt: Option<Attempt>,
}

/// Carried by a host call and handed back with its reply.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resume {
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

/// A core call a step asks for, and the token its reply goes back with.
#[derive(Clone, Debug)]
pub struct Call {
    pub request: Request,
    pub resume: Resume,
}

/// The core call a step asks for, with its argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Downloaded puzzles near a rating (`localPuzzles`).
    LocalPuzzles(LocalQuery),
    /// The next Lichess puzzle (`puzzleNext`).
    PuzzleNext(Draw),
    /// A solved puzzle's result for Lichess (`puzzleSolve`).
    PuzzleSolve(Solve),
}

/// The downloaded puzzles asked for.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalQuery {
    pub theme: String,
    pub min_rating: f64,
    pub max_rating: f64,
    pub count: u32,
}

/// The Lichess puzzle asked for; no colour for either.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Draw {
    pub account: String,
    pub angle: String,
    pub difficulty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// A puzzle's result as Lichess takes it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Solve {
    pub account: String,
    pub angle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub win: bool,
    pub rated: bool,
}

/// What one step asks of the host.
#[derive(Clone, Debug, Default)]
pub struct Step {
    pub call: Option<Call>,
    /// An attempt for the host to keep in the local log (`recordPuzzleAttempt`).
    pub record: Option<Value>,
    /// The step failed in a way worth a warning in the log.
    pub warn: bool,
}

/// The puzzle training session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PuzzleSession {
    state: State,
    internal: Internal,
}

/// One step's working copy: the session, what it asks of the host, and the day.
struct Session<'a> {
    state: &'a mut State,
    internal: &'a mut Internal,
    call: Option<Call>,
    record: Option<Value>,
    warn: bool,
    day: &'a str,
}

/// What the board knew at the verdict.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    #[serde(default)]
    pub revealed: bool,
    #[serde(default)]
    pub hinted: bool,
    #[serde(default)]
    pub total: u32,
}

/// The offline rating offset by difficulty; an unknown difficulty has none (NaN).
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

/// A non-array object with a `needsReconnect` property.
pub fn is_reconnect(value: &Value) -> bool {
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

/// Rated results move the Lichess rating; practice ones are sent unrated, so that Lichess counts
/// the puzzle as played and draws a new one next time.
fn syncs_to_lichess(state: &State, selection: &Selection) -> bool {
    (selection.mode == "rated" || selection.mode == "practice")
        && !state.is_retry
        && !selection.account.is_empty()
}

/// The arguments of `puzzleNext`: the colour is left out when it is random.
fn draw_args(account: &str, selection: &Selection) -> Request {
    Request::PuzzleNext(Draw {
        account: account.to_string(),
        angle: selection.angle.clone(),
        difficulty: selection.difficulty.clone(),
        color: (selection.color != "random").then(|| selection.color.clone()),
    })
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
pub type Reply = Result<Value, String>;

impl Session<'_> {
    fn invalidate(&mut self) {
        self.end_attempt();
        self.internal.load_gen += 1;
        self.internal.attempt = None;
        self.state.phase = "idle".into();
    }

    /// Show `puzzle` as a new attempt; `retry` (tried again from the history) is logged, not
    /// counted. It is this attempt's alone: kept in the state, it carried over to the puzzles
    /// shown after a retry.
    fn show(&mut self, puzzle: Value, selection: &Selection, retry: bool) {
        self.end_attempt();
        self.state.is_retry = retry;
        self.state.current = Some(puzzle);
        self.state.puzzle_key += 1;
        self.state.attempt_outcome = None;
        self.internal.attempt = Some(Attempt {
            key: self.state.puzzle_key,
            account: selection.account.clone(),
            angle: selection.angle.clone(),
            mode: selection.mode.clone(),
            anonymous: false,
            retry,
            tries: Vec::new(),
            wrong_moves: Vec::new(),
            verdict: None,
        });
        self.state.phase = "ready".into();
    }

    fn retry(&mut self, puzzle: Value, selection: &Selection) {
        self.end_attempt();
        self.internal.load_gen += 1;
        self.state.last_report = None;
        self.show(puzzle, selection, true);
    }

    /// No puzzle left for the theme and difficulty, on Lichess or in the local database: an
    /// expected state with its own phase (the page offers other choices), not a failure.
    fn exhausted(&mut self) {
        self.state.phase = "exhausted".into();
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
        self.end_attempt();
        self.internal.load_gen += 1;
        let generation = self.internal.load_gen;
        self.state.phase = "loading".into();
        self.state.train_error = String::new();
        self.state.last_report = None;
        self.state.is_retry = false;
        if selection.mode == "offline" {
            let centre = self.state.rating.unwrap_or(1500.0) + offset(&selection.difficulty);
            self.call = Some(Call {
                request: Request::LocalPuzzles(LocalQuery {
                    theme: selection.angle.clone(),
                    min_rating: (centre - 150.0).max(0.0),
                    max_rating: centre + 150.0,
                    count: 1,
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
            request: draw_args(&selection.account, selection),
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
                Some(puzzle) if !puzzle.is_null() => self.show(puzzle.clone(), &selection, false),
                _ => self.exhausted(),
            },
            "draw" => {
                if is_reconnect(&value) {
                    if selection.mode == "rated" {
                        self.state.phase = "reconnect".into();
                        return;
                    }
                    // Practice does not need the permission: take a puzzle without the account.
                    self.call = Some(Call {
                        request: draw_args("", &selection),
                        resume: resume("anonymous", token.generation, &selection),
                    });
                    return;
                }
                if let Some(rating) = value
                    .get("glicko")
                    .and_then(|glicko| glicko.get("rating"))
                    .and_then(Value::as_f64)
                {
                    self.state.rating = Some(js::round(rating));
                }
                match value.get("puzzle").filter(|puzzle| !puzzle.is_null()) {
                    Some(puzzle) => self.show(puzzle.clone(), &selection, false),
                    None => self.exhausted(),
                }
            }
            _ => {
                if is_reconnect(&value) {
                    return;
                }
                match value.get("puzzle").filter(|puzzle| !puzzle.is_null()) {
                    Some(puzzle) => {
                        self.show(puzzle.clone(), &selection, false);
                        if let Some(attempt) = self.internal.attempt.as_mut() {
                            attempt.anonymous = true;
                        }
                    }
                    None => self.exhausted(),
                }
            }
        }
    }

    /// A player move of the shown puzzle: the first try at each move of the solution is its score,
    /// and a wrong one is kept as played (`uci`, when the board says).
    fn played(&mut self, index: usize, correct: bool, uci: Option<&str>) {
        if self.state.phase != "ready" || index >= MAX_TRIES {
            return;
        }
        let Some(attempt) = self.internal.attempt.as_mut() else {
            return;
        };
        if attempt.tries.len() <= index {
            attempt.tries.resize(index + 1, 0);
        }
        if attempt.tries[index] == 0 {
            attempt.tries[index] = if correct { 1 } else { 2 };
            if let (false, Some(uci)) = (correct, uci) {
                // The player moves first, so their move `index` is the solution's `2 × index`.
                attempt.wrong_moves.push(WrongMove {
                    index: 2 * index,
                    uci: uci.to_string(),
                });
            }
        }
    }

    /// The attempt is over (solved, solution shown, or the puzzle replaced): count it, unless it is
    /// a retry, and hand it to the host's log. An attempt without a verdict was not played.
    fn end_attempt(&mut self) {
        let puzzle = self.state.current.clone().filter(|value| !value.is_null());
        let Some(attempt) = self.internal.attempt.as_mut() else {
            return;
        };
        let (Some(facts), Some(puzzle)) = (attempt.verdict.take(), puzzle) else {
            return;
        };
        let count = |score: u8| attempt.tries.iter().filter(|&&t| t == score).count() as u32;
        let verdict = Verdict {
            win: facts.win,
            revealed: facts.revealed,
            hinted: facts.hinted,
            found: count(1),
            wrong: count(2),
            total: facts.total,
        };
        let (mode, retry) = (attempt.mode.clone(), attempt.retry);
        let wrong_moves = std::mem::take(&mut attempt.wrong_moves);
        let outcome = classify(&verdict);
        let (found, tried) = moves(outcome, &verdict);
        let mut record = json!({
            "puzzle": puzzle,
            "mode": mode,
            "retry": retry,
            "win": verdict.win,
            "revealed": verdict.revealed,
            "hinted": verdict.hinted,
            "found": verdict.found,
            "wrong": verdict.wrong,
            "total": verdict.total,
            "day": self.day,
        });
        if !wrong_moves.is_empty() {
            record["wrongMoves"] = json!(wrong_moves);
        }
        self.record = Some(record);
        if !retry {
            self.state.session.count(outcome, found, tried);
        }
    }

    /// The first verdict on the shown puzzle: a rated or practice result goes to Lichess now, while
    /// the local score waits for the attempt to end. A retry is never sent to Lichess.
    fn report_begin(&mut self, win: bool, detail: Option<Detail>, selection: &Selection) {
        let attempt = self.internal.attempt.clone();
        let puzzle = self.state.current.clone().filter(|value| !value.is_null());
        let (Some(puzzle), Some(started)) = (puzzle, attempt) else {
            return;
        };
        if self.state.phase != "ready"
            || self.state.attempt_outcome.is_some()
            || started.account != selection.account
            || started.mode != selection.mode
            || started.angle != selection.angle
        {
            return;
        }
        self.state.attempt_outcome = Some(win);
        let detail = detail.unwrap_or_default();
        if let Some(attempt) = self.internal.attempt.as_mut() {
            attempt.verdict = Some(VerdictFacts {
                win,
                revealed: detail.revealed,
                hinted: detail.hinted,
                total: detail.total,
            });
        }
        if self.state.is_retry {
            return;
        }
        if !syncs_to_lichess(self.state, selection) || started.anonymous {
            return;
        }
        let puzzle_id = puzzle.get("id").cloned().unwrap_or(Value::Null);
        let solve = Solve {
            account: started.account.clone(),
            angle: started.angle.clone(),
            id: puzzle.get("id").cloned(),
            win,
            rated: counts_for_rating(&started.mode, detail.hinted),
        };
        let mut token = resume("solve", self.internal.load_gen, selection);
        token.version = self.internal.session_version;
        token.key = started.key;
        token.win = win;
        token.puzzle_id = puzzle_id;
        self.call = Some(Call {
            request: Request::PuzzleSolve(solve),
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
        let rated = token.selection.mode == "rated";
        if is_reconnect(&result) {
            // Practice goes on without the account; only rated puzzles need it back.
            if rated {
                self.state.phase = "reconnect".into();
            }
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
        // A practice result changes nothing to show; only a rated one has a rating change.
        if same_puzzle && rated {
            self.state.last_report = Some(Report {
                win,
                rating_diff,
                failed: None,
            });
        }
    }

    fn finish(self) -> Step {
        Step {
            call: self.call,
            record: self.record,
            warn: self.warn,
        }
    }
}

impl Default for PuzzleSession {
    fn default() -> Self {
        Self::new()
    }
}

impl PuzzleSession {
    /// The session before its first puzzle.
    pub fn new() -> PuzzleSession {
        PuzzleSession {
            state: initial_state(),
            internal: Internal::default(),
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn phase(&self) -> &str {
        &self.state.phase
    }

    /// The puzzle shown, if any.
    pub fn current(&self) -> Option<&Value> {
        self.state
            .current
            .as_ref()
            .filter(|current| !current.is_null())
    }

    /// Changes with each puzzle shown.
    pub fn puzzle_key(&self) -> u64 {
        self.state.puzzle_key
    }

    fn step(&mut self, day: &str, body: impl FnOnce(&mut Session)) -> Step {
        let mut session = Session {
            state: &mut self.state,
            internal: &mut self.internal,
            call: None,
            record: None,
            warn: false,
            day,
        };
        body(&mut session);
        session.finish()
    }

    /// Ends the attempt on the board without a result, so no reply still on its way counts.
    pub fn invalidate(&mut self, day: &str) -> Step {
        self.step(day, |session| session.invalidate())
    }

    /// Asks for the next puzzle of `selection`, from Lichess when `online`.
    pub fn load_begin(&mut self, selection: &Selection, online: bool, day: &str) -> Step {
        self.step(day, |session| session.load_begin(selection, online))
    }

    /// The reply to a load, counted only while it is still current for `selection`.
    pub fn load_settled(
        &mut self,
        resume: Resume,
        reply: Reply,
        selection: &Selection,
        day: &str,
    ) -> Step {
        self.step(day, |session| {
            session.load_settled(resume, reply, selection)
        })
    }

    /// Plays an earlier `puzzle` again, unrated and uncounted.
    pub fn retry(&mut self, puzzle: Value, selection: &Selection, day: &str) -> Step {
        self.step(day, |session| session.retry(puzzle, selection))
    }

    /// Shows a chosen `puzzle`, replacing whatever a load still on its way would show.
    pub fn show(&mut self, puzzle: Value, selection: &Selection, day: &str) -> Step {
        self.step(day, |session| {
            session.internal.load_gen += 1;
            session.show(puzzle, selection, false);
        })
    }

    /// The attempt's result, sent to Lichess when the selection syncs.
    pub fn report_begin(
        &mut self,
        win: bool,
        detail: Option<Detail>,
        selection: &Selection,
    ) -> Step {
        self.step("", |session| session.report_begin(win, detail, selection))
    }

    /// A move played on the board: its `index` in the solution, whether `correct`, and its `uci`.
    pub fn played(&mut self, index: usize, correct: bool, uci: Option<&str>) -> Step {
        self.step("", |session| session.played(index, correct, uci))
    }

    /// The puzzle is over: its attempt is counted and logged.
    pub fn end(&mut self, day: &str) -> Step {
        self.step(day, |session| session.end_attempt())
    }

    /// Lichess's reply to a result, counted only while it is still current for `selection`.
    pub fn report_settled(&mut self, resume: Resume, reply: Reply, selection: &Selection) -> Step {
        self.step("", |session| {
            session.report_settled(resume, reply, selection)
        })
    }

    /// Starts the session's tally again.
    pub fn reset_session(&mut self) -> Step {
        self.step("", |session| {
            session.internal.session_version += 1;
            session.state.session = Tally::default();
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection() -> Selection {
        Selection {
            account: String::new(),
            angle: "mix".into(),
            difficulty: "normal".into(),
            color: String::new(),
            mode: "offline".into(),
        }
    }

    fn puzzle(id: &str) -> Value {
        json!({
            "id": id,
            "fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "solution": ["e2e4", "e7e5"],
            "rating": 1500,
            "themes": ["mix"],
            "plays": 1,
        })
    }

    /// A puzzle tried again from the history marked the session as retrying, and that mark
    /// carried over: a puzzle shown next was neither counted nor synced to Lichess.
    #[test]
    fn a_retry_marks_only_its_own_attempt() {
        let mut session = PuzzleSession::new();
        session.retry(puzzle("old"), &selection(), "");
        assert!(session.state().is_retry);
        session.show(puzzle("new"), &selection(), "");
        assert!(!session.state().is_retry);
        assert!(session.internal.attempt.as_ref().is_some_and(|a| !a.retry));
    }

    /// The local database answering with no puzzle (`[null]`) is out of puzzles, not one to show.
    #[test]
    fn an_offline_load_without_a_puzzle_is_exhausted() {
        let mut session = PuzzleSession::new();
        let call = session.load_begin(&selection(), true, "").call.unwrap();
        session.load_settled(call.resume, Ok(json!([null])), &selection(), "");
        assert_eq!(session.state().phase, "exhausted");
        assert!(session.state().current.is_none());
    }

    /// A load still on its way replaced the puzzle chosen while it loaded.
    #[test]
    fn a_shown_puzzle_is_not_replaced_by_an_earlier_load() {
        let mut session = PuzzleSession::new();
        let call = session.load_begin(&selection(), true, "").call.unwrap();
        session.show(puzzle("chosen"), &selection(), "");
        let late = Ok(json!([puzzle("loaded")]));
        session.load_settled(call.resume, late, &selection(), "");
        assert_eq!(session.state().current.as_ref().unwrap()["id"], "chosen");
    }

    #[test]
    fn a_hint_takes_an_attempt_off_the_rating() {
        assert!(counts_for_rating("rated", false));
        assert!(!counts_for_rating("rated", true));
        assert!(!counts_for_rating("practice", false));
    }
}
