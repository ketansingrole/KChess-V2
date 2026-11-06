//! Guess the move: a game is played through and one side's moves are guessed. A guess scores full
//! points when it is the move played, and otherwise by how much worse the engine finds it than its
//! own best move, so a guess about as good as the best still scores.

use serde::{Deserialize, Serialize};

/// Points for guessing the move played.
pub const FULL_POINTS: u32 = 5;

/// Evaluations past this are clamped: in a won position, any move that keeps it won is fine.
const CLAMP_CP: i32 = 1000;
/// A mate counts as this, less the moves it takes, before clamping.
const MATE_CP: i32 = 100_000;

/// How a guess compares with the best move, in centipawns lost: the most a grade allows, and
/// the points it is worth.
const BANDS: [(u32, Grade, u32); 3] = [
    (20, Grade::Best, 4),
    (50, Grade::Good, 2),
    (100, Grade::Fair, 1),
];

/// An engine evaluation from the point of view of the side that moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Eval {
    Cp(i32),
    /// Mate in this many moves: positive when the side moving mates, negative when it is mated.
    Mate(i32),
}

impl Eval {
    /// The evaluation from White's side (the engine's lines are), turned to `white`'s side.
    pub fn for_side(self, white: bool) -> Eval {
        if white {
            return self;
        }
        match self {
            Eval::Cp(cp) => Eval::Cp(-cp),
            Eval::Mate(moves) => Eval::Mate(-moves),
        }
    }

    /// In centipawns, clamped to what still matters.
    fn centipawns(self) -> i32 {
        let raw = match self {
            Eval::Cp(cp) => cp,
            // Mate 0 is the side to move already mated.
            Eval::Mate(moves) if moves <= 0 => -MATE_CP - moves,
            Eval::Mate(moves) => MATE_CP - moves,
        };
        raw.clamp(-CLAMP_CP, CLAMP_CP)
    }
}

/// How good a guess was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Grade {
    /// The move played.
    Master,
    /// About as good as the engine's best move.
    Best,
    Good,
    Fair,
    Miss,
}

/// A scored guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Judged {
    pub grade: Grade,
    pub points: u32,
    /// Centipawns lost against the engine's best move, when the engine judged it.
    pub loss: Option<u32>,
}

/// Whether two UCI moves are the same move: castling may be written king-to-rook (`e1h1`) or
/// king-to-destination (`e1g1`).
pub fn same_move(a: &str, b: &str) -> bool {
    fn king_move(uci: &str) -> &str {
        match uci {
            "e1h1" => "e1g1",
            "e1a1" => "e1c1",
            "e8h8" => "e8g8",
            "e8a8" => "e8c8",
            other => other,
        }
    }
    king_move(a) == king_move(b)
}

/// Score `guess` against the move `played`, and failing that against the engine: `best` is the
/// position's evaluation with the engine's best move, `guessed` its evaluation after the guess,
/// both from the guessing side's point of view. Without them only the move played scores.
pub fn judge(guess: &str, played: &str, best: Option<Eval>, guessed: Option<Eval>) -> Judged {
    if same_move(guess, played) {
        return Judged {
            grade: Grade::Master,
            points: FULL_POINTS,
            loss: Some(0),
        };
    }
    let Some((best, guessed)) = best.zip(guessed) else {
        return Judged {
            grade: Grade::Miss,
            points: 0,
            loss: None,
        };
    };
    // A guess the engine finds better than its own best (a shallower search) lost nothing.
    let loss = (best.centipawns() - guessed.centipawns()).max(0).unsigned_abs();
    let (grade, points) = BANDS
        .iter()
        .find(|(most, _, _)| loss <= *most)
        .map_or((Grade::Miss, 0), |(_, grade, points)| (*grade, *points));
    Judged {
        grade,
        points,
        loss: Some(loss),
    }
}

/// How close the guesses came to the master's play over a game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Closeness {
    /// Nothing guessed yet.
    None,
    Master,
    Close,
    Fair,
    Far,
}

impl Closeness {
    pub fn label(self) -> &'static str {
        match self {
            Closeness::None => "No moves guessed",
            Closeness::Master => "Played like the master",
            Closeness::Close => "Close to the master",
            Closeness::Fair => "Some way from the master",
            Closeness::Far => "Far from the master",
        }
    }
}

/// The total over a game's guesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub points: u32,
    /// What every guess scoring full points would have made.
    pub most: u32,
    /// Guesses that were the move played.
    pub matched: u32,
    pub guesses: u32,
    /// Points as a share of the most, rounded to a whole percent.
    pub percent: u32,
    pub closeness: Closeness,
}

