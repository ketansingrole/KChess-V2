//! Puzzle training. The session's
//! transitions are Rust (`kchess-domain/src/trainer/puzzle_session.rs`); this keeps its state and
//! private state, and hands the page the host call a transition asks for (a puzzle to fetch, a
//! result to send) and the attempt to log. A single puzzle on the board is `PuzzleState`, moved on
//! by the rules too.

use serde_json::{Map, Value, json};

pub use kchess_domain::trainer::puzzle_session as transitions;

/// One of the session's transitions (`transitions::load_begin`, …).
pub type Transition = fn(&Value) -> Result<Value, String>;

/// A host call a transition is waiting for, and what to hand back with its reply.
pub struct Call {
    pub method: String,
    pub args: Value,
    pub resume: Value,
}

/// What one transition asks of the page.
pub struct Step {
    pub call: Option<Call>,
    /// An attempt for the local log (`recordPuzzleAttempt`).
    pub record: Option<Value>,
}

/// `localDay`: the local calendar day, `YYYY-MM-DD`, which the day streak counts.
pub fn local_day() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub struct PuzzleSession {
    /// `PuzzleSessionState`: phase, the puzzle, its key, the session's tally, the last report.
    pub state: Value,
    internal: Value,
}

impl PuzzleSession {
    pub fn new() -> PuzzleSession {
        let initial = transitions::initial();
        PuzzleSession {
            state: initial["state"].clone(),
            internal: initial["internal"].clone(),
        }
    }

    /// Run one transition and keep its state.
    pub fn apply(&mut self, transition: Transition, input: Value) -> Step {
        let mut args = Map::new();
        args.insert("state".into(), self.state.clone());
        args.insert("internal".into(), self.internal.clone());
        // An attempt replaced by this transition ends on this day.
        args.insert("day".into(), json!(local_day()));
        if let Value::Object(input) = input {
            args.extend(input);
        }
        let step = match transition(&Value::Object(args)) {
            Ok(step) => step,
            Err(cause) => {
                crate::log::write(
                    kchess_contracts::Level::Warn,
                    "puzzles",
                    &format!("a puzzle session step failed: {cause}"),
                );
                return Step {
                    call: None,
                    record: None,
                };
            }
        };
        self.state = step["state"].clone();
        self.internal = step["internal"].clone();
        let call = step["call"].as_object().map(|call| Call {
            method: call["method"].as_str().unwrap_or_default().to_string(),
            args: call["args"].clone(),
            resume: call["resume"].clone(),
        });
        Step {
            call,
            record: Some(step["record"].clone()).filter(|record| !record.is_null()),
        }
    }

    pub fn phase(&self) -> &str {
        self.state["phase"].as_str().unwrap_or("idle")
    }

    pub fn current(&self) -> Option<&Value> {
        Some(&self.state["current"]).filter(|current| !current.is_null())
    }

    pub fn puzzle_key(&self) -> u64 {
        super::whole(&self.state["puzzleKey"])
    }
}

pub use kchess_games::puzzle::OnBoard;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_starts_idle_and_asks_for_a_puzzle() {
        let mut session = PuzzleSession::new();
        assert_eq!(session.phase(), "idle");
        let step = session.apply(
            transitions::load_begin,
            json!({
                "selection": { "account": "", "angle": "mix", "difficulty": "normal", "color": "random", "mode": "practice" },
                "online": true,
            }),
        );
        assert_eq!(session.phase(), "loading");
        assert_eq!(step.call.map(|call| call.method), Some("puzzleNext".into()));
    }
}
