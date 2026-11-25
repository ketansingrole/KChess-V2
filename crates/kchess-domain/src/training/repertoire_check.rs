//! A repertoire's move against the games of the databases: whether games played it, how often,
//! and whether it scored as well as the move most played instead.

/// A move's games and how they ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Counts {
    pub games: u64,
    pub white: u64,
    pub draws: u64,
    pub black: u64,
}

impl Counts {
    /// The share of the points won by the side that played the move, in its games with a result;
    /// `None` without one.
    pub fn score(&self, white_played: bool) -> Option<f64> {
        let decided = self.white + self.draws + self.black;
        let won = if white_played { self.white } else { self.black };
        (decided > 0).then(|| (won as f64 + self.draws as f64 / 2.) / decided as f64)
    }
}

/// What the games say of a repertoire's move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Played often enough, and scoring about as well as the main alternative; or too few games
    /// reached the position to say otherwise.
    Played,
    /// No game that reached the position played it.
    Unplayed,
    /// Played in under `RARE` of the games.
    Rare,
    /// It scored at least `WEAKER` below the most played other move.
    Weaker,
}

/// The fewest games a position needs before a move of it is judged.
pub const ENOUGH: u64 = 20;
/// The share of a position's games under which a move is rare.
pub const RARE: f64 = 0.05;
/// The fewest games a move needs for its score to count.
pub const SCORED: u64 = 10;
/// How far below the main alternative's score a move's must be to be weaker.
pub const WEAKER: f64 = 0.08;

/// The verdict on a move with `yours` of a position's `total` games, played by White when
/// `white`, beside `main`, the most played of the others.
pub fn verdict(yours: Counts, main: Option<Counts>, total: u64, white: bool) -> Verdict {
    if total < ENOUGH {
        return Verdict::Played;
    }
    if yours.games == 0 {
        return Verdict::Unplayed;
    }
    if (yours.games as f64) < RARE * total as f64 {
        return Verdict::Rare;
    }
    let weaker = main
        .filter(|main| main.games >= SCORED && yours.games >= SCORED)
        .and_then(|main| Some((yours.score(white)?, main.score(white)?)))
        .is_some_and(|(mine, theirs)| mine + WEAKER <= theirs);
    if weaker {
        Verdict::Weaker
    } else {
        Verdict::Played
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(games: u64, white: u64, draws: u64) -> Counts {
        Counts {
            games,
            white,
            draws,
            black: games - white - draws,
        }
    }

    #[test]
    fn a_move_is_judged_once_enough_games_reached_its_position() {
        let none = Counts::default();
        assert_eq!(verdict(none, None, ENOUGH - 1, true), Verdict::Played);
        assert_eq!(verdict(none, None, ENOUGH, true), Verdict::Unplayed);
    }

    #[test]
    fn a_move_few_games_played_is_rare() {
        assert_eq!(verdict(counts(4, 2, 2), None, 100, true), Verdict::Rare);
        assert_eq!(verdict(counts(5, 2, 2), None, 100, true), Verdict::Played);
    }

    #[test]
    fn a_move_scoring_well_below_the_main_one_is_weaker() {
        // White's move scores 40% (8 wins of 20, no draws), the main one 50%.
        let main = counts(80, 40, 0);
        assert_eq!(
            verdict(counts(20, 8, 0), Some(main), 100, true),
            Verdict::Weaker
        );
        // 45% is close enough.
        assert_eq!(
            verdict(counts(20, 9, 0), Some(main), 100, true),
            Verdict::Played
        );
        // For Black the same games read the other way: 60% against 50%.
        assert_eq!(
            verdict(counts(20, 8, 0), Some(main), 100, false),
            Verdict::Played
        );
        // Too few games of the main move to compare.
        assert_eq!(
            verdict(counts(20, 8, 0), Some(counts(9, 9, 0)), 100, true),
            Verdict::Played
        );
    }

    #[test]
    fn a_score_counts_the_games_with_a_result() {
        let unfinished = Counts {
            games: 3,
            ..Counts::default()
        };
        assert_eq!(unfinished.score(true), None);
        assert_eq!(counts(4, 1, 2).score(true), Some(0.5));
        assert_eq!(counts(4, 1, 2).score(false), Some(0.5));
        assert_eq!(counts(4, 3, 1).score(false), Some(0.125));
    }
}
