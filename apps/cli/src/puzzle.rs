//! `puzzle [id|link]`: solve Lichess's daily puzzle, or the one named, by typing moves, as on the
//! Puzzles page.

use kchess_contracts::calls;
use kchess_domain::position::{play_san, play_uci};
use kchess_domain::rules::setup_start;
use kchess_games::puzzle::OnBoard;
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::core::{ClientEvent, CoreClient};

/// A puzzle being solved, on the board as the Puzzles page plays it
/// (`kchess_games::puzzle::OnBoard`): the rules decide whether a move is the solution's (any
/// checkmate is), and play the opponent's replies.
pub struct Solving {
    board: OnBoard,
}

/// What a typed move did.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// Not a legal move here.
    Illegal,
    /// Legal, but not the solution; the position stays as it was.
    Wrong { san: String },
    /// The right move, and the opponent's reply when the puzzle goes on.
    Right {
        san: String,
        reply: Option<String>,
        solved: bool,
    },
}

impl Solving {
    /// The puzzle, unless the rules cannot set it up or it has no solution.
    pub fn new(puzzle: &Value) -> Option<Solving> {
        let board = OnBoard::start(puzzle);
        (!board.fen().is_empty() && board.player_moves() > 0).then_some(Solving { board })
    }

    pub fn solved(&self) -> bool {
        self.board.status() == "solved"
    }

    /// The position now, as FEN.
    pub fn fen(&self) -> String {
        self.board.fen().to_string()
    }

    /// The side the solver plays.
    pub fn solver(&self) -> &'static str {
        if self.board.player_color().is_white() {
            "white"
        } else {
            "black"
        }
    }

    /// Play `text` (SAN or UCI). As on Lichess, any checkmate solves the puzzle, not only the
    /// one in its solution.
    pub fn answer(&mut self, text: &str) -> Answer {
        if self.board.status() != "playing" {
            return Answer::Illegal;
        }
        let Some(position) = setup_start(self.board.variant(), self.board.fen()) else {
            return Answer::Illegal;
        };
        let Some(played) =
            play_uci(&position, &text.to_lowercase()).or_else(|| play_san(&position, text))
        else {
            return Answer::Illegal;
        };
        let (correct, reply) = self.board.player_move(&played.uci);
        if !correct {
            return Answer::Wrong { san: played.san };
        }
        let reply = reply.map(|uci| {
            self.board.opponent_reply(&uci);
            self.board.last_san()
        });
        Answer::Right {
            san: played.san,
            reply,
            solved: self.solved(),
        }
    }

    /// The square of the piece to move next, as the Puzzles page's hint shows it.
    pub fn hint(&self) -> Option<String> {
        if self.solved() {
            return None;
        }
        self.board.next_squares().map(|(from, _)| from.to_string())
    }

    /// The moves left, in SAN; the puzzle ends shown.
    pub fn give_up(&mut self) -> Vec<String> {
        let mut shown = Vec::new();
        while self.board.status() == "playing" {
            let Some(uci) = self.board.solution(self.board.index()) else {
                break;
            };
            let (correct, reply) = self.board.player_move(&uci);
            if !correct {
                break;
            }
            shown.push(self.board.last_san());
            if let Some(reply) = reply {
                self.board.opponent_reply(&reply);
                shown.push(self.board.last_san());
            }
        }
        shown
    }
}

/// The puzzle id in a word typed after `puzzle`: an id, or a lichess.org/training link.
pub fn named(word: &str) -> String {
    kchess_domain::links::training_link(word).unwrap_or_else(|| word.trim().to_string())
}

