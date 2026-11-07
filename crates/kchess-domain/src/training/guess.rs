//! Guess the move: a game is played through and one side's moves are guessed. The engine looks at
//! the move played as well as the guess: the move played scores full points when it holds up by
//! today's engines, and a guess as good as the engine's best scores full points too when the
//! player went wrong there. Otherwise a guess scores by how much worse the engine finds it than
//! its best, so a guess about as good as the best still scores.

use serde::{Deserialize, Serialize};

use crate::review::js_exp;

/// Points for guessing the move played, or a better one where it went wrong.
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

/// The share of the winning chances (from -1 to 1) a move gives away to be each flaw, worst
/// first: Lichess's analysis's thresholds.
const FLAWS: [(f64, Flaw); 3] = [
    (0.3, Flaw::Blunder),
    (0.2, Flaw::Mistake),
    (0.1, Flaw::Inaccuracy),
];

/// The least flaw that costs the move played its full points and lets a better guess have them:
/// an inaccuracy is marked, but separate searches differ by about that much.
const WENT_WRONG: Flaw = Flaw::Mistake;

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

    /// The side's winning chances, from -1 (lost) to 1 (won), on Lichess's curve.
    fn chances(self) -> f64 {
        let curve = |cp: f64| 2. / (1. + js_exp(-0.003_682_08 * cp)) - 1.;
        match self {
            // Mate 0 is the side to move already mated.
            Eval::Mate(0) => -1.,
            Eval::Mate(moves) => {
                let cp = f64::from(21 - moves.abs().min(10)) * 100.;
                curve(if moves > 0 { cp } else { -cp })
            }
            Eval::Cp(cp) => curve(f64::from(cp.clamp(-CLAMP_CP, CLAMP_CP))),
        }
    }
}

/// What the engine made of one move of the game, each from the guessing side's point of view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Evals {
    /// The position before the move, with the engine's best move.
    pub best: Option<Eval>,
    /// After the guess.
    pub guessed: Option<Eval>,
    /// After the move played.
    pub played: Option<Eval>,
}

/// How far wrong a move went, by today's engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Flaw {
    Inaccuracy,
    Mistake,
    Blunder,
}

impl Flaw {
    /// Its sign after the move: ?!, ? or ??.
    pub fn glyph(self) -> &'static str {
        match self {
            Flaw::Inaccuracy => "?!",
            Flaw::Mistake => "?",
            Flaw::Blunder => "??",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Flaw::Inaccuracy => "An inaccuracy",
            Flaw::Mistake => "A mistake",
            Flaw::Blunder => "A blunder",
        }
    }

    /// Whether the move went wrong enough to score against it: a mistake or a blunder.
    pub fn went_wrong(self) -> bool {
        self >= WENT_WRONG
    }
}

/// The flaw in a move that leaves the position at `after`, where the best move keeps `best`.
pub fn flaw(best: Eval, after: Eval) -> Option<Flaw> {
    let lost = best.chances() - after.chances();
    FLAWS
        .iter()
        .find(|(most, _)| lost >= *most)
        .map(|(_, flaw)| *flaw)
}

/// How good a guess was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Grade {
    /// The move played, and it holds up (an inaccuracy at worst).
    Master,
    /// The move played, though it was a mistake or a blunder: it scores what it was worth.
    Played,
    /// As good as the engine's best where the move played went wrong.
    Better,
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
    /// What was wrong with the move played, when the engine found it flawed.
    pub flaw: Option<Flaw>,
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

/// Centipawns lost by a move leaving `after` against the best move's `best`. A move the engine
/// finds better than its own best (a shallower search) lost nothing.
fn loss(best: Eval, after: Eval) -> u32 {
    (best.centipawns() - after.centipawns())
        .max(0)
        .unsigned_abs()
}

/// The grade and points a move losing `loss` centipawns earns.
fn banded(loss: u32) -> (Grade, u32) {
    BANDS
        .iter()
        .find(|(most, _, _)| loss <= *most)
        .map_or((Grade::Miss, 0), |(_, grade, points)| (*grade, *points))
}

