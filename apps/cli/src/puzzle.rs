//! `puzzle [id|link]`: solve Lichess's daily puzzle, or the one named, by typing moves, as on the
//! Puzzles page.

use kchess_domain::position::{Played, play_san, play_uci};
use kchess_domain::rules::setup_start;
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::core::{CoreClient, Event};

/// A puzzle being solved: its position, its solution (UCI, the solver's moves first) and the moves
/// found so far, the opponent's replies included.
pub struct Solving {
    fen: String,
    solution: Vec<String>,
    played: Vec<String>,
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
    pub fn new(puzzle: &Value) -> Option<Solving> {
        let fen = puzzle["fen"].as_str()?.to_string();
        let solution: Vec<String> = puzzle["solution"]
            .as_array()?
            .iter()
            .filter_map(|uci| uci.as_str().map(str::to_string))
            .collect();
        setup_start("standard", &fen)?;
        (!solution.is_empty()).then_some(Solving {
            fen,
            solution,
            played: Vec::new(),
        })
    }

    pub fn solved(&self) -> bool {
        self.played.len() >= self.solution.len()
    }

    /// The position now, as FEN.
    pub fn fen(&self) -> String {
        self.after(&self.played)
            .map_or_else(|| self.fen.clone(), |played| played.position.fen)
    }

    /// The side the solver plays.
    pub fn solver(&self) -> &'static str {
        if self.fen.split(' ').nth(1) == Some("b") {
            "black"
        } else {
            "white"
        }
    }

    /// The last of `moves` played from the start, or `None` before the first.
    fn after(&self, moves: &[String]) -> Option<Played> {
        let mut position = setup_start("standard", &self.fen)?;
        let mut last = None;
        for uci in moves {
            let played = play_uci(&position, uci)?;
            position = setup_start("standard", &played.position.fen)?;
            last = Some(played);
        }
        last
    }

    /// Play `text` (SAN or UCI). As on Lichess, any checkmate solves the puzzle, not only the
    /// one in its solution.
    pub fn answer(&mut self, text: &str) -> Answer {
        let Some(position) = setup_start("standard", &self.fen()) else {
            return Answer::Illegal;
        };
        let Some(played) =
            play_uci(&position, &text.to_lowercase()).or_else(|| play_san(&position, text))
        else {
            return Answer::Illegal;
        };
        let expected = &self.solution[self.played.len()];
        if played.uci != *expected && !played.position.checkmate {
            return Answer::Wrong { san: played.san };
        }
        if played.position.checkmate {
            self.played = self.solution.clone();
            return Answer::Right {
                san: played.san,
                reply: None,
                solved: true,
            };
        }
        self.played.push(expected.clone());
        let reply = self
            .solution
            .get(self.played.len())
            .cloned()
            .and_then(|uci| {
                let reply = play_uci(&setup_start("standard", &played.position.fen)?, &uci)?;
                self.played.push(uci);
                Some(reply.san)
            });
        Answer::Right {
            san: played.san,
            reply,
            solved: self.solved(),
        }
    }

    /// The moves left, in SAN; the puzzle ends shown.
    pub fn give_up(&mut self) -> Vec<String> {
        let mut shown = Vec::new();
        while !self.solved() {
            let Some(played) = setup_start("standard", &self.fen())
                .and_then(|position| play_uci(&position, &self.solution[self.played.len()]))
            else {
                break;
            };
            shown.push(played.san);
            self.played.push(self.solution[self.played.len()].clone());
        }
        shown
    }
}

/// The puzzle id in a word typed after `puzzle`: an id, or a lichess.org/training link.
pub fn named(word: &str) -> String {
    let id = word.split("/training/").last().unwrap_or(word);
    let id = id.split(['?', '#', '/']).next().unwrap_or(id);
    id.to_string()
}

pub async fn interactive(
    core: &CoreClient,
    events: &mut UnboundedReceiver<Event>,
    id: Option<&str>,
) -> Result<(), String> {
    let puzzle = match id {
        Some(word) => core.call("puzzleById", vec![json!(named(word))]).await?,
        None => core.call("puzzleDaily", vec![]).await?,
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
                if event.name == "host:request" {
                    core.answer(&event.payload).await;
                } else if event.name == "core:closed" {
                    break;
                }
            }
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let shown = match line.trim() {
                    "" => continue,
                    "quit" => break,
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
        assert_eq!(
            solving.answer("Bc5"),
            Answer::Right {
                san: "Bc5".into(),
                reply: Some("Qa5+".into()),
                solved: false,
            }
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
    fn giving_up_shows_the_moves_left() {
        let mut solving = daily();
        solving.answer("Bc5");
        assert_eq!(solving.give_up(), ["b4", "Nxb4", "Nxb7+", "Qxc5", "Nxc5"]);
        assert!(solving.solved());
    }
}
