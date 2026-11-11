//! Puzzle training: the session is the rules' (`kchess_domain::trainer::puzzle_session`), which
//! hand the page the host call each step asks for (a puzzle to fetch, a result to send) and the
//! attempt to log. A single puzzle on the board is `OnBoard`, moved on by the rules too.

pub use kchess_domain::trainer::puzzle_session::{PuzzleSession, Selection, Step};
pub use kchess_games::puzzle::OnBoard;

/// `localDay`: the local calendar day, `YYYY-MM-DD`, which the day streak counts.
pub fn local_day() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_starts_idle_and_asks_for_a_puzzle() {
        let mut session = PuzzleSession::new();
        assert_eq!(session.phase(), "idle");
        let selection = Selection {
            account: String::new(),
            angle: "mix".into(),
            difficulty: "normal".into(),
            color: "random".into(),
            mode: "practice".into(),
        };
        let step = session.load_begin(&selection, true, &local_day());
        assert_eq!(session.phase(), "loading");
        assert!(matches!(
            step.call.map(|call| call.request),
            Some(kchess_domain::trainer::puzzle_session::Request::PuzzleNext(
                _
            ))
        ));
    }
}