/// Score `guess` against the move `played` and what the engine made of both. Without the
/// engine only the move played scores, in full.
pub fn judge(guess: &str, played: &str, evals: Evals) -> Judged {
    let flaw = evals
        .best
        .zip(evals.played)
        .and_then(|(best, after)| flaw(best, after));
    if same_move(guess, played) {
        let lost = evals
            .best
            .zip(evals.played)
            .map(|(best, after)| loss(best, after));
        return match flaw.filter(|flaw| flaw.went_wrong()) {
            None => Judged {
                grade: Grade::Master,
                points: FULL_POINTS,
                loss: lost.or(Some(0)),
                flaw,
            },
            Some(_) => Judged {
                grade: Grade::Played,
                points: lost.map_or(0, |lost| banded(lost).1),
                loss: lost,
                flaw,
            },
        };
    }
    let Some((best, guessed)) = evals.best.zip(evals.guessed) else {
        return Judged {
            grade: Grade::Miss,
            points: 0,
            loss: None,
            flaw,
        };
    };
    let lost = loss(best, guessed);
    let (grade, points) = match banded(lost) {
        (Grade::Best, _) if flaw.is_some_and(Flaw::went_wrong) => (Grade::Better, FULL_POINTS),
        banded => banded,
    };
    Judged {
        grade,
        points,
        loss: Some(lost),
        flaw,
    }
}

/// How close the guesses came over a game.
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
            Closeness::Master => "Spot on",
            Closeness::Close => "Close",
            Closeness::Fair => "Some way off",
            Closeness::Far => "Far off",
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
    /// Guesses as good as the engine's best where the move played went wrong.
    pub better: u32,
    /// Moves guessed at that were mistakes or blunders.
    pub flawed: u32,
    pub guesses: u32,
    /// Points as a share of the most, rounded to a whole percent.
    pub percent: u32,
    pub closeness: Closeness,
}