/// Add up a game's guesses.
pub fn summarize(guesses: &[Judged]) -> Summary {
    let count = u32::try_from(guesses.len()).unwrap_or(u32::MAX);
    let points = guesses.iter().map(|guess| guess.points).sum::<u32>();
    let matched = guesses
        .iter()
        .filter(|guess| guess.grade == Grade::Master)
        .count();
    let most = count.saturating_mul(FULL_POINTS);
    let percent = if most == 0 {
        0
    } else {
        ((f64::from(points) * 100. / f64::from(most)).round()) as u32
    };
    let closeness = match percent {
        _ if most == 0 => Closeness::None,
        80.. => Closeness::Master,
        60.. => Closeness::Close,
        35.. => Closeness::Fair,
        _ => Closeness::Far,
    };
    Summary {
        points,
        most,
        matched: u32::try_from(matched).unwrap_or(u32::MAX),
        guesses: count,
        percent,
        closeness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_move_played_scores_full_points_without_an_engine() {
        let judged = judge("g1f3", "g1f3", None, None);
        assert_eq!(judged.grade, Grade::Master);
        assert_eq!(judged.points, FULL_POINTS);
    }

    #[test]
    fn castling_either_way_is_the_move_played() {
        assert_eq!(judge("e1h1", "e1g1", None, None).grade, Grade::Master);
        assert_eq!(judge("e8c8", "e8a8", None, None).grade, Grade::Master);
        assert!(!same_move("e1g1", "e1c1"));
    }

    #[test]
    fn another_move_scores_nothing_without_an_engine() {
        let judged = judge("b1c3", "g1f3", None, None);
        assert_eq!(judged.grade, Grade::Miss);
        assert_eq!((judged.points, judged.loss), (0, None));
        assert_eq!(judge("b1c3", "g1f3", Some(Eval::Cp(30)), None).points, 0);
    }

    #[test]
    fn a_guess_near_the_best_move_still_scores() {
        let best = Some(Eval::Cp(40));
        let cases = [
            (Eval::Cp(40), Grade::Best, 4, 0),
            (Eval::Cp(20), Grade::Best, 4, 20),
            (Eval::Cp(19), Grade::Good, 2, 21),
            (Eval::Cp(-10), Grade::Good, 2, 50),
            (Eval::Cp(-60), Grade::Fair, 1, 100),
            (Eval::Cp(-61), Grade::Miss, 0, 101),
        ];
        for (guessed, grade, points, loss) in cases {
            let judged = judge("b1c3", "g1f3", best, Some(guessed));
            assert_eq!(
                (judged.grade, judged.points, judged.loss),
                (grade, points, Some(loss)),
                "{guessed:?}"
            );
        }
    }

    #[test]
    fn a_guess_the_engine_likes_more_than_its_best_lost_nothing() {
        let judged = judge("b1c3", "g1f3", Some(Eval::Cp(10)), Some(Eval::Cp(35)));
        assert_eq!((judged.grade, judged.loss), (Grade::Best, Some(0)));
    }

    #[test]
    fn any_move_that_keeps_a_won_game_won_is_good() {
        // +15 against +12: both clamp to a clear win.
        let judged = judge("a1a2", "h1h2", Some(Eval::Cp(1500)), Some(Eval::Cp(1200)));
        assert_eq!(judged.grade, Grade::Best);
        // A slower mate is still a mate.
        let judged = judge("a1a2", "h1h2", Some(Eval::Mate(2)), Some(Eval::Mate(5)));
        assert_eq!(judged.grade, Grade::Best);
    }

    #[test]
    fn missing_a_mate_or_walking_into_one_misses() {
        let judged = judge("a1a2", "h1h2", Some(Eval::Mate(1)), Some(Eval::Cp(0)));
        assert_eq!((judged.grade, judged.loss), (Grade::Miss, Some(1000)));
        let judged = judge("a1a2", "h1h2", Some(Eval::Cp(0)), Some(Eval::Mate(-3)));
        assert_eq!((judged.grade, judged.loss), (Grade::Miss, Some(1000)));
    }

    #[test]
    fn evaluations_turn_to_the_side_that_moves() {
        assert_eq!(Eval::Cp(31).for_side(false), Eval::Cp(-31));
        assert_eq!(Eval::Mate(-2).for_side(false), Eval::Mate(2));
        assert_eq!(Eval::Cp(31).for_side(true), Eval::Cp(31));
    }

    #[test]
    fn a_game_adds_up_to_how_close_it_came() {
        assert_eq!(summarize(&[]).closeness, Closeness::None);
        let master = judge("e2e4", "e2e4", None, None);
        let best = judge("d2d4", "e2e4", Some(Eval::Cp(30)), Some(Eval::Cp(25)));
        let miss = judge("a2a4", "e2e4", Some(Eval::Cp(30)), Some(Eval::Cp(-200)));
        let summary = summarize(&[master, best, miss, master]);
        assert_eq!(
            summary,
            Summary {
                points: 14,
                most: 20,
                matched: 2,
                guesses: 4,
                percent: 70,
                closeness: Closeness::Close,
            }
        );
        assert_eq!(summarize(&[master; 3]).closeness, Closeness::Master);
        assert_eq!(summarize(&[miss, best]).closeness, Closeness::Fair);
        assert_eq!(summarize(&[miss]).closeness, Closeness::Far);
        assert_eq!(summary.closeness.label(), "Close to the master");
    }
}
