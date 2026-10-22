//! Puzzle training (`crates/kchess-wasm/js/puzzleSession.ts` and `puzzle.ts`). The session's
//! transitions are Rust (`kchess-domain/src/trainer/puzzle_session.rs`); this keeps its state and
//! private state, and hands the page the host call a transition asks for (a puzzle to fetch, a
//! result to send) and the attempt to log. A single puzzle on the board is `PuzzleState`, moved on
//! by the rules too.

use serde_json::{Map, Value, json};

use super::rules;

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
        let initial = rules("puzzleSessionInitial", &[]).unwrap_or(Value::Null);
        PuzzleSession {
            state: initial["state"].clone(),
            internal: initial["internal"].clone(),
        }
    }

    /// Run one transition and keep its state.
    pub fn apply(&mut self, method: &str, input: Value) -> Step {
        let mut args = Map::new();
        args.insert("state".into(), self.state.clone());
        args.insert("internal".into(), self.internal.clone());
        // An attempt replaced by this transition ends on this day.
        args.insert("day".into(), json!(local_day()));
        if let Value::Object(input) = input {
            args.extend(input);
        }
        let step = match rules(method, &[Value::Object(args)]) {
            Ok(step) => step,
            Err(cause) => {
                crate::log::write(
                    kchess_core::Level::Warn,
                    "puzzles",
                    &format!("{method} failed: {cause}"),
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

/// One puzzle on the board (`PuzzleState`): the position, the next solution move to find, and
/// whether it is still being played.
pub struct OnBoard {
    pub puzzle: Value,
    pub state: Value,
}

impl OnBoard {
    pub fn start(puzzle: &Value) -> OnBoard {
        OnBoard {
            puzzle: puzzle.clone(),
            state: rules("startPuzzle", std::slice::from_ref(puzzle)).unwrap_or(Value::Null),
        }
    }

    pub fn fen(&self) -> &str {
        self.state["fen"].as_str().unwrap_or_default()
    }

    /// `playing`, `solved` or `failed`.
    pub fn status(&self) -> &str {
        self.state["status"].as_str().unwrap_or("playing")
    }

    /// The index in the solution of the move the player must make next.
    pub fn index(&self) -> usize {
        super::whole(&self.state["index"]) as usize
    }

    pub fn player_color(&self) -> shakmaty::Color {
        match rules("playerColor", std::slice::from_ref(&self.puzzle)) {
            Ok(color) if color == "black" => shakmaty::Color::Black,
            _ => shakmaty::Color::White,
        }
    }

    /// The last move's squares.
    pub fn last_move(&self) -> Option<(shakmaty::Square, shakmaty::Square)> {
        let squares = self.state["lastMove"].as_array()?;
        Some((
            squares.first()?.as_str()?.parse().ok()?,
            squares.get(1)?.as_str()?.parse().ok()?,
        ))
    }

    /// The SAN of the last move played.
    pub fn last_san(&self) -> String {
        self.state["sans"]
            .as_array()
            .and_then(|sans| sans.last())
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// The player's move: whether it was the solution's, and the opponent's reply to play next.
    pub fn player_move(&mut self, uci: &str) -> (bool, Option<String>) {
        let Ok(result) = rules(
            "playerMove",
            &[self.puzzle.clone(), self.state.clone(), json!(uci)],
        ) else {
            return (false, None);
        };
        let correct = result["correct"].as_bool() == Some(true);
        if correct {
            self.state = result["state"].clone();
        }
        (correct, result["reply"].as_str().map(str::to_string))
    }

    pub fn opponent_reply(&mut self, uci: &str) {
        if let Ok(state) = rules("opponentReply", &[self.state.clone(), json!(uci)]) {
            self.state = state;
        }
    }

    /// The next solution move's squares (the hint, and the solution's arrow).
    pub fn next_squares(&self) -> Option<(shakmaty::Square, shakmaty::Square)> {
        let squares = rules(
            "nextSolutionSquares",
            &[self.puzzle.clone(), self.state.clone()],
        )
        .ok()?;
        let squares = squares.as_array()?;
        Some((
            squares.first()?.as_str()?.parse().ok()?,
            squares.get(1)?.as_str()?.parse().ok()?,
        ))
    }

    /// The solution's move at `index`.
    pub fn solution(&self, index: usize) -> Option<String> {
        self.puzzle["solution"][index].as_str().map(str::to_string)
    }

    /// Player moves in the solution.
    pub fn player_moves(&self) -> usize {
        self.puzzle["solution"]
            .as_array()
            .map_or(0, Vec::len)
            .div_ceil(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn puzzle() -> Value {
        // White mates with Qxf7# after Black's Nf6? (the opponent's move sets the puzzle).
        json!({
            "id": "test",
            "fen": "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
            "solution": ["h5f7"],
            "rating": 600,
            "themes": ["mateIn1"],
            "plays": 1
        })
    }

    #[test]
    fn the_solution_is_checked_by_the_rules() {
        let mut board = OnBoard::start(&puzzle());
        assert_eq!(board.status(), "playing");
        assert_eq!(board.player_color(), shakmaty::Color::White);
        assert_eq!(
            board.next_squares().map(|(from, _)| from.to_string()),
            Some("h5".into())
        );
        assert!(!board.player_move("c4f7").0);
        assert_eq!(board.status(), "playing");
        assert!(board.player_move("h5f7").0);
        assert_eq!(board.status(), "solved");
        assert_eq!(board.last_san(), "Qxf7#");
    }

    /// The rules write the solution index as a JavaScript number (`2.0`): read as an integer it
    /// was 0, and Solution stopped after the opponent's first reply.
    #[test]
    fn the_solution_index_moves_on_after_a_reply() {
        let mut board = OnBoard::start(&json!({
            "id": "line",
            "fen": crate::game::STANDARD_FEN,
            "solution": ["e2e4", "e7e5", "g1f3"],
            "rating": 1000,
            "themes": [],
        }));
        let (correct, reply) = board.player_move("e2e4");
        assert!(correct);
        assert_eq!(reply.as_deref(), Some("e7e5"));
        board.opponent_reply("e7e5");
        assert_eq!(board.index(), 2);
        assert_eq!(board.solution(board.index()).as_deref(), Some("g1f3"));
    }

    #[test]
    fn a_session_starts_idle_and_asks_for_a_puzzle() {
        let mut session = PuzzleSession::new();
        assert_eq!(session.phase(), "idle");
        let step = session.apply(
            "puzzleSessionLoadBegin",
            json!({
                "selection": { "account": "", "angle": "mix", "difficulty": "normal", "color": "random", "mode": "practice" },
                "online": true,
            }),
        );
        assert_eq!(session.phase(), "loading");
        assert_eq!(step.call.map(|call| call.method), Some("puzzleNext".into()));
    }
}