/// Add up a game's guesses.
pub fn summarize(guesses: &[Judged]) -> Summary {
    let count = |keep: fn(&Judged) -> bool| {
        u32::try_from(guesses.iter().filter(|guess| keep(guess)).count()).unwrap_or(u32::MAX)
    };
    let guessed = u32::try_from(guesses.len()).unwrap_or(u32::MAX);
    let points = guesses.iter().map(|guess| guess.points).sum::<u32>();
    let most = guessed.saturating_mul(FULL_POINTS);
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
        matched: count(|guess| matches!(guess.grade, Grade::Master | Grade::Played)),
        better: count(|guess| guess.grade == Grade::Better),
        flawed: count(|guess| guess.flaw.is_some_and(Flaw::went_wrong)),
        guesses: guessed,
        percent,
        closeness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine's best and the guess's evaluation, the move played not looked at.
    fn engine(best: Eval, guessed: Eval) -> Evals {
        Evals {
            best: Some(best),
            guessed: Some(guessed),
            played: None,
        }
    }

    #[test]
    fn the_move_played_scores_full_points_without_an_engine() {
        let judged = judge("g1f3", "g1f3", Evals::default());
        assert_eq!(judged.grade, Grade::Master);
        assert_eq!(judged.points, FULL_POINTS);
        assert_eq!(judged.flaw, None);
    }

    #[test]
    fn castling_either_way_is_the_move_played() {
        assert_eq!(judge("e1h1", "e1g1", Evals::default()).grade, Grade::Master);
        assert_eq!(judge("e8c8", "e8a8", Evals::default()).grade, Grade::Master);
        assert!(!same_move("e1g1", "e1c1"));
    }

    #[test]
    fn another_move_scores_nothing_without_an_engine() {
        let judged = judge("b1c3", "g1f3", Evals::default());
        assert_eq!(judged.grade, Grade::Miss);
        assert_eq!((judged.points, judged.loss), (0, None));
        let best_only = Evals {
            best: Some(Eval::Cp(30)),
            ..Evals::default()
        };
        assert_eq!(judge("b1c3", "g1f3", best_only).points, 0);
    }

    #[test]
    fn a_guess_near_the_best_move_still_scores() {
        let best = Eval::Cp(40);
        let cases = [
            (Eval::Cp(40), Grade::Best, 4, 0),
            (Eval::Cp(20), Grade::Best, 4, 20),
            (Eval::Cp(19), Grade::Good, 2, 21),
            (Eval::Cp(-10), Grade::Good, 2, 50),
            (Eval::Cp(-60), Grade::Fair, 1, 100),
            (Eval::Cp(-61), Grade::Miss, 0, 101),
        ];
        for (guessed, grade, points, loss) in cases {
            let judged = judge("b1c3", "g1f3", engine(best, guessed));
            assert_eq!(
                (judged.grade, judged.points, judged.loss),
                (grade, points, Some(loss)),
                "{guessed:?}"
            );
        }
    }

    #[test]
    fn a_guess_the_engine_likes_more_than_its_best_lost_nothing() {
        let judged = judge("b1c3", "g1f3", engine(Eval::Cp(10), Eval::Cp(35)));
        assert_eq!((judged.grade, judged.loss), (Grade::Best, Some(0)));
    }

    #[test]
    fn any_move_that_keeps_a_won_game_won_is_good() {
        // +15 against +12: both clamp to a clear win.
        let judged = judge("a1a2", "h1h2", engine(Eval::Cp(1500), Eval::Cp(1200)));
        assert_eq!(judged.grade, Grade::Best);
        // A slower mate is still a mate.
        let judged = judge("a1a2", "h1h2", engine(Eval::Mate(2), Eval::Mate(5)));
        assert_eq!(judged.grade, Grade::Best);
    }

    #[test]
    fn missing_a_mate_or_walking_into_one_misses() {
        let judged = judge("a1a2", "h1h2", engine(Eval::Mate(1), Eval::Cp(0)));
        assert_eq!((judged.grade, judged.loss), (Grade::Miss, Some(1000)));
        let judged = judge("a1a2", "h1h2", engine(Eval::Cp(0), Eval::Mate(-3)));
        assert_eq!((judged.grade, judged.loss), (Grade::Miss, Some(1000)));
    }

    #[test]
    fn evaluations_turn_to_the_side_that_moves() {
        assert_eq!(Eval::Cp(31).for_side(false), Eval::Cp(-31));
        assert_eq!(Eval::Mate(-2).for_side(false), Eval::Mate(2));
        assert_eq!(Eval::Cp(31).for_side(true), Eval::Cp(31));
    }

    #[test]
    fn flaws_follow_the_winning_chances_given_away() {
        assert_eq!(flaw(Eval::Cp(30), Eval::Cp(10)), None);
        assert_eq!(flaw(Eval::Cp(30), Eval::Cp(-30)), Some(Flaw::Inaccuracy));
        assert_eq!(flaw(Eval::Cp(30), Eval::Cp(-90)), Some(Flaw::Mistake));
        assert_eq!(flaw(Eval::Cp(30), Eval::Cp(-300)), Some(Flaw::Blunder));
        // Going from +9 to +8 changes nothing that matters.
        assert_eq!(flaw(Eval::Cp(900), Eval::Cp(800)), None);
        // Letting a mate slip, or walking into one, is a blunder.
        assert_eq!(flaw(Eval::Mate(2), Eval::Cp(0)), Some(Flaw::Blunder));
        assert_eq!(flaw(Eval::Cp(0), Eval::Mate(-1)), Some(Flaw::Blunder));
        assert_eq!(Flaw::Mistake.glyph(), "?");
    }

    #[test]
    fn a_sound_move_played_scores_in_full() {
        let evals = Evals {
            best: Some(Eval::Cp(40)),
            guessed: None,
            played: Some(Eval::Cp(25)),
        };
        let judged = judge("g1f3", "g1f3", evals);
        assert_eq!(
            judged,
            Judged {
                grade: Grade::Master,
                points: FULL_POINTS,
                loss: Some(15),
                flaw: None
            }
        );
    }

    /// The player blundered: guessing their move is no longer the right answer, and the
    /// engine's move is.
    #[test]
    fn a_blunder_played_scores_what_it_was_worth_and_the_better_move_scores_in_full() {
        let evals = Evals {
            best: Some(Eval::Cp(40)),
            guessed: Some(Eval::Cp(35)),
            played: Some(Eval::Cp(-300)),
        };
        let matched = judge("d1h5", "d1h5", evals);
        assert_eq!(
            (matched.grade, matched.points, matched.flaw),
            (Grade::Played, 0, Some(Flaw::Blunder))
        );
        let better = judge("g1f3", "d1h5", evals);
        assert_eq!(
            (better.grade, better.points, better.flaw),
            (Grade::Better, FULL_POINTS, Some(Flaw::Blunder))
        );
        // A move only good, not best, is graded as ever.
        let good = judge(
            "b1c3",
            "d1h5",
            Evals {
                guessed: Some(Eval::Cp(0)),
                ..evals
            },
        );
        assert_eq!((good.grade, good.points), (Grade::Good, 2));
    }

    /// An inaccuracy is marked, but within what separate searches differ by: the move played
    /// still scores in full, and a guess matching the engine is only best.
    #[test]
    fn an_inaccuracy_played_is_marked_but_still_the_answer() {
        let evals = Evals {
            best: Some(Eval::Cp(40)),
            guessed: Some(Eval::Cp(40)),
            played: Some(Eval::Cp(-20)),
        };
        let matched = judge("a2a3", "a2a3", evals);
        assert_eq!(
            (matched.grade, matched.points, matched.loss, matched.flaw),
            (Grade::Master, FULL_POINTS, Some(60), Some(Flaw::Inaccuracy))
        );
        let best = judge("g1f3", "a2a3", evals);
        assert_eq!((best.grade, best.points), (Grade::Best, 4));
        assert_eq!(summarize(&[matched]).flawed, 0);
    }

    #[test]
    fn a_mistake_played_scores_its_band() {
        let evals = Evals {
            best: Some(Eval::Cp(40)),
            guessed: None,
            played: Some(Eval::Cp(-100)),
        };
        let matched = judge("a2a3", "a2a3", evals);
        assert_eq!(
            (matched.grade, matched.points, matched.loss, matched.flaw),
            (Grade::Played, 0, Some(140), Some(Flaw::Mistake))
        );
    }

    #[test]
    fn a_game_adds_up_to_how_close_it_came() {
        assert_eq!(summarize(&[]).closeness, Closeness::None);
        let master = judge("e2e4", "e2e4", Evals::default());
        let best = judge("d2d4", "e2e4", engine(Eval::Cp(30), Eval::Cp(25)));
        let miss = judge("a2a4", "e2e4", engine(Eval::Cp(30), Eval::Cp(-200)));
        let flawed = Evals {
            best: Some(Eval::Cp(30)),
            guessed: Some(Eval::Cp(30)),
            played: Some(Eval::Cp(-400)),
        };
        let better = judge("g1f3", "f2f3", flawed);
        let summary = summarize(&[master, best, miss, master, better]);
        assert_eq!(
            summary,
            Summary {
                points: 19,
                most: 25,
                matched: 2,
                better: 1,
                flawed: 1,
                guesses: 5,
                percent: 76,
                closeness: Closeness::Close,
            }
        );
        assert_eq!(summarize(&[master; 3]).closeness, Closeness::Master);
        assert_eq!(summarize(&[miss, best]).closeness, Closeness::Fair);
        assert_eq!(summarize(&[miss]).closeness, Closeness::Far);
        assert_eq!(summary.closeness.label(), "Close");
    }
}
