//! The arguments of the puzzle, training-run and voice-log methods.

use serde::de::{self, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use kchess_domain::color;
use kchess_domain::misc::patterns;
use kchess_domain::misc::value::J;

use super::*;
use crate::types::{
    LocalLadderQuery, LocalPuzzleQuery, PUZZLE_DIFFICULTIES, PUZZLE_TRAIN_MODES, Puzzle,
    PuzzleAttemptInput, PuzzleProgressQuery, PuzzleRequest, PuzzleSolveRequest, PuzzleWrongMove,
    RUN_KINDS, RunInput, RunKind, VOICE_OUTCOMES, VOICE_SOURCES, VoiceAttemptInput, VoiceUpdate,
    VoiceWord,
};

const USERNAME: &str = "Enter a valid Lichess username.";
const ATTEMPT: &str = "Invalid puzzle attempt.";
const VOICE: &str = "Invalid voice entry.";
const THEME: &str = "Invalid puzzle theme.";
const MAX_SAFE: u64 = 9_007_199_254_740_991;

fn angle(text: &str) -> Result<(), Refusal> {
    matches(text, patterns::puzzle_angle, Some(THEME), "", "")
}

fn uci(text: &str) -> Result<(), Refusal> {
    matches(text, patterns::uci_move, Some("Invalid move."), "", "")
}

fn local_day(text: &str, msg: &'static str) -> Result<(), Refusal> {
    matches(text, patterns::local_day, Some(msg), "", "")
}

/// A Lichess username, trimmed.
fn username(text: String) -> Result<String, Refusal> {
    Username::validate(J::Str(text)).map(|name| name.0)
}

/// A whole number from `min` to `max`; any rejection is `msg`.
fn integer(value: J, min: u64, max: u64, msg: &'static str) -> Result<u64, Refusal> {
    let n: u64 = decode(value, plain).map_err(|_| msg.to_owned())?;
    within(n, min, max, Some(msg), "")?;
    Ok(n)
}

/// A wire enum's value from text `one_of` already passed.
fn wire<T: serde::de::DeserializeOwned>(text: String) -> Result<T, Refusal> {
    serde_json::from_value(Value::String(text)).map_err(|error| error.to_string())
}

/// Converts a checked whole number to the narrower type a field keeps.
fn narrow(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/* ── Lichess puzzles ── */

/// A puzzle's id, as a lichess.org/training link names it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
#[serde(transparent)]
pub struct PuzzleId(pub String);

impl Validate for PuzzleId {
    fn validate(value: J) -> Result<Self, Refusal> {
        const MSG: &str = "Invalid puzzle.";
        let text: String = decode(value, |_| Some(MSG))?;
        matches(&text, patterns::puzzle_id, Some(MSG), "", "")?;
        Ok(PuzzleId(text))
    }
}

/// How many days of results to show: 1 to 365.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct Days(pub u32);

impl Validate for Days {
    fn validate(value: J) -> Result<Self, Refusal> {
        integer(value, 1, 365, "Invalid number of days.").map(|n| Days(narrow(n)))
    }
}

/// How many recent puzzle results to show: 1 to 200.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct ActivityMax(pub u32);

impl Validate for ActivityMax {
    fn validate(value: J) -> Result<Self, Refusal> {
        integer(value, 1, 200, "Invalid number of puzzles.").map(|n| ActivityMax(narrow(n)))
    }
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentPuzzleRequest {
    account: String,
    angle: String,
    difficulty: String,
    color: Option<String>,
}

impl Validate for PuzzleRequest {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentPuzzleRequest = decode(value, |keys| match keys {
            ["account"] => Some("Invalid account."),
            ["angle"] => Some(THEME),
            ["difficulty"] => Some("Invalid difficulty."),
            ["color"] => Some("Invalid color."),
            _ => Some("Invalid puzzle request."),
        })?;
        let account = match sent.account.is_empty() {
            true => sent.account,
            false => username(sent.account)?,
        };
        angle(&sent.angle)?;
        let difficulty = Some("Invalid difficulty.");
        one_of(&sent.difficulty, PUZZLE_DIFFICULTIES, difficulty, "")?;
        let color = match sent.color {
            Some(name) => {
                let side = color::from_name(&name);
                ensure(side.is_some(), Some("Invalid color."), "", String::new)?;
                side
            }
            None => None,
        };
        Ok(PuzzleRequest {
            account,
            angle: sent.angle,
            difficulty: wire(sent.difficulty)?,
            color,
        })
    }
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentPuzzleSolve {
    account: String,
    angle: String,
    id: String,
    win: bool,
    rated: bool,
}

impl Validate for PuzzleSolveRequest {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentPuzzleSolve = decode(value, |keys| match keys {
            ["account"] => Some(USERNAME),
            ["angle"] => Some(THEME),
            ["id"] => Some("Invalid puzzle."),
            _ => Some("Invalid puzzle result."),
        })?;
        let account = username(sent.account)?;
        angle(&sent.angle)?;
        let msg = Some("Invalid puzzle.");
        matches(&sent.id, patterns::puzzle_id, msg, "", "")?;
        Ok(PuzzleSolveRequest {
            account,
            angle: sent.angle,
            id: sent.id,
            win: sent.win,
            rated: sent.rated,
        })
    }
}

/* ── The puzzle database and the attempt log ── */

#[derive(Deserialize)]
#[serde(expecting = "an object", rename_all = "camelCase")]
struct SentLocalQuery {
    theme: Option<String>,
    min_rating: Option<u64>,
    max_rating: Option<u64>,
    count: u64,
}

impl Validate for LocalPuzzleQuery {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentLocalQuery = decode(value, |keys| match keys {
            ["theme"] => Some(THEME),
            ["minRating" | "maxRating"] => Some("Invalid rating."),
            ["count"] => Some("Invalid puzzle count."),
            _ => Some("Invalid puzzle query."),
        })?;
        if let Some(theme) = &sent.theme {
            angle(theme)?;
        }
        for rating in sent.min_rating.iter().chain(&sent.max_rating) {
            within(*rating, 0, 4000, Some("Invalid rating."), "")?;
        }
        within(sent.count, 1, 300, Some("Invalid puzzle count."), "")?;
        Ok(LocalPuzzleQuery {
            theme: sent.theme,
            min_rating: sent.min_rating.map(narrow),
            max_rating: sent.max_rating.map(narrow),
            count: narrow(sent.count),
        })
    }
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentLadderQuery {
    from: u64,
    to: u64,
    count: u64,
}

impl Validate for LocalLadderQuery {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentLadderQuery = decode(value, |keys| match keys {
            ["from" | "to"] => Some("Invalid rating."),
            ["count"] => Some("Invalid puzzle count."),
            _ => Some("Invalid puzzle query."),
        })?;
        within(sent.from, 0, 4000, Some("Invalid rating."), "")?;
        within(sent.to, 0, 4000, Some("Invalid rating."), "")?;
        within(sent.count, 1, 300, Some("Invalid puzzle count."), "")?;
        Ok(LocalLadderQuery {
            from: narrow(sent.from),
            to: narrow(sent.to),
            count: narrow(sent.count),
        })
    }
}

/// A puzzle as the trainer shows it, as the attempt log keeps it.
#[derive(Deserialize)]
#[serde(expecting = "an object", rename_all = "camelCase")]
struct SentAttemptPuzzle {
    id: String,
    fen: String,
    last_move: Option<String>,
    solution: Vec<String>,
    rating: u64,
    themes: Vec<String>,
    plays: Option<u64>,
    game_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentWrongMove {
    index: u64,
    uci: String,
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentAttempt {
    puzzle: SentAttemptPuzzle,
    mode: String,
    retry: bool,
    win: bool,
    revealed: bool,
    hinted: bool,
    found: u64,
    wrong: u64,
    total: u64,
    day: String,
    #[serde(rename = "wrongMoves")]
    wrong_moves: Option<Vec<SentWrongMove>>,
}

impl Validate for PuzzleAttemptInput {
    fn validate(value: J) -> Result<Self, Refusal> {
        const MSG: Msg = Some(ATTEMPT);
        let sent: SentAttempt = decode(value, |keys| match keys {
            ["puzzle", "lastMove"] | ["puzzle", "solution", "[]"] => Some("Invalid move."),
            ["puzzle", "themes", "[]"] => Some(THEME),
            ["puzzle", "gameId"] => Some("Invalid game id."),
            _ => Some(ATTEMPT),
        })?;
        let puzzle = &sent.puzzle;
        matches(&puzzle.id, patterns::puzzle_id, MSG, "", "")?;
        fen(&puzzle.fen, MSG, MSG, "")?;
        if let Some(last) = &puzzle.last_move {
            uci(last)?;
        }
        puzzle.solution.iter().try_for_each(|m| uci(m))?;
        ensure(!puzzle.solution.is_empty(), MSG, "", String::new)?;
        max_items(&puzzle.solution, 64, MSG, "")?;
        within(puzzle.rating, 0, 4000, MSG, "")?;
        puzzle.themes.iter().try_for_each(|theme| angle(theme))?;
        max_items(&puzzle.themes, 40, MSG, "")?;
        if let Some(plays) = puzzle.plays {
            within(plays, 0, MAX_SAFE, MSG, "")?;
        }
        if let Some(id) = &puzzle.game_id {
            matches(id, patterns::game_id, Some("Invalid game id."), "", "")?;
        }
        one_of(&sent.mode, PUZZLE_TRAIN_MODES, MSG, "")?;
        within(sent.found, 0, 32, MSG, "")?;
        within(sent.wrong, 0, 32, MSG, "")?;
        within(sent.total, 1, 32, MSG, "")?;
        local_day(&sent.day, ATTEMPT)?;
        if let Some(wrong) = &sent.wrong_moves {
            max_items(wrong, 32, MSG, "")?;
            for entry in wrong {
                within(entry.index, 0, 63, MSG, "")?;
                uci(&entry.uci)?;
            }
        }
        let SentAttempt {
            puzzle,
            wrong_moves,
            ..
        } = sent;
        Ok(PuzzleAttemptInput {
            puzzle: Puzzle {
                id: puzzle.id,
                fen: puzzle.fen,
                last_move: puzzle.last_move,
                solution: puzzle.solution,
                rating: puzzle.rating as f64,
                themes: puzzle.themes,
                plays: puzzle.plays.map(|plays| plays as f64),
                game_id: puzzle.game_id,
            },
            mode: wire(sent.mode)?,
            retry: sent.retry,
            win: sent.win,
            revealed: sent.revealed,
            hinted: sent.hinted,
            found: narrow(sent.found),
            wrong: narrow(sent.wrong),
            total: narrow(sent.total),
            day: sent.day,
            wrong_moves: wrong_moves.map(|moves| {
                moves
                    .into_iter()
                    .map(|entry| PuzzleWrongMove {
                        index: narrow(entry.index),
                        uci: entry.uci,
                    })
                    .collect()
            }),
        })
    }
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentProgressQuery {
    today: String,
    limit: u64,
}

impl Validate for PuzzleProgressQuery {
    fn validate(value: J) -> Result<Self, Refusal> {
        const MSG: &str = "Invalid puzzle history query.";
        let sent: SentProgressQuery = decode(value, |_| Some(MSG))?;
        local_day(&sent.today, MSG)?;
        within(sent.limit, 1, 500, Some(MSG), "")?;
        Ok(PuzzleProgressQuery {
            today: sent.today,
            limit: narrow(sent.limit),
        })
    }
}

/* ── Training runs ── */

impl Validate for RunKind {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, RUN_KINDS, Some("Invalid kind of run."))
    }
}

/// One value of a run's detail: a string, number or flag.
enum DetailValue {
    Flag(bool),
    Number(f64),
    Text(String),
}

impl<'de> Deserialize<'de> for DetailValue {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Scalar;
        impl Visitor<'_> for Scalar {
            type Value = DetailValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a string, number or boolean")
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<DetailValue, E> {
                Ok(DetailValue::Flag(b))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<DetailValue, E> {
                Ok(DetailValue::Number(n as f64))
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<DetailValue, E> {
                Ok(DetailValue::Number(n as f64))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<DetailValue, E> {
                Ok(DetailValue::Number(n))
            }
            fn visit_str<E: de::Error>(self, text: &str) -> Result<DetailValue, E> {
                Ok(DetailValue::Text(text.to_owned()))
            }
        }
        deserializer.deserialize_any(Scalar)
    }
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentRun {
    kind: String,
    variant: String,
    score: f64,
    detail: crate::types::Keyed<DetailValue>,
}

impl Validate for RunInput {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentRun = decode(value, |keys| match keys {
            ["kind"] => Some("Invalid kind of run."),
            ["score"] => Some("Invalid score."),
            ["detail", _] => None,
            _ => Some("Invalid run."),
        })?;
        one_of(&sent.kind, RUN_KINDS, Some("Invalid kind of run."), "")?;
        let msg = Some("Invalid run.");
        matches(&sent.variant, patterns::run_variant, msg, "", "")?;
        ensure(
            sent.score.is_finite(),
            Some("Invalid score."),
            "",
            String::new,
        )?;
        within(sent.score, 0.0, 1_000_000.0, None, "score")?;
        let mut detail = serde_json::Map::new();
        for (key, value) in &sent.detail.0 {
            matches(key, patterns::run_detail_key, msg, "", "")?;
            let path = format!("detail.{key}");
            let value = match value {
                DetailValue::Text(text) => {
                    max_chars(text, 80, None, &path)?;
                    Value::String(text.clone())
                }
                DetailValue::Number(n) => {
                    ensure(n.is_finite(), None, &path, || "a finite number".into())?;
                    serde_json::Number::from_f64(*n).map_or(Value::Null, Value::Number)
                }
                DetailValue::Flag(flag) => Value::Bool(*flag),
            };
            detail.insert(key.clone(), value);
        }
        max_items(&sent.detail.0, 20, msg, "")?;
        Ok(RunInput {
            kind: wire(sent.kind)?,
            variant: sent.variant,
            score: sent.score,
            detail,
        })
    }
}

/* ── The voice log ── */

/// A logged phrase's id.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct VoiceId(pub i64);

impl Validate for VoiceId {
    fn validate(value: J) -> Result<Self, Refusal> {
        integer(value, 1, MAX_SAFE, VOICE).map(|n| VoiceId(n as i64))
    }
}

/// How many logged phrases to list: 1 to 5000.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct VoiceLimit(pub u32);

impl Validate for VoiceLimit {
    fn validate(value: J) -> Result<Self, Refusal> {
        integer(value, 1, 5000, "Invalid number of entries.").map(|n| VoiceLimit(narrow(n)))
    }
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentWord {
    word: String,
    conf: f64,
}

#[derive(Deserialize)]
#[serde(expecting = "an object", rename_all = "camelCase")]
struct SentVoiceAttempt {
    source: String,
    heard: String,
    confidence: f64,
    words: Vec<SentWord>,
    outcome: String,
    parsed: Option<String>,
    expected: Option<String>,
    fen: Option<String>,
    retry_of: Option<u64>,
}

/// A recognizer's confidence, from 0 to 1.
fn score(n: f64, path: &str) -> Result<(), Refusal> {
    within(n, 0.0, 1.0, None, path)
}

impl Validate for VoiceAttemptInput {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentVoiceAttempt = decode(value, |keys| match keys {
            ["words"] | ["words", "[]"] => None,
            _ => Some(VOICE),
        })?;
        one_of(&sent.source, VOICE_SOURCES, Some(VOICE), "")?;
        max_chars(&sent.heard, 200, None, "heard")?;
        score(sent.confidence, "confidence")?;
        for word in &sent.words {
            max_chars(&word.word, 40, None, "words.word")?;
            score(word.conf, "words.conf")?;
        }
        max_items(&sent.words, 40, Some(VOICE), "")?;
        one_of(&sent.outcome, VOICE_OUTCOMES, Some(VOICE), "")?;
        if let Some(parsed) = &sent.parsed {
            max_chars(parsed, 120, None, "parsed")?;
        }
        if let Some(expected) = &sent.expected {
            max_chars(expected, 20, None, "expected")?;
        }
        if let Some(position) = &sent.fen {
            fen(position, None, Some("Invalid position."), "fen")?;
        }
        if let Some(retry) = sent.retry_of {
            within(retry, 1, MAX_SAFE, Some(VOICE), "")?;
        }
        Ok(VoiceAttemptInput {
            source: wire(sent.source)?,
            heard: sent.heard,
            confidence: sent.confidence,
            words: sent
                .words
                .into_iter()
                .map(|word| VoiceWord {
                    word: word.word,
                    conf: word.conf,
                })
                .collect(),
            outcome: wire(sent.outcome)?,
            parsed: sent.parsed,
            expected: sent.expected,
            fen: sent.fen,
            retry_of: sent.retry_of.map(|id| id as i64),
        })
    }
}

#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentVoiceUpdate {
    outcome: Option<String>,
    expected: Option<String>,
}

impl Validate for VoiceUpdate {
    fn validate(value: J) -> Result<Self, Refusal> {
        let sent: SentVoiceUpdate = decode(value, |_| Some(VOICE))?;
        let outcome = match sent.outcome {
            Some(outcome) => {
                one_of(&outcome, VOICE_OUTCOMES, Some(VOICE), "")?;
                Some(wire(outcome)?)
            }
            None => None,
        };
        if let Some(expected) = &sent.expected {
            max_chars(expected, 20, None, "expected")?;
        }
        Ok(VoiceUpdate {
            outcome,
            expected: sent.expected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    #[test]
    fn a_puzzle_id_and_counts_keep_their_messages() {
        assert_eq!(
            check::<PuzzleId>(J::Str("no!".into())),
            Err("Invalid puzzle.".into())
        );
        assert_eq!(
            check::<PuzzleId>(J::Num(1.0)),
            Err("Invalid puzzle.".into())
        );
        assert_eq!(
            check::<VoiceLimit>(J::Num(0.0)),
            Err("Invalid number of entries.".into())
        );
        assert_eq!(check::<VoiceId>(J::Num(3.0)), Ok(VoiceId(3)));
    }

    #[test]
    fn a_puzzle_request_trims_its_account_and_reads_its_color() {
        let sent = obj(vec![
            ("account", J::Str(" Alice ".into())),
            ("angle", J::Str("mix".into())),
            ("difficulty", J::Str("normal".into())),
            ("color", J::Str("black".into())),
        ]);
        let request = check::<PuzzleRequest>(sent).unwrap();
        assert_eq!(request.account, "Alice");
        assert_eq!(request.color, Some(color::Color::Black));
        let bad = obj(vec![
            ("account", J::Str(String::new())),
            ("angle", J::Str("mix".into())),
            ("difficulty", J::Str("normal".into())),
            ("color", J::Str("red".into())),
        ]);
        assert_eq!(check::<PuzzleRequest>(bad), Err("Invalid color.".into()));
    }

    #[test]
    fn a_run_keeps_its_detail_in_order() {
        let detail = obj(vec![("b", J::Num(2.0)), ("a", J::Str("x".into()))]);
        let run = obj(vec![
            ("kind", J::Str("storm".into())),
            ("variant", J::Str("3min".into())),
            ("score", J::Num(4.0)),
            ("detail", detail),
        ]);
        let run = check::<RunInput>(run).unwrap();
        let keys: Vec<&String> = run.detail.keys().collect();
        assert_eq!(keys, ["b", "a"]);
    }
}