pub async fn interactive(
    core: &CoreClient,
    events: &mut UnboundedReceiver<ClientEvent>,
    id: Option<&str>,
) -> Result<(), String> {
    let puzzle = match id {
        Some(word) => {
            core.request(calls::PuzzleById {
                id: json!(named(word)),
            })
            .await?
        }
        None => core.request(calls::PuzzleDaily {}).await?,
    };
    let mut solving =
        Solving::new(&puzzle).ok_or("Lichess sent a puzzle KChess could not read.")?;
    println!(
        "{}",
        json!({
            "puzzle": puzzle["id"],
            "rating": puzzle["rating"],
            "themes": puzzle["themes"],
            "fen": solving.fen(),
            "toMove": solving.solver(),
        })
    );
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    loop {
        tokio::select! {
            _ = &mut interrupted => break,
            Some(event) = events.recv() => {
                if event == ClientEvent::Closed {
                    break;
                }
            }
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let shown = match line.trim() {
                    "" => continue,
                    "quit" => break,
                    "hint" => json!({ "hint": solving.hint() }),
                    "solution" => {
                        let moves = solving.give_up();
                        json!({ "solution": moves, "fen": solving.fen() })
                    }
                    text => match solving.answer(text) {
                        Answer::Illegal => {
                            eprintln!("Illegal move.");
                            continue;
                        }
                        Answer::Wrong { san } => json!({ "move": san, "correct": false }),
                        Answer::Right { san, reply, solved } => json!({
                            "move": san,
                            "correct": true,
                            "reply": reply,
                            "solved": solved,
                            "fen": solving.fen(),
                        }),
                    },
                };
                println!("{shown}");
                if solving.solved() {
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Answer, Solving};
    use serde_json::json;

    /// Lichess's daily puzzle of 4 November 2026: a discovered check wins the queen.
    fn daily() -> Solving {
        Solving::new(&json!({
            "fen": "r1b4r/ppqpkpb1/2nNpnp1/7p/5P1P/4BNP1/PPP5/R2QKB1R w KQ - 1 1",
            "solution": ["e3c5", "c7a5", "b2b4", "c6b4", "d6b7", "a5c5", "b7c5"],
        }))
        .expect("the puzzle reads")
    }

    #[test]
    fn the_solution_is_typed_in_san_or_uci_and_the_replies_are_played() {
        let mut solving = daily();
        assert_eq!(solving.solver(), "white");
        let start = "r1b4r/ppqpkpb1/2nNpnp1/7p/5P1P/4BNP1/PPP5/R2QKB1R w KQ - 1 1";
        assert_eq!(solving.fen(), start);
        assert_eq!(
            solving.answer("Bc5"),
            Answer::Right {
                san: "Bc5".into(),
                reply: Some("Qa5+".into()),
                solved: false,
            }
        );
        // The bishop moved and the queen answered.
        assert_eq!(
            solving.fen(),
            "r1b4r/pp1pkpb1/2nNpnp1/q1B4p/5P1P/5NP1/PPP5/R2QKB1R w KQ - 3 2"
        );
        assert_eq!(solving.answer("Bc5"), Answer::Illegal);
        assert_eq!(solving.answer("Qd2"), Answer::Wrong { san: "Qd2".into() });
        assert!(matches!(
            solving.answer("b2b4"),
            Answer::Right { solved: false, .. }
        ));
        assert!(matches!(
            solving.answer("Nxb7+"),
            Answer::Right { solved: false, .. }
        ));
        assert_eq!(
            solving.answer("Nxc5"),
            Answer::Right {
                san: "Nxc5".into(),
                reply: None,
                solved: true,
            }
        );
        assert!(solving.solved());
        assert_eq!(solving.answer("Kd2"), Answer::Illegal);
    }

    #[test]
    fn any_checkmate_solves_a_mate_puzzle() {
        // Mate in one with two mating moves: Ra8 and Rb8 both mate; the solution names one.
        let mut solving = Solving::new(&json!({
            "fen": "6k1/5ppp/8/8/8/8/8/RR4K1 w - - 0 1",
            "solution": ["a1a8"],
        }))
        .expect("the puzzle reads");
        assert!(matches!(
            solving.answer("Rb8#"),
            Answer::Right { solved: true, .. }
        ));
    }

    #[test]
    fn a_puzzle_is_named_by_its_id_or_link() {
        assert_eq!(super::named("FPFg0"), "FPFg0");
        assert_eq!(super::named("https://lichess.org/training/FPFg0"), "FPFg0");
        assert_eq!(super::named("lichess.org/training/FPFg0/?a#b"), "FPFg0");
    }

    #[test]
    fn a_hint_names_the_square_of_the_piece_to_move() {
        let mut solving = daily();
        assert_eq!(solving.hint().as_deref(), Some("e3"));
        solving.answer("Bc5");
        // After the reply, White's next move is the b-pawn's.
        assert_eq!(solving.hint().as_deref(), Some("b2"));
        solving.give_up();
        assert_eq!(solving.hint(), None);
    }

    #[test]
    fn giving_up_shows_the_moves_left() {
        let mut solving = daily();
        solving.answer("Bc5");
        assert_eq!(solving.give_up(), ["b4", "Nxb4", "Nxb7+", "Qxc5", "Nxc5"]);
        assert!(solving.solved());
    }
}
